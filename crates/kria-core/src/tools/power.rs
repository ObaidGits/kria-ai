//! Power/session tools: `lock_screen`, `sleep`, `hibernate`, `shutdown_system`,
//! `reboot_system`.
//!
//! linux-os-control-production **Task 2.4** — "Migrate lock, suspend,
//! hibernate, shutdown and reboot" (OSC-004, OSC-005, OSC-020).
//!
//! These handlers no longer build a `sh -c` string, call
//! `tokio::process::Command` directly, or dispatch through
//! `vm_dispatch_command_with_sudo`. They reach host effects **only** through
//! the injected [`OsControlRuntime`] +
//! `os_control::power::session::PowerSessionControl` provider (see
//! `os_power_session_unavailable` below, mirroring
//! `system_config.rs::os_audio_unavailable` / `os_power_unavailable`). Until a
//! live `logind`/`loginctl` provider is composed into the runtime by a
//! desktop/server startup root, the handlers fail closed with the frozen
//! `Unavailable` envelope and **never** fall back to an ungoverned subprocess
//! or a `sudo` privilege-escalation path — D-Bus/Polkit denial for these
//! operations remains denied with no fallback (OSC-004).
//!
//! `ToolContext` deliberately does not carry the `ExecutionGrant`/resource-
//! lease/audit-admission plumbing a real mutation requires (the same Tasks
//! 2.1–2.3 scoping decision), so a tool handler's only reachable outcome here
//! is the frozen `Unavailable` envelope; the governed
//! `PowerSessionControl` lifecycle itself (idempotency, dispatch,
//! verification, hibernate-availability, accepted semantics, no-rollback) is
//! covered end-to-end against a fake transport in
//! `tests/os_control_session_lifecycle.rs`.

use crate::infra::ToolResult;
use crate::os_control::contract::SafeText;
use crate::os_control::{OsControlError, OsControlRuntime};
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::ToolContext;
use async_trait::async_trait;
use std::sync::Arc;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

/// Return the governed OS-control `Unavailable` envelope for a power-session
/// tool.
///
/// Migrated power-session handlers (`lock_screen`/`sleep`/`hibernate`/
/// `shutdown_system`/`reboot_system`) reach host effects **only** through the
/// injected [`OsControlRuntime`] +
/// `os_control::power::session::PowerSessionControl` provider — never a
/// direct subprocess, `sh -c` string, or `vm_dispatch_command_with_sudo` call
/// (Task 2.4 completion proof). Until a live `logind`/`loginctl` provider is
/// composed into the runtime (desktop startup root), the handlers fail closed
/// with this frozen envelope. D-Bus/Polkit denial has no broader fallback:
/// this is the *only* non-success outcome a handler here can ever produce.
/// Drive one governed power-session operation.
///
/// All five session operations (lock, suspend, hibernate, shutdown, reboot) share
/// this path; only the op and its canonical params differ. Four of them end the
/// session, so they can never reach a comparator-driven `Verified` state — the
/// request's own frozen comparator encodes that, which is why it is read from the
/// request rather than chosen here.
async fn run_power_session(
    ctx: &ToolContext,
    tool: &str,
    op: crate::os_control::power::session::PowerSessionOp,
    params: serde_json::Value,
) -> ToolResult {
    use crate::tools::os_governed as gov;

    let resolved = match gov::resolve(ctx, tool) {
        Ok(resolved) => resolved,
        Err(result) => return result,
    };
    let provider = match resolved.runtime.power_session(tool) {
        Ok(provider) => provider,
        Err(error) => return gov::os_error(&error),
    };
    let call = match gov::mutation_call(ctx, &resolved.runtime, tool) {
        Ok(call) => call,
        Err(result) => return result,
    };

    let request = crate::os_control::power::session::PowerSessionRequest {
        action: tool.to_string(),
        params,
        op,
    };
    let desired = request.desired_state();
    let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);

    gov::run_mutation(
        tool,
        &resolved.runtime,
        provider,
        call,
        &request,
        &desired,
        &plan,
    )
    .await
}

fn os_power_session_unavailable(runtime: Option<&Arc<OsControlRuntime>>, tool: &str) -> ToolResult {
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

struct LockScreen;

#[async_trait]
impl ToolHandler for LockScreen {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        // No context path: cannot reach the governed runtime; fail closed with
        // the frozen envelope rather than invoking any process directly.
        os_power_session_unavailable(None, "lock_screen")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        if !cfg!(target_os = "linux") {
            return ToolResult::err("lock_screen not implemented for this OS");
        }

        run_power_session(
            &ctx,
            "lock_screen",
            crate::os_control::power::session::PowerSessionOp::Lock,
            serde_json::json!({}),
        )
        .await
    }
}

struct Sleep;

#[async_trait]
impl ToolHandler for Sleep {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_power_session_unavailable(None, "sleep")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        if !cfg!(target_os = "linux") {
            return ToolResult::err("sleep not implemented for this OS");
        }

        // Session-ending: the governed PowerSessionControl provider owns the
        // actual suspend dispatch, reaching only `Accepted` (never `Verified`)
        // through the runtime.
        run_power_session(
            &ctx,
            "sleep",
            crate::os_control::power::session::PowerSessionOp::Suspend,
            serde_json::json!({}),
        )
        .await
    }
}

struct Hibernate;

#[async_trait]
impl ToolHandler for Hibernate {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_power_session_unavailable(None, "hibernate")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        if !cfg!(target_os = "linux") {
            return ToolResult::err("hibernate not implemented for this OS");
        }

        // The governed PowerSessionControl provider probes hibernate
        // availability *before* dispatch and reports `Unsupported`/
        // `Unavailable` rather than a fabricated acceptance when hibernation
        // is not available on this host.
        run_power_session(
            &ctx,
            "hibernate",
            crate::os_control::power::session::PowerSessionOp::Hibernate,
            serde_json::json!({}),
        )
        .await
    }
}

struct ShutdownSystem;

#[async_trait]
impl ToolHandler for ShutdownSystem {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_power_session_unavailable(None, "shutdown_system")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        if !cfg!(target_os = "linux") {
            return ToolResult::err("shutdown_system not implemented for this OS");
        }

        // Validate/thread the requested delay; the governed
        // PowerSessionControl provider owns the actual dispatch (an
        // immediate `loginctl poweroff`/logind `PowerOff` call). KRIA-owned
        // cancellable delayed-shutdown scheduling is Task 3.8's job — this
        // slice never builds a `shutdown +N` shell string.
        // The delay is threaded to the provider as a canonical parameter; this
        // slice never builds a `shutdown +N` shell string (Task 3.8 owns
        // KRIA-side cancellable scheduling).
        let delay_minutes = params["delay_minutes"].as_u64().unwrap_or(0);
        run_power_session(
            &ctx,
            "shutdown_system",
            crate::os_control::power::session::PowerSessionOp::Shutdown { delay_minutes },
            serde_json::json!({ "delay_minutes": delay_minutes }),
        )
        .await
    }
}

struct RebootSystem;

#[async_trait]
impl ToolHandler for RebootSystem {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_power_session_unavailable(None, "reboot_system")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        if !cfg!(target_os = "linux") {
            return ToolResult::err("reboot_system not implemented for this OS");
        }

        // Session-ending, non-cancellable once accepted: the governed
        // PowerSessionControl provider owns the actual dispatch, reaching
        // only `Accepted` through the runtime and never claiming rollback
        // (OSC-006.6).
        run_power_session(
            &ctx,
            "reboot_system",
            crate::os_control::power::session::PowerSessionOp::Reboot,
            serde_json::json!({}),
        )
        .await
    }
}

pub fn register(reg: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        // GREEN
        (
            ToolDef {
                name: "lock_screen".into(),
                description: "Lock the screen".into(),
                category: "power".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(LockScreen),
        ),
        // YELLOW
        (
            ToolDef {
                name: "sleep".into(),
                description: "Put system to sleep".into(),
                category: "power".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(Sleep),
        ),
        (
            ToolDef {
                name: "hibernate".into(),
                description: "Hibernate the system".into(),
                category: "power".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(Hibernate),
        ),
        // RED
        (
            ToolDef {
                name: "shutdown_system".into(),
                description: "Shutdown the system".into(),
                category: "power".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![param(
                    "delay_minutes",
                    "integer",
                    "Delay in minutes (default 0)",
                    false,
                )],
            },
            Arc::new(ShutdownSystem),
        ),
        (
            ToolDef {
                name: "reboot_system".into(),
                description: "Reboot the system".into(),
                category: "power".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(RebootSystem),
        ),
    ];
    for (def, handler) in tools {
        reg.register(def, handler);
    }
}
