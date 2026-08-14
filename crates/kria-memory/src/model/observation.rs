//! Observation/trace value objects: [`ToolObservation`], [`RetrievalTrace`],
//! and [`RetrievalTraceItem`] (design §4.3, task F2.1.1).
//!
//! A [`ToolObservation`] is a start/completion-linked outcome record for a tool
//! invocation — **never an authorization grant** (glossary). A
//! [`RetrievalTrace`] captures the provenance of one retrieval response, and a
//! [`RetrievalTraceItem`] is one `(trace, record, strategy)` candidate row.
//! `graph_revision` is carried as a [`GraphRevision`] value object even though
//! the schema stores it as a plain INTEGER (no hard FK).

use serde::{Deserialize, Serialize};

use super::{
    EventId, GoalId, GraphRevision, PolicyPartition, RetrievalTraceId, SourceId, ToolObservationId,
    UtcTimestamp,
};

/// A start/completion-linked tool outcome record (`tool_observations` row —
/// design §4.3). Unique per `invocation_id`. NOT an authorization grant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolObservation {
    /// Stable identity (`tool_observations.id`).
    pub id: ToolObservationId,
    /// The invocation this observation records (unique).
    pub invocation_id: String,
    /// The kind of tool (native/mcp/openclaw/sidecar).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_kind: Option<String>,
    /// The tool identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<String>,
    /// The tool version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,
    /// The capability exercised (never granted by this row).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,
    /// The outcome disposition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// The goal this invocation served, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<GoalId>,
    /// The environment class the tool ran in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_class: Option<String>,
    /// A privacy-safe fingerprint of the input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_fingerprint: Option<String>,
    /// A summary of the result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_summary: Option<String>,
    /// The error class, if the invocation failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
    /// Observed latency in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// Retry count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_count: Option<u32>,
    /// Recovery action taken, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_action: Option<String>,
    /// Policy partition.
    pub policy: PolicyPartition,
    /// Contributing source id.
    pub source_id: SourceId,
    /// Effective policy version tag.
    pub policy_version: String,
    /// The invocation start event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_event_id: Option<EventId>,
    /// The invocation completion event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_event_id: Option<EventId>,
    /// Transaction-time creation instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<UtcTimestamp>,
}

/// The provenance of one retrieval response (`retrieval_traces` row — design
/// §4.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievalTrace {
    /// Stable identity (`retrieval_traces.id`).
    pub id: RetrievalTraceId,
    /// The response this trace explains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    /// The task this retrieval served.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// A stable hash of the query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_hash: Option<String>,
    /// The classified query class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_class: Option<String>,
    /// The query classifier version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifier_version: Option<String>,
    /// The active RRF profile id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// The authority revision the retrieval read at (plain INTEGER in schema).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_revision: Option<GraphRevision>,
    /// The effective policy hash applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_hash: Option<String>,
    /// The token budget for the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u32>,
    /// Retrieval status (empty/partial/etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Degradation report (validated JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degradation_json: Option<String>,
    /// The embedding model version used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed_model_version: Option<String>,
    /// The rerank model version used, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rerank_model_version: Option<String>,
    /// Transaction-time creation instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<UtcTimestamp>,
}

/// One candidate row in a retrieval trace (`retrieval_trace_items` row — design
/// §4.3). Keyed by `(trace_id, record_id, strategy)`. Unauthorized items use
/// opaque reason rows without hidden record IDs (write-path redaction).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievalTraceItem {
    /// The owning trace.
    pub trace_id: RetrievalTraceId,
    /// The candidate record id (opaque here; endpoint validated at write time).
    pub record_id: String,
    /// The strategy that surfaced the candidate.
    pub strategy: String,
    /// Per-strategy rank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_rank: Option<u32>,
    /// Per-strategy score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_score: Option<f64>,
    /// Fusion weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    /// Reciprocal-rank-fusion contribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rrf_contribution: Option<f64>,
    /// The gate disposition (accepted/rejected/…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_disposition: Option<String>,
    /// An opaque reason code (used for redacted/unauthorized rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    /// The token cost of the candidate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_cost: Option<u32>,
    /// Tokens allocated to the candidate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allocated_tokens: Option<u32>,
    /// The final injected order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub injected_order: Option<u32>,
    /// The goal this candidate served, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<GoalId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_observation_serde_roundtrips() {
        let t = ToolObservation {
            id: ToolObservationId::new_v7(),
            invocation_id: "inv-1".into(),
            tool_kind: Some("native".into()),
            tool_id: Some("file_ops".into()),
            tool_version: Some("1".into()),
            capability_id: Some("fs.read".into()),
            outcome: Some("ok".into()),
            goal_id: None,
            environment_class: Some("local".into()),
            input_fingerprint: None,
            result_summary: Some("read 3 files".into()),
            error_class: None,
            latency_ms: Some(42),
            retry_count: Some(0),
            recovery_action: None,
            policy: PolicyPartition::new("user", "chat", 0).unwrap(),
            source_id: SourceId::new_v7(),
            policy_version: "p1".into(),
            start_event_id: Some(EventId::new_v7()),
            completion_event_id: Some(EventId::new_v7()),
            created_at: Some(UtcTimestamp::now()),
        };
        let back: ToolObservation =
            serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn retrieval_trace_roundtrips() {
        let tr = RetrievalTrace {
            id: RetrievalTraceId::new_v7(),
            response_id: Some("resp-1".into()),
            task_id: None,
            query_hash: Some("h".into()),
            query_class: Some("factual".into()),
            classifier_version: Some("v1".into()),
            profile_id: Some("p".into()),
            graph_revision: Some(GraphRevision::new(7)),
            policy_hash: Some("ph".into()),
            token_budget: Some(2048),
            status: Some("ok".into()),
            degradation_json: None,
            embed_model_version: Some("all-MiniLM-L6-v2".into()),
            rerank_model_version: None,
            created_at: Some(UtcTimestamp::now()),
        };
        let back: RetrievalTrace =
            serde_json::from_str(&serde_json::to_string(&tr).unwrap()).unwrap();
        assert_eq!(back, tr);

        let item = RetrievalTraceItem {
            trace_id: tr.id.clone(),
            record_id: RetrievalTraceId::new_v7().into_string(),
            strategy: "vector".into(),
            strategy_rank: Some(1),
            strategy_score: Some(0.9),
            weight: Some(1.0),
            rrf_contribution: Some(0.5),
            gate_disposition: Some("accepted".into()),
            reason_code: None,
            token_cost: Some(120),
            allocated_tokens: Some(120),
            injected_order: Some(0),
            goal_id: None,
        };
        let back: RetrievalTraceItem =
            serde_json::from_str(&serde_json::to_string(&item).unwrap()).unwrap();
        assert_eq!(back, item);
    }
}
