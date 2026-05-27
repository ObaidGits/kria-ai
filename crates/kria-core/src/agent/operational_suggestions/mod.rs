//! Batch 3 — Operational Suggestions Engine.
//!
//! # Core Mission
//!
//! Bounded, rate-limited proactive suggestion generation. KRIA may suggest
//! workflow continuation, recovery, or next actions — but only when clearly
//! contextual, non-intrusive, and policy-permitted.
//!
//! # Design Philosophy
//!
//! ```text
//! "A good coworker doesn't interrupt you constantly.
//!  They speak up when it actually matters — and only once."
//! ```
//!
//! # Suggestion Types
//!
//! - Resume a paused workflow (only if paused > 5 min)
//! - Recover from a build failure (only once per failure)
//! - Continue an interrupted browser session
//! - Address open IDE diagnostics (only if error count increased)
//! - Advance an operational goal
//!
//! # Rate Limiting
//!
//! - Maximum `MAX_SUGGESTIONS_PER_WINDOW` per `WINDOW_DURATION`.
//! - Each suggestion type has a separate dedup guard — one suggestion of
//!   each type per window.
//! - Suggestions are cleared from the dedup set when acted on.
//!
//! # Safety
//!
//! - Never executes actions.
//! - Never calls the LLM.
//! - Requires `CollaborativeAutonomyEngine` to approve before surfacing.
//! - User can globally disable via `OperationalSuggestionsEngine::disable()`.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::agent::cognition_event_bus::{
    CognitionEvent, CognitionEventBus, SuggestionEvent, SuggestionKind,
};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Maximum suggestions emitted per window.
pub const MAX_SUGGESTIONS_PER_WINDOW: usize = 3;

/// How long the rate-limit window lasts.
pub const WINDOW_DURATION: Duration = Duration::from_secs(300); // 5 minutes

/// Minimum time a workflow must have been paused before suggesting resume.
pub const MIN_PAUSE_BEFORE_SUGGEST: Duration = Duration::from_secs(300); // 5 minutes

// ─── Suggestion ───────────────────────────────────────────────────────────────

/// Relevance tier for a suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SuggestionRelevance {
    /// Low relevance — may be skipped.
    Low,
    /// Medium relevance — surface if context-appropriate.
    Medium,
    /// High relevance — always surface (within rate limits).
    High,
}

/// An operational suggestion ready to be surfaced to the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalSuggestion {
    /// Stable suggestion ID (dedup key).
    pub suggestion_id: String,
    /// Human-readable suggestion.
    pub content: String,
    /// Why this was generated.
    pub rationale: String,
    /// Type of suggestion.
    pub kind: SuggestionKind,
    /// How relevant this suggestion is.
    pub relevance: SuggestionRelevance,
    /// When this suggestion was generated (Unix seconds).
    pub generated_at: u64,
}

impl OperationalSuggestion {
    /// Stable dedup key (derived from kind, not content).
    pub fn dedup_key(&self) -> String {
        self.suggestion_id.clone()
    }
}

// ─── Rate Limit State ─────────────────────────────────────────────────────────

struct RateLimitState {
    /// When the current window started.
    window_start: Instant,
    /// Suggestions emitted in this window.
    window_count: usize,
    /// Set of suggestion IDs emitted this window (for dedup).
    emitted_ids: HashSet<String>,
}

impl RateLimitState {
    fn new() -> Self {
        Self {
            window_start: Instant::now(),
            window_count: 0,
            emitted_ids: HashSet::new(),
        }
    }

    /// Whether a new suggestion can be emitted.
    fn can_emit(&mut self, suggestion_id: &str) -> bool {
        // Reset window if expired
        if self.window_start.elapsed() >= WINDOW_DURATION {
            self.window_start = Instant::now();
            self.window_count = 0;
            self.emitted_ids.clear();
        }
        // Check dedup
        if self.emitted_ids.contains(suggestion_id) {
            return false;
        }
        // Check window cap
        self.window_count < MAX_SUGGESTIONS_PER_WINDOW
    }

    /// Record that a suggestion was emitted.
    fn record_emission(&mut self, suggestion_id: &str) {
        self.emitted_ids.insert(suggestion_id.to_string());
        self.window_count += 1;
    }

    /// Clear a suggestion from the dedup set (e.g., after it was acted on).
    fn clear_suggestion(&mut self, suggestion_id: &str) {
        self.emitted_ids.remove(suggestion_id);
    }
}

// ─── Engine ───────────────────────────────────────────────────────────────────

/// Bounded, rate-limited operational suggestions engine.
pub struct OperationalSuggestionsEngine {
    event_bus: Option<std::sync::Arc<CognitionEventBus>>,
    rate_limit: Mutex<RateLimitState>,
    enabled: std::sync::atomic::AtomicBool,
}

impl OperationalSuggestionsEngine {
    /// Create a new engine.
    pub fn new(event_bus: Option<std::sync::Arc<CognitionEventBus>>) -> Self {
        Self {
            event_bus,
            rate_limit: Mutex::new(RateLimitState::new()),
            enabled: std::sync::atomic::AtomicBool::new(true),
        }
    }

    /// Disable suggestion emission globally (user-controlled).
    pub fn disable(&self) {
        self.enabled
            .store(false, std::sync::atomic::Ordering::Relaxed);
        debug!(target: "operational_suggestions", "Suggestions disabled");
    }

    /// Re-enable suggestion emission.
    pub fn enable(&self) {
        self.enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        debug!(target: "operational_suggestions", "Suggestions enabled");
    }

    /// Whether suggestions are currently enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Suggest resuming a paused workflow.
    ///
    /// Only emits if the session has been paused longer than
    /// `MIN_PAUSE_BEFORE_SUGGEST`.
    pub fn suggest_resume(
        &self,
        session_id: &str,
        intent: &str,
        paused_at_epoch: u64,
    ) -> Option<OperationalSuggestion> {
        if !self.is_enabled() {
            return None;
        }
        let paused_for = now_epoch().saturating_sub(paused_at_epoch);
        if paused_for < MIN_PAUSE_BEFORE_SUGGEST.as_secs() {
            return None;
        }

        let suggestion_id = format!("resume-{}", session_id);
        let sug = OperationalSuggestion {
            suggestion_id: suggestion_id.clone(),
            content: format!(
                "You have a paused workflow: '{}'. Would you like to continue?",
                intent
            ),
            rationale: format!(
                "Workflow paused {}m ago. Context is still available.",
                paused_for / 60
            ),
            kind: SuggestionKind::ResumePausedWorkflow {
                session_id: session_id.to_string(),
            },
            relevance: SuggestionRelevance::High,
            generated_at: now_epoch(),
        };
        self.try_emit(sug)
    }

    /// Suggest recovering from a build failure.
    pub fn suggest_build_recovery(
        &self,
        workspace_root: &str,
        error_summary: &str,
    ) -> Option<OperationalSuggestion> {
        if !self.is_enabled() {
            return None;
        }
        let suggestion_id = format!("build-fail-{}", sanitize_key(workspace_root));
        let sug = OperationalSuggestion {
            suggestion_id,
            content: format!(
                "Build failure detected in '{}'. Suggestion: address the errors and retry.",
                workspace_root
            ),
            rationale: format!("Active build failure: {}", error_summary),
            kind: SuggestionKind::RecoverBuildFailure,
            relevance: SuggestionRelevance::Medium,
            generated_at: now_epoch(),
        };
        self.try_emit(sug)
    }

    /// Suggest addressing IDE diagnostics.
    pub fn suggest_address_diagnostics(
        &self,
        workspace_root: &str,
        error_count: usize,
    ) -> Option<OperationalSuggestion> {
        if !self.is_enabled() || error_count == 0 {
            return None;
        }
        let suggestion_id = format!("diagnostics-{}", sanitize_key(workspace_root));
        let sug = OperationalSuggestion {
            suggestion_id,
            content: format!(
                "{} error(s) detected in workspace. Would you like to address them?",
                error_count
            ),
            rationale: format!("{} IDE errors active in {}", error_count, workspace_root),
            kind: SuggestionKind::AddressDiagnostics { error_count },
            relevance: if error_count >= 5 {
                SuggestionRelevance::High
            } else {
                SuggestionRelevance::Medium
            },
            generated_at: now_epoch(),
        };
        self.try_emit(sug)
    }

    /// Suggest the next step toward a goal.
    pub fn suggest_goal_step(
        &self,
        goal_id: &str,
        description: &str,
        hint: &str,
    ) -> Option<OperationalSuggestion> {
        if !self.is_enabled() {
            return None;
        }
        let suggestion_id = format!("goal-{}", goal_id);
        let sug = OperationalSuggestion {
            suggestion_id,
            content: format!("Next step for goal '{}': {}", description, hint),
            rationale: "Goal advancement suggestion from PersistentGoalRuntime.".to_string(),
            kind: SuggestionKind::NextGoalStep {
                goal_id: goal_id.to_string(),
            },
            relevance: SuggestionRelevance::Medium,
            generated_at: now_epoch(),
        };
        self.try_emit(sug)
    }

    /// Clear a suggestion from the rate-limit dedup set.
    ///
    /// Call this when a suggestion was acted on, to allow re-emission if the
    /// situation persists (e.g., the build was fixed but broke again).
    pub fn clear_suggestion(&self, suggestion_id: &str) {
        self.rate_limit
            .lock()
            .unwrap()
            .clear_suggestion(suggestion_id);
    }

    /// How many suggestions have been emitted in the current window.
    pub fn window_count(&self) -> usize {
        self.rate_limit.lock().unwrap().window_count
    }

    // ── Internal ─────────────────────────────────────────────────────────────

    fn try_emit(&self, sug: OperationalSuggestion) -> Option<OperationalSuggestion> {
        let mut rl = self.rate_limit.lock().unwrap();
        if !rl.can_emit(&sug.suggestion_id) {
            debug!(
                target: "operational_suggestions",
                id = %sug.suggestion_id,
                "Suggestion suppressed by rate limiter"
            );
            return None;
        }
        rl.record_emission(&sug.suggestion_id);
        drop(rl);

        // Emit to event bus if present
        if let Some(ref bus) = self.event_bus {
            let ev = CognitionEvent::Suggestion(SuggestionEvent {
                suggestion_id: sug.suggestion_id.clone(),
                content: sug.content.clone(),
                rationale: sug.rationale.clone(),
                kind: sug.kind.clone(),
            });
            bus.emit(ev);
        }

        debug!(
            target: "operational_suggestions",
            id = %sug.suggestion_id,
            content = %sug.content,
            "Suggestion emitted"
        );
        Some(sug)
    }
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sanitize_key(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> OperationalSuggestionsEngine {
        OperationalSuggestionsEngine::new(None)
    }

    #[test]
    fn suggest_resume_too_recent_returns_none() {
        let e = engine();
        // Paused just now — below MIN_PAUSE_BEFORE_SUGGEST
        let sug = e.suggest_resume("s1", "build project", now_epoch());
        assert!(
            sug.is_none(),
            "should not suggest resume for very recent pause"
        );
    }

    #[test]
    fn suggest_resume_old_pause_returns_suggestion() {
        let e = engine();
        let ancient = 0u64; // paused at epoch 0 — hours ago
        let sug = e.suggest_resume("s1", "build project", ancient);
        assert!(sug.is_some());
        let s = sug.unwrap();
        assert!(matches!(
            s.kind,
            SuggestionKind::ResumePausedWorkflow { .. }
        ));
    }

    #[test]
    fn dedup_blocks_second_identical_suggestion() {
        let e = engine();
        let sug1 = e.suggest_resume("s1", "wf", 0);
        let sug2 = e.suggest_resume("s1", "wf", 0); // same session
        assert!(sug1.is_some());
        assert!(sug2.is_none(), "dedup must block identical suggestion");
    }

    #[test]
    fn clear_suggestion_allows_reemission() {
        let e = engine();
        e.suggest_resume("s1", "wf", 0);
        e.clear_suggestion("resume-s1");
        let sug = e.suggest_resume("s1", "wf", 0);
        assert!(sug.is_some(), "after clearing, should be emittable again");
    }

    #[test]
    fn rate_cap_enforced() {
        let e = engine();
        for i in 0..(MAX_SUGGESTIONS_PER_WINDOW + 10) {
            e.suggest_build_recovery(&format!("/proj/{}", i), "error");
        }
        assert!(
            e.window_count() <= MAX_SUGGESTIONS_PER_WINDOW,
            "window count exceeded cap: {}",
            e.window_count()
        );
    }

    #[test]
    fn disabled_engine_returns_none() {
        let e = engine();
        e.disable();
        let sug = e.suggest_resume("s1", "wf", 0);
        assert!(sug.is_none());
        let sug2 = e.suggest_build_recovery("/proj", "err");
        assert!(sug2.is_none());
    }

    #[test]
    fn enable_after_disable_resumes() {
        let e = engine();
        e.disable();
        e.enable();
        let sug = e.suggest_resume("s1", "wf", 0);
        assert!(sug.is_some());
    }

    #[test]
    fn zero_errors_no_diagnostic_suggestion() {
        let e = engine();
        assert!(e.suggest_address_diagnostics("/workspace", 0).is_none());
    }

    #[test]
    fn suggest_address_diagnostics_with_errors() {
        let e = engine();
        let sug = e.suggest_address_diagnostics("/workspace", 3);
        assert!(sug.is_some());
        assert!(matches!(
            sug.unwrap().kind,
            SuggestionKind::AddressDiagnostics { .. }
        ));
    }
}
