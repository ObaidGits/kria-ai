//! v2 command catalogue: kinds, limits, and command/unified routers (design §8, F3.9).
//!
//! This module defines:
//! - [`CommandKind`] — wire-names for command-type operations (preview/commit/undo,
//!   lifecycle, source, goal, health/capabilities, local interchange, async jobs).
//! - [`CommandLimits`] — compile-time hard caps specific to command operations.
//! - [`CommandRouter`] — dispatches command operations, validates payload size,
//!   and handles async job stubs.
//! - [`UnifiedRouter`] — tries [`super::operations::OperationRouter`] first,
//!   then falls back to [`CommandRouter`].

use super::contract::CallerContext;
use super::dto::{GraphRequestV2, GraphResponseV2, TotalSemantics};
use super::error::MemoryApiErrorV2;
use super::operations::OperationRouter;

// ─────────────────────────────────────────────────────────────────────────────
// CommandKind
// ─────────────────────────────────────────────────────────────────────────────

/// All v2 command-type operations that may appear on the wire as
/// `request.operation`.
///
/// Command operations are distinct from query operations ([`super::operations::OperationKind`])
/// in that they mutate state or trigger side-effects (commit, undo, export, …).
/// `CommandKind::from_str` is the canonical parser; callers must not match
/// operation strings by hand.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CommandKind {
    /// Show the projected outcome of a command without committing it
    /// (`"command.preview"`).
    Preview,
    /// Commit a previously previewed command to the authority (`"command.commit"`).
    Commit,
    /// Undo the last committed command (`"command.undo"`).
    Undo,
    /// Entity lifecycle operations — forget/restore/delete/crypto-shred
    /// (`"lifecycle"`).
    Lifecycle,
    /// Source management — consent/ingest/cancel/resume/delete (`"source"`).
    Source,
    /// Goal management — activate/pause/complete/priority (`"goal"`).
    Goal,
    /// Health and capabilities report (`"health"`).
    Health,
    /// Async local authority export to interchange package (`"export"`).
    Export,
    /// Async local authority import from interchange package (`"import"`).
    Import,
    /// Async authority index/vector rebuild (`"rebuild"`).
    Rebuild,
}

impl CommandKind {
    /// Parse a command operation name from its wire-format string.
    ///
    /// Returns `None` for any string that is not a recognized v2 command name.
    /// The router treats `None` as `MemoryApiErrorV2::Unsupported`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use kria_core::memory::api::v2::commands::CommandKind;
    ///
    /// assert_eq!(CommandKind::from_str("command.preview"), Some(CommandKind::Preview));
    /// assert_eq!(CommandKind::from_str("export"), Some(CommandKind::Export));
    /// assert_eq!(CommandKind::from_str("EXPORT"), None); // case-sensitive
    /// assert_eq!(CommandKind::from_str("unknown"), None);
    /// ```
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "command.preview" => Some(Self::Preview),
            "command.commit" => Some(Self::Commit),
            "command.undo" => Some(Self::Undo),
            "lifecycle" => Some(Self::Lifecycle),
            "source" => Some(Self::Source),
            "goal" => Some(Self::Goal),
            "health" => Some(Self::Health),
            "export" => Some(Self::Export),
            "import" => Some(Self::Import),
            "rebuild" => Some(Self::Rebuild),
            _ => None,
        }
    }

    /// The canonical wire-format name for this command kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Preview => "command.preview",
            Self::Commit => "command.commit",
            Self::Undo => "command.undo",
            Self::Lifecycle => "lifecycle",
            Self::Source => "source",
            Self::Goal => "goal",
            Self::Health => "health",
            Self::Export => "export",
            Self::Import => "import",
            Self::Rebuild => "rebuild",
        }
    }

    /// Returns `true` if this command kind triggers an asynchronous background job.
    ///
    /// Async jobs (`Export`, `Import`, `Rebuild`) return `revision: -1` and a
    /// job ID in `recovery_cursor` instead of a direct result.
    pub fn is_async_job(&self) -> bool {
        matches!(self, Self::Export | Self::Import | Self::Rebuild)
    }

    /// Returns `true` if this command validates its `params_json` payload size
    /// against [`CommandLimits::MAX_COMMAND_PAYLOAD_BYTES`].
    pub fn validates_payload(&self) -> bool {
        matches!(self, Self::Preview | Self::Commit | Self::Undo)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CommandLimits
// ─────────────────────────────────────────────────────────────────────────────

/// Hard caps and deadlines specific to command-type operations (design §8.1).
///
/// These are compile-time constants. Command payloads are smaller than query
/// results; async jobs have a longer deadline to accommodate export/import work.
/// No code path may exceed these caps (design invariant A6 Boundedness).
pub struct CommandLimits;

impl CommandLimits {
    /// Maximum command parameter payload in bytes (64 KiB).
    ///
    /// Command params (preview/commit/undo) are smaller than query result
    /// payloads; this cap is intentionally lower than
    /// [`super::contract::OperationLimits::MAX_PAYLOAD_BYTES`] (1 MiB).
    pub const MAX_COMMAND_PAYLOAD_BYTES: u32 = 65_536;

    /// Maximum items per page for goal/source list operations.
    pub const MAX_PAGE_ITEMS: u32 = 200;

    /// Deadline for asynchronous export/import/rebuild jobs in milliseconds
    /// (30 seconds).
    pub const ASYNC_JOB_DEADLINE_MS: u64 = 30_000;

    /// Deadline for preview operations in milliseconds (2 seconds).
    ///
    /// Preview must be fast: the user is waiting for a projected diff before
    /// deciding to commit.
    pub const PREVIEW_DEADLINE_MS: u64 = 2_000;
}

// ─────────────────────────────────────────────────────────────────────────────
// Hash helper (FNV-1a, same as operations.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn fnv1a_hex(s: &str) -> String {
    const FNV_OFFSET: u64 = 14_695_981_039_346_656_037;
    const FNV_PRIME: u64 = 1_099_511_628_211;
    let mut hash: u64 = FNV_OFFSET;
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{:016x}", hash)
}

// ─────────────────────────────────────────────────────────────────────────────
// CommandRouter
// ─────────────────────────────────────────────────────────────────────────────

/// Stub command router that validates command request envelopes and dispatches
/// to the appropriate stub handler.
///
/// # Contract
///
/// - Unknown `operation` string → [`MemoryApiErrorV2::Unsupported`].
/// - `Preview`/`Commit`/`Undo`: payload size (approximated by JSON serialization)
///   must be ≤ [`CommandLimits::MAX_COMMAND_PAYLOAD_BYTES`]; otherwise
///   [`MemoryApiErrorV2::Limit`].
/// - Async jobs (`Export`/`Import`/`Rebuild`): returns `revision: -1` with a
///   stub job ID in `recovery_cursor`.
/// - `Health`: returns one item `{"status": "healthy"}`.
/// - All other commands: returns a stub empty [`GraphResponseV2`].
pub struct CommandRouter;

impl CommandRouter {
    /// Validate the command request envelope and return a stub response.
    ///
    /// # Errors
    ///
    /// - [`MemoryApiErrorV2::Unsupported`] when `request.operation` is not a
    ///   recognized v2 command name.
    /// - [`MemoryApiErrorV2::Limit`] when `Preview`/`Commit`/`Undo` payload
    ///   exceeds [`CommandLimits::MAX_COMMAND_PAYLOAD_BYTES`].
    pub fn dispatch(
        _caller: &CallerContext,
        request: &GraphRequestV2,
    ) -> Result<GraphResponseV2, MemoryApiErrorV2> {
        // 1. Parse the command kind.
        let kind = CommandKind::from_str(&request.operation).ok_or_else(|| {
            MemoryApiErrorV2::Unsupported {
                feature: request.operation.clone(),
            }
        })?;

        // 2. Validate payload size for state-mutating commands.
        if kind.validates_payload() {
            let payload_bytes = serde_json::to_vec(&request.params_json)
                .map(|v| v.len())
                .unwrap_or(0) as u32;
            if payload_bytes > CommandLimits::MAX_COMMAND_PAYLOAD_BYTES {
                return Err(MemoryApiErrorV2::Limit {
                    operation: request.operation.clone(),
                    limit: format!(
                        "params_json payload {} bytes exceeds hard cap {} bytes",
                        payload_bytes,
                        CommandLimits::MAX_COMMAND_PAYLOAD_BYTES
                    ),
                });
            }
        }

        // 3. Build the appropriate stub response.
        let query_hash = fnv1a_hex(kind.as_str());

        // Async jobs: revision -1 + stub job ID in recovery_cursor.
        if kind.is_async_job() {
            return Ok(GraphResponseV2 {
                schema_version: "2.0".to_string(),
                revision: -1,
                query_hash,
                items: Vec::new(),
                total_count: TotalSemantics::Exact(0),
                truncated: false,
                truncation_reason: None,
                recovery_cursor: Some(format!("job:{}", kind.as_str())),
                warnings: Vec::new(),
                degradation: None,
            });
        }

        // Health: return one item with status=healthy.
        if kind == CommandKind::Health {
            return Ok(GraphResponseV2 {
                schema_version: "2.0".to_string(),
                revision: 0,
                query_hash,
                items: vec![serde_json::json!({"status": "healthy"})],
                total_count: TotalSemantics::Exact(1),
                truncated: false,
                truncation_reason: None,
                recovery_cursor: None,
                warnings: Vec::new(),
                degradation: None,
            });
        }

        // All other commands: stub empty response.
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
// UnifiedRouter
// ─────────────────────────────────────────────────────────────────────────────

/// Unified router that dispatches any v2 `memory.v2` request.
///
/// Tries [`OperationRouter`] first (query operations: search, neighborhood,
/// path, …). Falls back to [`CommandRouter`] for command operations (preview,
/// commit, health, export, …). Returns `Unsupported` if neither router
/// recognizes the operation.
///
/// This is the single entry point for the `memory.v2` public contract.
pub struct UnifiedRouter;

impl UnifiedRouter {
    /// Dispatch any v2 memory request to the appropriate router.
    ///
    /// # Errors
    ///
    /// - [`MemoryApiErrorV2::Unsupported`] when neither the operation router
    ///   nor the command router recognizes `request.operation`.
    /// - Any error that `OperationRouter::dispatch` or `CommandRouter::dispatch`
    ///   would return for their respective operations.
    pub fn dispatch(
        caller: &CallerContext,
        request: &GraphRequestV2,
    ) -> Result<GraphResponseV2, MemoryApiErrorV2> {
        // Try the query operation router first.
        match OperationRouter::dispatch(caller, request) {
            Ok(resp) => return Ok(resp),
            Err(MemoryApiErrorV2::Unsupported { .. }) => {
                // Not a query operation — fall through to the command router.
            }
            Err(other) => return Err(other),
        }

        // Fall back to the command router.
        CommandRouter::dispatch(caller, request)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::api::v2::contract::TransportKind;
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

    // ── CommandKind::from_str ─────────────────────────────────────────────────

    #[test]
    fn from_str_command_preview_returns_preview() {
        assert_eq!(
            CommandKind::from_str("command.preview"),
            Some(CommandKind::Preview)
        );
    }

    #[test]
    fn from_str_command_commit_returns_commit() {
        assert_eq!(
            CommandKind::from_str("command.commit"),
            Some(CommandKind::Commit)
        );
    }

    #[test]
    fn from_str_command_undo_returns_undo() {
        assert_eq!(
            CommandKind::from_str("command.undo"),
            Some(CommandKind::Undo)
        );
    }

    #[test]
    fn from_str_lifecycle_returns_lifecycle() {
        assert_eq!(
            CommandKind::from_str("lifecycle"),
            Some(CommandKind::Lifecycle)
        );
    }

    #[test]
    fn from_str_source_returns_source() {
        assert_eq!(CommandKind::from_str("source"), Some(CommandKind::Source));
    }

    #[test]
    fn from_str_goal_returns_goal() {
        assert_eq!(CommandKind::from_str("goal"), Some(CommandKind::Goal));
    }

    #[test]
    fn from_str_health_returns_health() {
        assert_eq!(CommandKind::from_str("health"), Some(CommandKind::Health));
    }

    #[test]
    fn from_str_export_returns_export() {
        assert_eq!(CommandKind::from_str("export"), Some(CommandKind::Export));
    }

    #[test]
    fn from_str_import_returns_import() {
        assert_eq!(CommandKind::from_str("import"), Some(CommandKind::Import));
    }

    #[test]
    fn from_str_rebuild_returns_rebuild() {
        assert_eq!(CommandKind::from_str("rebuild"), Some(CommandKind::Rebuild));
    }

    #[test]
    fn from_str_unknown_returns_none() {
        assert_eq!(CommandKind::from_str("unknown_cmd"), None);
    }

    #[test]
    fn from_str_is_case_sensitive() {
        assert_eq!(CommandKind::from_str("EXPORT"), None);
        assert_eq!(CommandKind::from_str("Export"), None);
        assert_eq!(CommandKind::from_str("HEALTH"), None);
    }

    #[test]
    fn from_str_empty_string_returns_none() {
        assert_eq!(CommandKind::from_str(""), None);
    }

    // ── CommandLimits ─────────────────────────────────────────────────────────

    #[test]
    fn max_command_payload_bytes_is_64kib() {
        assert_eq!(CommandLimits::MAX_COMMAND_PAYLOAD_BYTES, 65_536);
    }

    #[test]
    fn max_page_items_is_200() {
        assert_eq!(CommandLimits::MAX_PAGE_ITEMS, 200);
    }

    #[test]
    fn async_job_deadline_ms_is_30_seconds() {
        assert_eq!(CommandLimits::ASYNC_JOB_DEADLINE_MS, 30_000);
    }

    #[test]
    fn preview_deadline_ms_is_2_seconds() {
        assert_eq!(CommandLimits::PREVIEW_DEADLINE_MS, 2_000);
    }

    // ── CommandRouter::dispatch — unknown command → Unsupported ───────────────

    #[test]
    fn command_router_dispatch_unknown_returns_unsupported() {
        let caller = make_caller();
        let req = make_request("totally_unknown");
        let result = CommandRouter::dispatch(&caller, &req);
        assert!(
            matches!(result, Err(MemoryApiErrorV2::Unsupported { ref feature }) if feature == "totally_unknown"),
            "expected Unsupported, got {:?}",
            result
        );
    }

    // ── CommandRouter::dispatch — health → items with status=healthy ──────────

    #[test]
    fn command_router_dispatch_health_returns_healthy_item() {
        let caller = make_caller();
        let req = make_request("health");
        let resp = CommandRouter::dispatch(&caller, &req).expect("health should succeed");
        assert_eq!(resp.schema_version, "2.0");
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0]["status"], "healthy");
        assert_eq!(resp.total_count, TotalSemantics::Exact(1));
    }

    // ── CommandRouter::dispatch — async jobs → revision -1 + job_id cursor ───

    #[test]
    fn command_router_dispatch_export_returns_async_stub() {
        let caller = make_caller();
        let req = make_request("export");
        let resp = CommandRouter::dispatch(&caller, &req).expect("export should succeed");
        assert_eq!(resp.revision, -1, "async jobs must return revision -1");
        assert!(
            resp.recovery_cursor
                .as_deref()
                .is_some_and(|c| c.starts_with("job:")),
            "async job must set recovery_cursor to a job: prefixed ID"
        );
    }

    #[test]
    fn command_router_dispatch_import_returns_async_stub() {
        let caller = make_caller();
        let req = make_request("import");
        let resp = CommandRouter::dispatch(&caller, &req).expect("import should succeed");
        assert_eq!(resp.revision, -1);
        assert!(resp.recovery_cursor.is_some());
    }

    #[test]
    fn command_router_dispatch_rebuild_returns_async_stub() {
        let caller = make_caller();
        let req = make_request("rebuild");
        let resp = CommandRouter::dispatch(&caller, &req).expect("rebuild should succeed");
        assert_eq!(resp.revision, -1);
        assert!(resp.recovery_cursor.is_some());
    }

    // ── CommandRouter::dispatch — payload size limit for preview/commit/undo ──

    #[test]
    fn command_router_dispatch_preview_payload_too_large_returns_limit() {
        let caller = make_caller();
        // Build a params_json that serializes to more than 64 KiB.
        let big_string = "x".repeat(70_000);
        let mut req = make_request("command.preview");
        req.params_json = json!({"data": big_string});
        let result = CommandRouter::dispatch(&caller, &req);
        assert!(
            matches!(result, Err(MemoryApiErrorV2::Limit { ref operation, .. }) if operation == "command.preview"),
            "expected Limit error for oversized payload, got {:?}",
            result
        );
    }

    #[test]
    fn command_router_dispatch_preview_valid_payload_succeeds() {
        let caller = make_caller();
        let mut req = make_request("command.preview");
        req.params_json = json!({"entity_id": "e-123", "proposed": {"label": "New Name"}});
        let result = CommandRouter::dispatch(&caller, &req);
        assert!(result.is_ok(), "small payload should succeed: {:?}", result);
    }

    // ── CommandRouter::dispatch — stub responses for non-health/non-async ─────

    #[test]
    fn command_router_dispatch_lifecycle_returns_stub() {
        let caller = make_caller();
        let resp = CommandRouter::dispatch(&caller, &make_request("lifecycle"))
            .expect("lifecycle should succeed");
        assert_eq!(resp.schema_version, "2.0");
        assert_eq!(resp.revision, 0);
        assert!(resp.items.is_empty());
    }

    #[test]
    fn command_router_dispatch_goal_returns_stub() {
        let caller = make_caller();
        let resp =
            CommandRouter::dispatch(&caller, &make_request("goal")).expect("goal should succeed");
        assert_eq!(resp.schema_version, "2.0");
        assert!(resp.items.is_empty());
    }

    // ── UnifiedRouter::dispatch ───────────────────────────────────────────────

    #[test]
    fn unified_router_dispatch_search_succeeds() {
        let caller = make_caller();
        let req = make_request("search");
        let result = UnifiedRouter::dispatch(&caller, &req);
        assert!(
            result.is_ok(),
            "search via UnifiedRouter should succeed: {:?}",
            result
        );
        assert_eq!(result.unwrap().schema_version, "2.0");
    }

    #[test]
    fn unified_router_dispatch_health_succeeds() {
        let caller = make_caller();
        let req = make_request("health");
        let resp = UnifiedRouter::dispatch(&caller, &req)
            .expect("health via UnifiedRouter should succeed");
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0]["status"], "healthy");
    }

    #[test]
    fn unified_router_dispatch_totally_unknown_returns_unsupported() {
        let caller = make_caller();
        let req = make_request("totally_unknown");
        let result = UnifiedRouter::dispatch(&caller, &req);
        assert!(
            matches!(result, Err(MemoryApiErrorV2::Unsupported { ref feature }) if feature == "totally_unknown"),
            "expected Unsupported from UnifiedRouter, got {:?}",
            result
        );
    }

    #[test]
    fn unified_router_dispatch_export_returns_async_stub() {
        let caller = make_caller();
        let req = make_request("export");
        let resp = UnifiedRouter::dispatch(&caller, &req)
            .expect("export via UnifiedRouter should succeed");
        assert_eq!(resp.revision, -1);
    }

    #[test]
    fn unified_router_dispatch_neighborhood_succeeds() {
        let caller = make_caller();
        let req = make_request("neighborhood");
        let result = UnifiedRouter::dispatch(&caller, &req);
        assert!(
            result.is_ok(),
            "neighborhood via UnifiedRouter should succeed: {:?}",
            result
        );
    }
}
