//! Execution Trace — causal chain tracking for tool execution.
//!
//! Tracks the causal chain of tool calls within a turn:
//! - Which tool was called because of which prior result
//! - Dependency graph between tool calls
//! - Failure classification and recovery trace
//! - Execution audit with timing
//!
//! # Design
//! - Zero LLM calls — purely structural tracking
//! - Bounded: cleared at turn end, never persists across turns
//! - Observable: full trace serializable to JSON
//! - Lightweight: no async, no locks in hot path

use std::collections::HashMap;
use std::time::{Duration, Instant};

// ─── Trace Node ───────────────────────────────────────────────────────────────

/// A single tool call in the execution trace.
#[derive(Debug, Clone)]
pub struct TraceNode {
    /// Unique ID for this call within the turn (sequential)
    pub call_id: u32,
    /// Tool name
    pub tool_name: String,
    /// ID of the parent call that caused this call (None = user-initiated)
    pub caused_by: Option<u32>,
    /// Causal reason (why was this tool called?)
    pub cause_reason: CauseReason,
    /// When this call started
    pub started_at: Instant,
    /// How long the call took
    pub duration: Option<Duration>,
    /// Whether the call succeeded
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
    /// Failure classification
    pub failure_class: Option<FailureClass>,
    /// Whether this call was retried
    pub was_retry: bool,
    /// Round number in the agent loop
    pub round: u32,
}

/// Why a tool was called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CauseReason {
    /// Directly requested by the user
    UserRequest,
    /// LLM decided to call this tool
    LlmDecision,
    /// Called as a prerequisite for another tool (e.g., search before read)
    Prerequisite,
    /// Called as a fallback after another tool failed
    Fallback,
    /// Called to verify a previous tool's result
    Verification,
    /// Called as part of a multi-step workflow (e.g., Colab bootstrap)
    WorkflowStep,
    /// Called by the intent fallback system
    IntentFallback,
    /// Called because of live-fact detection
    LiveFactInjection,
}

impl CauseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserRequest => "user_request",
            Self::LlmDecision => "llm_decision",
            Self::Prerequisite => "prerequisite",
            Self::Fallback => "fallback",
            Self::Verification => "verification",
            Self::WorkflowStep => "workflow_step",
            Self::IntentFallback => "intent_fallback",
            Self::LiveFactInjection => "live_fact_injection",
        }
    }
}

/// Classification of a tool failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// Tool not available (not registered, tier mismatch)
    ToolUnavailable,
    /// Authentication/authorization failure
    AuthFailure,
    /// Network/connectivity failure
    NetworkFailure,
    /// Rate limit or quota exceeded
    RateLimited,
    /// Input validation failure (bad parameters)
    ValidationFailure,
    /// Timeout
    Timeout,
    /// Target mismatch (execution authority blocked)
    TargetMismatch,
    /// Preflight blocked
    PreflightBlocked,
    /// User denied approval
    UserDenied,
    /// Unknown/unclassified failure
    Unknown,
}

impl FailureClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolUnavailable => "tool_unavailable",
            Self::AuthFailure => "auth_failure",
            Self::NetworkFailure => "network_failure",
            Self::RateLimited => "rate_limited",
            Self::ValidationFailure => "validation_failure",
            Self::Timeout => "timeout",
            Self::TargetMismatch => "target_mismatch",
            Self::PreflightBlocked => "preflight_blocked",
            Self::UserDenied => "user_denied",
            Self::Unknown => "unknown",
        }
    }

    /// Classify a failure from an error message.
    pub fn from_error(error: &str) -> Self {
        let lower = error.to_ascii_lowercase();
        if lower.contains("not available")
            || lower.contains("not registered")
            || lower.contains("tier")
        {
            Self::ToolUnavailable
        } else if lower.contains("auth")
            || lower.contains("unauthorized")
            || lower.contains("forbidden")
            || lower.contains("credentials")
        {
            Self::AuthFailure
        } else if lower.contains("timeout") || lower.contains("timed out") {
            Self::Timeout
        } else if lower.contains("rate limit") || lower.contains("quota") || lower.contains("429") {
            Self::RateLimited
        } else if lower.contains("network")
            || lower.contains("connection")
            || lower.contains("dns")
            || lower.contains("502")
            || lower.contains("503")
        {
            Self::NetworkFailure
        } else if lower.contains("invalid")
            || lower.contains("required")
            || lower.contains("missing")
        {
            Self::ValidationFailure
        } else if lower.contains("target mismatch") || lower.contains("execution_blocked") {
            Self::TargetMismatch
        } else if lower.contains("preflight") {
            Self::PreflightBlocked
        } else if lower.contains("denied") || lower.contains("not executed") {
            Self::UserDenied
        } else {
            Self::Unknown
        }
    }
}

// ─── Execution Trace ──────────────────────────────────────────────────────────

/// Per-turn execution trace.
/// Tracks the full causal chain of tool calls.
#[derive(Debug)]
pub struct ExecutionTrace {
    /// Session ID
    pub session_id: String,
    /// Turn ID
    pub turn_id: String,
    /// All tool calls in order
    nodes: Vec<TraceNode>,
    /// Next call ID
    next_id: u32,
    /// Map from tool_name → last call_id (for dependency tracking)
    last_call_by_tool: HashMap<String, u32>,
}

impl ExecutionTrace {
    pub fn new(session_id: impl Into<String>, turn_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            nodes: Vec::new(),
            next_id: 0,
            last_call_by_tool: HashMap::new(),
        }
    }

    /// Record the start of a tool call.
    /// Returns the call_id for this call (use in `record_end`).
    pub fn record_start(
        &mut self,
        tool_name: &str,
        cause_reason: CauseReason,
        caused_by_tool: Option<&str>,
        round: u32,
    ) -> u32 {
        let call_id = self.next_id;
        self.next_id += 1;

        // Resolve caused_by from the last call of the causing tool
        let caused_by = caused_by_tool.and_then(|name| self.last_call_by_tool.get(name).copied());

        self.nodes.push(TraceNode {
            call_id,
            tool_name: tool_name.to_string(),
            caused_by,
            cause_reason,
            started_at: Instant::now(),
            duration: None,
            success: false,
            error: None,
            failure_class: None,
            was_retry: self.last_call_by_tool.contains_key(tool_name),
            round,
        });

        self.last_call_by_tool
            .insert(tool_name.to_string(), call_id);
        call_id
    }

    /// Record the completion of a tool call.
    pub fn record_end(&mut self, call_id: u32, success: bool, error: Option<&str>) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.call_id == call_id) {
            node.duration = Some(node.started_at.elapsed());
            node.success = success;
            if let Some(err) = error {
                node.failure_class = Some(FailureClass::from_error(err));
                node.error = Some(err.chars().take(200).collect());
            }
        }
    }

    /// Get all nodes in the trace.
    pub fn nodes(&self) -> &[TraceNode] {
        &self.nodes
    }

    /// Get the causal chain leading to a specific call.
    pub fn causal_chain(&self, call_id: u32) -> Vec<&TraceNode> {
        let mut chain = Vec::new();
        let mut current_id = Some(call_id);

        while let Some(id) = current_id {
            if let Some(node) = self.nodes.iter().find(|n| n.call_id == id) {
                chain.push(node);
                current_id = node.caused_by;
            } else {
                break;
            }
        }

        chain.reverse();
        chain
    }

    /// Count failures by class.
    pub fn failure_summary(&self) -> HashMap<&'static str, usize> {
        let mut summary: HashMap<&'static str, usize> = HashMap::new();
        for node in &self.nodes {
            if !node.success {
                if let Some(class) = node.failure_class {
                    *summary.entry(class.as_str()).or_insert(0) += 1;
                }
            }
        }
        summary
    }

    /// Total number of tool calls.
    pub fn total_calls(&self) -> usize {
        self.nodes.len()
    }

    /// Number of successful calls.
    pub fn successful_calls(&self) -> usize {
        self.nodes.iter().filter(|n| n.success).count()
    }

    /// Number of failed calls.
    pub fn failed_calls(&self) -> usize {
        self.nodes.iter().filter(|n| !n.success).count()
    }

    /// Total execution time across all tool calls.
    pub fn total_duration(&self) -> Duration {
        self.nodes
            .iter()
            .filter_map(|n| n.duration)
            .fold(Duration::ZERO, |acc, d| acc + d)
    }

    /// Serialize to JSON for logging and observability.
    pub fn to_json(&self) -> serde_json::Value {
        let nodes: Vec<serde_json::Value> = self
            .nodes
            .iter()
            .map(|n| {
                serde_json::json!({
                    "call_id": n.call_id,
                    "tool": n.tool_name,
                    "caused_by": n.caused_by,
                    "cause_reason": n.cause_reason.as_str(),
                    "round": n.round,
                    "success": n.success,
                    "duration_ms": n.duration.map(|d| d.as_millis()),
                    "error": n.error,
                    "failure_class": n.failure_class.map(|f| f.as_str()),
                    "was_retry": n.was_retry,
                })
            })
            .collect();

        serde_json::json!({
            "session_id": self.session_id,
            "turn_id": self.turn_id,
            "total_calls": self.total_calls(),
            "successful_calls": self.successful_calls(),
            "failed_calls": self.failed_calls(),
            "total_duration_ms": self.total_duration().as_millis(),
            "failure_summary": self.failure_summary(),
            "nodes": nodes,
        })
    }

    /// Build a compact summary for pipeline trace logging.
    pub fn compact_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "calls": self.total_calls(),
            "ok": self.successful_calls(),
            "failed": self.failed_calls(),
            "duration_ms": self.total_duration().as_millis(),
            "failures": self.failure_summary(),
        })
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_records_causal_chain() {
        let mut trace = ExecutionTrace::new("session-1", "turn-1");

        let id1 = trace.record_start("web_search", CauseReason::LiveFactInjection, None, 1);
        trace.record_end(id1, true, None);

        let id2 = trace.record_start(
            "fetch_webpage",
            CauseReason::LlmDecision,
            Some("web_search"),
            1,
        );
        trace.record_end(id2, true, None);

        assert_eq!(trace.total_calls(), 2);
        assert_eq!(trace.successful_calls(), 2);

        let chain = trace.causal_chain(id2);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].tool_name, "web_search");
        assert_eq!(chain[1].tool_name, "fetch_webpage");
    }

    #[test]
    fn failure_classification_from_error() {
        assert_eq!(
            FailureClass::from_error("timeout after 30s"),
            FailureClass::Timeout
        );
        assert_eq!(
            FailureClass::from_error("rate limit exceeded"),
            FailureClass::RateLimited
        );
        assert_eq!(
            FailureClass::from_error("unauthorized: invalid token"),
            FailureClass::AuthFailure
        );
        assert_eq!(
            FailureClass::from_error("connection refused"),
            FailureClass::NetworkFailure
        );
        assert_eq!(
            FailureClass::from_error("EXECUTION_BLOCKED: target mismatch"),
            FailureClass::TargetMismatch
        );
    }

    #[test]
    fn trace_counts_failures_correctly() {
        let mut trace = ExecutionTrace::new("s", "t");
        let id1 = trace.record_start("web_search", CauseReason::UserRequest, None, 1);
        trace.record_end(id1, false, Some("timeout after 30s"));

        let id2 = trace.record_start("search_news", CauseReason::Fallback, Some("web_search"), 1);
        trace.record_end(id2, true, None);

        assert_eq!(trace.failed_calls(), 1);
        assert_eq!(trace.successful_calls(), 1);

        let summary = trace.failure_summary();
        assert_eq!(summary.get("timeout"), Some(&1));
    }

    #[test]
    fn retry_detection_works() {
        let mut trace = ExecutionTrace::new("s", "t");
        let id1 = trace.record_start("web_search", CauseReason::UserRequest, None, 1);
        trace.record_end(id1, false, Some("timeout"));

        let id2 = trace.record_start("web_search", CauseReason::Fallback, None, 2);
        trace.record_end(id2, true, None);

        assert!(!trace.nodes()[0].was_retry);
        assert!(trace.nodes()[1].was_retry);
    }

    #[test]
    fn json_serialization_is_complete() {
        let mut trace = ExecutionTrace::new("session-abc", "turn-xyz");
        let id = trace.record_start("gw_gmail_send", CauseReason::LlmDecision, None, 1);
        trace.record_end(id, true, None);

        let json = trace.to_json();
        assert_eq!(json["total_calls"].as_u64().unwrap(), 1);
        assert_eq!(json["session_id"].as_str().unwrap(), "session-abc");
        assert!(json["nodes"].as_array().unwrap().len() == 1);
    }
}
