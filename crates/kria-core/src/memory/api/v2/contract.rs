//! v2 API contract types: CallerContext, TransportKind, OperationLimits, ApiVersion.
//!
//! These are the foundational types for the memory.v2 public contract
//! (design §8, F3.9). All hard limits are `const` values here so adapters,
//! tests, and error messages can assert the same values without repetition.

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// TransportKind
// ─────────────────────────────────────────────────────────────────────────────

/// The transport channel through which the caller reaches the memory API.
///
/// Adapters record their transport in the [`CallerContext`] they build at their
/// boundary; the domain core never infers transport from other fields.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    /// Tauri IPC — local desktop; loopback-only; highest trust tier.
    Tauri,
    /// Axum HTTP/WebSocket — may be loopback or remote; security configuration
    /// in `kria-server` determines whether remote is permitted.
    Http,
    /// In-process call — used by background workers, tests, and the cognitive
    /// scheduler calling the domain core directly.
    Internal,
}

// ─────────────────────────────────────────────────────────────────────────────
// CallerContext
// ─────────────────────────────────────────────────────────────────────────────

/// Per-request caller context attached by the adapter at the transport boundary.
///
/// Adapters (Tauri command handler, Axum route handler) are the sole constructors
/// of `CallerContext`; the domain core reads but never mutates or invents these
/// fields (design §3 Backend-First Ownership, invariant A5 Isolation).
///
/// Fields are intentionally plain `String` / `u8` to remain serializable across
/// the Tauri IPC bridge and future interchange formats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallerContext {
    /// Stable, opaque identity of the calling actor (user ID, service account,
    /// skill ID, …). Must not be empty; adapters must reject requests with no
    /// authenticated identity.
    pub caller_id: String,

    /// Policy namespace: determines which records are visible to this caller.
    /// Corresponds to `namespace` in the authority schema policy columns.
    pub namespace: String,

    /// Fine-grained scope within the namespace (e.g. `"personal"`, `"work"`).
    /// Empty string means "no scope restriction beyond namespace".
    pub scope: String,

    /// Sensitivity ceiling the caller is authorized to read (0 = public …
    /// 3 = restricted). The effective policy is `min(caller_sensitivity,
    /// record_sensitivity)`; the domain never elevates this.
    pub sensitivity: u8,

    /// Pinned policy version this caller was authorized against. Drift between
    /// the stored policy hash and this value causes a `Refetch` error.
    pub policy_version: String,

    /// Transport channel that delivered this request.
    pub transport: TransportKind,
}

// ─────────────────────────────────────────────────────────────────────────────
// OperationLimits
// ─────────────────────────────────────────────────────────────────────────────

/// Hard caps and deadlines that govern every v2 operation (design §8.1).
///
/// These are immutable compile-time constants. Runtime configuration may set
/// *lower* values (e.g. a per-operation page size), but no code path may exceed
/// the hard caps defined here. Limit errors never switch to unbounded behaviour
/// (design invariant A6 Boundedness).
pub struct OperationLimits;

impl OperationLimits {
    /// Maximum graph traversal depth in hops (design §8.1 `neighborhood`).
    pub const MAX_DEPTH: u8 = 3;

    /// Maximum items returned by a single paginated operation (design §8.1).
    pub const MAX_ITEMS: u32 = 500;

    /// Maximum request/response payload in bytes (1 MiB).
    pub const MAX_PAYLOAD_BYTES: u32 = 1_048_576;

    /// Maximum label length in Unicode scalar values (design §8.1).
    pub const MAX_LABEL_LEN: u16 = 256;

    /// Per-request deadline in milliseconds (design §8.1 default deadlines).
    pub const DEADLINE_MS: u64 = 5_000;
}

// ─────────────────────────────────────────────────────────────────────────────
// ApiVersion
// ─────────────────────────────────────────────────────────────────────────────

/// The supported memory API version.
///
/// Only `V2` is defined here; `V1` is the legacy `api.rs` contract that
/// coexists until the hard cutover (design §3, task F3.9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApiVersion {
    V2,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_limits_max_depth_is_3() {
        assert_eq!(OperationLimits::MAX_DEPTH, 3);
    }

    #[test]
    fn operation_limits_max_items_is_500() {
        assert_eq!(OperationLimits::MAX_ITEMS, 500);
    }

    #[test]
    fn operation_limits_max_payload_bytes_is_1mib() {
        assert_eq!(OperationLimits::MAX_PAYLOAD_BYTES, 1_048_576);
    }

    #[test]
    fn operation_limits_max_label_len_is_256() {
        assert_eq!(OperationLimits::MAX_LABEL_LEN, 256);
    }

    #[test]
    fn operation_limits_deadline_ms_is_5000() {
        assert_eq!(OperationLimits::DEADLINE_MS, 5_000);
    }

    #[test]
    fn caller_context_round_trips_json() {
        let ctx = CallerContext {
            caller_id: "user-1".to_string(),
            namespace: "personal".to_string(),
            scope: "work".to_string(),
            sensitivity: 2,
            policy_version: "v1-abc".to_string(),
            transport: TransportKind::Tauri,
        };
        let json = serde_json::to_string(&ctx).expect("serializes");
        let back: CallerContext = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(ctx, back);
    }

    #[test]
    fn api_version_v2_serializes_as_screaming_snake_case() {
        let v = ApiVersion::V2;
        let json = serde_json::to_string(&v).expect("serializes");
        assert_eq!(json, r#""V2""#);
    }
}
