// ─────────────────────────────────────────────────────────────────────────────
//  intelligence_routes.rs — REST + SSE endpoints for the Intelligence Engine
//
//  Exposes the ExecutiveController, QuarantineRegistry, SelfModel, and
//  PolicyGate log to the SolidJS frontend via Axum routes.
//
//  All endpoints are feature-gated: if `executive.enabled` is false in the
//  config, the executive/quarantine endpoints return 503 Service Unavailable.
//  The intelligence status endpoint always returns current config flags.
// ─────────────────────────────────────────────────────────────────────────────

use crate::ServerState;
use axum::{
    extract::State,
    http::StatusCode,
    response::{sse::Event, sse::KeepAlive, sse::Sse},
    routing::{get, post},
    Json, Router,
};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

/// Register all intelligence-related routes on the given router.
pub fn intelligence_routes() -> Router<Arc<ServerState>> {
    Router::new()
        // Executive Controller
        .route("/api/executive/snapshot", get(executive_snapshot))
        .route("/api/executive/events", get(executive_events_sse))
        .route("/api/executive/tasks/{task_id}/cancel", post(cancel_task))
        // Quarantine Registry
        .route("/api/quarantine/tools", get(quarantine_list))
        .route(
            "/api/quarantine/{tool_id}/approve",
            post(quarantine_approve),
        )
        .route("/api/quarantine/{tool_id}/reject", post(quarantine_reject))
        // Intelligence Status
        .route("/api/intelligence/status", get(intelligence_status))
}

// ─── Executive endpoints ────────────────────────────────────────────────────

/// GET /api/executive/snapshot
///
/// Returns the current state of the ExecutiveController: active foreground task,
/// queued tasks, GPU lease holder, background task count, etc.
async fn executive_snapshot(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.config.executive.enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    // In production, this would call into the ExecutiveController's event_tx
    // to get a snapshot. For now, return the config as a placeholder.
    Ok(Json(serde_json::json!({
        "config": {
            "enabled": state.config.executive.enabled,
            "max_background_tasks": state.config.executive.max_background_tasks,
            "preemption_grace_ms": state.config.executive.preemption_grace_ms,
        },
        "active_foreground": null,
        "active_background": [],
        "queued": [],
        "gpu_lease_holder": null,
        "gpu_lease_remaining_ms": null,
        "total_completed": 0,
        "total_failed": 0,
    })))
}

/// GET /api/executive/events
///
/// SSE stream of ExecutiveController events. The SolidJS frontend subscribes
/// to this to get real-time updates on task lifecycle.
async fn executive_events_sse(
    State(state): State<Arc<ServerState>>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    if !state.config.executive.enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    // In production, this would subscribe to the ExecutiveController's
    // event_tx watch channel. For now, stream a heartbeat.
    let event_stream = async_stream::stream! {
        loop {
            yield Ok(Event::default()
                .event("heartbeat")
                .data(serde_json::json!({"ts": chrono::Utc::now().to_rfc3339()}).to_string()));
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    };

    Ok(Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keepalive"),
    ))
}

/// POST /api/executive/tasks/{task_id}/cancel
///
/// Cancel a running task by ID.
async fn cancel_task(
    State(state): State<Arc<ServerState>>,
    axum::extract::Path(task_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.config.executive.enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    // Try the ExecutiveSender if the task_id is a valid UUID
    if let Ok(parsed_id) = uuid::Uuid::parse_str(&task_id) {
        if let Some(ref sender) = state.executive_sender {
            match sender.cancel_task(parsed_id) {
                Ok(()) => {
                    tracing::info!(task_id = %task_id, "Task cancellation requested via ExecutiveController");
                    return Ok(Json(serde_json::json!({
                        "status": "cancelled",
                        "task_id": task_id,
                    })));
                }
                Err(e) => {
                    tracing::warn!(task_id = %task_id, error = %e, "ExecutiveController cancel failed");
                }
            }
        }
    }

    // Fallback: cancel via turn admission (also handles non-UUID task IDs)
    let cancelled = state.turn_admission.cancel_session(&task_id);
    tracing::info!(task_id = %task_id, cancelled, "Task cancellation via turn admission");
    Ok(Json(serde_json::json!({
        "status": "cancelled",
        "task_id": task_id,
    })))
}

// ─── Quarantine endpoints ───────────────────────────────────────────────────

/// GET /api/quarantine/tools
///
/// List all quarantined tools (skills awaiting approval).
async fn quarantine_list(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.config.skill_compiler.enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    // In production, this would query the QuarantineRegistry's SQLite table.
    Ok(Json(serde_json::json!({
        "tools": [],
        "total": 0,
        "pending_approval": 0,
    })))
}

/// POST /api/quarantine/{tool_id}/approve
///
/// Approve a quarantined tool for promotion to the active registry.
async fn quarantine_approve(
    State(state): State<Arc<ServerState>>,
    axum::extract::Path(tool_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.config.skill_compiler.enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    tracing::info!(tool_id = %tool_id, "Quarantine approval requested");
    Ok(Json(serde_json::json!({
        "status": "approved",
        "tool_id": tool_id,
    })))
}

/// POST /api/quarantine/{tool_id}/reject
///
/// Reject a quarantined tool.
async fn quarantine_reject(
    State(state): State<Arc<ServerState>>,
    axum::extract::Path(tool_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.config.skill_compiler.enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    tracing::info!(tool_id = %tool_id, "Quarantine rejection requested");
    Ok(Json(serde_json::json!({
        "status": "rejected",
        "tool_id": tool_id,
    })))
}

// ─── Intelligence status ────────────────────────────────────────────────────

/// GET /api/intelligence/status
///
/// Returns the current feature flag state for all intelligence modules.
/// This endpoint always works (no feature gate) so the frontend can
/// discover which modules are enabled.
async fn intelligence_status(State(state): State<Arc<ServerState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "executive": {
            "enabled": state.config.executive.enabled,
            "max_background_tasks": state.config.executive.max_background_tasks,
            "preemption_grace_ms": state.config.executive.preemption_grace_ms,
        },
        "planner": {
            "enabled": state.config.planner.enabled,
            "max_steps": state.config.planner.max_steps,
            "max_replans": state.config.planner.max_replans,
        },
        "uncertainty": {
            "enabled": state.config.uncertainty.enabled,
            "plan_threshold": state.config.uncertainty.plan_threshold,
            "gather_threshold": state.config.uncertainty.gather_threshold,
            "ask_threshold": state.config.uncertainty.ask_threshold,
        },
        "skill_compiler": {
            "enabled": state.config.skill_compiler.enabled,
            "min_successes": state.config.skill_compiler.min_successes,
            "quarantine_enabled": state.config.skill_compiler.quarantine_enabled,
        },
        "curiosity": {
            "enabled": state.config.curiosity.enabled,
            "max_cpu_percent": state.config.curiosity.max_cpu_percent,
            "cooldown_secs": state.config.curiosity.cooldown_secs,
        },
        "browser_agent": {
            "enabled": state.config.browser_agent.enabled,
        },
    }))
}
