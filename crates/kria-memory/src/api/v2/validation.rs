//! Runtime validation for v2 request envelopes (design §8.2, task 4.1.1).
//!
//! This module enforces all DTO constraints at the API boundary before any
//! domain logic executes. Unknown required schema versions and empty operations
//! are rejected here so the domain core never sees malformed input.
//!
//! ## Responsibilities
//!
//! - [`KNOWN_SCHEMA_VERSIONS`] — the exhaustive set of accepted schema version
//!   strings; anything else returns [`MemoryApiErrorV2::Unsupported`].
//! - [`validate_request`] — the primary validation entry point; checks schema
//!   version, non-empty operation, deadline cap, and non-negative revision.
//! - [`ValidatedRequest`] — a newtype wrapper that can only be constructed
//!   from a request that has passed `validate_request`; the domain core
//!   accepts only `ValidatedRequest` so the compile-time type prevents
//!   unvalidated requests from reaching domain logic.
//! - [`validate_schema_version`] — standalone predicate for quick checks.
//! - [`is_valid_degradation_code`] — checks whether a degradation level code
//!   string is a known [`super::dto::DegradationLevel`] variant.

use super::contract::OperationLimits;
use super::dto::{DegradationLevel, GraphRequestV2};
use super::error::MemoryApiErrorV2;

// ─────────────────────────────────────────────────────────────────────────────
// Known schema versions
// ─────────────────────────────────────────────────────────────────────────────

/// The exhaustive set of schema version strings accepted by this deployment.
///
/// Any value not present here causes [`validate_request`] to return
/// [`MemoryApiErrorV2::Unsupported`]. Add new versions here (and nowhere else)
/// when the wire contract is extended.
pub const KNOWN_SCHEMA_VERSIONS: &[&str] = &["2.0"];

// ─────────────────────────────────────────────────────────────────────────────
// validate_schema_version
// ─────────────────────────────────────────────────────────────────────────────

/// Returns `true` if `version` is in [`KNOWN_SCHEMA_VERSIONS`].
///
/// Use this for quick UI/adapter guards; use [`validate_request`] for full
/// envelope validation at the domain boundary.
#[inline]
pub fn validate_schema_version(version: &str) -> bool {
    KNOWN_SCHEMA_VERSIONS.contains(&version)
}

// ─────────────────────────────────────────────────────────────────────────────
// is_valid_degradation_code
// ─────────────────────────────────────────────────────────────────────────────

/// Returns `true` if `code` is the snake_case wire-format string for a known
/// [`DegradationLevel`] variant (`"partial"`, `"degraded"`, `"offline"`).
///
/// Mirrors the `#[serde(rename_all = "snake_case")]` derivation on
/// `DegradationLevel`. Use this to validate untrusted degradation code strings
/// (e.g. from JSON that bypassed deserialization).
pub fn is_valid_degradation_code(code: &str) -> bool {
    matches!(code, "partial" | "degraded" | "offline")
}

impl DegradationLevel {
    /// Returns `true` if `code` is the wire-format string for any known
    /// degradation level.
    ///
    /// Equivalent to the free function [`is_valid_degradation_code`]; provided
    /// as an associated function for ergonomic call sites that already have the
    /// type in scope.
    pub fn is_valid_code(code: &str) -> bool {
        is_valid_degradation_code(code)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// validate_request
// ─────────────────────────────────────────────────────────────────────────────

/// Validate all envelope-level constraints on a [`GraphRequestV2`].
///
/// This is the canonical validation entry point at the domain boundary.
/// Validations run in this order so the first failure returns immediately:
///
/// 1. `schema_version` must be in [`KNOWN_SCHEMA_VERSIONS`] →
///    [`MemoryApiErrorV2::Unsupported`].
/// 2. `operation` must be non-empty →
///    [`MemoryApiErrorV2::InvalidRequest`] (`field = "operation"`).
/// 3. `deadline_ms`, when present, must be ≤ [`OperationLimits::DEADLINE_MS`] →
///    [`MemoryApiErrorV2::Limit`].
/// 4. `revision`, when present, must be ≥ 0 →
///    [`MemoryApiErrorV2::InvalidRequest`] (`field = "revision"`).
///
/// A successful return guarantees all four constraints hold.
pub fn validate_request(req: &GraphRequestV2) -> Result<(), MemoryApiErrorV2> {
    // 1. Schema version must be known.
    if !validate_schema_version(&req.schema_version) {
        return Err(MemoryApiErrorV2::Unsupported {
            feature: format!("schema_version:{}", req.schema_version),
        });
    }

    // 2. Operation must be non-empty.
    if req.operation.is_empty() {
        return Err(MemoryApiErrorV2::InvalidRequest {
            field: "operation".to_string(),
            message: "operation cannot be empty".to_string(),
        });
    }

    // 3. Deadline, if present, must not exceed the hard cap.
    if let Some(deadline_ms) = req.deadline_ms {
        if deadline_ms > OperationLimits::DEADLINE_MS {
            return Err(MemoryApiErrorV2::Limit {
                operation: req.operation.clone(),
                limit: format!(
                    "deadline_ms {} exceeds hard cap {}",
                    deadline_ms,
                    OperationLimits::DEADLINE_MS
                ),
            });
        }
    }

    // 4. Revision, if present, must be non-negative.
    if let Some(revision) = req.revision {
        if revision < 0 {
            return Err(MemoryApiErrorV2::InvalidRequest {
                field: "revision".to_string(),
                message: "revision must be non-negative".to_string(),
            });
        }
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// ValidatedRequest
// ─────────────────────────────────────────────────────────────────────────────

/// A [`GraphRequestV2`] that has passed all envelope-level validation checks.
///
/// The only constructors are [`ValidatedRequest::parse`] and
/// [`ValidatedRequest::from_request`], both of which call [`validate_request`]
/// internally. Domain logic should accept `&ValidatedRequest` instead of
/// `&GraphRequestV2` to prevent unvalidated input from reaching query paths.
///
/// # Example
///
/// ```rust
/// use crate::api::v2::{
///     dto::GraphRequestV2,
///     validation::ValidatedRequest,
/// };
///
/// let req = GraphRequestV2 {
///     operation: "search".to_string(),
///     params_json: serde_json::json!({}),
///     revision: None,
///     schema_version: "2.0".to_string(),
///     policy_hash: None,
///     cursor: None,
///     deadline_ms: None,
/// };
/// let validated = ValidatedRequest::from_request(req).expect("valid request");
/// assert_eq!(validated.inner().operation, "search");
/// ```
#[derive(Debug, Clone)]
pub struct ValidatedRequest {
    /// The validated request. Private to prevent construction without running
    /// validation.
    inner: GraphRequestV2,
}

impl ValidatedRequest {
    /// Parse and validate a raw JSON value as a [`GraphRequestV2`] envelope.
    ///
    /// Deserializes the JSON first; deserialization failure returns
    /// [`MemoryApiErrorV2::InvalidRequest`] with `field = "envelope"`.
    /// On success, calls [`validate_request`] and returns a `ValidatedRequest`
    /// only if all constraints pass.
    pub fn parse(raw: serde_json::Value) -> Result<Self, MemoryApiErrorV2> {
        let req: GraphRequestV2 =
            serde_json::from_value(raw).map_err(|e| MemoryApiErrorV2::InvalidRequest {
                field: "envelope".to_string(),
                message: format!("JSON deserialization failed: {}", e),
            })?;
        Self::from_request(req)
    }

    /// Validate an already-deserialized [`GraphRequestV2`].
    ///
    /// Runs all envelope-level checks via [`validate_request`] and wraps the
    /// request in a `ValidatedRequest` on success.
    pub fn from_request(req: GraphRequestV2) -> Result<Self, MemoryApiErrorV2> {
        validate_request(&req)?;
        Ok(Self { inner: req })
    }

    /// Borrow the validated inner request.
    #[inline]
    pub fn inner(&self) -> &GraphRequestV2 {
        &self.inner
    }

    /// Consume the wrapper and return the inner request.
    #[inline]
    pub fn into_inner(self) -> GraphRequestV2 {
        self.inner
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── test helpers ─────────────────────────────────────────────────────────

    fn valid_req() -> GraphRequestV2 {
        GraphRequestV2 {
            operation: "search".to_string(),
            params_json: json!({}),
            revision: None,
            schema_version: "2.0".to_string(),
            policy_hash: None,
            cursor: None,
            deadline_ms: None,
        }
    }

    // ── validate_schema_version ───────────────────────────────────────────────

    #[test]
    fn known_schema_version_returns_true() {
        assert!(validate_schema_version("2.0"));
    }

    #[test]
    fn unknown_schema_version_returns_false() {
        assert!(!validate_schema_version("99.0"));
        assert!(!validate_schema_version("1.0"));
        assert!(!validate_schema_version(""));
        assert!(!validate_schema_version("2"));
    }

    // ── validate_request — schema_version ────────────────────────────────────

    #[test]
    fn valid_schema_version_ok() {
        let req = valid_req(); // schema_version = "2.0"
        assert!(validate_request(&req).is_ok());
    }

    #[test]
    fn unknown_schema_version_returns_unsupported() {
        let mut req = valid_req();
        req.schema_version = "99.0".to_string();
        let err = validate_request(&req).expect_err("should fail");
        assert!(
            matches!(err, MemoryApiErrorV2::Unsupported { ref feature } if feature == "schema_version:99.0"),
            "got: {:?}",
            err
        );
    }

    // ── validate_request — operation ─────────────────────────────────────────

    #[test]
    fn non_empty_operation_ok() {
        let req = valid_req();
        assert!(validate_request(&req).is_ok());
    }

    #[test]
    fn empty_operation_returns_invalid_request() {
        let mut req = valid_req();
        req.operation = String::new();
        let err = validate_request(&req).expect_err("should fail");
        assert!(
            matches!(err, MemoryApiErrorV2::InvalidRequest { ref field, .. } if field == "operation"),
            "got: {:?}",
            err
        );
    }

    // ── validate_request — deadline_ms ───────────────────────────────────────

    #[test]
    fn deadline_absent_ok() {
        let req = valid_req(); // deadline_ms = None
        assert!(validate_request(&req).is_ok());
    }

    #[test]
    fn deadline_at_cap_ok() {
        let mut req = valid_req();
        req.deadline_ms = Some(OperationLimits::DEADLINE_MS);
        assert!(validate_request(&req).is_ok());
    }

    #[test]
    fn deadline_above_cap_returns_limit() {
        let mut req = valid_req();
        req.deadline_ms = Some(OperationLimits::DEADLINE_MS + 1);
        let err = validate_request(&req).expect_err("should fail");
        assert!(
            matches!(err, MemoryApiErrorV2::Limit { ref operation, .. } if operation == "search"),
            "got: {:?}",
            err
        );
    }

    #[test]
    fn deadline_well_above_cap_returns_limit() {
        let mut req = valid_req();
        req.deadline_ms = Some(u64::MAX);
        let err = validate_request(&req).expect_err("should fail");
        assert!(matches!(err, MemoryApiErrorV2::Limit { .. }));
    }

    // ── validate_request — revision ──────────────────────────────────────────

    #[test]
    fn revision_absent_ok() {
        let req = valid_req(); // revision = None
        assert!(validate_request(&req).is_ok());
    }

    #[test]
    fn revision_zero_ok() {
        let mut req = valid_req();
        req.revision = Some(0);
        assert!(validate_request(&req).is_ok());
    }

    #[test]
    fn revision_positive_ok() {
        let mut req = valid_req();
        req.revision = Some(42);
        assert!(validate_request(&req).is_ok());
    }

    #[test]
    fn negative_revision_returns_invalid_request() {
        let mut req = valid_req();
        req.revision = Some(-1);
        let err = validate_request(&req).expect_err("should fail");
        assert!(
            matches!(err, MemoryApiErrorV2::InvalidRequest { ref field, .. } if field == "revision"),
            "got: {:?}",
            err
        );
    }

    #[test]
    fn large_negative_revision_returns_invalid_request() {
        let mut req = valid_req();
        req.revision = Some(i64::MIN);
        let err = validate_request(&req).expect_err("should fail");
        assert!(
            matches!(err, MemoryApiErrorV2::InvalidRequest { ref field, .. } if field == "revision")
        );
    }

    // ── ValidatedRequest::from_request ───────────────────────────────────────

    #[test]
    fn from_request_valid_returns_ok() {
        let req = valid_req();
        let validated = ValidatedRequest::from_request(req).expect("should succeed");
        assert_eq!(validated.inner().operation, "search");
        assert_eq!(validated.inner().schema_version, "2.0");
    }

    #[test]
    fn from_request_invalid_schema_returns_err_unsupported() {
        let mut req = valid_req();
        req.schema_version = "0.1".to_string();
        let err = ValidatedRequest::from_request(req).expect_err("should fail");
        assert!(
            matches!(err, MemoryApiErrorV2::Unsupported { .. }),
            "got: {:?}",
            err
        );
    }

    #[test]
    fn from_request_empty_operation_returns_err_invalid_request() {
        let mut req = valid_req();
        req.operation = String::new();
        let err = ValidatedRequest::from_request(req).expect_err("should fail");
        assert!(
            matches!(err, MemoryApiErrorV2::InvalidRequest { ref field, .. } if field == "operation")
        );
    }

    #[test]
    fn from_request_negative_revision_returns_err() {
        let mut req = valid_req();
        req.revision = Some(-5);
        let err = ValidatedRequest::from_request(req).expect_err("should fail");
        assert!(
            matches!(err, MemoryApiErrorV2::InvalidRequest { ref field, .. } if field == "revision")
        );
    }

    // ── ValidatedRequest::parse ───────────────────────────────────────────────

    #[test]
    fn parse_valid_json_returns_ok() {
        let raw = json!({
            "operation": "neighborhood",
            "params_json": {},
            "schema_version": "2.0",
            "revision": null,
            "policy_hash": null,
            "cursor": null,
            "deadline_ms": null
        });
        let validated = ValidatedRequest::parse(raw).expect("should succeed");
        assert_eq!(validated.inner().operation, "neighborhood");
    }

    #[test]
    fn parse_malformed_json_returns_invalid_request() {
        // A JSON object that cannot deserialize into GraphRequestV2
        let raw = json!({ "totally_wrong": true });
        let err = ValidatedRequest::parse(raw).expect_err("should fail");
        assert!(
            matches!(err, MemoryApiErrorV2::InvalidRequest { ref field, .. } if field == "envelope"),
            "got: {:?}",
            err
        );
    }

    #[test]
    fn parse_valid_json_with_unknown_schema_returns_unsupported() {
        let raw = json!({
            "operation": "search",
            "params_json": {},
            "schema_version": "3.0",
            "revision": null,
            "policy_hash": null,
            "cursor": null,
            "deadline_ms": null
        });
        let err = ValidatedRequest::parse(raw).expect_err("should fail");
        assert!(matches!(err, MemoryApiErrorV2::Unsupported { .. }));
    }

    // ── ValidatedRequest::into_inner ─────────────────────────────────────────

    #[test]
    fn into_inner_returns_original_request() {
        let req = valid_req();
        let cloned = req.clone();
        let validated = ValidatedRequest::from_request(req).unwrap();
        let inner = validated.into_inner();
        assert_eq!(inner.operation, cloned.operation);
        assert_eq!(inner.schema_version, cloned.schema_version);
    }

    // ── is_valid_degradation_code ─────────────────────────────────────────────

    #[test]
    fn degradation_partial_is_valid() {
        assert!(is_valid_degradation_code("partial"));
        assert!(DegradationLevel::is_valid_code("partial"));
    }

    #[test]
    fn degradation_degraded_is_valid() {
        assert!(is_valid_degradation_code("degraded"));
        assert!(DegradationLevel::is_valid_code("degraded"));
    }

    #[test]
    fn degradation_offline_is_valid() {
        assert!(is_valid_degradation_code("offline"));
        assert!(DegradationLevel::is_valid_code("offline"));
    }

    #[test]
    fn unknown_degradation_code_is_invalid() {
        assert!(!is_valid_degradation_code("unknown"));
        assert!(!DegradationLevel::is_valid_code("PARTIAL"));
        assert!(!DegradationLevel::is_valid_code(""));
        assert!(!DegradationLevel::is_valid_code("full"));
    }

    // ── KNOWN_SCHEMA_VERSIONS ────────────────────────────────────────────────

    #[test]
    fn known_schema_versions_contains_2_0() {
        assert!(KNOWN_SCHEMA_VERSIONS.contains(&"2.0"));
    }

    #[test]
    fn known_schema_versions_does_not_contain_legacy() {
        assert!(!KNOWN_SCHEMA_VERSIONS.contains(&"1.0"));
        assert!(!KNOWN_SCHEMA_VERSIONS.contains(&"0.1"));
    }
}
