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

    let sender = state
        .executive_sender
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    serde_json::to_value(sender.snapshot())
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
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

    let mut events = state
        .executive_sender
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .subscribe_events();
    let event_stream = async_stream::stream! {
        loop {
            match events.recv().await {
                Ok(event) => match serde_json::to_string(&event) {
                    Ok(payload) => yield Ok(Event::default().event("executive").data(payload)),
                    Err(error) => {
                        tracing::warn!(%error, "Failed to serialize Executive event");
                    }
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    yield Ok(Event::default().event("lagged").data(
                        serde_json::json!({ "missed": count }).to_string(),
                    ));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
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

    let parsed_id = uuid::Uuid::parse_str(&task_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let sender = state
        .executive_sender
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    sender.cancel_task(parsed_id).map_err(|error| {
        tracing::warn!(%task_id, %error, "ExecutiveController cancel failed");
        StatusCode::SERVICE_UNAVAILABLE
    })?;
    tracing::info!(%task_id, "Executive task cancellation requested");
    Ok(Json(serde_json::json!({
        "status": "cancel_requested",
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

    let registry = state.quarantine_registry.clone();
    let tools = tokio::task::spawn_blocking(move || registry.all())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let pending_approval = tools.iter().filter(|tool| {
        tool.status == kria_core::tools::quarantine::QuarantineStatus::PendingApproval
    }).count();
    Ok(Json(serde_json::json!({
        "total": tools.len(),
        "pending_approval": pending_approval,
        "tools": tools,
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

    let registry = state.quarantine_registry.clone();
    let approved_id = tool_id.clone();
    tokio::task::spawn_blocking(move || {
        registry.approve(&approved_id, Some("Approved from server quarantine API"))
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::CONFLICT)?;
    tracing::info!(%tool_id, "Quarantined tool approved");
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

    let registry = state.quarantine_registry.clone();
    let rejected_id = tool_id.clone();
    tokio::task::spawn_blocking(move || {
        registry.reject(&rejected_id, Some("Rejected from server quarantine API"))
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::CONFLICT)?;
    tracing::info!(%tool_id, "Quarantined tool rejected");
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
