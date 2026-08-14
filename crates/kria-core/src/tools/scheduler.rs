//! Scheduler tools: `list_scheduled_tasks`, `create_scheduled_task`,
//! `delete_scheduled_task`.
//!
//! linux-os-control-production **Task 2.5** — "Migrate files, processes,
//! applications, packages, scheduler, disk, clipboard and notifications"
//! (OSC-027).
//!
//! `list_scheduled_tasks` no longer spawns `tokio::process::Command::new("crontab")`/
//! `("systemctl")` directly. It reaches host effects **only** through the
//! injected [`OsControlRuntime`] + `os_control::automation::AutomationControl`
//! provider. Until a live transport is composed into the runtime, the handler
//! fails closed with the frozen `Unavailable` envelope.
//!
//! # Deferred: `create_scheduled_task` / `delete_scheduled_task`
//!
//! The frozen manifest assigns these two operations to **Task 4.5**
//! ("Implement typed automation and event subscriptions", phase F4): they
//! require a typed `TypedSchedule` schema, a `CanonicalCapabilityInvocation`
//! for the contained action, and the `contained_action_risk(action)` risk
//! resolver — none of which this task's F1/F2 foundation provides. Migrating
//! them onto `os_control::automation::AutomationControl` now would mean
//! inventing that authority ad hoc, which the design explicitly reserves for
//! Task 4.5. These two handlers therefore remain on their pre-migration
//! direct `crontab` pipe-write implementation for this task, with an
//! explicit doc-comment marking them as deferred rather than silently
//! migrated. They are **not** claimed as part of this task's completion
//! proof; Task 2.6's direct-process static scan and the design's F4 gate own
//! their eventual cutover.

use crate::infra::ToolResult;
use crate::os_control::contract::SafeText;
use crate::os_control::{OsControlError, OsControlRuntime};
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::os_governed as gov;
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

/// Return the governed OS-control `Unavailable` envelope for
/// `list_scheduled_tasks`.
fn os_automation_unavailable(runtime: Option<&Arc<OsControlRuntime>>, tool: &str) -> ToolResult {
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

struct ListScheduledTasks;
#[async_trait]
impl ToolHandler for ListScheduledTasks {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_automation_unavailable(None, "list_scheduled_tasks")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        // The governed AutomationControl provider owns the actual
        // `crontab -l`/`systemctl --user list-timers` structured-command
        // reads through the runtime.
        let resolved = match gov::resolve(&ctx, "list_scheduled_tasks") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.automation("list_scheduled_tasks") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "list_scheduled_tasks") {
            Ok(call) => call,
            Err(result) => return result,
        };
        match provider.list(call.observation()).await {
            Ok(listing) => ToolResult::ok(serde_json::json!({
                "cron_jobs": listing.cron_jobs,
                "systemd_timers": listing.systemd_timers,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

/// **Deferred to Task 4.5** (see module docs): retains the pre-migration
/// direct `crontab` pipe-write implementation pending the typed
/// `TypedSchedule`/`CanonicalCapabilityInvocation` authority.
struct CreateScheduledTask;
#[async_trait]
impl ToolHandler for CreateScheduledTask {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let ctx_free = params;
        let _ = ctx_free;
        // See `execute_with_context`: there is no ungoverned path.
        gov::os_unavailable(None, "create_scheduled_task")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        // HARD CUTOVER (task 2.6, OSC-027). This handler previously spawned
        // `crontab` directly, installing a persistent scheduled job with NO
        // policy decision, grant, resource lease, audit record or verification —
        // an ungoverned host mutation that bypassed the entire safety layer.
        //
        // That path is deleted rather than kept working: a silent bypass is worse
        // than an unavailable feature. The typed replacement (governed automation
        // create/delete with event subscriptions) is task 4.5's job; the
        // `AutomationControlPort` currently exposes reads only, so until it gains
        // typed mutations this reports the frozen envelope.
        gov::os_unavailable(ctx.os_runtime.as_ref(), "create_scheduled_task")
    }
}

/// **Deferred to Task 4.5** (see module docs).
struct DeleteScheduledTask;
#[async_trait]
impl ToolHandler for DeleteScheduledTask {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let ctx_free = params;
        let _ = ctx_free;
        // See `execute_with_context`: there is no ungoverned path.
        gov::os_unavailable(None, "delete_scheduled_task")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        // HARD CUTOVER (task 2.6, OSC-027). This handler previously spawned
        // `crontab` directly, installing a persistent scheduled job with NO
        // policy decision, grant, resource lease, audit record or verification —
        // an ungoverned host mutation that bypassed the entire safety layer.
        //
        // That path is deleted rather than kept working: a silent bypass is worse
        // than an unavailable feature. The typed replacement (governed automation
        // create/delete with event subscriptions) is task 4.5's job; the
        // `AutomationControlPort` currently exposes reads only, so until it gains
        // typed mutations this reports the frozen envelope.
        gov::os_unavailable(ctx.os_runtime.as_ref(), "delete_scheduled_task")
    }
}

pub fn register(reg: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "list_scheduled_tasks".into(),
                description: "List cron jobs and systemd timers".into(),
                category: "scheduler".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(ListScheduledTasks),
        ),
        (
            ToolDef {
                name: "create_scheduled_task".into(),
                description: "Create a cron job or scheduled task".into(),
                category: "scheduler".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    param(
                        "schedule",
                        "string",
                        "Cron schedule (e.g. '0 * * * *')",
                        true,
                    ),
                    param("command", "string", "Command to run", true),
                ],
            },
            Arc::new(CreateScheduledTask),
        ),
        (
            ToolDef {
                name: "delete_scheduled_task".into(),
                description: "Delete a cron job by pattern".into(),
                category: "scheduler".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![param(
                    "pattern",
                    "string",
                    "Text pattern to match in cron entry",
                    true,
                )],
            },
            Arc::new(DeleteScheduledTask),
        ),
    ];
    for (def, handler) in tools {
        reg.register(def, handler);
    }
}
