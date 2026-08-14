//! Automation tool handlers — scheduled-task patching and in-tree workflows.
//!
//! linux-os-control-production task **4.5** (OSC-023).
//!
//! # The hole this file must not reopen
//!
//! An earlier implementation of `create_scheduled_task` / `delete_scheduled_task`
//! wrote a `crontab` pipe directly, with no policy, grant, lease, audit or
//! verification. It was **deleted** rather than migrated, because a scheduled task
//! that can run an arbitrary command later is a persistent arbitrary-execution
//! hole that outlives the session and bypasses every guardrail here.
//!
//! So the patch this handler accepts is a **typed** structure —
//! [`TypedAutomationPatch::parse`] validates a closed schedule shape and a
//! *canonical action*, never a shell string. There is deliberately no way to
//! express "run this command" through it.
//!
//! # Optimistic concurrency
//!
//! Both mutations carry the revision the caller read. A task whose configuration
//! changed in between is refused rather than patched against a stale view — the
//! user may have edited it, and silently overwriting that would lose their change.

use std::sync::Arc;

use async_trait::async_trait;

use crate::infra::ToolResult;
use crate::os_control::automation::typed::{
    AutomationId, Revision, TypedAutomationPatch, WorkflowId,
};
use crate::os_control::automation::{AutomationOp, AutomationRequest};
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

/// Read the caller's observed revision.
///
/// Required, not defaulted: a missing revision would mean "patch whatever is
/// there now", which is exactly the stale-write this check exists to prevent.
fn required_revision(params: &serde_json::Value) -> Result<Revision, ToolResult> {
    params["expected_revision"].as_u64().ok_or_else(|| {
        ToolResult::err(
            "`expected_revision` is required: it is the revision you read before deciding, and it \
             stops a concurrent edit from being silently overwritten",
        )
    })
}

/// Drive one governed automation mutation.
async fn run_automation(
    ctx: &ToolContext,
    tool: &str,
    params: serde_json::Value,
    op: AutomationOp,
) -> ToolResult {
    let resolved = match gov::resolve(ctx, tool) {
        Ok(resolved) => resolved,
        Err(result) => return result,
    };
    let provider = match resolved.runtime.automation(tool) {
        Ok(provider) => provider,
        Err(error) => return gov::os_error(&error),
    };
    let call = match gov::mutation_call(ctx, &resolved.runtime, tool) {
        Ok(call) => call,
        Err(result) => return result,
    };
    let request = AutomationRequest {
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
        &call,
        &request,
        &desired,
        &plan,
    )
    .await
}

struct ModifyScheduledTask;

#[async_trait]
impl ToolHandler for ModifyScheduledTask {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "modify_scheduled_task")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "modify_scheduled_task";
        let task_id = match params["task_id"].as_str() {
            Some(raw) => match AutomationId::parse(raw.trim()) {
                Ok(id) => id,
                Err(error) => return gov::os_error(&error),
            },
            None => {
                return ToolResult::err(
                    "`task_id` is required: a task's display NAME is not an identity (it is neither unique nor stable)",
                )
            }
        };
        let expected_revision = match required_revision(&params) {
            Ok(revision) => revision,
            Err(result) => return result,
        };
        // The patch is parsed into a closed typed shape. A shell string cannot be
        // expressed here, which is the whole point.
        let patch = match TypedAutomationPatch::parse(&params["patch"]) {
            Ok(patch) => patch,
            Err(error) => return gov::os_error(&error),
        };
        run_automation(
            &ctx,
            tool,
            params,
            AutomationOp::UpdateTask {
                task_id,
                expected_revision,
                patch,
            },
        )
        .await
    }
}

struct ListWorkflows;

#[async_trait]
impl ToolHandler for ListWorkflows {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "list_workflows")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "list_workflows";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.automation(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        // A read admission: listing reviewed in-tree workflows runs nothing.
        let _call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let cursor = params["cursor"].as_str();
        let limit = params["limit"].as_u64().and_then(|v| usize::try_from(v).ok());

        match provider.list_workflows(cursor, limit) {
            // An empty list is a real answer: no workflow has been reviewed into
            // the in-tree registry yet. It is not an error, and it is not a
            // failure to read.
            Ok(page) => ToolResult::ok(serde_json::json!({
                "workflows": page.items.iter().map(|w| serde_json::json!({
                    "workflow": w.id,
                    "steps": w.steps.len(),
                    "max_step_risk": format!("{:?}", w.max_step_risk()),
                    "fully_reversible": w.fully_reversible(),
                    "revision": w.revision,
                })).collect::<Vec<_>>(),
                "next_cursor": page.next_cursor.as_deref(),
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct RunWorkflow;

#[async_trait]
impl ToolHandler for RunWorkflow {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "run_workflow")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "run_workflow";
        let workflow_id = match params["workflow"].as_str().or(params["workflow_id"].as_str()) {
            Some(raw) => match WorkflowId::parse(raw.trim()) {
                Ok(id) => id,
                Err(error) => return gov::os_error(&error),
            },
            None => return ToolResult::err("`workflow` is required (a reviewed workflow id)"),
        };
        let expected_revision = match required_revision(&params) {
            Ok(revision) => revision,
            Err(result) => return result,
        };
        // Only a workflow already reviewed into the in-tree registry can run: the
        // caller supplies an id, never a definition, so a model cannot author a
        // new sequence of privileged steps at call time.
        run_automation(
            &ctx,
            tool,
            params,
            AutomationOp::RunWorkflow {
                workflow_id,
                expected_revision,
            },
        )
        .await
    }
}

/// Register the automation tool surface.
pub fn register(registry: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "modify_scheduled_task".into(),
                description: "Patch an existing scheduled task's schedule, action, or enabled state"
                    .into(),
                category: "automation".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("task_id", "string", "The task's stable id (not its name)", true),
                    param(
                        "expected_revision",
                        "integer",
                        "The revision you read before deciding; a concurrent edit is refused rather than overwritten",
                        true,
                    ),
                    param(
                        "patch",
                        "object",
                        "Typed patch: schedule, canonical action, and/or enabled. A shell command cannot be expressed.",
                        true,
                    ),
                ],
            },
            Arc::new(ModifyScheduledTask),
        ),
        (
            ToolDef {
                name: "list_workflows".into(),
                description: "List reviewed in-tree workflows and their risk profile".into(),
                category: "automation".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("cursor", "string", "Page cursor", false),
                    param("limit", "integer", "Maximum rows", false),
                ],
            },
            Arc::new(ListWorkflows),
        ),
        (
            ToolDef {
                name: "run_workflow".into(),
                description: "Run a reviewed in-tree workflow by id".into(),
                category: "automation".into(),
                // A workflow runs several steps; its own max step risk can be higher.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("workflow", "string", "The reviewed workflow's id", true),
                    param(
                        "expected_revision",
                        "integer",
                        "The workflow definition revision you read",
                        true,
                    ),
                ],
            },
            Arc::new(RunWorkflow),
        ),
    ];

    for (def, handler) in tools {
        registry.register(def, handler);
    }
}
