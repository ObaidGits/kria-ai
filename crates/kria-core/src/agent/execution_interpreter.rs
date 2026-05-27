//! Execution Result Interpreter — Generic tool result analysis.
//!
//! This module provides a universal result interpretation layer that works
//! for ANY tool without tool-specific hardcoding.
//!
//! It extracts key facts from tool results:
//! - Outcome classification (Success, Partial, Failure)
//! - Key facts (counts, status, resources)
//! - Asset counts (items found, created, processed)
//! - Brief description (what happened)
//!
//! This is used by the synthesis layer to provide LLM with structured facts
//! instead of raw tool payloads, enabling natural-language response synthesis.

use serde_json::Value;

/// Classification of execution outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionOutcome {
    /// Tool completed successfully and achieved the goal.
    Success,
    /// Tool completed but with limitations or partial results.
    Partial,
    /// Tool execution failed or produced no results.
    Failure,
}

/// Interpreted execution facts for synthesis layer.
#[derive(Debug, Clone)]
pub struct ExecutionInterpretation {
    /// Overall outcome classification.
    pub outcome: ExecutionOutcome,
    /// Key facts extracted from result (e.g., "12 containers found").
    pub key_facts: Vec<String>,
    /// Number of items/assets involved (containers, files, results, etc.).
    pub asset_count: Option<u64>,
    /// Brief human-readable status.
    pub status: String,
    /// Execution duration in seconds (if available).
    pub duration_secs: Option<f64>,
}

impl ExecutionInterpretation {
    /// Create a success interpretation with facts.
    pub fn success(facts: Vec<String>, asset_count: Option<u64>) -> Self {
        let status = if facts.is_empty() {
            "Executed successfully".into()
        } else {
            facts.get(0).cloned().unwrap_or_default()
        };

        Self {
            outcome: ExecutionOutcome::Success,
            key_facts: facts,
            asset_count,
            status,
            duration_secs: None,
        }
    }

    /// Create a partial interpretation.
    pub fn partial(reason: String, facts: Vec<String>, asset_count: Option<u64>) -> Self {
        Self {
            outcome: ExecutionOutcome::Partial,
            key_facts: facts,
            asset_count,
            status: reason,
            duration_secs: None,
        }
    }

    /// Create a failure interpretation.
    pub fn failure(reason: String) -> Self {
        Self {
            outcome: ExecutionOutcome::Failure,
            key_facts: Vec::new(),
            asset_count: None,
            status: reason,
            duration_secs: None,
        }
    }

    /// Add duration information.
    pub fn with_duration(mut self, secs: f64) -> Self {
        self.duration_secs = Some(secs);
        self
    }
}

/// Interpret a tool result generically.
///
/// This function applies heuristics that work for most tools:
/// - Count arrays/items
/// - Extract count/status fields
/// - Check for error/failure markers
/// - Infer success/partial/failure
pub fn interpret_tool_result(
    _tool_name: &str,
    result: &Value,
    success: bool,
) -> ExecutionInterpretation {
    if !success {
        // Execution failed — extract error message
        let error_msg = result
            .get("error")
            .and_then(|v| v.as_str())
            .or_else(|| result.as_str())
            .unwrap_or("Execution failed");
        let clipped: String = error_msg.chars().take(150).collect();
        return ExecutionInterpretation::failure(clipped);
    }

    // ── Success path: extract facts generically ───────────────────────────

    let mut facts = Vec::new();
    let mut asset_count = None;
    let mut duration_secs = None;

    // Try to extract structured fields
    let payload = result.get("data").unwrap_or(result);

    // Check for elapsed time
    if let Some(ms) = payload.get("elapsed_ms").and_then(|v| v.as_u64()) {
        duration_secs = Some(ms as f64 / 1000.0);
    } else if let Some(ms) = result.get("elapsed_ms").and_then(|v| v.as_u64()) {
        duration_secs = Some(ms as f64 / 1000.0);
    }

    // Count items in common collection fields
    for key in &[
        "items",
        "results",
        "messages",
        "events",
        "files",
        "rows",
        "containers",
        "entries",
    ] {
        if let Some(arr) = payload.get(key).and_then(|v| v.as_array()) {
            let count = arr.len() as u64;
            asset_count = Some(count);
            let item_name = pluralize_key(key);
            facts.push(format!("Found {} {}", count, item_name));
            break;
        }
    }

    // Check for array at root
    if asset_count.is_none() {
        if let Some(arr) = payload.as_array() {
            let count = arr.len() as u64;
            asset_count = Some(count);
            facts.push(format!("Found {} items", count));
        }
    }

    // Extract count field
    if let Some(count) = payload.get("count").and_then(|v| v.as_u64()) {
        if asset_count.is_none() {
            asset_count = Some(count);
            facts.push(format!("Count: {}", count));
        }
    }

    // Extract status field
    if let Some(status) = payload.get("status").and_then(|v| v.as_str()) {
        if !status.is_empty() && !status.to_lowercase().contains("success") {
            facts.push(format!("Status: {}", status));
        }
    }

    // Extract message field
    if let Some(msg) = payload.get("message").and_then(|v| v.as_str()) {
        if !msg.is_empty() && !msg.to_lowercase().contains("success") {
            let clipped: String = msg.chars().take(100).collect();
            facts.push(clipped);
        }
    }

    // Infer outcome
    let outcome = if let Some(count) = asset_count {
        if count > 0 {
            ExecutionOutcome::Success
        } else {
            ExecutionOutcome::Partial
        }
    } else if facts.is_empty() {
        // Completed but no obvious facts
        facts.push("Completed successfully".into());
        ExecutionOutcome::Success
    } else {
        ExecutionOutcome::Success
    };

    let status = if facts.is_empty() {
        "Executed successfully".into()
    } else {
        facts.get(0).cloned().unwrap_or_default()
    };

    let mut interp = ExecutionInterpretation {
        outcome,
        key_facts: facts,
        asset_count,
        status,
        duration_secs,
    };

    if let Some(secs) = duration_secs {
        interp = interp.with_duration(secs);
    }

    interp
}

/// Convert plural key names to human-readable form.
fn pluralize_key(key: &str) -> &'static str {
    match key {
        "items" => "items",
        "results" => "results",
        "messages" => "messages",
        "events" => "events",
        "files" => "files",
        "rows" => "rows",
        "containers" => "containers",
        "entries" => "entries",
        _ => "items",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpret_failure() {
        let result = serde_json::json!({ "error": "Connection refused" });
        let interp = interpret_tool_result("execute_bash", &result, false);
        assert_eq!(interp.outcome, ExecutionOutcome::Failure);
        assert!(interp.status.contains("Connection refused"));
    }

    #[test]
    fn test_interpret_array_result() {
        let result = serde_json::json!([
            { "id": "1", "name": "container1" },
            { "id": "2", "name": "container2" },
        ]);
        let interp = interpret_tool_result("docker", &result, true);
        assert_eq!(interp.outcome, ExecutionOutcome::Success);
        assert_eq!(interp.asset_count, Some(2));
        assert!(interp.key_facts.iter().any(|f| f.contains("2 items")));
    }

    #[test]
    fn test_interpret_structured_data() {
        let result = serde_json::json!({
            "status": "complete",
            "results": [1, 2, 3, 4, 5],
            "elapsed_ms": 1500
        });
        let interp = interpret_tool_result("search", &result, true);
        assert_eq!(interp.outcome, ExecutionOutcome::Success);
        assert_eq!(interp.asset_count, Some(5));
        assert_eq!(interp.duration_secs, Some(1.5));
    }

    #[test]
    fn test_interpret_no_items() {
        let result = serde_json::json!({ "status": "completed", "data": {} });
        let interp = interpret_tool_result("tool", &result, true);
        assert_eq!(interp.outcome, ExecutionOutcome::Success);
        assert!(interp.key_facts.iter().any(|f| f.contains("Completed")));
    }
}
