//! Process tools: `set_process_priority`, `get_active_connections`.
//!
//! linux-os-control-production **Task 2.5** — "Migrate files, processes,
//! applications, packages, scheduler, disk, clipboard and notifications"
//! (OSC-013).
//!
//! `set_process_priority` no longer spawns `tokio::process::Command::new("renice")`.
//! It reaches host effects **only** through the injected [`OsControlRuntime`]
//! + `os_control::processes::ProcessControl` provider (see
//! `os_process_unavailable` below, mirroring
//! `power.rs::os_power_session_unavailable`). Until a live native-syscall
//! provider is composed into the runtime (desktop startup root), the handler
//! fails closed with the frozen `Unavailable` envelope and **never** falls
//! back to an ungoverned `renice` subprocess.
//!
//! `get_active_connections` is a diagnostic network-connections view (`ss
//! -tuln`), a distinct subsystem from the canonical `ConnectivityControl`
//! DTOs — the legacy-difference report records it explicitly as out of scope
//! for the v1 OS-control manifest, so it is left as-is (read-only, no host
//! mutation).
//!
//! `ToolContext` deliberately does not carry the `ExecutionGrant`/resource-
//! lease/audit-admission plumbing a real mutation requires (the same Tasks
//! 2.1–2.4 scoping decision), so `set_process_priority`'s only reachable
//! outcome here is the frozen `Unavailable` envelope; the governed
//! `ProcessControl` lifecycle itself (idempotency, native syscall dispatch,
//! verification, PID-reuse safety) is covered end-to-end against a fake
//! transport in `tests/os_control_process_lifecycle.rs`.

use crate::infra::ToolResult;
use crate::os_control::contract::SafeText;
use crate::os_control::{OsControlError, OsControlRuntime};
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::os_governed as gov;
use crate::tools::ToolContext;
use async_trait::async_trait;
use std::sync::Arc;

/// Parse a `{pid, start_time}` `ProcessIdentity` param object (frozen
/// manifest schema). `start_time` defaults to `0` ("not captured") for
/// backward compatibility with a bare pid — see
/// `os_control::processes::ProcessIdentity`'s doc comment on the narrower
/// guarantee that implies.
fn parse_process_identity(
    params: &serde_json::Value,
) -> Result<crate::os_control::processes::ProcessIdentity, ToolResult> {
    let process = &params["process"];
    let pid = process["pid"]
        .as_u64()
        .or_else(|| params["pid"].as_u64())
        .ok_or_else(|| ToolResult::err("process.pid is required"))?;
    let start_time = process["start_time"].as_u64().unwrap_or(0);
    Ok(crate::os_control::processes::ProcessIdentity::new(
        pid as u32,
        start_time,
    ))
}

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

/// Return the governed OS-control `Unavailable` envelope for a process tool.
///
/// The migrated `set_process_priority` handler reaches host effects **only**
/// through the injected [`OsControlRuntime`] +
/// `os_control::processes::ProcessControl` provider — never a direct
/// `renice` subprocess (Task 2.5 completion proof). Until a live
/// native-syscall provider is composed into the runtime, the handler fails
/// closed with this frozen envelope.
fn os_process_unavailable(runtime: Option<&Arc<OsControlRuntime>>, tool: &str) -> ToolResult {
    let err = match runtime {
        Some(rt) => rt.unavailable(tool),
        None => OsControlError::Unavailable {
            provider: None,
            reason: SafeText::new("OS control runtime is not injected in this build"),
            retryable: false,
        },
    };
    ToolResult::err_with_data(err.code(), err.to_envelope())
}

struct SetProcessPriority;

#[async_trait]
impl ToolHandler for SetProcessPriority {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        // No context path: cannot reach the governed runtime; fail closed
        // with the frozen envelope rather than invoking `renice` directly.
        os_process_unavailable(None, "set_process_priority")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        // The governed ProcessControl provider owns the actual
        // `setpriority(2)` syscall + verification through the runtime.
        let resolved = match gov::resolve(&ctx, "set_process_priority") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.processes("set_process_priority") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "set_process_priority") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let pid = params["pid"].as_u64().unwrap_or(0) as u32;
        let start_time = params["start_time"].as_u64().unwrap_or(0);
        let nice = params["nice"].as_i64().unwrap_or(0).clamp(-20, 19) as i32;
        let identity = crate::os_control::processes::ProcessIdentity::new(pid, start_time);
        let request = crate::os_control::processes::ProcessRequest {
            action: "set_process_priority".to_string(),
            params: params.clone(),
            op: crate::os_control::processes::ProcessOp::SetPriority { identity, nice },
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "set_process_priority",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

struct ListProcesses;
#[async_trait]
impl ToolHandler for ListProcesses {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_process_unavailable(None, "list_processes")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        // The governed ProcessControl provider owns the actual content-free
        // process-table read through the runtime. `command_metadata` on
        // every returned observation is always `NotRequested` — normal
        // process listing can never expose command content (OSC-013.4).
        let resolved = match gov::resolve(&ctx, "list_processes") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.processes("list_processes") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "list_processes") {
            Ok(call) => call,
            Err(result) => return result,
        };
        // Content-free listing: never argv, environment, or cwd (OSC-013.5).
        let filter = crate::os_control::processes::ProcessFilter {
            state: None,
            owner: None,
            app_id: None,
            min_cpu_percent: None,
            min_memory_bytes: None,
        };
        let limit = params["limit"].as_u64().unwrap_or(50).min(500) as usize;
        let cursor = params["cursor"].as_u64().unwrap_or(0) as usize;
        match provider
            .list_observations(call.observation(), &filter, cursor, limit)
            .await
        {
            Ok(page) => ToolResult::ok(serde_json::json!({
                "processes": page
                    .items
                    .iter()
                    .map(|o| serde_json::json!({
                    "pid": o.identity.pid,
                    "executable": o.executable_label,
                    "owner": o.owner,
                    "cpu_percent": o.cpu_percent,
                    "memory_bytes": o.memory_bytes,
                    "start_time_ms": o.start_time_ms,
                }))
                    .collect::<Vec<_>>(),
                "truncated": page.truncated,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct GetProcessInfo;
#[async_trait]
impl ToolHandler for GetProcessInfo {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_process_unavailable(None, "get_process_info")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        if let Err(err) = parse_process_identity(&params) {
            return err;
        }
        // The governed ProcessControl provider owns the actual PID-reuse-safe
        // content-free observation read through the runtime.
        let resolved = match gov::resolve(&ctx, "get_process_info") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.processes("get_process_info") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "get_process_info") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let pid = params["pid"].as_u64().unwrap_or(0) as u32;
        // start_time binds the identity so a recycled PID cannot be confused for
        // the original process; 0 means "not captured by the caller".
        let start_time = params["start_time"].as_u64().unwrap_or(0);
        let identity = crate::os_control::processes::ProcessIdentity::new(pid, start_time);
        match provider.read_observation(call.observation(), identity).await {
            Ok(o) => ToolResult::ok(serde_json::json!({
                    "pid": o.identity.pid,
                    "executable": o.executable_label,
                    "owner": o.owner,
                    "cpu_percent": o.cpu_percent,
                    "memory_bytes": o.memory_bytes,
                    "start_time_ms": o.start_time_ms,
                })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct GetProcessCommandMetadata;
#[async_trait]
impl ToolHandler for GetProcessCommandMetadata {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_process_unavailable(None, "get_process_command_metadata")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        if let Err(err) = parse_process_identity(&params) {
            return err;
        }
        let purpose = params["purpose"].as_str().unwrap_or("").trim();
        if purpose.is_empty() {
            return ToolResult::err("purpose is required");
        }
        // The governed ProcessControl provider owns the actual bounded-argv
        // read through the runtime. The RED risk tier + mandatory approval
        // (registered below) gate reaching this handler at all; the result
        // — when a live provider is composed — is a `BoundedCommandMetadata`
        // that is EphemeralCurrentTurn (never persisted, never re-served on
        // a later turn). Never returns environment or cwd (OSC-013.5).
        let resolved = match gov::resolve(&ctx, "get_process_command_metadata") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.processes("get_process_command_metadata") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "get_process_command_metadata") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let pid = params["pid"].as_u64().unwrap_or(0) as u32;
        let start_time = params["start_time"].as_u64().unwrap_or(0);
        let identity = crate::os_control::processes::ProcessIdentity::new(pid, start_time);
        // RED and privacy-sensitive: a stated purpose is required, and the provider
        // never returns environment or cwd (OSC-013.5).
        let purpose = params["purpose"].as_str().unwrap_or("diagnostics");
        match provider
            .read_command_metadata(call.observation(), identity, purpose)
            .await
        {
            Ok(meta) => ToolResult::ok(serde_json::json!({ "metadata": format!("{meta:?}") })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct GetActiveConnections;
#[async_trait]
impl ToolHandler for GetActiveConnections {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        let output = tokio::process::Command::new("ss")
            .args(["-tuln"])
            .output()
            .await;
        match output {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout).to_string();
                ToolResult::ok_text(text)
            }
            _ => ToolResult::err("failed to get active connections"),
        }
    }
}

pub fn register(reg: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        // GREEN
        (
            ToolDef {
                name: "get_active_connections".into(),
                description: "Get active network connections".into(),
                category: "process".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetActiveConnections),
        ),
        // GREEN (content-free by default — never argv/environment/cwd)
        (
            ToolDef {
                name: "list_processes".into(),
                description: "List running processes with content-free details: identity, executable label, owner, state, CPU, memory, and start time. Never includes command arguments, environment variables, or working directory — use get_process_command_metadata for those (requires separate approval).".into(),
                category: "process".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("state", "string", "Filter by lifecycle state: running, sleeping, stopped, zombie", false),
                    param("owner", "string", "Filter by owning local identity", false),
                    param("app_id", "string", "Filter by associated application id", false),
                    param("min_cpu_percent", "integer", "Only include processes at or above this CPU percentage", false),
                    param("min_memory_bytes", "integer", "Only include processes at or above this memory usage in bytes", false),
                    param("cursor", "string", "Pagination cursor from a previous call", false),
                    param("limit", "integer", "Maximum processes to return per page", false),
                ],
            },
            Arc::new(ListProcesses),
        ),
        (
            ToolDef {
                name: "get_process_info".into(),
                description: "Get content-free details for one process by its (pid, start_time) identity: executable label, owner, state, CPU, memory, and start time. Never includes command arguments, environment variables, or working directory.".into(),
                category: "process".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("process", "object", "Process identity {pid, start_time} (from list_processes)", true),
                    param("pid", "integer", "Bare PID (backward-compatible fallback when process object is omitted)", false),
                ],
            },
            Arc::new(GetProcessInfo),
        ),
        // RED (mandatory approval, ephemeral bounded argv only — never
        // environment/cwd; see os_control::processes::BoundedCommandMetadata)
        (
            ToolDef {
                name: "get_process_command_metadata".into(),
                description: "Show the command-line arguments of a specific process by its (pid, start_time) identity. This is a SEPARATE, explicitly-approved action from normal process listing — call this only when the user explicitly asks to see command arguments. Never returns environment variables or the working directory. The result is ephemeral to this turn and is never persisted, remembered, or re-servable later.".into(),
                category: "process".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    param("process", "object", "Process identity {pid, start_time} (from list_processes/get_process_info)", true),
                    param("purpose", "string", "Bounded purpose text explaining why command arguments are needed", true),
                ],
            },
            Arc::new(GetProcessCommandMetadata),
        ),
        // RED
        (
            ToolDef {
                name: "set_process_priority".into(),
                description: "Set process priority/niceness".into(),
                category: "process".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    param("pid", "integer", "Process ID", true),
                    param("priority", "integer", "Nice value (-20 to 19)", true),
                    param("start_time", "integer", "Process start time in ms since epoch, for PID-reuse-safe targeting (optional; from get_process_info)", false),
                ],
            },
            Arc::new(SetProcessPriority),
        ),
    ];
    for (def, handler) in tools {
        reg.register(def, handler);
    }
}
