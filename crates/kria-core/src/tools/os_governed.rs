//! Shared plumbing for canonical OS tool handlers (linux-os-control-production).
//!
//! Every OS handler performs the same three steps — resolve the governed runtime,
//! drive the domain provider through the runtime, render the receipt — so those
//! steps live here once rather than 46 times. A handler's own job is only to parse
//! its input and build its domain request, desired state, and mutation plan.
//!
//! Nothing here touches a process, a bus, or a device: all host contact happens
//! inside the provider, behind the runtime's sealed mutation permit.

use std::sync::Arc;

use crate::infra::ToolResult;
use crate::os_control::contract::SafeText;
use crate::os_control::governed::{
    audit_store, execute_governed_mutation, execute_governed_read, GovernedOutcome, OsGovernedCall,
};
use crate::os_control::runtime::{MutationPlan, OsControlRuntime};
use crate::os_control::{ComparatorKind, OsControlError, ProviderId, ReceiptId, Tolerance};
use crate::tools::ToolContext;

/// The frozen `Unavailable` envelope for a tool that cannot reach a provider.
///
/// Used whenever the runtime seam is absent, no provider is composed for the
/// domain, or the action was not admitted — never a bare string, so the caller
/// always receives the same machine-readable shape.
#[must_use]
pub fn os_unavailable(runtime: Option<&Arc<OsControlRuntime>>, tool: &str) -> ToolResult {
    let error = match runtime {
        Some(rt) => rt.unavailable(tool),
        None => OsControlError::Unavailable {
            provider: None,
            reason: SafeText::new("OS control runtime is not injected in this build"),
            retryable: false,
        },
    };
    ToolResult::err_with_data(error.code(), error.to_envelope())
}

/// Render any `OsControlError` as the frozen envelope.
#[must_use]
pub fn os_error(error: &OsControlError) -> ToolResult {
    ToolResult::err_with_data(error.code(), error.to_envelope())
}

/// Render a governed mutation receipt as a tool result.
///
/// Reports only what the receipt actually proves: `changed` and `verified` come
/// from the runtime's own verification, never from the fact that a command was
/// dispatched. `durably_recorded` distinguishes a closed action from one whose
/// terminal audit record is pending recovery.
#[must_use]
pub fn render_receipt<O>(tool: &str, outcome: &GovernedOutcome<O>) -> ToolResult {
    let summary = outcome.receipt.safe_summary();
    ToolResult::ok(serde_json::json!({
        "tool": tool,
        "lifecycle": summary.lifecycle().as_str(),
        "changed": summary.changed(),
        "verified": matches!(
            summary.lifecycle(),
            crate::os_control::ActionLifecycle::Verified
        ),
        "rollback_available": outcome.receipt.rollback_available(),
        "durably_recorded": outcome.durably_recorded(),
        "incident_codes": summary
            .incident_codes()
            .iter()
            .map(|code| code.as_str().to_string())
            .collect::<Vec<_>>(),
    }))
}

/// A mutation plan with the domain's own comparator and tolerance.
///
/// The receipt id is fresh per action so two runs of the same tool are never
/// conflated in the audit ledger.
#[must_use]
pub fn plan_for(
    provider: ProviderId,
    comparator: ComparatorKind,
    tolerance: Option<Tolerance>,
) -> MutationPlan {
    MutationPlan {
        receipt_id: ReceiptId::new(uuid::Uuid::now_v7().to_string()),
        provider,
        comparator,
        tolerance,
        deadline_ms: 500,
        // Never advertise an inverse the runtime cannot actually perform; domains
        // that mint a rollback token override this in their own handler.
        rollback: crate::os_control::runtime::RollbackPlan::Unavailable,
        latency_ms: 0,
    }
}

/// The resolved pieces a governed handler needs, or the envelope explaining why
/// it cannot proceed.
pub struct Resolved {
    /// The governed runtime seam.
    pub runtime: Arc<OsControlRuntime>,
    /// The provider identity for the mutation plan.
    pub provider_id: ProviderId,
}

/// Resolve the runtime and provider identity for `tool`, or return the frozen
/// envelope.
///
/// Handlers call this first so the "no runtime / no provider" paths are identical
/// everywhere.
pub fn resolve(ctx: &ToolContext, tool: &str) -> Result<Resolved, ToolResult> {
    let Some(runtime) = ctx.os_runtime.clone() else {
        return Err(os_unavailable(None, tool));
    };
    let provider_id = match runtime.probe_provider(tool) {
        Ok(id) => id,
        Err(error) => return Err(os_error(&error)),
    };
    Ok(Resolved {
        runtime,
        provider_id,
    })
}

/// The governed call for a MUTATION, or the frozen envelope when the action was
/// not admitted with a permit.
pub fn mutation_call<'a>(
    ctx: &'a ToolContext,
    runtime: &Arc<OsControlRuntime>,
    tool: &str,
) -> Result<&'a OsGovernedCall, ToolResult> {
    match ctx.os_call() {
        Some(call) if call.is_mutation() => Ok(call),
        // Either the gate did not admit a host mutation (blocked / awaiting
        // approval) or this action was admitted read-only. Fail closed.
        _ => Err(os_unavailable(Some(runtime), tool)),
    }
}

/// The governed call for a READ, or the frozen envelope when none was admitted.
pub fn read_call<'a>(
    ctx: &'a ToolContext,
    runtime: &Arc<OsControlRuntime>,
    tool: &str,
) -> Result<&'a OsGovernedCall, ToolResult> {
    ctx.os_call()
        .ok_or_else(|| os_unavailable(Some(runtime), tool))
}

/// Drive one governed mutation and render its receipt.
pub async fn run_mutation<R, O, P>(
    tool: &str,
    runtime: &OsControlRuntime,
    provider: &P,
    call: &OsGovernedCall,
    request: &R,
    desired: &O,
    plan: &MutationPlan,
) -> ToolResult
where
    R: Send + Sync,
    O: crate::os_control::NormalizedObservation + Clone + Send + Sync,
    P: crate::os_control::contract::DesiredStateControl<R, O> + ?Sized,
{
    match execute_governed_mutation(
        runtime,
        provider,
        call,
        audit_store(),
        request,
        desired,
        plan,
    )
    .await
    {
        Ok(outcome) => render_receipt(tool, &outcome),
        Err(error) => os_error(&error),
    }
}

/// Drive one governed read and render the observation with a caller-supplied
/// projection.
///
/// The projection keeps domain shapes out of this module while guaranteeing every
/// read surfaces through the same success/error path.
pub async fn run_read<R, O, P, F>(
    provider: &P,
    call: &OsGovernedCall,
    request: &R,
    project: F,
) -> ToolResult
where
    R: Send + Sync,
    O: crate::os_control::NormalizedObservation + Clone + Send + Sync,
    P: crate::os_control::contract::DesiredStateControl<R, O> + ?Sized,
    F: FnOnce(&O) -> serde_json::Value,
{
    match execute_governed_read(call, provider, request).await {
        Ok(observed) => ToolResult::ok(project(&observed)),
        Err(error) => os_error(&error),
    }
}
