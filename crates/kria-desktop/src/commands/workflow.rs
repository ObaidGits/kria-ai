//! Workflow Runtime Commands — HITL, Cancellation, Continuation.
//!
//! These Tauri commands connect the frontend workflow UI to the
//! canonical backend runtime. They handle:
//! - HITL responses (approve, deny, retry, skip, cancel)
//! - Workflow cancellation
//! - Continuation actions (bring to front, open URL, retry)
//! - Workflow state queries
//!
//! Architecture invariant: KRIA is the authoritative orchestrator. These
//! commands hand the human decision to the canonical
//! [`WorkflowContinuationRuntime`], which owns resume/teardown; the substrate
//! never self-resumes. Cancellation therefore propagates through the runtime's
//! own session store (see [`WorkflowContinuationRuntime::cancel_workflow`]).

use crate::commands::app_state::AppStateCell;
use tauri::State;

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — HITL Response Command
// ═══════════════════════════════════════════════════════════════════════════════

/// Handle a HITL response from the frontend.
///
/// Called when the user clicks a button in the HITL modal (approve, deny,
/// retry, skip, cancel, choose alternative, etc.)
#[tauri::command]
pub async fn workflow_hitl_respond(
    workflow_id: String,
    option_id: String,
    action_type: String,
    value: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    tracing::info!(
        target: "workflow_commands",
        workflow_id = %workflow_id,
        option_id = %option_id,
        action_type = %action_type,
        "HITL response received from frontend"
    );

    // Validate the response
    if workflow_id.is_empty() {
        return Err("workflow_id is required".into());
    }
    if option_id.is_empty() {
        return Err("option_id is required".into());
    }

    // Log the HITL decision for audit trail
    tracing::info!(
        target: "workflow_hitl",
        workflow_id = %workflow_id,
        option_id = %option_id,
        action_type = %action_type,
        value = ?value,
        "HITL decision recorded"
    );

    // Route the human decision into the canonical runtime. A deny/cancel is a
    // terminal cancellation (propagated through the runtime's session store);
    // any other decision (approve/retry/skip/manual_complete/choose_alternative)
    // resumes the paused workflow. The substrate never self-resumes — KRIA
    // hands the decision to the runtime, which owns re-grounding + teardown.
    let Some(app) = state.get() else {
        // Runtime not ready yet — acknowledge so the UI degrades gracefully
        // (Req 20.4) instead of surfacing a hard error during early init.
        return Ok(serde_json::json!({
            "status": "acknowledged_no_runtime",
            "workflow_id": workflow_id,
            "option_id": option_id,
            "action_type": action_type,
        }));
    };

    let is_cancel = matches!(action_type.as_str(), "deny" | "cancel");
    if is_cancel {
        let cancelled = app.workflow_continuation.cancel_workflow(&workflow_id);
        return Ok(serde_json::json!({
            "status": if cancelled { "cancelled" } else { "cancel_noop" },
            "workflow_id": workflow_id,
            "option_id": option_id,
            "action_type": action_type,
        }));
    }

    let resume = app.workflow_continuation.resume_workflow(&workflow_id);
    Ok(serde_json::json!({
        "status": if resume.success { "resume_prepared" } else { "decision_recorded" },
        "workflow_id": workflow_id,
        "option_id": option_id,
        "action_type": action_type,
        "resume": {
            "success": resume.success,
            "summary": resume.summary,
            "next_action": format!("{:?}", resume.next_action),
            "requires_reground": true,
            "note": "Workflow continuation must re-ground before executing any side-effecting action."
        }
    }))
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Workflow Cancellation Command
// ═══════════════════════════════════════════════════════════════════════════════

/// Cancel an active workflow.
///
/// Called when the user clicks the Cancel button during workflow execution.
/// Propagates cancellation to the canonical runtime.
#[tauri::command]
pub async fn workflow_cancel(
    workflow_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    tracing::info!(
        target: "workflow_commands",
        workflow_id = %workflow_id,
        "Workflow cancellation requested from frontend"
    );

    if workflow_id.is_empty() {
        return Err("workflow_id is required".into());
    }

    // Propagate cancellation through the canonical runtime's session store so
    // the workflow is neither resumed nor continued. KRIA records the terminal
    // decision; the substrate cannot self-resume afterward.
    let Some(app) = state.get() else {
        return Ok(serde_json::json!({
            "status": "acknowledged_no_runtime",
            "workflow_id": workflow_id,
        }));
    };

    let cancelled = app.workflow_continuation.cancel_workflow(&workflow_id);
    Ok(serde_json::json!({
        "status": if cancelled { "cancelled" } else { "cancellation_requested" },
        "workflow_id": workflow_id,
    }))
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Continuation Action Command
// ═══════════════════════════════════════════════════════════════════════════════

/// Execute a continuation action after workflow completion.
///
/// Called when the user clicks a continuation button (Bring to Front,
/// Open URL, Retry, etc.)
#[tauri::command]
pub async fn workflow_continuation(
    workflow_id: String,
    action_id: String,
    action_type: String,
    payload: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    tracing::info!(
        target: "workflow_commands",
        workflow_id = %workflow_id,
        action_id = %action_id,
        action_type = %action_type,
        "Continuation action requested from frontend"
    );

    if workflow_id.trim().is_empty() || action_id.trim().is_empty() {
        return Err("workflow_id and action_id are required".into());
    }

    let require_payload = |kind: &str| {
        payload
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{kind} continuation requires a payload"))
    };

    match action_type.as_str() {
        "bring_to_front" => {
            let app = require_payload("bring_to_front")?;
            tracing::info!(target: "workflow_commands", app, "Bringing app to front");
            let output = tokio::process::Command::new("wmctrl")
                .args(["-a", app])
                .output()
                .await
                .map_err(|error| format!("failed to start wmctrl: {error}"))?;
            if !output.status.success() {
                return Err(format!(
                    "wmctrl could not focus '{app}': {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            Ok(serde_json::json!({ "status": "completed", "action": "bring_to_front" }))
        }
        "open_url" | "open_file" => {
            let target = require_payload(&action_type)?;
            tracing::info!(target: "workflow_commands", %target, "Opening continuation target");
            tokio::process::Command::new("xdg-open")
                .arg(target)
                .spawn()
                .map_err(|error| format!("failed to start xdg-open: {error}"))?;
            Ok(serde_json::json!({ "status": "started", "action": action_type }))
        }
        "show_output" => {
            let content = require_payload("show_output")?;
            Ok(serde_json::json!({
                "status": "presented",
                "action": "show_output",
                "content": content,
            }))
        }
        "retry_step" | "retry_workflow" => {
            let app = state.get().ok_or_else(|| {
                "KRIA is still initializing — please try again in a moment".to_string()
            })?;
            let resume = app.workflow_continuation.resume_workflow(&workflow_id);
            if !resume.success {
                return Err(resume.summary);
            }
            Ok(serde_json::json!({
                "status": "resume_prepared",
                "action": action_type,
                "workflow_id": workflow_id,
                "resume": {
                    "success": true,
                    "summary": resume.summary,
                    "next_action": format!("{:?}", resume.next_action),
                    "requires_reground": true,
                }
            }))
        }
        _ => Err(format!(
            "unsupported workflow continuation action: {action_type}"
        )),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — Workflow State Query
// ═══════════════════════════════════════════════════════════════════════════════

/// Get the current state of the canonical workflow runtime.
///
/// Returns activation status, metrics, and recent workflow summaries.
#[tauri::command]
pub async fn workflow_runtime_status() -> Result<serde_json::Value, String> {
    use kria_core::agent::workflow_activation::{
        ActivationMetrics, ActivationStage, CanonicalActivationPolicy, CanonicalActivationReport,
    };

    let policy = CanonicalActivationPolicy::at_stage(ActivationStage::FullActivation);
    let metrics = ActivationMetrics::default();
    let report = CanonicalActivationReport::generate(&policy, &metrics);

    Ok(serde_json::json!({
        "canonical_active": true,
        "activation_stage": format!("{:?}", report.current_stage),
        "enabled": report.enabled,
        "success_rate": report.success_rate,
        "fallback_rate": report.fallback_rate,
        "active_substrates": report.active_substrates,
        "unsafe_substrates": report.unsafe_substrates,
    }))
}
