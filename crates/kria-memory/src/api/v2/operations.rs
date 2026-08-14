//! v2 operation catalogue: kinds, defaults, and stub router (design §8, F3.9).
//!
//! This module defines:
//! - [`OperationKind`] — the exhaustive set of supported operation names.
//! - [`OperationDefaults`] — const-fn defaults and hard maxima for each operation.
//! - [`OperationRouter`] — the stub wire-contract router that validates the
//!   request envelope and returns a correctly-shaped [`GraphResponseV2`].
//!
//! The router is intentionally a **stub**: it establishes the wire contract and
//! validates the envelope (unknown operation → `Unsupported`, deadline above
//! hard cap → `Limit`) but delegates no real DB queries. Actual query
//! implementations are introduced in later tasks.

use super::contract::{CallerContext, OperationLimits};
use super::dto::{GraphRequestV2, GraphResponseV2, TotalSemantics};
use super::error::MemoryApiErrorV2;

// ─────────────────────────────────────────────────────────────────────────────
// OperationKind
// ─────────────────────────────────────────────────────────────────────────────

/// All v2 memory operations that may appear on the wire as `request.operation`.
///
/// Variants map 1-to-1 to the operation name strings defined in design §8.
/// `OperationKind::from_str` is the canonical parser; callers must not match
/// operation strings by hand.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OperationKind {
    /// Full-corpus text/vector/graph search (design §8.1 `search`).
    Search,
    /// Local neighborhood traversal from one or more seed nodes (design §8.1
    /// `neighborhood`).
    Neighborhood,
    /// Shortest / bounded-cost path between two nodes (design §8.1 `path`).
    Path,
    /// Retrieval-trace fetch — why a record was (or wasn't) injected into a
    /// response (design §8.1 `trace.get`).
    TraceGet,
    /// Aggregation/faceting over the authorized corpus (design §8.1
    /// `aggregate`).
    Aggregate,
    /// Relationship or record predictions for a seed entity (design §8.1
    /// `predict`).
    Predict,
    /// Valid-time or transaction-time diff between two authority revisions
    /// (design §8.1 `temporal.diff`).
    TemporalDiff,
    /// List of authority patches available for cursor / revision binding
    /// (design §8.1 `patch.list`).
    PatchList,
    /// Seven-section lazy inspector for a single record or entity (design §8.1
    /// `inspect`).
    Inspect,
}

impl OperationKind {
    /// Parse an operation name from its wire-format string.
    ///
    /// Returns `None` for any string that is not a recognized v2 operation
    /// name. The router treats `None` as `MemoryApiErrorV2::Unsupported`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use crate::api::v2::operations::OperationKind;
    ///
    /// assert_eq!(OperationKind::from_str("search"), Some(OperationKind::Search));
    /// assert_eq!(OperationKind::from_str("trace.get"), Some(OperationKind::TraceGet));
    /// assert_eq!(OperationKind::from_str("SEARCH"), None);   // case-sensitive
    /// assert_eq!(OperationKind::from_str("unknown"), None);
    /// ```
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "search" => Some(Self::Search),
            "neighborhood" => Some(Self::Neighborhood),
            "path" => Some(Self::Path),
            "trace.get" => Some(Self::TraceGet),
            "aggregate" => Some(Self::Aggregate),
            "predict" => Some(Self::Predict),
            "temporal.diff" => Some(Self::TemporalDiff),
            "patch.list" => Some(Self::PatchList),
            "inspect" => Some(Self::Inspect),
            _ => None,
        }
    }

    /// The canonical wire-format name for this operation kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Neighborhood => "neighborhood",
            Self::Path => "path",
            Self::TraceGet => "trace.get",
            Self::Aggregate => "aggregate",
            Self::Predict => "predict",
            Self::TemporalDiff => "temporal.diff",
            Self::PatchList => "patch.list",
            Self::Inspect => "inspect",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// OperationDefaults
// ─────────────────────────────────────────────────────────────────────────────

/// Compile-time default values and hard maxima for each v2 operation
/// (design §8.1).
///
/// These are `const fn` so they can be used in array initialisers, test
/// assertions, and match guard expressions without a runtime call. Runtime
/// configuration may only lower these values, never raise them above the hard
/// caps in [`OperationLimits`].
pub struct OperationDefaults;

impl OperationDefaults {
    /// Default number of items returned per `search` page when the caller does
    /// not specify `params_json.page_size`.
    pub const fn search_page_size() -> u32 {
        50
    }

    /// Default maximum graph hops for a `neighborhood` traversal when the
    /// caller does not specify a hop limit.
    ///
    /// Must not exceed [`OperationLimits::MAX_DEPTH`] (3).
    pub const fn neighborhood_max_hops() -> u8 {
        2
    }

    /// Default maximum graph hops for a `path` query.
    ///
    /// Equals [`OperationLimits::MAX_DEPTH`] (3) — paths are bounded to the
    /// same hard cap as neighborhood depth.
    pub const fn path_max_hops() -> u8 {
        3
    }

    /// Default maximum items processed by an `aggregate` operation.
    pub const fn aggregate_max_items() -> u32 {
        100
    }

    /// Number of lazy sections served by an `inspect` response (Identity,
    /// Truth, Evidence, Relationships, Use, History, Actions).
    pub const fn inspect_sections() -> u8 {
        7
    }

    /// Default maximum prediction candidates returned by a `predict` operation.
    pub const fn predict_max_candidates() -> u32 {
        20
    }

    /// Default maximum change rows returned by a `temporal.diff` operation.
    pub const fn temporal_diff_max_changes() -> u32 {
        200
    }

    /// Default maximum patch records returned by a `patch.list` operation.
    pub const fn patch_list_max_patches() -> u32 {
        100
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Hash helper (no external dependency required)
// ─────────────────────────────────────────────────────────────────────────────

/// Produce a deterministic, compact hex-encoded hash of an operation name
/// string for use in response `query_hash` fields.
///
/// Uses a simple FNV-1a 64-bit hash — sufficient for the stub wire contract.
/// The hash must be stable for the same input across runs.
fn fnv1a_hex(s: &str) -> String {
    const FNV_OFFSET: u64 = 14_695_981_039_346_656_037;
    const FNV_PRIME: u64 = 1_099_511_628_211;
    let mut hash: u64 = FNV_OFFSET;
    for byte in s.bytes() {
        hash ^= u64(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{:016x}", hash)
}

#[allow(clippy::cast_lossless)]
fn u64(b: u8) -> u64 {
    b as u64
}

// ─────────────────────────────────────────────────────────────────────────────
// OperationRouter
// ─────────────────────────────────────────────────────────────────────────────

/// Stub operation router that validates the request envelope and dispatches to
/// the appropriate (currently stub) operation handler.
///
/// # Contract
///
/// - Unknown `operation` string → [`MemoryApiErrorV2::Unsupported`].
/// - `deadline_ms` above [`OperationLimits::DEADLINE_MS`] → [`MemoryApiErrorV2::Limit`].
/// - Valid requests → [`GraphResponseV2`] with `schema_version = "2.0"`,
///   `revision = 0`, `query_hash` derived from the operation name, empty items,
///   `truncated = false`, no warnings, no cursor, and
///   `total_count = TotalSemantics::Exact(0)`.
///
/// All semantic query work (FTS5, vector, graph traversal, …) is introduced
/// in later tasks; this module only establishes the wire envelope.
pub struct OperationRouter;

impl OperationRouter {
    /// Validate the request envelope and return a stub [`GraphResponseV2`].
    ///
    /// # Errors
    ///
    /// - [`MemoryApiErrorV2::Unsupported`] when `request.operation` is not a
    ///   recognized v2 operation name.
    /// - [`MemoryApiErrorV2::Limit`] when `request.deadline_ms` exceeds
    ///   [`OperationLimits::DEADLINE_MS`].
    pub fn dispatch(
        _caller: &CallerContext,
        request: &GraphRequestV2,
    ) -> Result<GraphResponseV2, MemoryApiErrorV2> {
        // 1. Parse the operation name.
        let kind = OperationKind::from_str(&request.operation).ok_or_else(|| {
            MemoryApiErrorV2::Unsupported {
                feature: request.operation.clone(),
            }
        })?;

        // 2. Validate deadline_ms hard cap.
        if let Some(deadline_ms) = request.deadline_ms {
            if deadline_ms > OperationLimits::DEADLINE_MS {
                return Err(MemoryApiErrorV2::Limit {
                    operation: request.operation.clone(),
                    limit: format!(
                        "deadline_ms {} exceeds hard cap {}",
                        deadline_ms,
                        OperationLimits::DEADLINE_MS
                    ),
                });
            }
        }

        // 3. Build a correctly-shaped stub response.
        //    Actual query logic is introduced in subsequent tasks.
        let query_hash = fnv1a_hex(kind.as_str());

        Ok(GraphResponseV2 {
            schema_version: "2.0".to_string(),
            revision: 0,
            query_hash,
            items: Vec::new(),
            total_count: TotalSemantics::Exact(0),
            truncated: false,
            truncation_reason: None,
            recovery_cursor: None,
            warnings: Vec::new(),
            degradation: None,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::v2::contract::TransportKind;
    use serde_json::json;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_caller() -> CallerContext {
        CallerContext {
            caller_id: "test-user".to_string(),
            namespace: "personal".to_string(),
            scope: "".to_string(),
            sensitivity: 0,
            policy_version: "v1".to_string(),
            transport: TransportKind::Internal,
        }
    }

    fn make_request(operation: &str) -> GraphRequestV2 {
        GraphRequestV2 {
            operation: operation.to_string(),
            params_json: json!({}),
            revision: None,
            schema_version: "2.0".to_string(),
            policy_hash: None,
            cursor: None,
            deadline_ms: None,
        }
    }

    // ── OperationKind::from_str ───────────────────────────────────────────────

    #[test]
    fn from_str_search_returns_search() {
        assert_eq!(
            OperationKind::from_str("search"),
            Some(OperationKind::Search)
        );
    }

    #[test]
    fn from_str_neighborhood_returns_neighborhood() {
        assert_eq!(
            OperationKind::from_str("neighborhood"),
            Some(OperationKind::Neighborhood)
        );
    }

    #[test]
    fn from_str_path_returns_path() {
        assert_eq!(OperationKind::from_str("path"), Some(OperationKind::Path));
    }

    #[test]
    fn from_str_trace_get_returns_trace_get() {
        assert_eq!(
            OperationKind::from_str("trace.get"),
            Some(OperationKind::TraceGet)
        );
    }

    #[test]
    fn from_str_aggregate_returns_aggregate() {
        assert_eq!(
            OperationKind::from_str("aggregate"),
            Some(OperationKind::Aggregate)
        );
    }

    #[test]
    fn from_str_predict_returns_predict() {
        assert_eq!(
            OperationKind::from_str("predict"),
            Some(OperationKind::Predict)
        );
    }

    #[test]
    fn from_str_temporal_diff_returns_temporal_diff() {
        assert_eq!(
            OperationKind::from_str("temporal.diff"),
            Some(OperationKind::TemporalDiff)
        );
    }

    #[test]
    fn from_str_patch_list_returns_patch_list() {
        assert_eq!(
            OperationKind::from_str("patch.list"),
            Some(OperationKind::PatchList)
        );
    }

    #[test]
    fn from_str_inspect_returns_inspect() {
        assert_eq!(
            OperationKind::from_str("inspect"),
            Some(OperationKind::Inspect)
        );
    }

    #[test]
    fn from_str_unknown_returns_none() {
        assert_eq!(OperationKind::from_str("unknown_op"), None);
    }

    #[test]
    fn from_str_is_case_sensitive_uppercase_returns_none() {
        assert_eq!(OperationKind::from_str("SEARCH"), None);
        assert_eq!(OperationKind::from_str("Search"), None);
    }

    #[test]
    fn from_str_empty_string_returns_none() {
        assert_eq!(OperationKind::from_str(""), None);
    }

    // ── OperationDefaults ─────────────────────────────────────────────────────

    #[test]
    fn search_page_size_is_50() {
        assert_eq!(OperationDefaults::search_page_size(), 50);
    }

    #[test]
    fn neighborhood_max_hops_is_2() {
        assert_eq!(OperationDefaults::neighborhood_max_hops(), 2);
    }

    #[test]
    fn path_max_hops_is_3() {
        assert_eq!(OperationDefaults::path_max_hops(), 3);
    }

    #[test]
    fn path_max_hops_matches_operation_limits_max_depth() {
        // path_max_hops must equal MAX_DEPTH (3) per design §8.1.
        assert_eq!(
            OperationDefaults::path_max_hops() as u32,
            OperationLimits::MAX_DEPTH as u32
        );
    }

    #[test]
    fn aggregate_max_items_is_100() {
        assert_eq!(OperationDefaults::aggregate_max_items(), 100);
    }

    #[test]
    fn inspect_sections_is_7() {
        assert_eq!(OperationDefaults::inspect_sections(), 7);
    }

    #[test]
    fn predict_max_candidates_is_20() {
        assert_eq!(OperationDefaults::predict_max_candidates(), 20);
    }

    #[test]
    fn temporal_diff_max_changes_is_200() {
        assert_eq!(OperationDefaults::temporal_diff_max_changes(), 200);
    }

    #[test]
    fn patch_list_max_patches_is_100() {
        assert_eq!(OperationDefaults::patch_list_max_patches(), 100);
    }

    // ── OperationRouter::dispatch — envelope errors ──────────────────────────

    #[test]
    fn dispatch_unknown_operation_returns_unsupported() {
        let caller = make_caller();
        let req = make_request("totally_unknown_op");
        let result = OperationRouter::dispatch(&caller, &req);
        assert!(
            matches!(result, Err(MemoryApiErrorV2::Unsupported { ref feature }) if feature == "totally_unknown_op"),
            "expected Unsupported error, got {:?}",
            result
        );
    }

    #[test]
    fn dispatch_empty_operation_returns_unsupported() {
        let caller = make_caller();
        let req = make_request("");
        let result = OperationRouter::dispatch(&caller, &req);
        assert!(matches!(result, Err(MemoryApiErrorV2::Unsupported { .. })));
    }

    #[test]
    fn dispatch_deadline_above_hard_cap_returns_limit() {
        let caller = make_caller();
        let mut req = make_request("search");
        req.deadline_ms = Some(OperationLimits::DEADLINE_MS + 1);
        let result = OperationRouter::dispatch(&caller, &req);
        assert!(
            matches!(result, Err(MemoryApiErrorV2::Limit { ref operation, .. }) if operation == "search"),
            "expected Limit error, got {:?}",
            result
        );
    }

    #[test]
    fn dispatch_deadline_at_hard_cap_is_accepted() {
        let caller = make_caller();
        let mut req = make_request("search");
        req.deadline_ms = Some(OperationLimits::DEADLINE_MS);
        let result = OperationRouter::dispatch(&caller, &req);
        assert!(result.is_ok(), "deadline exactly at cap should be accepted");
    }

    #[test]
    fn dispatch_no_deadline_is_accepted() {
        let caller = make_caller();
        let req = make_request("search");
        // deadline_ms is None — no cap check needed.
        assert!(OperationRouter::dispatch(&caller, &req).is_ok());
    }

    // ── OperationRouter::dispatch — valid operations ──────────────────────────

    #[test]
    fn dispatch_search_returns_valid_response() {
        let caller = make_caller();
        let req = make_request("search");
        let resp = OperationRouter::dispatch(&caller, &req).expect("should succeed");

        assert_eq!(resp.schema_version, "2.0");
        assert_eq!(resp.revision, 0);
        assert!(!resp.query_hash.is_empty());
        assert!(resp.items.is_empty());
        assert_eq!(resp.total_count, TotalSemantics::Exact(0));
        assert!(!resp.truncated);
        assert!(resp.truncation_reason.is_none());
        assert!(resp.recovery_cursor.is_none());
        assert!(resp.warnings.is_empty());
        assert!(resp.degradation.is_none());
    }

    #[test]
    fn dispatch_neighborhood_returns_valid_response() {
        let caller = make_caller();
        let resp = OperationRouter::dispatch(&caller, &make_request("neighborhood"))
            .expect("should succeed");
        assert_eq!(resp.schema_version, "2.0");
        assert!(resp.items.is_empty());
    }

    #[test]
    fn dispatch_path_returns_valid_response() {
        let caller = make_caller();
        let resp =
            OperationRouter::dispatch(&caller, &make_request("path")).expect("should succeed");
        assert_eq!(resp.schema_version, "2.0");
        assert!(resp.items.is_empty());
    }

    #[test]
    fn dispatch_trace_get_returns_valid_response() {
        let caller = make_caller();
        let resp =
            OperationRouter::dispatch(&caller, &make_request("trace.get")).expect("should succeed");
        assert_eq!(resp.schema_version, "2.0");
    }

    #[test]
    fn dispatch_aggregate_returns_valid_response() {
        let caller = make_caller();
        let resp =
            OperationRouter::dispatch(&caller, &make_request("aggregate")).expect("should succeed");
        assert_eq!(resp.schema_version, "2.0");
    }

    #[test]
    fn dispatch_predict_returns_valid_response() {
        let caller = make_caller();
        let resp =
            OperationRouter::dispatch(&caller, &make_request("predict")).expect("should succeed");
        assert_eq!(resp.schema_version, "2.0");
    }

    #[test]
    fn dispatch_temporal_diff_returns_valid_response() {
        let caller = make_caller();
        let resp = OperationRouter::dispatch(&caller, &make_request("temporal.diff"))
            .expect("should succeed");
        assert_eq!(resp.schema_version, "2.0");
    }

    #[test]
    fn dispatch_patch_list_returns_valid_response() {
        let caller = make_caller();
        let resp = OperationRouter::dispatch(&caller, &make_request("patch.list"))
            .expect("should succeed");
        assert_eq!(resp.schema_version, "2.0");
    }

    #[test]
    fn dispatch_inspect_returns_valid_response() {
        let caller = make_caller();
        let resp =
            OperationRouter::dispatch(&caller, &make_request("inspect")).expect("should succeed");
        assert_eq!(resp.schema_version, "2.0");
    }

    // ── query_hash stability ──────────────────────────────────────────────────

    #[test]
    fn dispatch_same_operation_produces_same_query_hash() {
        let caller = make_caller();
        let r1 = OperationRouter::dispatch(&caller, &make_request("search")).unwrap();
        let r2 = OperationRouter::dispatch(&caller, &make_request("search")).unwrap();
        assert_eq!(r1.query_hash, r2.query_hash);
    }

    #[test]
    fn dispatch_different_operations_produce_different_query_hashes() {
        let caller = make_caller();
        let r_search = OperationRouter::dispatch(&caller, &make_request("search")).unwrap();
        let r_path = OperationRouter::dispatch(&caller, &make_request("path")).unwrap();
        assert_ne!(r_search.query_hash, r_path.query_hash);
    }
}
