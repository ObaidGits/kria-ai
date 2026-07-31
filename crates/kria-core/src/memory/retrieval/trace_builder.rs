//! Trace header and item builder for the retrieval pipeline (design §6.4 step 5, task F3.5.5).
//!
//! Builds `RetrievalTraceRecord` (header) and `Vec<RetrievalTraceItem>` from pipeline outputs.
//!
//! # Design invariants
//! * Unauthorized items use OPAQUE record IDs — never expose hidden record IDs.
//! * Reason codes for unauthorized items are the constants from retrieval_gates::ReasonCode.
//! * All policy fields in the trace header must not reveal hidden record counts or IDs.

use crate::memory::retrieval::trace_store::{RetrievalTraceItem, RetrievalTraceRecord};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Opaque record ID used in trace items for unauthorized candidates.
/// Must not reveal any hidden identifier.
pub const OPAQUE_UNAUTHORIZED_RECORD_ID: &str = "opaque:unauthorized";

// ── Input types ───────────────────────────────────────────────────────────────

/// Inputs collected from all pipeline stages, used to build the trace.
#[derive(Debug)]
pub struct TraceInputs {
    // Header fields
    pub trace_id: String,
    pub response_id: Option<String>,
    pub task_id: Option<String>,
    pub query_hash: String,
    pub query_class: String,        // from classifier
    pub classifier_version: String, // from classifier
    pub profile_id: String,         // from rrf_profile
    pub graph_revision: Option<i64>,
    pub policy_hash: Option<String>,
    pub token_budget: usize,
    pub status: String,                   // "pending" initially
    pub degradation_json: Option<String>, // JSON: which strategies are unavailable
    pub embed_model_version: Option<String>,
    pub k_value: f32,
    pub availability_json: String, // from trace_store::availability_to_json
    pub weights_json: String,      // from trace_store::weights_to_json
    pub created_at: String,        // UTC RFC3339

    // Per-candidate items
    pub candidates: Vec<TraceItemInput>,
}

/// Input for one trace item.
#[derive(Debug)]
pub struct TraceItemInput {
    /// Semantic record ID — MUST be opaque for unauthorized candidates.
    pub record_id: String,
    /// Whether this candidate is unauthorized (forces opaque ID in output).
    pub is_unauthorized: bool,
    pub strategy: String,
    pub strategy_rank: Option<i64>,
    pub strategy_score: Option<f64>,
    pub weight: Option<f64>,
    pub rrf_contribution: Option<f64>,
    pub gate_disposition: String, // "included" | "excluded" | "filtered" | "unauthorized"
    pub reason_code: Option<String>,
    pub token_cost: Option<i64>,
    pub allocated_tokens: Option<i64>,
    pub injected_order: Option<i64>,
    pub goal_id: Option<String>,
    pub evidence_contribution: Option<f64>,
    pub memory_worth_contribution: Option<f64>,
}

// ── Builder functions ─────────────────────────────────────────────────────────

/// Build a `RetrievalTraceRecord` from trace inputs.
pub fn build_trace_record(inputs: &TraceInputs) -> RetrievalTraceRecord {
    RetrievalTraceRecord {
        trace_id: inputs.trace_id.clone(),
        response_id: inputs.response_id.clone(),
        task_id: inputs.task_id.clone(),
        query_hash: inputs.query_hash.clone(),
        query_class: inputs.query_class.clone(),
        classifier_version: inputs.classifier_version.clone(),
        profile_id: inputs.profile_id.clone(),
        graph_revision: inputs.graph_revision,
        policy_hash: inputs.policy_hash.clone(),
        token_budget: Some(inputs.token_budget as i64),
        status: inputs.status.clone(),
        degradation_json: inputs.degradation_json.clone(),
        embed_model_version: inputs.embed_model_version.clone(),
        k_value: inputs.k_value,
        availability_json: inputs.availability_json.clone(),
        weights_json: inputs.weights_json.clone(),
        // Aggregate contributions are zero at build time; callers update these
        // after fusion scoring completes.
        evidence_contribution: 0.0,
        memory_worth_contribution: 0.0,
        goal_contribution_total: 0.0,
        created_at: inputs.created_at.clone(),
    }
}

/// Build all `RetrievalTraceItem` rows from trace inputs.
///
/// Unauthorized candidates are redacted: their record_id is replaced with
/// `OPAQUE_UNAUTHORIZED_RECORD_ID` to ensure no hidden IDs are exposed.
/// In addition, strategy_rank and rrf_contribution are cleared for unauthorized
/// items since they would reveal ranking information about hidden records.
pub fn build_trace_items(inputs: &TraceInputs) -> Vec<RetrievalTraceItem> {
    inputs
        .candidates
        .iter()
        .map(|c| {
            let (record_id, strategy_rank, rrf_contribution) = if c.is_unauthorized {
                // Redact: opaque ID, no rank, no RRF contribution
                (OPAQUE_UNAUTHORIZED_RECORD_ID.to_string(), None, None)
            } else {
                (c.record_id.clone(), c.strategy_rank, c.rrf_contribution)
            };

            RetrievalTraceItem {
                trace_id: inputs.trace_id.clone(),
                record_id,
                strategy: c.strategy.clone(),
                strategy_rank,
                strategy_score: c.strategy_score,
                weight: c.weight,
                rrf_contribution,
                gate_disposition: Some(c.gate_disposition.clone()),
                reason_code: c.reason_code.clone(),
                token_cost: c.token_cost,
                allocated_tokens: c.allocated_tokens,
                injected_order: c.injected_order,
                goal_id: c.goal_id.clone(),
                evidence_contribution: c.evidence_contribution,
                memory_worth_contribution: c.memory_worth_contribution,
            }
        })
        .collect()
}

/// Build a degradation JSON string from a list of unavailable strategy names.
///
/// Format: `{"unavailable": ["graph", "goal"]}` or `{}` if all available.
pub fn build_degradation_json(unavailable_strategies: &[&str]) -> String {
    if unavailable_strategies.is_empty() {
        return "{}".to_string();
    }
    let names: Vec<String> = unavailable_strategies
        .iter()
        .map(|s| format!("\"{}\"", s))
        .collect();
    format!("{{\"unavailable\": [{}]}}", names.join(", "))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_inputs_with_candidates(candidates: Vec<TraceItemInput>) -> TraceInputs {
        TraceInputs {
            trace_id: "trace-test-1".to_string(),
            response_id: Some("resp-abc".to_string()),
            task_id: Some("task-xyz".to_string()),
            query_hash: "qhash123".to_string(),
            query_class: "exploratory".to_string(),
            classifier_version: "classifier-v2".to_string(),
            profile_id: "rrf-general-v1".to_string(),
            graph_revision: Some(42),
            policy_hash: Some("polhash-99".to_string()),
            token_budget: 4096,
            status: "pending".to_string(),
            degradation_json: None,
            embed_model_version: Some("fastembed-v1".to_string()),
            k_value: 60.0,
            availability_json: r#"{"fts": 1, "vector": 1}"#.to_string(),
            weights_json: r#"{"fts": 1.0, "vector": 1.2}"#.to_string(),
            created_at: "2024-01-15T10:30:00Z".to_string(),
            candidates,
        }
    }

    fn make_authorized_item(record_id: &str) -> TraceItemInput {
        TraceItemInput {
            record_id: record_id.to_string(),
            is_unauthorized: false,
            strategy: "fts".to_string(),
            strategy_rank: Some(1),
            strategy_score: Some(0.9),
            weight: Some(1.0),
            rrf_contribution: Some(0.016),
            gate_disposition: "included".to_string(),
            reason_code: None,
            token_cost: Some(100),
            allocated_tokens: Some(100),
            injected_order: Some(0),
            goal_id: None,
            evidence_contribution: Some(0.0),
            memory_worth_contribution: Some(0.0),
        }
    }

    fn make_unauthorized_item(record_id: &str) -> TraceItemInput {
        TraceItemInput {
            record_id: record_id.to_string(),
            is_unauthorized: true,
            strategy: "vector".to_string(),
            strategy_rank: Some(2),
            strategy_score: Some(0.85),
            weight: Some(1.2),
            rrf_contribution: Some(0.015),
            gate_disposition: "unauthorized".to_string(),
            reason_code: Some("unauthorized".to_string()),
            token_cost: None,
            allocated_tokens: None,
            injected_order: None,
            goal_id: None,
            evidence_contribution: None,
            memory_worth_contribution: None,
        }
    }

    // 1. build_trace_record_sets_all_header_fields
    #[test]
    fn build_trace_record_sets_all_header_fields() {
        let inputs = make_inputs_with_candidates(vec![]);
        let record = build_trace_record(&inputs);

        assert_eq!(record.trace_id, "trace-test-1");
        assert_eq!(record.response_id, Some("resp-abc".to_string()));
        assert_eq!(record.task_id, Some("task-xyz".to_string()));
        assert_eq!(record.query_hash, "qhash123");
        assert_eq!(record.query_class, "exploratory");
        assert_eq!(record.classifier_version, "classifier-v2");
        assert_eq!(record.profile_id, "rrf-general-v1");
        assert_eq!(record.graph_revision, Some(42));
        assert_eq!(record.policy_hash, Some("polhash-99".to_string()));
        assert_eq!(record.token_budget, Some(4096));
        assert_eq!(record.status, "pending");
        assert_eq!(record.degradation_json, None);
        assert_eq!(record.embed_model_version, Some("fastembed-v1".to_string()));
        assert!((record.k_value - 60.0_f32).abs() < 1e-5);
        assert_eq!(record.availability_json, r#"{"fts": 1, "vector": 1}"#);
        assert_eq!(record.weights_json, r#"{"fts": 1.0, "vector": 1.2}"#);
        assert_eq!(record.created_at, "2024-01-15T10:30:00Z");
    }

    // 2. build_trace_items_authorized_preserves_record_id
    #[test]
    fn build_trace_items_authorized_preserves_record_id() {
        let inputs =
            make_inputs_with_candidates(vec![make_authorized_item("real-record-uuid-abc")]);
        let items = build_trace_items(&inputs);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].record_id, "real-record-uuid-abc");
    }

    // 3. build_trace_items_unauthorized_uses_opaque_id
    #[test]
    fn build_trace_items_unauthorized_uses_opaque_id() {
        let inputs =
            make_inputs_with_candidates(vec![make_unauthorized_item("secret-hidden-id-999")]);
        let items = build_trace_items(&inputs);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].record_id, OPAQUE_UNAUTHORIZED_RECORD_ID);
        // The original hidden ID must not appear anywhere in the trace item
        assert_ne!(items[0].record_id, "secret-hidden-id-999");
    }

    // 4. build_trace_items_unauthorized_has_no_rrf_score
    #[test]
    fn build_trace_items_unauthorized_has_no_rrf_score() {
        let inputs = make_inputs_with_candidates(vec![make_unauthorized_item("secret-id")]);
        let items = build_trace_items(&inputs);

        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].strategy_rank, None,
            "unauthorized item must have no strategy_rank"
        );
        assert_eq!(
            items[0].rrf_contribution, None,
            "unauthorized item must have no rrf_contribution"
        );
    }

    // 5. build_trace_items_length_equals_input
    #[test]
    fn build_trace_items_length_equals_input() {
        let candidates = vec![
            make_authorized_item("rec-1"),
            make_unauthorized_item("hidden-1"),
            make_authorized_item("rec-2"),
            make_unauthorized_item("hidden-2"),
            make_authorized_item("rec-3"),
        ];
        let inputs = make_inputs_with_candidates(candidates);
        let items = build_trace_items(&inputs);

        assert_eq!(items.len(), 5);
    }

    // 6. build_degradation_json_empty_is_empty_object
    #[test]
    fn build_degradation_json_empty_is_empty_object() {
        let json = build_degradation_json(&[]);
        assert_eq!(json, "{}");
    }

    // 7. build_degradation_json_some_unavailable
    #[test]
    fn build_degradation_json_some_unavailable() {
        let json = build_degradation_json(&["graph", "goal"]);
        assert!(json.contains("graph"), "JSON must contain 'graph'");
        assert!(json.contains("goal"), "JSON must contain 'goal'");
        assert!(
            json.contains("unavailable"),
            "JSON must have 'unavailable' key"
        );
    }

    // 8. opaque_id_constant_has_colon_prefix
    #[test]
    fn opaque_id_constant_has_colon_prefix() {
        assert!(
            OPAQUE_UNAUTHORIZED_RECORD_ID.starts_with("opaque:"),
            "OPAQUE_UNAUTHORIZED_RECORD_ID must start with 'opaque:' to signal it is not a UUID"
        );
    }
}
