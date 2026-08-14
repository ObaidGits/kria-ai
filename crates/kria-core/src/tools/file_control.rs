//! File-control tool handlers — trash restore, archive listing, and ownership.
//!
//! linux-os-control-production tasks **3.1** (OSC-011).
//!
//! Every handler routes through [`crate::tools::os_governed`].
//!
//! # Why this file is deliberately small
//!
//! Task 3.1 names six tools. Three have a governed domain behind them and are
//! implemented here. The other three — `delete_permanently`,
//! `set_file_permissions`, `append_file` — have **no port operation yet**, and a
//! handler that invented one would be an ungoverned write to the user's files.
//! They stay unimplemented rather than half-implemented; the ratchet in
//! `tests/os_control_handler_wiring.rs` records that honestly.
//!
//! # The hazard this domain guards
//!
//! Restoring from Trash writes into a path the user may have refilled since. An
//! occupied target is a **conflict to report**, never a silent overwrite — so the
//! resolution is an explicit caller choice, and the default is to fail.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::infra::ToolResult;
use crate::os_control::files::trash::{RestoreResolution, TrashItemId, TrashOp, TrashRequest};
use crate::safety::RiskLevel;
use crate::tools::os_governed as gov;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::ToolContext;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

/// Read a required path parameter.
///
/// Rejected rather than normalised: `..` is refused outright here, and the domain
/// re-validates identity (device + inode) before acting. Escaping a traversal
/// would still leave the caller believing it addressed a different file.
fn required_path(params: &serde_json::Value, field: &str) -> Result<PathBuf, ToolResult> {
    let raw = params[field].as_str().unwrap_or("").trim();
    if raw.is_empty() {
        return Err(ToolResult::err(format!("`{field}` is required")));
    }
    if raw.chars().any(char::is_control) || raw.contains('\0') {
        return Err(ToolResult::err(format!(
            "`{field}` contains control characters"
        )));
    }
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(ToolResult::err(format!("`{field}` must be an absolute path")));
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ToolResult::err(format!(
            "`{field}` must not contain `..`: the resolved target could differ from the one approved"
        )));
    }
    Ok(path)
}

// ─────────────────────────────────────────────────────────────────────────────
// restore_trash_item
// ─────────────────────────────────────────────────────────────────────────────

struct RestoreTrashItem;

#[async_trait]
impl ToolHandler for RestoreTrashItem {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "restore_trash_item")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "restore_trash_item";
        let item = params["item_id"].as_str().unwrap_or("").trim().to_string();
        if item.is_empty() {
            return ToolResult::err(
                "`item_id` is required: a Trash item is identified by its stored id, not by its original path (several trashed items can share one)",
            );
        }

        // The default is `Fail`. Overwriting a file the user put back at the
        // original path would destroy data they never mentioned.
        let resolution = match params["on_conflict"].as_str().unwrap_or("fail") {
            "fail" => RestoreResolution::Fail,
            "rename" => RestoreResolution::Rename,
            "replace" => RestoreResolution::Replace,
            other => {
                return ToolResult::err(format!(
                    "`on_conflict` must be one of fail, rename, replace (got `{other}`)"
                ))
            }
        };

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.trash(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let request = TrashRequest {
            action: tool.to_string(),
            params,
            op: TrashOp::Restore {
                item_id: TrashItemId::new(item),
                resolution,
            },
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            tool,
            &resolved.runtime,
            provider,
            &call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// list_archive
// ─────────────────────────────────────────────────────────────────────────────

struct ListArchive;

#[async_trait]
impl ToolHandler for ListArchive {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "list_archive")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "list_archive";
        let archive = match required_path(&params, "archive") {
            Ok(path) => path,
            Err(result) => return result,
        };
        let cursor = params["cursor"]
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or(0);
        let limit = params["limit"]
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or(200);

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.archive(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        // Listing extracts nothing: a read admission is all this needs.
        let _call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };

        match provider.list_entries(&archive, cursor, limit).await {
            Ok(page) => ToolResult::ok(serde_json::json!({
                "entries": page.entries.iter().map(|e| serde_json::json!({
                    "name": e.name,
                    "uncompressed_size": e.uncompressed_size,
                    "compressed_size": e.compressed_size,
                    "is_directory": e.is_dir,
                })).collect::<Vec<_>>(),
                // The total lets a caller tell a full listing from a bounded page.
                "total_entries": page.total_entries,
                "returned": page.entries.len(),
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// set_file_ownership
// ─────────────────────────────────────────────────────────────────────────────

struct SetFileOwnership;

#[async_trait]
impl ToolHandler for SetFileOwnership {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_file_ownership")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "set_file_ownership";
        let path = match required_path(&params, "path") {
            Ok(path) => path,
            Err(result) => return result,
        };
        let owner = params["owner"].as_str().unwrap_or("").trim().to_string();
        if owner.is_empty() {
            return ToolResult::err("`owner` is required (an existing local user name or uid)");
        }

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.ownership(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };

        // The owner must be an EXISTING local identity, resolved by the domain.
        // Accepting a bare uid would let a typo hand a file to an unrelated
        // account, and this operation needs the privileged broker anyway — it
        // reports Unavailable until that service is installed, never a silent
        // no-op.
        // The owner must already exist locally: the provider verifies the identity
        // before applying, so a typo cannot create or orphan an account. This
        // operation needs the privileged broker, so it reports Unavailable until
        // that service is installed — never a silent no-op.
        let request = crate::os_control::files::OwnershipRequest {
            action: tool.to_string(),
            params: params.clone(),
            path,
            owner: crate::os_control::broker::protocol::ExistingLocalIdentity {
                uid: u32::try_from(params["uid"].as_u64().unwrap_or(0)).unwrap_or(0),
                name: crate::os_control::contract::SafeText::new(owner.as_str()),
            },
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            tool,
            &resolved.runtime,
            provider,
            &call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Direct file mutations: permissions, append, permanent delete
// ─────────────────────────────────────────────────────────────────────────────

/// Drive one governed direct-file mutation.
///
/// The desired state is derived from the **observed** state, so an append's
/// postcondition is the real current size plus what is being written rather than
/// an assumed one.
async fn run_file_attribute(
    ctx: &ToolContext,
    tool: &str,
    params: serde_json::Value,
    path: std::path::PathBuf,
    op: crate::os_control::files::attributes::FileAttributeOp,
) -> ToolResult {
    use crate::os_control::files::attributes::{FileAttributeRequest, FileAttributeState};

    let resolved = match gov::resolve(ctx, tool) {
        Ok(resolved) => resolved,
        Err(result) => return result,
    };
    let provider = match resolved.runtime.file_attributes(tool) {
        Ok(provider) => provider,
        Err(error) => return gov::os_error(&error),
    };
    let call = match gov::mutation_call(ctx, &resolved.runtime, tool) {
        Ok(call) => call,
        Err(result) => return result,
    };

    // Read the current facts first so the postcondition is derived, not guessed.
    let facts = match provider.facts(call.observation(), &path).await {
        Ok(facts) => facts,
        Err(error) => return gov::os_error(&error),
    };
    let observed = FileAttributeState {
        focus: op.focus(),
        exists: facts.exists,
        mode: facts.mode,
        size_bytes: facts.size_bytes,
    };

    let request = FileAttributeRequest {
        action: tool.to_string(),
        params,
        path,
        op,
    };
    let desired = request.desired_state(&observed);
    let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
    gov::run_mutation(
        tool,
        &resolved.runtime,
        provider,
        &call,
        &request,
        &desired,
        &plan,
    )
    .await
}

struct SetFilePermissions;

#[async_trait]
impl ToolHandler for SetFilePermissions {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_file_permissions")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        use crate::os_control::files::attributes::{validate_mode, FileAttributeOp};

        let tool = "set_file_permissions";
        let path = match required_path(&params, "path") {
            Ok(path) => path,
            Err(result) => return result,
        };
        // Accept an octal string ("644") or an integer. A decimal 644 would mean
        // something entirely different, so a string is parsed as octal explicitly.
        let mode = match params["mode"].as_str() {
            Some(raw) => match u32::from_str_radix(raw.trim().trim_start_matches("0o"), 8) {
                Ok(mode) => mode,
                Err(_) => {
                    return ToolResult::err(
                        "`mode` must be octal, e.g. \"644\" or \"755\"",
                    )
                }
            },
            None => match params["mode"].as_u64() {
                Some(raw) => match u32::try_from(raw) {
                    Ok(mode) => mode,
                    Err(_) => return ToolResult::err("`mode` is out of range"),
                },
                None => return ToolResult::err("`mode` is required (octal, e.g. \"644\")"),
            },
        };
        // setuid/setgid/sticky are refused here, before any admission is spent.
        let mode = match validate_mode(mode) {
            Ok(mode) => mode,
            Err(error) => return gov::os_error(&error),
        };
        run_file_attribute(&ctx, tool, params, path, FileAttributeOp::SetPermissions { mode }).await
    }
}

struct AppendFile;

#[async_trait]
impl ToolHandler for AppendFile {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "append_file")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        use crate::os_control::files::attributes::FileAttributeOp;

        let tool = "append_file";
        let path = match required_path(&params, "path") {
            Ok(path) => path,
            Err(result) => return result,
        };
        // Copied out before `params` moves into the request.
        let Some(content) = params["content"].as_str().map(str::to_string) else {
            return ToolResult::err("`content` is required");
        };
        // Bounded: an unbounded append would let one call fill the disk.
        const MAX_APPEND_BYTES: usize = 1024 * 1024;
        if content.len() > MAX_APPEND_BYTES {
            return ToolResult::err(format!(
                "`content` exceeds the {MAX_APPEND_BYTES}-byte append bound"
            ));
        }
        if content.is_empty() {
            // A zero-byte append has no postcondition to verify.
            return ToolResult::err("`content` must not be empty");
        }
        run_file_attribute(
            &ctx,
            tool,
            params,
            path,
            FileAttributeOp::Append {
                bytes: content.as_bytes().to_vec(),
            },
        )
        .await
    }
}

struct DeletePermanently;

#[async_trait]
impl ToolHandler for DeletePermanently {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "delete_permanently")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        use crate::os_control::files::attributes::FileAttributeOp;

        let tool = "delete_permanently";
        let path = match required_path(&params, "path") {
            Ok(path) => path,
            Err(result) => return result,
        };
        // Irreversible and bypasses Trash entirely, so the receipt claims no
        // rollback and the domain refuses a directory.
        run_file_attribute(&ctx, tool, params, path, FileAttributeOp::DeletePermanently).await
    }
}

/// Register the file-control tool surface.
pub fn register(registry: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "restore_trash_item".into(),
                description: "Restore an item from Trash to its original location".into(),
                category: "files".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param(
                        "item_id",
                        "string",
                        "Trash item id (several trashed items can share an original path)",
                        true,
                    ),
                    param(
                        "on_conflict",
                        "string",
                        "fail (default), rename, or replace — what to do if the original path is occupied",
                        false,
                    ),
                ],
            },
            Arc::new(RestoreTrashItem),
        ),
        (
            ToolDef {
                name: "list_archive".into(),
                description: "List the contents of an archive without extracting it".into(),
                category: "files".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("archive", "string", "Absolute path to the archive", true),
                    param("cursor", "integer", "Page offset", false),
                    param("limit", "integer", "Maximum entries", false),
                ],
            },
            Arc::new(ListArchive),
        ),
        (
            ToolDef {
                name: "set_file_ownership".into(),
                description: "Change a file's owner (requires the privileged broker)".into(),
                category: "files".into(),
                // Privileged and not self-reversible without knowing the old owner.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("path", "string", "Absolute path whose owner changes", true),
                    param("owner", "string", "An existing local user name or uid", true),
                ],
            },
            Arc::new(SetFileOwnership),
        ),
        (
            ToolDef {
                name: "set_file_permissions".into(),
                description: "Change a file's permission bits".into(),
                category: "files".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("path", "string", "Absolute path", true),
                    param(
                        "mode",
                        "string",
                        "Octal mode such as \"644\" or \"755\". setuid, setgid and sticky bits are refused.",
                        true,
                    ),
                ],
            },
            Arc::new(SetFilePermissions),
        ),
        (
            ToolDef {
                name: "append_file".into(),
                description: "Append text to an existing file".into(),
                category: "files".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("path", "string", "Absolute path to an existing file", true),
                    param("content", "string", "Text to append (bounded to 1 MiB)", true),
                ],
            },
            Arc::new(AppendFile),
        ),
        (
            ToolDef {
                name: "delete_permanently".into(),
                description: "Delete a file permanently, bypassing Trash — irreversible".into(),
                category: "files".into(),
                // Irreversible: there is no restore path.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![param("path", "string", "Absolute path to the file", true)],
            },
            Arc::new(DeletePermanently),
        ),
    ];

    for (def, handler) in tools {
        registry.register(def, handler);
    }
}
