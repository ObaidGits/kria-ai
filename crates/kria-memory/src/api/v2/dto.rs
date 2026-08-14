//! v2 request/response DTOs (Data Transfer Objects).
//!
//! These types are the wire-format contract between callers (Tauri adapter,
//! Axum route, tests) and the domain core (design §8.2, F3.9). They are
//! exclusively responsible for serialization shape; no domain logic lives here.
//!
//! All structs derive `Serialize` + `Deserialize` so they cross the Tauri IPC
//! bridge and are embeddable in JSON API responses without conversion.

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Request DTO
// ─────────────────────────────────────────────────────────────────────────────

/// Inbound request envelope for all v2 graph query operations.
///
/// The caller fills `operation` and `params_json` with the operation name
/// (e.g. `"search"`, `"neighborhood"`) and its typed parameter bag. The
/// remaining fields carry cursor pagination, revision binding, and optional
/// deadline override.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRequestV2 {
    /// Operation name (e.g. `"search"`, `"neighborhood"`, `"path"`,
    /// `"aggregate"`, `"trace.get"`, …). Unknown operations return
    /// `MemoryApiErrorV2::Unsupported`.
    pub operation: String,

    /// Operation-specific parameter bag serialized as a JSON value.
    /// Each operation defines its own schema; the domain validates it before
    /// execution (unknown required fields → `InvalidRequest`).
    pub params_json: serde_json::Value,

    /// Client-pinned graph revision. When present the domain enforces that
    /// the authority revision still matches; a drift returns
    /// `MemoryApiErrorV2::Revision` or `MemoryApiErrorV2::Refetch`.
    pub revision: Option<i64>,

    /// Caller-declared schema version (e.g. `"2.0"`). Unknown required
    /// versions return `MemoryApiErrorV2::Unsupported`.
    pub schema_version: String,

    /// Hash of the effective policy under which this request was authorized.
    /// Used to detect policy drift between cached responses and current state.
    pub policy_hash: Option<String>,

    /// Opaque pagination cursor produced by a previous response. The domain
    /// validates schema/query/policy/revision/sort/expiry binding and rejects
    /// expired or incompatible cursors (design §5.2).
    pub cursor: Option<String>,

    /// Optional per-request deadline override in milliseconds. When absent
    /// the operation uses its default deadline from [`super::contract::OperationLimits`].
    /// Values above the hard cap are clamped to the cap.
    pub deadline_ms: Option<u64>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Response DTOs
// ─────────────────────────────────────────────────────────────────────────────

/// Total-count semantics for paginated result sets (design §8.1).
///
/// The domain never invents a count it cannot prove:
/// - `Exact` — the authority returned a precise count within the policy filter.
/// - `AtLeast` — a lower bound is known but the full count was not computed
///   (e.g. cursor pagination stopped early).
/// - `Estimate` — a statistical approximation (e.g. FTS5 `matchinfo`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum TotalSemantics {
    Exact(u64),
    AtLeast(u64),
    Estimate(u64),
}

/// A single non-blocking advisory attached to a response.
///
/// Warnings do not prevent the response from being used but signal degraded
/// conditions the caller should surface (e.g. stale index, partial strategy,
/// expiring cursor).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiWarning {
    /// Stable machine-readable code (e.g. `"stale_index"`, `"partial_strategy"`).
    pub code: String,
    /// Human-readable description safe to display (no hidden data).
    pub message: String,
}

/// Degradation severity level for a response or capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradationLevel {
    /// Some strategies are unavailable but the result is still useful.
    Partial,
    /// Most strategies are unavailable; the result may be significantly
    /// incomplete.
    Degraded,
    /// All non-trivial strategies are offline; only the bare authority floor
    /// (SQLite read, policy) is available.
    Offline,
}

/// Structured degradation information attached to a response when the result
/// was produced with reduced capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DegradationInfo {
    /// Severity of the degradation.
    pub level: DegradationLevel,
    /// Names of retrieval/index strategies that were unavailable for this
    /// request (e.g. `["vector", "graph"]`).
    pub unavailable_strategies: Vec<String>,
    /// Human-readable reason safe to display (no hidden scope).
    pub reason: String,
}

/// Outbound response envelope for all v2 graph query operations.
///
/// Mirrors the envelope shape described in design §8 (`schemaVersion`,
/// `revision`, `policyHash`, `capabilitiesVersion`, …) in a Rust-native form.
/// The adapter serializes this to JSON/Tauri IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphResponseV2 {
    /// Schema version of this response (e.g. `"2.0"`). Clients must reject
    /// unknown future versions to avoid silently misinterpreting new fields.
    pub schema_version: String,

    /// Authority graph revision at which this response was produced. Clients
    /// pin this for patch / cursor validation.
    pub revision: i64,

    /// Hash of the query that produced these results. Used by the client to
    /// validate that a cursor or patch refetch matches the original request.
    pub query_hash: String,

    /// Paginated result items serialized as JSON values. Item shape is
    /// operation-specific; see operation DTO definitions in sibling modules.
    pub items: Vec<serde_json::Value>,

    /// Total count of matching items in the authority store (may be bounded
    /// or estimated depending on the operation).
    pub total_count: TotalSemantics,

    /// Whether the result was truncated at the operation's hard page cap.
    pub truncated: bool,

    /// Human-readable truncation reason when `truncated` is `true` (e.g.
    /// `"page_cap_exceeded"`, `"deadline_exceeded"`). `None` when not truncated.
    pub truncation_reason: Option<String>,

    /// Opaque cursor the client may use to fetch the next page. `None` when
    /// all results have been returned. Expired/incompatible cursors are
    /// rejected by the domain (design §5.2).
    pub recovery_cursor: Option<String>,

    /// Advisory warnings for the caller (does not prevent use of the result).
    pub warnings: Vec<ApiWarning>,

    /// Degradation info when the result was produced under reduced capability.
    /// `None` when all requested strategies were available.
    pub degradation: Option<DegradationInfo>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Command result DTO
// ─────────────────────────────────────────────────────────────────────────────

/// Result returned after a successful durable command commit.
///
/// Carries the new graph revision, an audit-linkable command ID, idempotency
/// replay result (if the command was a replay), and the count of authority
/// records affected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommittedCommandV2 {
    /// Stable, globally unique ID for this command execution. Callers can
    /// use this to correlate with the audit log.
    pub command_id: String,

    /// New authority graph revision after the commit.
    pub revision: i64,

    /// Idempotency replay result when the command was a replay of a previous
    /// commit with the same idempotency key. `None` for a fresh commit.
    pub idempotency_result: Option<serde_json::Value>,

    /// Number of authority records (entities, memories, relationships, …)
    /// created, updated, or state-transitioned by this command.
    pub affected_count: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_semantics_exact_round_trips_json() {
        let t = TotalSemantics::Exact(5);
        let json = serde_json::to_value(&t).expect("serializes");
        assert_eq!(json["kind"], "exact");
        assert_eq!(json["value"], 5u64);
        let back: TotalSemantics = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back, TotalSemantics::Exact(5));
    }

    #[test]
    fn total_semantics_at_least_serializes_correctly() {
        let t = TotalSemantics::AtLeast(100);
        let json = serde_json::to_value(&t).expect("serializes");
        assert_eq!(json["kind"], "at_least");
        assert_eq!(json["value"], 100u64);
        let back: TotalSemantics = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back, TotalSemantics::AtLeast(100));
    }

    #[test]
    fn total_semantics_estimate_serializes_correctly() {
        let t = TotalSemantics::Estimate(1000);
        let json = serde_json::to_value(&t).expect("serializes");
        assert_eq!(json["kind"], "estimate");
        assert_eq!(json["value"], 1000u64);
        let back: TotalSemantics = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back, TotalSemantics::Estimate(1000));
    }

    #[test]
    fn graph_request_v2_round_trips_json() {
        let req = GraphRequestV2 {
            operation: "search".to_string(),
            params_json: serde_json::json!({"query": "project atlas", "page_size": 25}),
            revision: Some(42),
            schema_version: "2.0".to_string(),
            policy_hash: Some("abc123".to_string()),
            cursor: None,
            deadline_ms: Some(250),
        };
        let json = serde_json::to_string(&req).expect("serializes");
        let back: GraphRequestV2 = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.operation, "search");
        assert_eq!(back.revision, Some(42));
    }

    #[test]
    fn committed_command_v2_round_trips_json() {
        let cmd = CommittedCommandV2 {
            command_id: "cmd-1".to_string(),
            revision: 7,
            idempotency_result: None,
            affected_count: 3,
        };
        let json = serde_json::to_string(&cmd).expect("serializes");
        let back: CommittedCommandV2 = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.command_id, "cmd-1");
        assert_eq!(back.revision, 7);
        assert_eq!(back.affected_count, 3);
    }
}
