//! v2 typed error enum for the memory API (design §8.2, F3.9).
//!
//! `MemoryApiErrorV2` is the single error type returned by both the
//! `AuthorityCommandBus` and `GraphQueryPort` traits. Every variant maps to a
//! stable `MemoryApiErrorCodeV2` value (design §8.2); adapters translate these
//! to HTTP status codes or Tauri error strings without adding new semantic
//! meaning.
//!
//! Variants carrying `String` fields must never include user memory content,
//! entity labels, or other protected data — only correlation IDs, field names,
//! or operation names.

use serde::{Deserialize, Serialize};

/// Typed error returned by all v2 memory API operations.
///
/// Adapters map variants to transport-level errors; the domain never maps
/// transport errors back into this type. The design §8.2 JSON wire shape is:
///
/// ```json
/// {"schemaVersion":2,"error":{"code":"RevisionConflict","message":"…","correlationId":"…","retry":"refresh_preview"}}
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "PascalCase")]
pub enum MemoryApiErrorV2 {
    /// The caller presented no valid identity or their token has expired.
    Unauthorized,

    /// The caller is authenticated but lacks permission for this operation
    /// or resource.
    Forbidden {
        /// Human-readable reason (no protected data).
        reason: String,
    },

    /// The request structure or parameter values are invalid.
    InvalidRequest {
        /// Name of the first field that failed validation (e.g. `"page_size"`).
        field: String,
        /// Human-readable validation message.
        message: String,
    },

    /// A hard operation limit (page size, hop count, payload bytes, …) was
    /// exceeded (design §8.1). Limit errors never switch to unbounded
    /// behaviour (invariant A6).
    Limit {
        /// Name of the operation that hit the limit (e.g. `"search"`).
        operation: String,
        /// Human-readable description of the limit that was hit (e.g.
        /// `"page_size exceeds 500"`).
        limit: String,
    },

    /// The requested operation or schema version is not supported by this
    /// deployment (design §8.3 `UnsupportedCapability`). The caller should
    /// omit or disable the corresponding UI control.
    Unsupported {
        /// Name of the unsupported feature or operation (e.g. `"temporal.diff"`).
        feature: String,
    },

    /// The authority revision seen by the caller no longer matches the current
    /// revision (design §5.2 revision conflict). The caller should refresh
    /// their preview or cursor.
    Revision {
        /// Revision the caller expected.
        expected: i64,
        /// Actual current revision.
        actual: i64,
    },

    /// The provided pagination cursor is expired, incompatible (schema/query/
    /// policy/sort mismatch), or otherwise invalid (design §5.2).
    Cursor {
        /// Human-readable reason (e.g. `"cursor_expired"`, `"policy_mismatch"`).
        reason: String,
    },

    /// The client's cached query results are stale because the authority
    /// revision or policy has drifted; the caller must re-execute the query
    /// from scratch (design §5.2 refetch instruction).
    Refetch {
        /// Hash of the original query that must be re-executed.
        query_hash: String,
    },

    /// The operation exceeded its deadline (design §8.1).
    Timeout {
        /// Deadline in milliseconds that was exceeded.
        deadline_ms: u64,
    },

    /// The operation was explicitly cancelled by the caller (e.g. the user
    /// navigated away before the request completed).
    Cancelled,

    /// A required external dependency (embedder, FTS5 index, vector partition,
    /// …) is temporarily unavailable.
    Dependency {
        /// Name of the unavailable dependency (e.g. `"vector_store"`).
        name: String,
    },

    /// The authority database is temporarily busy (write-lock contention).
    /// The caller may retry with exponential backoff.
    Busy,

    /// Authority data is syntactically or semantically malformed in a way
    /// that prevents processing the request. This should not occur in normal
    /// operation; file a bug if seen.
    Malformed {
        /// Human-readable reason (no protected data).
        reason: String,
    },

    /// An integrity invariant was violated during the operation. The authority
    /// remains safe; the caller should record the correlation ID and contact
    /// the developer.
    Integrity {
        /// Stable correlation ID for the fault (safe to log, display, and report).
        correlation_id: String,
    },

    /// The system is in Recovery_Mode (design §5.3); only diagnostics and
    /// verified restore are permitted. All durable writes are blocked.
    Recovery {
        /// Correlation ID from the fault that triggered Recovery_Mode.
        correlation_id: String,
    },

    /// The submitted command matches a previous idempotency key but with a
    /// different command hash — the caller is replaying a different operation
    /// under the same key, which is a conflict.
    Idempotency {
        /// The result that was committed under this idempotency key previously.
        existing_result: serde_json::Value,
    },

    /// Application-level cryptographic erasure is not available (design §5.4).
    /// Hard Delete without key destruction is offered instead; the caller must
    /// not claim crypto-shredding occurred.
    Crypto {
        /// Human-readable reason (e.g. `"payload encryption not implemented"`).
        reason: String,
    },

    /// An unexpected internal error occurred. No domain semantics can be
    /// inferred; the caller should display a generic error and retry.
    Internal,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unauthorized_serializes_correctly() {
        let err = MemoryApiErrorV2::Unauthorized;
        let json = serde_json::to_value(&err).expect("serializes");
        assert_eq!(json["code"], "Unauthorized");
        let back: MemoryApiErrorV2 = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back, MemoryApiErrorV2::Unauthorized);
    }

    #[test]
    fn forbidden_round_trips_json() {
        let err = MemoryApiErrorV2::Forbidden {
            reason: "insufficient scope".to_string(),
        };
        let json = serde_json::to_value(&err).expect("serializes");
        assert_eq!(json["code"], "Forbidden");
        assert_eq!(json["reason"], "insufficient scope");
        let back: MemoryApiErrorV2 = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back, err);
    }

    #[test]
    fn revision_conflict_round_trips_json() {
        let err = MemoryApiErrorV2::Revision {
            expected: 42,
            actual: 43,
        };
        let json = serde_json::to_value(&err).expect("serializes");
        assert_eq!(json["code"], "Revision");
        assert_eq!(json["expected"], 42);
        assert_eq!(json["actual"], 43);
        let back: MemoryApiErrorV2 = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back, err);
    }

    #[test]
    fn limit_exceeded_round_trips_json() {
        let err = MemoryApiErrorV2::Limit {
            operation: "search".to_string(),
            limit: "page_size exceeds 500".to_string(),
        };
        let json = serde_json::to_value(&err).expect("serializes");
        assert_eq!(json["code"], "Limit");
        let back: MemoryApiErrorV2 = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back, err);
    }

    #[test]
    fn internal_error_serializes_as_pascal_case() {
        let err = MemoryApiErrorV2::Internal;
        let json = serde_json::to_value(&err).expect("serializes");
        assert_eq!(json["code"], "Internal");
        let back: MemoryApiErrorV2 = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back, MemoryApiErrorV2::Internal);
    }

    #[test]
    fn idempotency_conflict_round_trips_json() {
        let existing = serde_json::json!({"command_id": "prev-123", "revision": 10});
        let err = MemoryApiErrorV2::Idempotency {
            existing_result: existing.clone(),
        };
        let json = serde_json::to_value(&err).expect("serializes");
        assert_eq!(json["code"], "Idempotency");
        let back: MemoryApiErrorV2 = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back, err);
    }

    #[test]
    fn all_unit_variants_round_trip() {
        let variants = [
            MemoryApiErrorV2::Unauthorized,
            MemoryApiErrorV2::Cancelled,
            MemoryApiErrorV2::Busy,
            MemoryApiErrorV2::Internal,
        ];
        for v in &variants {
            let json = serde_json::to_value(v).expect("serializes");
            let back: MemoryApiErrorV2 = serde_json::from_value(json).expect("deserializes");
            assert_eq!(&back, v);
        }
    }
}
