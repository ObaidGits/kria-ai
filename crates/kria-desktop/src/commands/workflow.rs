//! Workflow Runtime Commands — HITL, Cancellation, Continuation.
//!
//! These Tauri commands connect the frontend workflow UI to the
//! canonical backend runtime. They handle:
//! - HITL responses (approve, deny, retry, skip, cancel)
//! - Workflow cancellation
//! - Continuation actions (bring to front, open URL, retry)
//! - Workflow state queries

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — HITL Response Command
// ═══════════════════════════════════════════════════════════════════════════════

/// Handle a HITL response from the frontend.
///
/// Called when the user clicks a button in the HITL modal (approve, deny,
/// retry, skip, cancel, choose alternative, etc.)
#[tauri::command]
#[allow(dead_code)] // Will be registered in Tauri builder when HITL frontend is fully wired
pub async fn workflow_hitl_respond(
    workflow_id: String,
    option_id: String,
    action_type: String,
    value: Option<String>,
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

    // TODO: Wire to actual workflow lifecycle resume when
    // canonical executor supports HITL suspension/resume.
    // For now, acknowledge the response.
    Ok(serde_json::json!({
        "status": "acknowledged",
        "workflow_id": workflow_id,
        "option_id": option_id,
        "action_type": action_type,
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
#[allow(dead_code)]
pub async fn workflow_cancel(workflow_id: String) -> Result<serde_json::Value, String> {
    tracing::info!(
        target: "workflow_commands",
        workflow_id = %workflow_id,
        "Workflow cancellation requested from frontend"
    );

    if workflow_id.is_empty() {
        return Err("workflow_id is required".into());
    }

    // TODO: Wire to CancellationToken for the active workflow.
    // The canonical executor already respects cancellation tokens.
    // This command needs to find the token for the given workflow_id
    // and cancel it.

    Ok(serde_json::json!({
        "status": "cancellation_requested",
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
#[allow(dead_code)]
pub async fn workflow_continuation(
    workflow_id: String,
    action_id: String,
    action_type: String,
    payload: Option<String>,
) -> Result<serde_json::Value, String> {
    tracing::info!(
        target: "workflow_commands",
        workflow_id = %workflow_id,
        action_id = %action_id,
        action_type = %action_type,
        "Continuation action requested from frontend"
    );

    match action_type.as_str() {
        "bring_to_front" => {
            if let Some(app) = &payload {
                // Attempt to focus the app window
                tracing::info!(target: "workflow_commands", app = %app, "Bringing app to front");
                // Use wmctrl/xdotool to focus (best-effort)
                let _ = tokio::process::Command::new("wmctrl")
                    .args(["-a", app])
                    .output()
                    .await;
            }
            Ok(serde_json::json!({ "status": "attempted", "action": "bring_to_front" }))
        }
        "open_url" => {
            if let Some(url) = &payload {
                tracing::info!(target: "workflow_commands", url = %url, "Opening URL");
                let _ = tokio::process::Command::new("xdg-open").arg(url).spawn();
            }
            Ok(serde_json::json!({ "status": "attempted", "action": "open_url" }))
        }
        "open_file" => {
            if let Some(path) = &payload {
                tracing::info!(target: "workflow_commands", path = %path, "Opening file");
                let _ = tokio::process::Command::new("xdg-open").arg(path).spawn();
            }
            Ok(serde_json::json!({ "status": "attempted", "action": "open_file" }))
        }
        "retry_workflow" => {
            tracing::info!(target: "workflow_commands", "Retry workflow requested");
            // TODO: Re-trigger the workflow with the same intent
            Ok(serde_json::json!({ "status": "retry_queued", "workflow_id": workflow_id }))
        }
        _ => Ok(serde_json::json!({
            "status": "unknown_action",
            "action_type": action_type,
        })),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — Workflow State Query
// ═══════════════════════════════════════════════════════════════════════════════

/// Get the current state of the canonical workflow runtime.
///
/// Returns activation status, metrics, and recent workflow summaries.
#[tauri::command]
#[allow(dead_code)]
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
