//! Thin adapter helpers for the Tauri and Axum transport layers (design §3,
//! F3.9, MGR-003, MGR-020).
//!
//! This module contains **no domain semantics**. It exists solely to:
//! 1. Name the two supported adapter kinds (`Tauri`, `Axum`).
//! 2. Surface per-adapter capability differences as explicit constants so the
//!    domain never guesses what the transport supports.
//! 3. Build a [`CallerContext`] from adapter-specific inputs.
//! 4. Validate that a [`GraphRequestV2`] is permitted for a given adapter kind.
//!
//! ## Capability differences
//!
//! | Capability | Tauri (local) | Axum (HTTP) |
//! |---|---|---|
//! | `export` | ✓ | ✗ |
//! | `import` | ✓ | ✗ |
//! | `rebuild` | ✓ | ✗ |
//! | Requires auth | No (always loopback) | Yes |
//! | Max request size | `MAX_PAYLOAD_BYTES` | `MAX_PAYLOAD_BYTES / 2` |
//!
//! Local-only operations are available only through the Tauri adapter because
//! they interact directly with the local file system or authority store in ways
//! that are not safe to expose over a network transport (design §3, MGR-003).

use super::contract::{CallerContext, OperationLimits, TransportKind};
use super::dto::GraphRequestV2;
use super::error::MemoryApiErrorV2;

// ─────────────────────────────────────────────────────────────────────────────
// LOCAL_ONLY_OPERATIONS
// ─────────────────────────────────────────────────────────────────────────────

/// Operations that are available only through the Tauri (local) adapter and
/// must be rejected by the Axum (HTTP) adapter (design §3, MGR-003).
///
/// These operations interact with the local file system or authority store in
/// ways that are unsafe to expose over a network transport:
/// - `export` — writes a verified interchange package to the local disk.
/// - `import` — reads a verified interchange package from the local disk.
/// - `rebuild` — triggers a full derived-index rebuild from authority.
pub const LOCAL_ONLY_OPERATIONS: &[&str] = &["export", "import", "rebuild"];

// ─────────────────────────────────────────────────────────────────────────────
// AdapterKind
// ─────────────────────────────────────────────────────────────────────────────

/// The two supported adapter transports for the memory v2 API.
///
/// The domain core never infers the adapter kind from other request fields; it
/// is always set explicitly by the adapter at the transport boundary and
/// recorded in the [`CallerContext`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdapterKind {
    /// Tauri IPC adapter — local desktop only; loopback; no authentication
    /// required; full operation set including local-only operations.
    Tauri,
    /// Axum HTTP/SSE adapter — may be loopback or remote; requires
    /// authentication; local-only operations are not permitted.
    Axum,
}

// ─────────────────────────────────────────────────────────────────────────────
// AdapterLimits
// ─────────────────────────────────────────────────────────────────────────────

/// Per-adapter-kind capability limits (design §3, MGR-003, MGR-020).
///
/// # Example
///
/// ```rust
/// use kria_memory::api::v2::adapters::{AdapterKind, AdapterLimits};
///
/// assert!(!AdapterLimits::requires_auth(AdapterKind::Tauri));
/// assert!(AdapterLimits::requires_auth(AdapterKind::Axum));
/// assert!(AdapterLimits::local_only_operations().contains(&"export"));
/// ```
pub struct AdapterLimits;

impl AdapterLimits {
    /// Returns the slice of operation names that are available only via the
    /// Tauri adapter and must be rejected by the Axum adapter.
    ///
    /// This is a shared constant independent of adapter kind because the
    /// _set_ of local-only operations is the same for both adapters — only the
    /// _permission_ differs (Tauri allows them; Axum rejects them).
    pub fn local_only_operations() -> &'static [&'static str] {
        LOCAL_ONLY_OPERATIONS
    }

    /// Returns `true` when this adapter kind requires caller authentication.
    ///
    /// - `Tauri` — always loopback/local; no authentication required.
    /// - `Axum` — may be remote; authentication is mandatory.
    pub fn requires_auth(adapter: AdapterKind) -> bool {
        match adapter {
            AdapterKind::Tauri => false,
            AdapterKind::Axum => true,
        }
    }

    /// Returns the maximum request size in bytes for this adapter kind.
    ///
    /// - `Tauri` — [`OperationLimits::MAX_PAYLOAD_BYTES`] (1 MiB); local
    ///   inter-process communication is cheap.
    /// - `Axum` — `MAX_PAYLOAD_BYTES / 2` (512 KiB); a conservative cap for
    ///   network requests to limit denial-of-service surface (MGR-003).
    pub fn max_request_size_bytes(adapter: AdapterKind) -> u32 {
        match adapter {
            AdapterKind::Tauri => OperationLimits::MAX_PAYLOAD_BYTES,
            AdapterKind::Axum => OperationLimits::MAX_PAYLOAD_BYTES / 2,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AdapterContext
// ─────────────────────────────────────────────────────────────────────────────

/// Builds a [`CallerContext`] from adapter-specific inputs.
///
/// Each adapter (Tauri command handler, Axum route handler) calls
/// `build_caller_context` at the transport boundary to produce the
/// [`CallerContext`] that is passed through to the domain core. The adapter
/// is the *only* constructor of [`CallerContext`]; the domain never mutates or
/// invents these fields (design §3, invariant A5 Isolation).
pub struct AdapterContext;

impl AdapterContext {
    /// Build a [`CallerContext`] from adapter-specific inputs.
    ///
    /// Sets `transport` from `adapter`:
    /// - `Tauri` → [`TransportKind::Tauri`]
    /// - `Axum` → [`TransportKind::Http`]
    ///
    /// # Parameters
    ///
    /// - `adapter` — the adapter kind making this request.
    /// - `caller_id` — stable, opaque identity of the calling actor.
    /// - `namespace` — policy namespace.
    /// - `scope` — fine-grained scope within the namespace.
    /// - `sensitivity` — sensitivity ceiling (0–3).
    /// - `policy_version` — pinned policy version string.
    ///
    /// # Panics
    ///
    /// Does not panic. Invalid values (e.g. empty `caller_id`) are passed
    /// through unchanged; adapters must validate before calling.
    pub fn build_caller_context(
        adapter: AdapterKind,
        caller_id: &str,
        namespace: &str,
        scope: &str,
        sensitivity: u8,
        policy_version: &str,
    ) -> CallerContext {
        let transport = match adapter {
            AdapterKind::Tauri => TransportKind::Tauri,
            AdapterKind::Axum => TransportKind::Http,
        };

        CallerContext {
            caller_id: caller_id.to_string(),
            namespace: namespace.to_string(),
            scope: scope.to_string(),
            sensitivity,
            policy_version: policy_version.to_string(),
            transport,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// validate_adapter_request
// ─────────────────────────────────────────────────────────────────────────────

/// Validate that a [`GraphRequestV2`] is permitted for the given adapter kind.
///
/// Currently enforces **one** capability constraint: operations in
/// [`LOCAL_ONLY_OPERATIONS`] are not permitted via the Axum adapter.
///
/// # Errors
///
/// - [`MemoryApiErrorV2::Unsupported`] — when `adapter` is [`AdapterKind::Axum`]
///   and `request.operation` is in [`LOCAL_ONLY_OPERATIONS`].
///
/// # Tauri
///
/// All operations are permitted through the Tauri adapter; this function
/// always returns `Ok(())` for `AdapterKind::Tauri`.
///
/// # Example
///
/// ```rust
/// use kria_memory::api::v2::adapters::{AdapterKind, validate_adapter_request};
/// use kria_memory::api::v2::dto::GraphRequestV2;
/// use kria_memory::api::v2::error::MemoryApiErrorV2;
/// use serde_json::json;
///
/// let export_req = GraphRequestV2 {
///     operation: "export".to_string(),
///     params_json: json!({}),
///     revision: None,
///     schema_version: "2.0".to_string(),
///     policy_hash: None,
///     cursor: None,
///     deadline_ms: None,
/// };
///
/// // Axum rejects local-only operations.
/// let axum_result = validate_adapter_request(AdapterKind::Axum, &export_req);
/// assert!(matches!(axum_result, Err(MemoryApiErrorV2::Unsupported { .. })));
///
/// // Tauri allows all operations.
/// let tauri_result = validate_adapter_request(AdapterKind::Tauri, &export_req);
/// assert!(tauri_result.is_ok());
/// ```
pub fn validate_adapter_request(
    adapter: AdapterKind,
    request: &GraphRequestV2,
) -> Result<(), MemoryApiErrorV2> {
    if adapter == AdapterKind::Axum && LOCAL_ONLY_OPERATIONS.contains(&request.operation.as_str()) {
        return Err(MemoryApiErrorV2::Unsupported {
            feature: request.operation.clone(),
        });
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── helpers ──────────────────────────────────────────────────────────────

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

    // ── AdapterContext::build_caller_context ──────────────────────────────────

    #[test]
    fn build_caller_context_tauri_sets_transport_tauri() {
        let ctx = AdapterContext::build_caller_context(
            AdapterKind::Tauri,
            "user-1",
            "personal",
            "default",
            0,
            "v1",
        );
        assert_eq!(ctx.transport, TransportKind::Tauri);
        assert_eq!(ctx.caller_id, "user-1");
        assert_eq!(ctx.namespace, "personal");
        assert_eq!(ctx.scope, "default");
        assert_eq!(ctx.sensitivity, 0);
        assert_eq!(ctx.policy_version, "v1");
    }

    #[test]
    fn build_caller_context_axum_sets_transport_http() {
        let ctx = AdapterContext::build_caller_context(
            AdapterKind::Axum,
            "svc-account",
            "work",
            "api",
            2,
            "v2-hash",
        );
        assert_eq!(ctx.transport, TransportKind::Http);
        assert_eq!(ctx.caller_id, "svc-account");
        assert_eq!(ctx.namespace, "work");
        assert_eq!(ctx.scope, "api");
        assert_eq!(ctx.sensitivity, 2);
        assert_eq!(ctx.policy_version, "v2-hash");
    }

    // ── validate_adapter_request ──────────────────────────────────────────────

    #[test]
    fn validate_axum_export_returns_unsupported() {
        let req = make_request("export");
        let result = validate_adapter_request(AdapterKind::Axum, &req);
        assert!(
            matches!(&result, Err(MemoryApiErrorV2::Unsupported { feature }) if feature == "export"),
            "expected Unsupported(export), got {:?}",
            result
        );
    }

    #[test]
    fn validate_axum_import_returns_unsupported() {
        let req = make_request("import");
        let result = validate_adapter_request(AdapterKind::Axum, &req);
        assert!(
            matches!(&result, Err(MemoryApiErrorV2::Unsupported { feature }) if feature == "import"),
            "expected Unsupported(import), got {:?}",
            result
        );
    }

    #[test]
    fn validate_axum_rebuild_returns_unsupported() {
        let req = make_request("rebuild");
        let result = validate_adapter_request(AdapterKind::Axum, &req);
        assert!(
            matches!(&result, Err(MemoryApiErrorV2::Unsupported { feature }) if feature == "rebuild"),
            "expected Unsupported(rebuild), got {:?}",
            result
        );
    }

    #[test]
    fn validate_tauri_export_succeeds() {
        let req = make_request("export");
        let result = validate_adapter_request(AdapterKind::Tauri, &req);
        assert!(
            result.is_ok(),
            "Tauri should allow export; got {:?}",
            result
        );
    }

    #[test]
    fn validate_tauri_import_succeeds() {
        let req = make_request("import");
        assert!(validate_adapter_request(AdapterKind::Tauri, &req).is_ok());
    }

    #[test]
    fn validate_tauri_rebuild_succeeds() {
        let req = make_request("rebuild");
        assert!(validate_adapter_request(AdapterKind::Tauri, &req).is_ok());
    }

    #[test]
    fn validate_axum_search_succeeds() {
        let req = make_request("search");
        let result = validate_adapter_request(AdapterKind::Axum, &req);
        assert!(result.is_ok(), "Axum should allow search; got {:?}", result);
    }

    #[test]
    fn validate_axum_neighborhood_succeeds() {
        let req = make_request("neighborhood");
        assert!(validate_adapter_request(AdapterKind::Axum, &req).is_ok());
    }

    #[test]
    fn validate_axum_inspect_succeeds() {
        let req = make_request("inspect");
        assert!(validate_adapter_request(AdapterKind::Axum, &req).is_ok());
    }

    // ── local_only_operations ─────────────────────────────────────────────────

    #[test]
    fn local_only_operations_includes_export() {
        assert!(
            AdapterLimits::local_only_operations().contains(&"export"),
            "local_only_operations should contain 'export'"
        );
    }

    #[test]
    fn local_only_operations_includes_import() {
        assert!(
            AdapterLimits::local_only_operations().contains(&"import"),
            "local_only_operations should contain 'import'"
        );
    }

    #[test]
    fn local_only_operations_includes_rebuild() {
        assert!(
            AdapterLimits::local_only_operations().contains(&"rebuild"),
            "local_only_operations should contain 'rebuild'"
        );
    }

    #[test]
    fn local_only_operations_has_exactly_three_entries() {
        assert_eq!(
            AdapterLimits::local_only_operations().len(),
            3,
            "expected exactly 3 local-only operations"
        );
    }

    // ── AdapterLimits ─────────────────────────────────────────────────────────

    #[test]
    fn tauri_does_not_require_auth() {
        assert!(!AdapterLimits::requires_auth(AdapterKind::Tauri));
    }

    #[test]
    fn axum_requires_auth() {
        assert!(AdapterLimits::requires_auth(AdapterKind::Axum));
    }

    #[test]
    fn tauri_max_request_size_equals_max_payload_bytes() {
        assert_eq!(
            AdapterLimits::max_request_size_bytes(AdapterKind::Tauri),
            OperationLimits::MAX_PAYLOAD_BYTES
        );
    }

    #[test]
    fn axum_max_request_size_is_half_of_max_payload_bytes() {
        assert_eq!(
            AdapterLimits::max_request_size_bytes(AdapterKind::Axum),
            OperationLimits::MAX_PAYLOAD_BYTES / 2
        );
    }

    #[test]
    fn local_only_operations_constant_and_method_agree() {
        assert_eq!(
            AdapterLimits::local_only_operations(),
            LOCAL_ONLY_OPERATIONS
        );
    }
}
