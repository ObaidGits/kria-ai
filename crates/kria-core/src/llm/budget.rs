//! Deterministic token and context budgeting infrastructure.
//!
//! # Design Principles
//! - Explicit ledger: every token consumed is tracked by category
//! - Provider-aware: budgets scale with the active provider's context window
//! - Deterministic: same inputs always produce same budget decisions
//! - Observable: all pressure transitions are logged
//! - Bounded: hard caps prevent runaway context growth
//!
//! # Architecture
//!
//! ```text
//! TurnTokenLedger          ContextBudgets
//!   ├── prompt_tokens         ├── context_window
//!   ├── completion_tokens     ├── system_reserve
//!   ├── tool_result_tokens    ├── response_reserve
//!   ├── retrieval_tokens      ├── tool_result_budget
//!   ├── system_tokens         ├── history_char_budget
//!   └── cumulative_total      └── history_item_char_cap
//! ```
//!
//! # Token Estimation Hierarchy
//! 1. Provider tokenizer API (llama.cpp `/tokenize`) — exact
//! 2. chars/4 heuristic — fallback when API unavailable
//!
//! The estimation method used is always logged.

use serde::Serialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ─── Estimation Method ───────────────────────────────────────────────────────

/// How a token count was estimated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EstimationMethod {
    /// Exact count from provider tokenizer API.
    Exact,
    /// Heuristic: chars / 4.
    Heuristic,
}

impl EstimationMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Heuristic => "heuristic",
        }
    }
}

/// Estimate tokens from text using the heuristic fallback (chars / 4).
/// Always available, never fails.
#[inline]
pub fn estimate_tokens_heuristic(text: &str) -> usize {
    text.chars().count() / 4
}

/// Estimate tokens from text, returning the count and the method used.
///
/// Uses the heuristic synchronously. For exact counts, use
/// `crate::llm::tokenize::count_tokens` (async).
pub fn estimate_tokens(text: &str) -> (usize, EstimationMethod) {
    (estimate_tokens_heuristic(text), EstimationMethod::Heuristic)
}

// ─── Context Budgets ─────────────────────────────────────────────────────────

/// Provider-aware context budget configuration.
///
/// All values are in tokens unless noted. Char budgets are derived from
/// token budgets using the heuristic (tokens × 4).
#[derive(Debug, Clone)]
pub struct ContextBudgets {
    /// Total context window for the active provider/model.
    pub context_window: usize,
    /// Tokens reserved for the system prompt (identity + tools catalog).
    pub system_reserve: usize,
    /// Tokens reserved for the model's response.
    pub response_reserve: usize,
    /// Per-tool result token budget.
    pub tool_result_budget: usize,
    /// Aggregate turn tool-output token budget.
    pub turn_tool_budget: usize,
    /// Maximum chars for conversation history (total).
    pub history_char_budget: usize,
    /// Maximum chars per individual history message.
    pub history_item_char_cap: usize,
    /// Maximum tools injected per turn.
    pub max_routed_tools: usize,
}

impl ContextBudgets {
    /// Default budgets for a 4K context local model.
    /// These match the existing hardcoded constants exactly.
    pub fn local_4k() -> Self {
        Self {
            context_window: 4096,
            system_reserve: 500,
            response_reserve: 1000,
            tool_result_budget: 1024,
            turn_tool_budget: 4096,
            history_char_budget: 12_000,
            history_item_char_cap: 900,
            max_routed_tools: 8,
        }
    }

    /// Scale budgets for a given context window size.
    ///
    /// Scaling rules (conservative):
    /// - History budget scales up to 8× base (capped)
    /// - Per-item cap scales up to 3× base
    /// - System reserve scales up to 2× base
    /// - Tool result budget stays fixed (always compact)
    /// - Turn tool budget scales up to 2× base
    /// - Max routed tools increases for large contexts
    pub fn for_context_window(context_window: usize) -> Self {
        let base = Self::local_4k();
        let scale = (context_window as f32 / 4096.0).clamp(1.0, 8.0);

        Self {
            context_window,
            system_reserve: (base.system_reserve as f32 * scale.min(2.0)) as usize,
            response_reserve: (base.response_reserve as f32 * scale.min(2.0)) as usize,
            // Tool result budget stays fixed — always compact regardless of context size
            tool_result_budget: base.tool_result_budget,
            // Turn tool budget scales moderately
            turn_tool_budget: (base.turn_tool_budget as f32 * scale.min(2.0)) as usize,
            // History scales most aggressively (this is where large contexts help most)
            history_char_budget: (base.history_char_budget as f32 * scale) as usize,
            // Per-item cap scales conservatively
            history_item_char_cap: (base.history_item_char_cap as f32 * scale.min(3.0)) as usize,
            // More tools for larger contexts
            max_routed_tools: if context_window > 16_000 { 12 } else { 8 },
        }
    }

    /// Available tokens for user content (history + retrieval + tool results).
    pub fn available_for_content(&self) -> usize {
        self.context_window
            .saturating_sub(self.system_reserve)
            .saturating_sub(self.response_reserve)
    }

    /// Pressure level based on how much of the content budget is used.
    pub fn pressure_level(&self, used_tokens: usize) -> PressureLevel {
        let available = self.available_for_content().max(1);
        let ratio = used_tokens as f32 / available as f32;
        if ratio >= 0.90 {
            PressureLevel::Critical
        } else if ratio >= 0.75 {
            PressureLevel::High
        } else if ratio >= 0.50 {
            PressureLevel::Medium
        } else {
            PressureLevel::Low
        }
    }

    /// Whether the context is approaching overflow.
    pub fn is_near_overflow(&self, used_tokens: usize) -> bool {
        matches!(
            self.pressure_level(used_tokens),
            PressureLevel::High | PressureLevel::Critical
        )
    }
}

impl Default for ContextBudgets {
    fn default() -> Self {
        Self::local_4k()
    }
}

// ─── Pressure Level ──────────────────────────────────────────────────────────

/// Context pressure level — how full the context window is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureLevel {
    /// < 50% used — comfortable
    Low,
    /// 50–75% used — monitor
    Medium,
    /// 75–90% used — compact history
    High,
    /// > 90% used — emergency compaction
    Critical,
}

impl PressureLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

// ─── Turn Token Ledger ───────────────────────────────────────────────────────

/// Cumulative token accounting for a single agent turn.
///
/// Tracks all token categories separately for observability.
/// All counts are approximate (heuristic) unless the provider tokenizer
/// API is available.
#[derive(Debug)]
pub struct TurnTokenLedger {
    /// Tokens in the system prompt (identity + tools catalog + context).
    pub system_tokens: AtomicUsize,
    /// Tokens in the conversation history (user + assistant messages).
    pub history_tokens: AtomicUsize,
    /// Tokens consumed by tool results this turn.
    pub tool_result_tokens: AtomicUsize,
    /// Tokens consumed by retrieval/RAG injection this turn.
    pub retrieval_tokens: AtomicUsize,
    /// Prompt tokens reported by the LLM provider (exact when available).
    pub provider_prompt_tokens: AtomicUsize,
    /// Completion tokens reported by the LLM provider (exact when available).
    pub provider_completion_tokens: AtomicUsize,
    /// Number of LLM calls made this turn.
    pub llm_call_count: AtomicUsize,
    /// Number of tool calls made this turn.
    pub tool_call_count: AtomicUsize,
    /// Number of compaction events triggered this turn.
    pub compaction_count: AtomicUsize,
    /// Estimation method used for most counts.
    estimation_method: std::sync::Mutex<EstimationMethod>,
}

impl TurnTokenLedger {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            system_tokens: AtomicUsize::new(0),
            history_tokens: AtomicUsize::new(0),
            tool_result_tokens: AtomicUsize::new(0),
            retrieval_tokens: AtomicUsize::new(0),
            provider_prompt_tokens: AtomicUsize::new(0),
            provider_completion_tokens: AtomicUsize::new(0),
            llm_call_count: AtomicUsize::new(0),
            tool_call_count: AtomicUsize::new(0),
            compaction_count: AtomicUsize::new(0),
            estimation_method: std::sync::Mutex::new(EstimationMethod::Heuristic),
        })
    }

    /// Record system prompt tokens.
    pub fn record_system(&self, tokens: usize, method: EstimationMethod) {
        self.system_tokens.store(tokens, Ordering::Relaxed);
        self.update_method(method);
    }

    /// Record history tokens.
    pub fn record_history(&self, tokens: usize) {
        self.history_tokens.store(tokens, Ordering::Relaxed);
    }

    /// Add tool result tokens (cumulative across the turn).
    pub fn add_tool_result(&self, tokens: usize) -> usize {
        let prev = self.tool_result_tokens.fetch_add(tokens, Ordering::Relaxed);
        self.tool_call_count.fetch_add(1, Ordering::Relaxed);
        prev + tokens
    }

    /// Add retrieval tokens.
    pub fn add_retrieval(&self, tokens: usize) {
        self.retrieval_tokens.fetch_add(tokens, Ordering::Relaxed);
    }

    /// Record provider-reported usage (exact counts from LLM response).
    pub fn record_provider_usage(&self, prompt_tokens: u32, completion_tokens: u32) {
        self.provider_prompt_tokens
            .store(prompt_tokens as usize, Ordering::Relaxed);
        self.provider_completion_tokens
            .store(completion_tokens as usize, Ordering::Relaxed);
        self.llm_call_count.fetch_add(1, Ordering::Relaxed);
        // Provider gave us exact counts — upgrade estimation method
        self.update_method(EstimationMethod::Exact);
    }

    /// Record a compaction event.
    pub fn record_compaction(&self) {
        self.compaction_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Total estimated tokens consumed this turn (all categories).
    pub fn total_estimated(&self) -> usize {
        self.system_tokens.load(Ordering::Relaxed)
            + self.history_tokens.load(Ordering::Relaxed)
            + self.tool_result_tokens.load(Ordering::Relaxed)
            + self.retrieval_tokens.load(Ordering::Relaxed)
    }

    /// Cumulative tool result tokens (for budget guard).
    pub fn tool_result_total(&self) -> usize {
        self.tool_result_tokens.load(Ordering::Relaxed)
    }

    /// Current estimation method.
    pub fn estimation_method(&self) -> EstimationMethod {
        *self
            .estimation_method
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Snapshot for logging/observability.
    pub fn snapshot(&self) -> LedgerSnapshot {
        LedgerSnapshot {
            system_tokens: self.system_tokens.load(Ordering::Relaxed),
            history_tokens: self.history_tokens.load(Ordering::Relaxed),
            tool_result_tokens: self.tool_result_tokens.load(Ordering::Relaxed),
            retrieval_tokens: self.retrieval_tokens.load(Ordering::Relaxed),
            provider_prompt_tokens: self.provider_prompt_tokens.load(Ordering::Relaxed),
            provider_completion_tokens: self.provider_completion_tokens.load(Ordering::Relaxed),
            llm_call_count: self.llm_call_count.load(Ordering::Relaxed),
            tool_call_count: self.tool_call_count.load(Ordering::Relaxed),
            compaction_count: self.compaction_count.load(Ordering::Relaxed),
            estimation_method: self.estimation_method(),
        }
    }

    fn update_method(&self, method: EstimationMethod) {
        if let Ok(mut m) = self.estimation_method.lock() {
            // Upgrade to exact if we get exact data; never downgrade
            if method == EstimationMethod::Exact {
                *m = EstimationMethod::Exact;
            }
        }
    }
}

/// Immutable snapshot of ledger state for logging.
#[derive(Debug, Clone, Serialize)]
pub struct LedgerSnapshot {
    pub system_tokens: usize,
    pub history_tokens: usize,
    pub tool_result_tokens: usize,
    pub retrieval_tokens: usize,
    pub provider_prompt_tokens: usize,
    pub provider_completion_tokens: usize,
    pub llm_call_count: usize,
    pub tool_call_count: usize,
    pub compaction_count: usize,
    pub estimation_method: EstimationMethod,
}
impl LedgerSnapshot {
    /// Total estimated tokens (all categories).
    pub fn total_estimated(&self) -> usize {
        self.system_tokens + self.history_tokens + self.tool_result_tokens + self.retrieval_tokens
    }

    /// Serialize to JSON for pipeline trace logging.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "system_tokens": self.system_tokens,
            "history_tokens": self.history_tokens,
            "tool_result_tokens": self.tool_result_tokens,
            "retrieval_tokens": self.retrieval_tokens,
            "provider_prompt_tokens": self.provider_prompt_tokens,
            "provider_completion_tokens": self.provider_completion_tokens,
            "total_estimated": self.total_estimated(),
            "llm_calls": self.llm_call_count,
            "tool_calls": self.tool_call_count,
            "compactions": self.compaction_count,
            "estimation_method": self.estimation_method.as_str(),
        })
    }
}

// ─── Inter-Tool Budget Check ─────────────────────────────────────────────────

/// Result of an inter-tool budget check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetCheckResult {
    /// Budget is healthy — continue tool execution.
    Ok,
    /// Budget is under pressure — compact history before next LLM call.
    CompactRequired,
    /// Budget is exhausted — break the tool loop.
    ExhaustedBreak,
}

/// Check whether the context budget allows another tool call.
///
/// Called after each tool result is appended to messages.
/// Returns a `BudgetCheckResult` indicating what action to take.
pub fn check_inter_tool_budget(
    messages: &[crate::llm::ChatMessage],
    budgets: &ContextBudgets,
) -> BudgetCheckResult {
    let total_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
    let estimated_tokens = estimate_tokens_heuristic(&" ".repeat(total_chars.min(1)));
    // Use chars directly since we're comparing against char budgets
    let context_window_chars = budgets.context_window * 4;

    if total_chars > (context_window_chars * 7 / 8) {
        BudgetCheckResult::ExhaustedBreak
    } else if total_chars > (context_window_chars * 3 / 4) {
        BudgetCheckResult::CompactRequired
    } else {
        let _ = estimated_tokens; // suppress unused warning
        BudgetCheckResult::Ok
    }
}

/// Check whether the cumulative tool result tokens exceed the turn budget.
pub fn check_tool_result_budget(cumulative_tool_tokens: usize, budgets: &ContextBudgets) -> bool {
    cumulative_tool_tokens >= budgets.turn_tool_budget
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── ContextBudgets ───────────────────────────────────────────────────────

    #[test]
    fn local_4k_matches_existing_constants() {
        let b = ContextBudgets::local_4k();
        assert_eq!(b.context_window, 4096);
        assert_eq!(b.tool_result_budget, 1024);
        assert_eq!(b.turn_tool_budget, 4096);
        assert_eq!(b.history_char_budget, 12_000);
        assert_eq!(b.history_item_char_cap, 900);
        assert_eq!(b.max_routed_tools, 8);
    }

    #[test]
    fn scaling_preserves_tool_result_budget() {
        // Tool result budget must stay fixed regardless of context window
        let b4k = ContextBudgets::for_context_window(4096);
        let b32k = ContextBudgets::for_context_window(32_768);
        let b128k = ContextBudgets::for_context_window(131_072);
        assert_eq!(b4k.tool_result_budget, b32k.tool_result_budget);
        assert_eq!(b4k.tool_result_budget, b128k.tool_result_budget);
    }

    #[test]
    fn scaling_increases_history_budget() {
        let b4k = ContextBudgets::for_context_window(4096);
        let b32k = ContextBudgets::for_context_window(32_768);
        assert!(b32k.history_char_budget > b4k.history_char_budget);
    }

    #[test]
    fn scaling_caps_at_8x() {
        let b4k = ContextBudgets::local_4k();
        let b_huge = ContextBudgets::for_context_window(1_000_000);
        // History budget should not exceed 8× base
        assert!(b_huge.history_char_budget <= b4k.history_char_budget * 8 + 1);
    }

    #[test]
    fn large_context_gets_more_tools() {
        let b4k = ContextBudgets::for_context_window(4096);
        let b32k = ContextBudgets::for_context_window(32_768);
        assert!(b32k.max_routed_tools >= b4k.max_routed_tools);
    }

    #[test]
    fn available_for_content_is_bounded() {
        let b = ContextBudgets::local_4k();
        let available = b.available_for_content();
        assert!(available < b.context_window);
        assert!(available > 0);
    }

    // ── Pressure levels ──────────────────────────────────────────────────────

    #[test]
    fn pressure_low_when_empty() {
        let b = ContextBudgets::local_4k();
        assert_eq!(b.pressure_level(0), PressureLevel::Low);
    }

    #[test]
    fn pressure_medium_at_60_percent() {
        let b = ContextBudgets::local_4k();
        let available = b.available_for_content();
        let used = (available as f32 * 0.60) as usize;
        assert_eq!(b.pressure_level(used), PressureLevel::Medium);
    }

    #[test]
    fn pressure_high_at_80_percent() {
        let b = ContextBudgets::local_4k();
        let available = b.available_for_content();
        let used = (available as f32 * 0.80) as usize;
        assert_eq!(b.pressure_level(used), PressureLevel::High);
    }

    #[test]
    fn pressure_critical_at_95_percent() {
        let b = ContextBudgets::local_4k();
        let available = b.available_for_content();
        let used = (available as f32 * 0.95) as usize;
        assert_eq!(b.pressure_level(used), PressureLevel::Critical);
    }

    #[test]
    fn near_overflow_at_high_and_critical() {
        let b = ContextBudgets::local_4k();
        let available = b.available_for_content();
        assert!(!b.is_near_overflow((available as f32 * 0.60) as usize));
        assert!(b.is_near_overflow((available as f32 * 0.80) as usize));
        assert!(b.is_near_overflow((available as f32 * 0.95) as usize));
    }

    // ── TurnTokenLedger ──────────────────────────────────────────────────────

    #[test]
    fn ledger_starts_at_zero() {
        let ledger = TurnTokenLedger::new();
        assert_eq!(ledger.total_estimated(), 0);
        assert_eq!(ledger.tool_result_total(), 0);
    }

    #[test]
    fn ledger_accumulates_tool_results() {
        let ledger = TurnTokenLedger::new();
        ledger.add_tool_result(100);
        ledger.add_tool_result(200);
        assert_eq!(ledger.tool_result_total(), 300);
    }

    #[test]
    fn ledger_total_sums_all_categories() {
        let ledger = TurnTokenLedger::new();
        ledger.record_system(500, EstimationMethod::Heuristic);
        ledger.record_history(1000);
        ledger.add_tool_result(300);
        ledger.add_retrieval(200);
        assert_eq!(ledger.total_estimated(), 2000);
    }

    #[test]
    fn ledger_provider_usage_upgrades_to_exact() {
        let ledger = TurnTokenLedger::new();
        assert_eq!(ledger.estimation_method(), EstimationMethod::Heuristic);
        ledger.record_provider_usage(1500, 200);
        assert_eq!(ledger.estimation_method(), EstimationMethod::Exact);
    }

    #[test]
    fn ledger_snapshot_is_consistent() {
        let ledger = TurnTokenLedger::new();
        ledger.record_system(400, EstimationMethod::Heuristic);
        ledger.record_history(800);
        ledger.add_tool_result(150);
        ledger.add_tool_result(250);
        let snap = ledger.snapshot();
        assert_eq!(snap.system_tokens, 400);
        assert_eq!(snap.history_tokens, 800);
        assert_eq!(snap.tool_result_tokens, 400);
        assert_eq!(snap.tool_call_count, 2);
        assert_eq!(snap.total_estimated(), 1600);
    }

    #[test]
    fn ledger_compaction_count_tracked() {
        let ledger = TurnTokenLedger::new();
        ledger.record_compaction();
        ledger.record_compaction();
        assert_eq!(ledger.snapshot().compaction_count, 2);
    }

    // ── Inter-tool budget check ──────────────────────────────────────────────

    #[test]
    fn budget_check_ok_when_empty() {
        let budgets = ContextBudgets::local_4k();
        let messages: Vec<crate::llm::ChatMessage> = vec![];
        assert_eq!(
            check_inter_tool_budget(&messages, &budgets),
            BudgetCheckResult::Ok
        );
    }

    #[test]
    fn budget_check_compact_at_75_percent() {
        let budgets = ContextBudgets::local_4k();
        let context_window_chars = budgets.context_window * 4;
        let large_content = "x".repeat((context_window_chars * 76 / 100).min(100_000));
        let messages = vec![crate::llm::ChatMessage {
            role: "user".into(),
            content: large_content,
            name: None,
            images: None,
        }];
        assert_eq!(
            check_inter_tool_budget(&messages, &budgets),
            BudgetCheckResult::CompactRequired
        );
    }

    #[test]
    fn budget_check_exhausted_at_88_percent() {
        let budgets = ContextBudgets::local_4k();
        let context_window_chars = budgets.context_window * 4;
        let large_content = "x".repeat((context_window_chars * 89 / 100).min(100_000));
        let messages = vec![crate::llm::ChatMessage {
            role: "user".into(),
            content: large_content,
            name: None,
            images: None,
        }];
        assert_eq!(
            check_inter_tool_budget(&messages, &budgets),
            BudgetCheckResult::ExhaustedBreak
        );
    }

    #[test]
    fn tool_result_budget_check_triggers_at_limit() {
        let budgets = ContextBudgets::local_4k();
        assert!(!check_tool_result_budget(
            budgets.turn_tool_budget - 1,
            &budgets
        ));
        assert!(check_tool_result_budget(budgets.turn_tool_budget, &budgets));
        assert!(check_tool_result_budget(
            budgets.turn_tool_budget + 100,
            &budgets
        ));
    }

    // ── Estimation ───────────────────────────────────────────────────────────

    #[test]
    fn heuristic_estimation_is_chars_div_4() {
        let text = "hello world"; // 11 chars
        assert_eq!(estimate_tokens_heuristic(text), 11 / 4);
    }

    #[test]
    fn estimate_tokens_returns_heuristic_method() {
        let (_, method) = estimate_tokens("some text");
        assert_eq!(method, EstimationMethod::Heuristic);
    }

    // ── Determinism ──────────────────────────────────────────────────────────

    #[test]
    fn budget_scaling_is_deterministic() {
        let b1 = ContextBudgets::for_context_window(32_768);
        let b2 = ContextBudgets::for_context_window(32_768);
        assert_eq!(b1.history_char_budget, b2.history_char_budget);
        assert_eq!(b1.tool_result_budget, b2.tool_result_budget);
        assert_eq!(b1.max_routed_tools, b2.max_routed_tools);
    }

    #[test]
    fn pressure_level_is_deterministic() {
        let b = ContextBudgets::local_4k();
        let used = 1500;
        assert_eq!(b.pressure_level(used), b.pressure_level(used));
    }

    // ── Provider-specific budgets ────────────────────────────────────────────

    #[test]
    fn anthropic_claude_200k_budget() {
        let b = ContextBudgets::for_context_window(200_000);
        // History should be much larger than 4K default
        assert!(b.history_char_budget > 12_000 * 4);
        // Tool result budget stays fixed
        assert_eq!(b.tool_result_budget, 1024);
        // More tools available
        assert_eq!(b.max_routed_tools, 12);
    }

    #[test]
    fn gemini_1m_budget() {
        let b = ContextBudgets::for_context_window(1_000_000);
        // Capped at 8× base
        let base = ContextBudgets::local_4k();
        assert!(b.history_char_budget <= base.history_char_budget * 8 + 100);
    }
}
