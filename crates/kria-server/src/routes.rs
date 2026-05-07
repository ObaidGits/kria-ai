use crate::ServerState;
use axum::{
    extract::{ws::Message, ws::WebSocket, ws::WebSocketUpgrade, Path, Query, State},
    http::StatusCode,
    response::{sse::Event, sse::KeepAlive, sse::Sse, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use kria_connection_control::manager::{ControlPlaneEvent, DockerEvalRequest, DockerHealthStatus};
use kria_core::infra::pipeline_trace::{log_pipeline_step, sanitize_text_for_logs};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use async_stream::stream;

pub fn api_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/chat", post(chat))
        .route("/api/sessions", get(list_sessions))
        .route("/api/models", get(list_models))
        .route("/api/settings", get(get_settings))
        .route("/api/settings", post(update_settings))
        .route("/api/fleet/events", get(fleet_events))
        .route("/api/fleet/terminal", get(fleet_terminal_ws))
        .route("/api/fleet/leases/{lease_id}/heartbeat", post(fleet_lease_heartbeat))
        .route("/api/fleet/docker-evals", post(fleet_docker_evals))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(serde::Deserialize)]
struct ChatRequest {
    message: String,
    session_id: Option<String>,
    /// Source of the message (e.g. "telegram", "web")
    #[serde(default)]
    source: Option<String>,
    /// Telegram chat ID (when source = "telegram")
    #[serde(default)]
    chat_id: Option<i64>,
    /// Sender name
    #[serde(default)]
    from_user: Option<String>,
}

async fn chat(
    State(_state): State<Arc<ServerState>>,
    Json(req): Json<ChatRequest>,
) -> Json<serde_json::Value> {
    let session_id = req
        .session_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    log_pipeline_step(
        &session_id,
        "server_chat_received",
        "Server /api/chat request received",
        Some(serde_json::json!({
            "source": req.source.clone().unwrap_or_else(|| "api".to_string()),
            "chat_id": req.chat_id,
            "from_user": req.from_user.clone(),
            "message_preview": sanitize_text_for_logs(&req.message, 260),
        })),
    );

    // TODO: In production, this routes to the AgentLoop and returns the response.
    // For now, return a structured response that the Telegram MCP server can parse.
    let message = req.message.clone();
    let response = serde_json::json!({
        "status": "received",
        "message": message.clone(),
        "source": req.source.unwrap_or_else(|| "api".to_string()),
        "chat_id": req.chat_id,
        "from_user": req.from_user,
        "session_id": session_id,
        "reply": format!("I received your message: \"{}\"", message),
    });

    log_pipeline_step(
        response
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("server"),
        "server_chat_done",
        "Server /api/chat stub response returned",
        Some(serde_json::json!({
            "reply_preview": sanitize_text_for_logs(
                response.get("reply").and_then(|v| v.as_str()).unwrap_or(""),
                220,
            ),
        })),
    );

    Json(response)
}

async fn list_sessions() -> Json<Vec<serde_json::Value>> {
    Json(vec![])
}

async fn list_models(State(state): State<Arc<ServerState>>) -> Json<serde_json::Value> {
    let paths = match state.config.resolve_paths() {
        Ok(p) => p,
        Err(_) => return Json(serde_json::json!({"models": []})),
    };
    let mgr = kria_core::llm::model_manager::ModelManager::new(paths.models_dir.join("llm"));
    let models = mgr.list_llm_models();
    Json(serde_json::json!({ "models": models }))
}

async fn get_settings(State(state): State<Arc<ServerState>>) -> Json<serde_json::Value> {
    Json(serde_json::to_value(&state.config).unwrap_or_default())
}

async fn update_settings(
    State(_state): State<Arc<ServerState>>,
    Json(_settings): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    // In production: validate and persist to config file
    Json(serde_json::json!({ "status": "updated" }))
}

#[derive(Debug, serde::Deserialize)]
struct FleetEventsQuery {
    #[serde(default)]
    lease_id: Option<String>,
}

async fn fleet_events(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<FleetEventsQuery>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.fleet.manager.subscribe_events();
    let snapshot_payload = state.fleet.snapshot_event_payload().await.to_string();
    let lease_filter = query
        .lease_id
        .as_deref()
        .and_then(|raw| Uuid::parse_str(raw).ok());

    let event_stream = stream! {
        yield Ok(Event::default().data(snapshot_payload));

        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Some(filter) = lease_filter {
                        if !event_matches_lease(&event, filter) {
                            continue;
                        }
                    }

                    let payload = crate::inventory::control_plane_event_json(&event).to_string();
                    yield Ok(Event::default().data(payload));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "fleet SSE consumer lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keepalive"),
    )
}

#[derive(Debug, serde::Deserialize)]
struct FleetTerminalQuery {
    target_id: String,
    #[serde(default)]
    lease_id: Option<String>,
}

async fn fleet_terminal_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<ServerState>>,
    Query(query): Query<FleetTerminalQuery>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_fleet_terminal_socket(socket, state, query))
}

async fn handle_fleet_terminal_socket(
    socket: WebSocket,
    state: Arc<ServerState>,
    query: FleetTerminalQuery,
) {
    let target_id = match Uuid::parse_str(query.target_id.trim()) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(error = %error, target_id = %query.target_id, "invalid target_id for terminal ws");
            return;
        }
    };

    let lease_id = query
        .lease_id
        .as_deref()
        .and_then(|raw| Uuid::parse_str(raw).ok());

    let session_id = Uuid::new_v4().to_string();
    if let Err(error) = state
        .fleet
        .manager
        .register_terminal_session(target_id, session_id.clone(), None)
        .await
    {
        tracing::warn!(error = %error, target_id = %target_id, "failed to register terminal session");
        return;
    }

    let mut rx = state.fleet.manager.subscribe_events();
    let (mut sender, mut receiver) = socket.split();
    let connected = serde_json::json!({
        "type": "connected",
        "target_id": target_id,
        "lease_id": lease_id,
        "session_id": session_id,
    });
    if sender
        .send(Message::Text(connected.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&text) {
                            let kind = payload.get("kind").and_then(|v| v.as_str()).unwrap_or_default();
                            if kind.eq_ignore_ascii_case("ping") {
                                let pong = serde_json::json!({"type": "pong"}).to_string();
                                if sender.send(Message::Text(pong.into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        tracing::warn!(error = %error, target_id = %target_id, "terminal websocket receive error");
                        let _ = state
                            .fleet
                            .manager
                            .report_terminal_ws_failure(
                                target_id,
                                session_id.clone(),
                                None,
                                error.to_string(),
                                true,
                            )
                            .await;
                        break;
                    }
                    None => break,
                }
            }
            event = rx.recv() => {
                match event {
                    Ok(event) => {
                        if !event_matches_target(&event, target_id) {
                            continue;
                        }

                        let payload = crate::inventory::control_plane_event_json(&event).to_string();
                        if sender.send(Message::Text(payload.into())).await.is_err() {
                            let _ = state
                                .fleet
                                .manager
                                .report_terminal_ws_failure(
                                    target_id,
                                    session_id.clone(),
                                    None,
                                    "terminal websocket closed while sending event",
                                    true,
                                )
                                .await;
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, target_id = %target_id, "terminal websocket event stream lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct FleetHeartbeatRequest {
    #[serde(default)]
    lease_id: Option<String>,
    #[serde(default)]
    sent_at_unix_ms: Option<i64>,
}

async fn fleet_lease_heartbeat(
    Path(lease_id): Path<Uuid>,
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<FleetHeartbeatRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if let Some(body_lease_id) = payload.lease_id.as_deref() {
        if let Ok(parsed) = Uuid::parse_str(body_lease_id) {
            if parsed != lease_id {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "status": "error",
                        "message": "lease id mismatch between path and body"
                    })),
                ));
            }
        }
    }

    match state.fleet.manager.heartbeat(lease_id).await {
        Ok(()) => Ok(Json(serde_json::json!({
            "type": "heartbeat_ack",
            "lease_id": lease_id,
            "received_sent_at_unix_ms": payload.sent_at_unix_ms,
            "ts_unix_ms": chrono::Utc::now().timestamp_millis(),
        }))),
        Err(error) => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "status": "error",
                "message": error.to_string(),
            })),
        )),
    }
}

#[derive(Debug, serde::Deserialize)]
struct FleetDockerEvalRequest {
    lease_id: String,
    target_id: String,
    #[serde(default)]
    suite_name: Option<String>,
}

async fn fleet_docker_evals(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<FleetDockerEvalRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let lease_id = Uuid::parse_str(request.lease_id.trim()).map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("invalid lease_id: {error}"),
            })),
        )
    })?;

    let target_id = Uuid::parse_str(request.target_id.trim()).map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("invalid target_id: {error}"),
            })),
        )
    })?;

    let suite_name = request
        .suite_name
        .unwrap_or_else(|| "kria_core_docker_suite".to_string());

    let summary = state
        .fleet
        .manager
        .run_docker_eval(DockerEvalRequest {
            lease_id,
            target_id,
            suite_name,
        })
        .await
        .map_err(|error| {
            (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "status": "error",
                    "message": error.to_string(),
                })),
            )
        })?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "summary": {
            "run_id": summary.run_id,
            "target_id": summary.target_id,
            "lease_id": summary.lease_id,
            "suite_name": summary.suite_name,
            "status": docker_health_label(summary.status),
            "passed_count": summary.passed_count,
            "failed_count": summary.failed_count,
            "started_at_unix_ms": summary.started_at_unix_ms,
            "finished_at_unix_ms": summary.finished_at_unix_ms,
            "cases": summary.cases,
        }
    })))
}

fn event_matches_lease(event: &ControlPlaneEvent, lease_id: Uuid) -> bool {
    match event {
        ControlPlaneEvent::FleetAlert { lease_id: Some(id), .. } => *id == lease_id,
        ControlPlaneEvent::TerminalLine { lease_id: Some(id), .. } => *id == lease_id,
        ControlPlaneEvent::TargetStatus { .. }
        | ControlPlaneEvent::DockerEvalUpdate { .. }
        | ControlPlaneEvent::TerminalGap { .. }
        | ControlPlaneEvent::ClockDrift { .. }
        | ControlPlaneEvent::FleetAlert { lease_id: None, .. }
        | ControlPlaneEvent::TerminalLine { lease_id: None, .. } => true,
    }
}

fn event_matches_target(event: &ControlPlaneEvent, target_id: Uuid) -> bool {
    match event {
        ControlPlaneEvent::TargetStatus { target_id: id, .. } => *id == target_id,
        ControlPlaneEvent::FleetAlert {
            target_id: Some(id), ..
        } => *id == target_id,
        ControlPlaneEvent::DockerEvalUpdate { target_id: id, .. } => *id == target_id,
        ControlPlaneEvent::TerminalGap { marker } => marker.target_id == target_id,
        ControlPlaneEvent::TerminalLine { target_id: id, .. } => *id == target_id,
        ControlPlaneEvent::ClockDrift { alert } => alert.target_id == target_id,
        ControlPlaneEvent::FleetAlert {
            target_id: None, ..
        } => false,
    }
}

fn docker_health_label(status: DockerHealthStatus) -> &'static str {
    match status {
        DockerHealthStatus::Unknown => "unknown",
        DockerHealthStatus::Running => "running",
        DockerHealthStatus::Pass => "pass",
        DockerHealthStatus::Fail => "fail",
    }
}
