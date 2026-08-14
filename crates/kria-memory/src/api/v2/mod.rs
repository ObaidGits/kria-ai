//! Memory API v2 — runtime-serializable public contract (F3.9).
//!
//! This module is the sole public surface for `memory.v2` operations.
//! All types defined here are runtime-serializable (via `serde`) so they cross
//! the Tauri IPC bridge, the Axum JSON layer, and test boundaries without
//! conversion (design §8, MGR-006–009, MGR-017, MGR-020).
//!
//! ## Module structure
//!
//! | Submodule | Contents |
//! |---|---|
//! | [`contract`] | `CallerContext`, `TransportKind`, `OperationLimits` (hard caps), `ApiVersion` |
//! | [`cursor`] | `CursorPayload`, `CursorManager`, `CursorError`, `DEFAULT_CURSOR_TTL_SECS` |
//! | [`dto`] | `GraphRequestV2`, `GraphResponseV2`, `CommittedCommandV2`, `TotalSemantics`, `ApiWarning`, `DegradationInfo` |
//! | [`error`] | `MemoryApiErrorV2` typed error enum |
//! | [`capabilities`] | `Capability`, `CapabilityStatus`, `CapabilityMatrix` |
//! | [`operations`] | `OperationKind`, `OperationDefaults`, `OperationRouter` |
//! | [`commands`] | `CommandKind`, `CommandLimits`, `CommandRouter`, `UnifiedRouter` |
//! | [`patch`] | `AuthorityPatch`, `PatchEntry`, `ChangeKind`, `PatchApplyResult`, `IgnoreReason`, `PatchRetentionPolicy`, `PatchValidator` |
//!
//! ## Design invariants
//!
//! - Adapters (Tauri, Axum) authenticate, build a [`contract::CallerContext`],
//!   and serialize. They contain no domain semantics.
//! - Unknown optional fields are preserved by interchange but ignored safely;
//!   unknown required enum variants or schema versions return
//!   [`error::MemoryApiErrorV2::Unsupported`] and deny writes.
//! - Operation hard caps are `const` values in [`contract::OperationLimits`];
//!   no code path may exceed them.
//! - Cursors bind `{schema_version, query_hash, policy_hash, revision,
//!   last_sort_key, expires_at}` under HMAC-SHA256; expired or incompatible
//!   cursors return `Cursor` or `Refetch` errors (design §5.2).

pub mod adapters;
pub mod capabilities;
pub mod commands;
pub mod contract;
pub mod cursor;
pub mod dto;
pub mod error;
pub mod operations;
pub mod patch;
pub mod validation;

// Convenience re-exports so callers can write `v2::CallerContext` etc.
pub use adapters::{
    validate_adapter_request, AdapterContext, AdapterKind, AdapterLimits, LOCAL_ONLY_OPERATIONS,
};
pub use capabilities::{Capability, CapabilityMatrix, CapabilityStatus};
pub use commands::{CommandKind, CommandLimits, CommandRouter, UnifiedRouter};
pub use contract::{ApiVersion, CallerContext, OperationLimits, TransportKind};
pub use cursor::{CursorError, CursorManager, CursorPayload, DEFAULT_CURSOR_TTL_SECS};
pub use dto::{
    ApiWarning, CommittedCommandV2, DegradationInfo, DegradationLevel, GraphRequestV2,
    GraphResponseV2, TotalSemantics,
};
pub use error::MemoryApiErrorV2;
pub use operations::{OperationDefaults, OperationKind, OperationRouter};
pub use patch::{
    AuthorityPatch, ChangeKind, IgnoreReason, PatchApplyResult, PatchEntry, PatchRetentionPolicy,
    PatchValidator,
};
pub use validation::{
    is_valid_degradation_code, validate_request, validate_schema_version, ValidatedRequest,
    KNOWN_SCHEMA_VERSIONS,
};
