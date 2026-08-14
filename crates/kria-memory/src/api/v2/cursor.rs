//! Authenticated pagination cursors for the memory v2 API (design §5.2, F3.9).
//!
//! ## Design contract (design §5.2)
//!
//! A read transaction captures `R = authority_meta.graph_revision`; all
//! sub-queries execute in that WAL snapshot. A cursor is an authenticated MAC
//! over `{schema_version, query_hash, policy_hash, revision, last_sort_key,
//! expires_at}`. Pages never hold a long transaction: the query is deterministic
//! against revisioned rows and rejects expired/incompatible cursors.
//!
//! ## Wire format
//!
//! ```text
//! <base64url(JSON payload)>.<hex(HMAC-SHA256)>
//! ```
//!
//! - The payload is `CursorPayload` serialized as compact JSON then base64url
//!   encoded (no padding).
//! - The HMAC covers the base64url payload bytes, not the raw JSON, so
//!   re-encoding is not required during validation.
//!
//! ## Key management
//!
//! `DEV_HMAC_KEY` is a fixed 32-byte key used for pre-production only. A
//! rotating key from configuration will replace it in production; the key is
//! never stored in the cursor itself (design §5.2).
//!
//! ## Error mapping
//!
//! | `CursorError` variant | Maps to `MemoryApiErrorV2` variant |
//! |---|---|
//! | `Tampered` | `Cursor { reason: "tampered" }` |
//! | `Expired` | `Cursor { reason: "expired" }` |
//! | `SchemaMismatch` | `Cursor { reason: "schema_mismatch: …" }` |
//! | `PolicyMismatch` | `Cursor { reason: "policy_mismatch" }` |
//! | `RevisionDrift` | `Refetch { query_hash }` |

use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use super::error::MemoryApiErrorV2;

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Default cursor TTL: 1 hour.
pub const DEFAULT_CURSOR_TTL_SECS: u64 = 3_600;

/// Fixed HMAC key for pre-production use only.
///
/// This is a 32-byte dev key embedded at compile time. Production deployments
/// will replace this with a rotating key sourced from configuration.
const DEV_HMAC_KEY: &[u8] = b"kria-cursor-v2-dev-key-32bytes!!";

// ─────────────────────────────────────────────────────────────────────────────
// CursorPayload
// ─────────────────────────────────────────────────────────────────────────────

/// Authenticated payload bound inside a pagination cursor (design §5.2).
///
/// This struct is never sent directly on the wire. It is serialized to compact
/// JSON, base64url-encoded, and HMAC-signed before being returned to the
/// caller. Validation restores and checks all fields before trusting any value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorPayload {
    /// Schema version that was active when the cursor was issued. Mismatches
    /// prevent stale clients from using a cursor after a schema migration.
    pub schema_version: String,

    /// SHA-256 hex hash of the canonicalized query. Mismatches mean the caller
    /// has changed their query parameters and must restart pagination.
    pub query_hash: String,

    /// SHA-256 hex hash of the effective policy used when the cursor was
    /// issued. Mismatches indicate a policy change; the cursor is invalid.
    pub policy_hash: String,

    /// WAL snapshot revision (`authority_meta.graph_revision`) captured at
    /// read-transaction start (design §5.2, invariant A7).
    pub revision: i64,

    /// The sort key of the last item returned on the previous page. Used for
    /// deterministic keyset pagination: the next query starts immediately after
    /// this key.
    pub last_sort_key: String,

    /// Unix timestamp (seconds since UNIX epoch) after which this cursor must
    /// be rejected. Zero means "no expiry" but callers SHOULD always set a
    /// finite TTL (typically [`DEFAULT_CURSOR_TTL_SECS`]).
    pub expires_at_unix: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// CursorError
// ─────────────────────────────────────────────────────────────────────────────

/// Reason a cursor was rejected during validation (design §5.2).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CursorError {
    /// The cursor's HMAC signature did not verify — either the cursor was
    /// tampered with in transit or it was issued by a different key.
    #[error("cursor HMAC verification failed (tampered or wrong key)")]
    Tampered,

    /// The cursor's `expires_at_unix` is in the past.
    #[error("cursor has expired")]
    Expired,

    /// The `schema_version` embedded in the cursor does not match the current
    /// schema version.
    #[error("schema mismatch: cursor has '{got}', current is '{expected}'")]
    SchemaMismatch {
        /// Schema version the current system requires.
        expected: String,
        /// Schema version stored in the cursor.
        got: String,
    },

    /// The `policy_hash` in the cursor does not match the current policy hash.
    /// The caller must re-execute their query with the new policy.
    #[error("policy hash changed since cursor was issued")]
    PolicyMismatch,

    /// The `revision` in the cursor does not match the current graph revision.
    /// The caller must re-execute their query from scratch (`Refetch`).
    #[error("revision drift: cursor has {cursor_revision}, current is {current_revision}")]
    RevisionDrift {
        /// Revision stored in the cursor.
        cursor_revision: i64,
        /// Current authority graph revision.
        current_revision: i64,
    },
}

impl CursorError {
    /// Convert this error into the appropriate [`MemoryApiErrorV2`] variant.
    ///
    /// - `Tampered`, `Expired`, `SchemaMismatch`, `PolicyMismatch` →
    ///   [`MemoryApiErrorV2::Cursor`] with a descriptive reason string.
    /// - `RevisionDrift` → [`MemoryApiErrorV2::Refetch`] so the caller knows
    ///   to re-execute the original query.
    ///
    /// `query_hash` must be the hash of the original query that produced the
    /// cursor; it is embedded verbatim in the `Refetch` variant.
    pub fn to_api_error(&self, query_hash: &str) -> MemoryApiErrorV2 {
        match self {
            CursorError::Tampered => MemoryApiErrorV2::Cursor {
                reason: "tampered".to_string(),
            },
            CursorError::Expired => MemoryApiErrorV2::Cursor {
                reason: "expired".to_string(),
            },
            CursorError::SchemaMismatch { expected, got } => MemoryApiErrorV2::Cursor {
                reason: format!("schema_mismatch: cursor has '{got}', current is '{expected}'"),
            },
            CursorError::PolicyMismatch => MemoryApiErrorV2::Cursor {
                reason: "policy_mismatch".to_string(),
            },
            CursorError::RevisionDrift { .. } => MemoryApiErrorV2::Refetch {
                query_hash: query_hash.to_string(),
            },
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CursorManager
// ─────────────────────────────────────────────────────────────────────────────

/// Issues and validates authenticated pagination cursors (design §5.2).
///
/// ## Thread safety
///
/// `CursorManager` holds no mutable state; all methods take `&self`. It is
/// safe to share across threads behind an `Arc`.
pub struct CursorManager;

impl CursorManager {
    /// Issue a new cursor string for the given payload.
    ///
    /// ## Wire format
    ///
    /// ```text
    /// <base64url_no_pad(JSON)>.<hex(HMAC-SHA256 over base64url bytes)>
    /// ```
    ///
    /// The HMAC covers the base64url-encoded payload bytes (not the raw JSON)
    /// so validation does not need to re-encode before checking.
    pub fn issue_cursor(&self, payload: &CursorPayload) -> String {
        let json = serde_json::to_string(payload)
            .expect("CursorPayload is always serializable; this is a bug");

        let encoded = URL_SAFE_NO_PAD.encode(json.as_bytes());
        let tag = Self::hmac_tag(encoded.as_bytes());
        format!("{encoded}.{tag}")
    }

    /// Validate a cursor string and return the decoded payload on success.
    ///
    /// Validation order (fail-fast):
    /// 1. Split and decode the wire format.
    /// 2. Verify the HMAC tag (tamper detection).
    /// 3. Check `expires_at_unix` against the current wall clock.
    /// 4. Check `schema_version` against `current_schema_version`.
    /// 5. Check `policy_hash` against `current_policy_hash`.
    /// 6. Check `revision` against `current_revision` (returns `RevisionDrift`).
    ///
    /// # Errors
    ///
    /// Returns [`CursorError`] describing the first validation failure.
    pub fn validate_cursor(
        &self,
        cursor: &str,
        current_revision: i64,
        current_policy_hash: &str,
        current_schema_version: &str,
    ) -> Result<CursorPayload, CursorError> {
        // ── 1. Parse wire format ──────────────────────────────────────────
        let (encoded, tag_hex) = Self::split_cursor(cursor).ok_or(CursorError::Tampered)?;

        // ── 2. Verify HMAC ────────────────────────────────────────────────
        let expected_tag = Self::hmac_tag(encoded.as_bytes());
        if !constant_time_eq(expected_tag.as_bytes(), tag_hex.as_bytes()) {
            return Err(CursorError::Tampered);
        }

        // ── Decode payload ────────────────────────────────────────────────
        let payload_bytes = URL_SAFE_NO_PAD
            .decode(encoded.as_bytes())
            .map_err(|_| CursorError::Tampered)?;
        let payload: CursorPayload =
            serde_json::from_slice(&payload_bytes).map_err(|_| CursorError::Tampered)?;

        // ── 3. Expiry ─────────────────────────────────────────────────────
        if payload.expires_at_unix != 0 {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if now >= payload.expires_at_unix {
                return Err(CursorError::Expired);
            }
        }

        // ── 4. Schema version ─────────────────────────────────────────────
        if payload.schema_version != current_schema_version {
            return Err(CursorError::SchemaMismatch {
                expected: current_schema_version.to_string(),
                got: payload.schema_version.clone(),
            });
        }

        // ── 5. Policy hash ────────────────────────────────────────────────
        if payload.policy_hash != current_policy_hash {
            return Err(CursorError::PolicyMismatch);
        }

        // ── 6. Revision ───────────────────────────────────────────────────
        if payload.revision != current_revision {
            return Err(CursorError::RevisionDrift {
                cursor_revision: payload.revision,
                current_revision,
            });
        }

        Ok(payload)
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Compute `hex(HMAC-SHA256(DEV_HMAC_KEY, data))`.
    fn hmac_tag(data: &[u8]) -> String {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(DEV_HMAC_KEY).expect("HMAC accepts any key size");
        mac.update(data);
        let result = mac.finalize().into_bytes();
        hex::encode(result)
    }

    /// Split `"<encoded>.<tag_hex>"` into the two parts.
    ///
    /// Returns `None` if there is no `.` separator or if either part is empty.
    fn split_cursor(cursor: &str) -> Option<(&str, &str)> {
        // Split on the LAST `.` so that base64 padding `=` characters in other
        // encodings would not confuse the parser (URL_SAFE_NO_PAD never emits
        // `.`, so the first `.` is the separator).
        let pos = cursor.find('.')?;
        let encoded = &cursor[..pos];
        let tag = &cursor[pos + 1..];
        if encoded.is_empty() || tag.is_empty() {
            return None;
        }
        Some((encoded, tag))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Constant-time comparison
// ─────────────────────────────────────────────────────────────────────────────

/// Constant-time byte-slice equality to prevent timing side-channels on HMAC
/// tag comparison.
///
/// Uses XOR folding so the compiler cannot short-circuit on early mismatch.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff: u8 = a
        .iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y));
    diff == 0
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a valid `CursorPayload` that expires 1 hour from now.
    fn make_payload(revision: i64) -> CursorPayload {
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + DEFAULT_CURSOR_TTL_SECS;
        CursorPayload {
            schema_version: "v2".to_string(),
            query_hash: "abc123".to_string(),
            policy_hash: "pol456".to_string(),
            revision,
            last_sort_key: "sort:42".to_string(),
            expires_at_unix: expires_at,
        }
    }

    // ── Round-trip ────────────────────────────────────────────────────────────

    #[test]
    fn issue_and_validate_roundtrip() {
        let mgr = CursorManager;
        let payload = make_payload(7);
        let cursor = mgr.issue_cursor(&payload);

        let result = mgr
            .validate_cursor(&cursor, 7, "pol456", "v2")
            .expect("valid cursor must succeed");

        assert_eq!(result, payload);
    }

    #[test]
    fn cursor_wire_format_has_two_dot_separated_parts() {
        let mgr = CursorManager;
        let cursor = mgr.issue_cursor(&make_payload(1));
        let parts: Vec<&str> = cursor.splitn(2, '.').collect();
        assert_eq!(parts.len(), 2, "cursor must have exactly one '.' separator");
        assert!(!parts[0].is_empty(), "encoded part must not be empty");
        assert!(!parts[1].is_empty(), "HMAC tag part must not be empty");
    }

    // ── Tampered cursor ───────────────────────────────────────────────────────

    #[test]
    fn tampered_cursor_returns_tampered_error() {
        let mgr = CursorManager;
        let cursor = mgr.issue_cursor(&make_payload(5));

        // Flip one byte in the encoded payload.
        let (encoded, tag) = cursor.split_once('.').unwrap();
        let mut bytes = encoded.as_bytes().to_vec();
        // XOR the last byte of the base64 to corrupt it.
        *bytes.last_mut().unwrap() ^= 0x01;
        // Build a tampered cursor: invalid base64 char or valid base64 with wrong HMAC
        let tampered = format!("TAMPERED{encoded}.{tag}");

        let err = mgr
            .validate_cursor(&tampered, 5, "pol456", "v2")
            .unwrap_err();
        assert_eq!(err, CursorError::Tampered);
    }

    #[test]
    fn cursor_with_modified_tag_returns_tampered() {
        let mgr = CursorManager;
        let cursor = mgr.issue_cursor(&make_payload(5));
        let (encoded, _tag) = cursor.split_once('.').unwrap();
        // Replace the HMAC tag with a zeroed one.
        let bad_tag = "0".repeat(64); // 32 zero bytes in hex
        let tampered = format!("{encoded}.{bad_tag}");

        let err = mgr
            .validate_cursor(&tampered, 5, "pol456", "v2")
            .unwrap_err();
        assert_eq!(err, CursorError::Tampered);
    }

    // ── Expired cursor ────────────────────────────────────────────────────────

    #[test]
    fn expired_cursor_returns_expired_error() {
        let mgr = CursorManager;
        // Set expiry to 1 second in the past.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let payload = CursorPayload {
            schema_version: "v2".to_string(),
            query_hash: "abc123".to_string(),
            policy_hash: "pol456".to_string(),
            revision: 3,
            last_sort_key: "sort:1".to_string(),
            expires_at_unix: now - 1, // already expired
        };
        let cursor = mgr.issue_cursor(&payload);

        let err = mgr.validate_cursor(&cursor, 3, "pol456", "v2").unwrap_err();
        assert_eq!(err, CursorError::Expired);
    }

    // ── Schema mismatch ───────────────────────────────────────────────────────

    #[test]
    fn schema_mismatch_returns_schema_mismatch_error() {
        let mgr = CursorManager;
        let payload = make_payload(2);
        let cursor = mgr.issue_cursor(&payload);

        let err = mgr
            .validate_cursor(&cursor, 2, "pol456", "v3") // different schema
            .unwrap_err();

        assert_eq!(
            err,
            CursorError::SchemaMismatch {
                expected: "v3".to_string(),
                got: "v2".to_string(),
            }
        );
    }

    #[test]
    fn schema_mismatch_maps_to_cursor_api_error() {
        let err = CursorError::SchemaMismatch {
            expected: "v3".to_string(),
            got: "v2".to_string(),
        };
        let api_err = err.to_api_error("qhash");
        match api_err {
            MemoryApiErrorV2::Cursor { reason } => {
                assert!(reason.contains("schema_mismatch"), "reason: {reason}");
                assert!(reason.contains("v2"), "reason: {reason}");
                assert!(reason.contains("v3"), "reason: {reason}");
            }
            other => panic!("expected Cursor, got {other:?}"),
        }
    }

    // ── Policy mismatch ───────────────────────────────────────────────────────

    #[test]
    fn policy_mismatch_returns_policy_mismatch_error() {
        let mgr = CursorManager;
        let payload = make_payload(4);
        let cursor = mgr.issue_cursor(&payload);

        let err = mgr
            .validate_cursor(&cursor, 4, "DIFFERENT_POLICY", "v2")
            .unwrap_err();
        assert_eq!(err, CursorError::PolicyMismatch);
    }

    #[test]
    fn policy_mismatch_maps_to_cursor_api_error() {
        let err = CursorError::PolicyMismatch;
        let api_err = err.to_api_error("qhash");
        assert_eq!(
            api_err,
            MemoryApiErrorV2::Cursor {
                reason: "policy_mismatch".to_string()
            }
        );
    }

    // ── Revision drift ────────────────────────────────────────────────────────

    #[test]
    fn revision_drift_returns_revision_drift_error() {
        let mgr = CursorManager;
        let payload = make_payload(10);
        let cursor = mgr.issue_cursor(&payload);

        // Validate against a different revision (simulating a write that bumped
        // the WAL revision between page requests).
        let err = mgr
            .validate_cursor(&cursor, 11, "pol456", "v2") // revision drifted
            .unwrap_err();

        assert_eq!(
            err,
            CursorError::RevisionDrift {
                cursor_revision: 10,
                current_revision: 11,
            }
        );
    }

    #[test]
    fn revision_drift_maps_to_refetch_api_error() {
        let err = CursorError::RevisionDrift {
            cursor_revision: 10,
            current_revision: 11,
        };
        let api_err = err.to_api_error("my_query_hash");
        assert_eq!(
            api_err,
            MemoryApiErrorV2::Refetch {
                query_hash: "my_query_hash".to_string()
            }
        );
    }

    // ── to_api_error coverage for remaining variants ──────────────────────────

    #[test]
    fn tampered_maps_to_cursor_api_error() {
        let api_err = CursorError::Tampered.to_api_error("qh");
        assert_eq!(
            api_err,
            MemoryApiErrorV2::Cursor {
                reason: "tampered".to_string()
            }
        );
    }

    #[test]
    fn expired_maps_to_cursor_api_error() {
        let api_err = CursorError::Expired.to_api_error("qh");
        assert_eq!(
            api_err,
            MemoryApiErrorV2::Cursor {
                reason: "expired".to_string()
            }
        );
    }

    // ── Malformed cursor inputs ───────────────────────────────────────────────

    #[test]
    fn empty_string_returns_tampered() {
        let mgr = CursorManager;
        let err = mgr.validate_cursor("", 1, "p", "v2").unwrap_err();
        assert_eq!(err, CursorError::Tampered);
    }

    #[test]
    fn no_dot_separator_returns_tampered() {
        let mgr = CursorManager;
        let err = mgr.validate_cursor("nodothere", 1, "p", "v2").unwrap_err();
        assert_eq!(err, CursorError::Tampered);
    }

    #[test]
    fn only_dot_returns_tampered() {
        let mgr = CursorManager;
        let err = mgr.validate_cursor(".", 1, "p", "v2").unwrap_err();
        assert_eq!(err, CursorError::Tampered);
    }

    // ── Validation order: schema checked before policy, policy before revision ──

    #[test]
    fn schema_mismatch_takes_priority_over_policy_mismatch() {
        let mgr = CursorManager;
        let payload = make_payload(1);
        let cursor = mgr.issue_cursor(&payload);

        // Both schema and policy differ; schema error should surface first.
        let err = mgr
            .validate_cursor(&cursor, 1, "WRONG_POLICY", "v3")
            .unwrap_err();
        assert!(
            matches!(err, CursorError::SchemaMismatch { .. }),
            "expected SchemaMismatch, got {err:?}"
        );
    }

    #[test]
    fn policy_mismatch_takes_priority_over_revision_drift() {
        let mgr = CursorManager;
        let payload = make_payload(1);
        let cursor = mgr.issue_cursor(&payload);

        // Both policy and revision differ; policy error should surface first.
        let err = mgr
            .validate_cursor(&cursor, 99, "WRONG_POLICY", "v2")
            .unwrap_err();
        assert_eq!(err, CursorError::PolicyMismatch);
    }
}
