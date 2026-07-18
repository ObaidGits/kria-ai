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
use kria_core::agent::loop_engine::StreamEvent;
use kria_core::infra::pipeline_trace::{log_pipeline_step, sanitize_text_for_logs};
use kria_core::llm::ChatMessage;
use kria_core::memory::conversation::{ConversationStore, ConversationTurn};
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
        .route(
            "/api/fleet/leases/{lease_id}/heartbeat",
            post(fleet_lease_heartbeat),
        )
        .route("/api/fleet/docker-evals", post(fleet_docker_evals))
        .route("/api/sessions/{session_id}/cancel", post(cancel_session))
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
    State(state): State<Arc<ServerState>>,
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

    // When enabled, ExecutiveController schedules the same real AgentLoop work used by
    // the direct REST path. The controller owns priority, cancellation, and observability;
    // it never substitutes a synthetic response.
    if let Some(ref executive) = state.executive_sender {
        use kria_core::agent::executive::types::*;

        let work_agent = state.agent_loop.clone();
        let cancel_agent = work_agent.clone();
        let work_session_id = session_id.clone();
        let cancel_session_id = session_id.clone();
        let work_message = req.message.clone();
        let description = work_message.clone();
        let session_store = state.session_store.clone();
        let payload = TaskPayload::new(description, async move {
            let started = std::time::Instant::now();
            let Some(work_agent) = work_agent else {
                return TaskResult::Failed {
                    reason: "agent runtime not initialized".to_string(),
                    total_duration: started.elapsed(),
                };
            };
            let (reply, error) = run_agent_turn(
                work_agent,
                &work_session_id,
                &work_message,
                session_store,
            ).await;
            match error {
                Some(reason) => TaskResult::Failed {
                    reason,
                    total_duration: started.elapsed(),
                },
                None => TaskResult::Success {
                    total_duration: started.elapsed(),
                    output: Some(reply.chars().take(512).collect()),
                },
            }
        }).with_cancel_handler(move || {
            if let Some(agent) = cancel_agent {
                agent.cancel_session(&cancel_session_id);
            }
        });
        let task = TaskRequest::new(
            TaskPriority::Interactive,
            TaskSource::TextChat,
            true,
            payload,
        );

        return match executive.submit(task) {
            Ok(()) => Json(serde_json::json!({
                "status": "submitted",
                "session_id": session_id,
                "source": req.source.unwrap_or_else(|| "api".to_string()),
                "message": "Task queued for processing by ExecutiveController",
            })),
            Err(_) => Json(serde_json::json!({
                "status": "error",
                "session_id": session_id,
                "message": "ExecutiveController unavailable",
            })),
        };
    }

    // ─── Real memory-driven agent path (executive disabled) ──────────
    // Runs the SAME `AgentLoop` the `/ws` path streams — memory-driven when the
    // headless runtime brought up the MemorySystem (grounding + observe +
    // learning). Non-streaming: we drain the event channel to a final reply so
    // Telegram/web REST callers get a complete answer. History is loaded from +
    // persisted to the shared conversation store, exactly like `/ws`.
    let Some(agent) = state.agent_loop.clone() else {
        return Json(serde_json::json!({
            "status": "unavailable",
            "session_id": session_id,
            "source": req.source.unwrap_or_else(|| "api".to_string()),
            "message": "agent runtime not initialized",
        }));
    };

    let (reply, error) = run_agent_turn(
        agent,
        &session_id,
        &req.message,
        state.session_store.clone(),
    )
    .await;

    log_pipeline_step(
        &session_id,
        "server_chat_done",
        "Server /api/chat agent response returned",
        Some(serde_json::json!({
            "reply_preview": sanitize_text_for_logs(&reply, 220),
            "error": error.as_deref().map(|e| sanitize_text_for_logs(e, 220)),
        })),
    );

    // Surface an honest status: a loop error, or an empty reply with no content,
    // is reported as "error" rather than a silent empty "ok".
    let status = if error.is_some() {
        "error"
    } else if reply.trim().is_empty() {
        "empty"
    } else {
        "ok"
    };

    Json(serde_json::json!({
        "status": status,
        "message": req.message,
        "source": req.source.unwrap_or_else(|| "api".to_string()),
        "chat_id": req.chat_id,
        "from_user": req.from_user,
        "session_id": session_id,
        "reply": reply,
        "error": error,
    }))
}

/// Run one non-streaming agent turn and return the final assistant text.
/// Loads recent history for continuity and persists the completed turn — the
/// REST twin of the `/ws` streaming path (shares the same AgentLoop + store).
async fn run_agent_turn(
    agent: Arc<kria_core::agent::AgentLoop>,
    session_id: &str,
    user_msg: &str,
    session_store: Option<Arc<ConversationStore>>,
) -> (String, Option<String>) {
    const HISTORY_LIMIT: usize = 20;

    let mut messages: Vec<ChatMessage> = Vec::new();
    if let Some(store) = session_store.as_ref() {
        if let Ok(turns) = store.get_recent_turns(session_id, HISTORY_LIMIT) {
            for turn in turns {
                messages.push(ChatMessage {
                    role: turn.role,
                    content: turn.content,
                    name: turn.tool_name,
                    images: None,
                });
            }
        }
    }
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: user_msg.to_string(),
        name: None,
        images: None,
    });

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();
    let agent_run = agent.clone();
    let sid = session_id.to_string();
    tokio::spawn(async move {
        agent_run.run(&sid, &mut messages, event_tx).await;
    });

    let mut reply = String::new();
    let mut error: Option<String> = None;
    while let Some(event) = event_rx.recv().await {
        match &event {
            StreamEvent::Token(t) => reply.push_str(t),
            StreamEvent::Done(final_text) if !final_text.is_empty() => {
                if reply.is_empty() {
                    reply = final_text.clone();
                }
            }
            StreamEvent::Error(e) => error = Some(e.clone()),
            _ => {}
        }
    }

    if let Some(store) = session_store.as_ref() {
        persist_turn(store, session_id, "user", user_msg);
        if !reply.is_empty() {
            persist_turn(store, session_id, "assistant", &reply);
        }
    }

    (reply, error)
}

/// Persist a single conversation turn (best-effort; logs on failure).
fn persist_turn(store: &Arc<ConversationStore>, session_id: &str, role: &str, content: &str) {
    let turn = ConversationTurn {
        id: None,
        session_id: session_id.to_string(),
        role: role.to_string(),
        content: content.to_string(),
        tool_name: None,
        tool_result: None,
        tokens_used: None,
        timestamp: chrono::Utc::now(),
    };
    if let Err(e) = store.store_turn(&turn) {
        tracing::warn!(error = %e, "failed to persist REST conversation turn");
    }
}

async fn list_sessions(State(state): State<Arc<ServerState>>) -> Json<Vec<serde_json::Value>> {
    let Some(store) = state.session_store.as_ref() else {
        return Json(vec![]);
    };
    match store.list_sessions() {
        Ok(sessions) => Json(
            sessions
                .into_iter()
                .map(|(session_id, turns, last_active)| {
                    serde_json::json!({
                        "session_id": session_id,
                        "turns": turns,
                        "last_active": last_active,
                    })
                })
                .collect(),
        ),
        Err(e) => {
            tracing::warn!(error = %e, "failed to list sessions");
            Json(vec![])
        }
    }
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

/// POST /api/sessions/{session_id}/cancel
///
/// Cancel the active turn for a session. Safe to call even when no turn is
/// active (returns 200 with `"cancelled": false`).
async fn cancel_session(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
) -> Json<serde_json::Value> {
    let cancelled = state.turn_admission.cancel_session(&session_id);

    if cancelled {
        log_pipeline_step(
            &session_id,
            "server_session_cancelled",
            "Session cancelled via HTTP endpoint",
            None,
        );
    }

    Json(serde_json::json!({
        "status": "ok",
        "session_id": session_id,
        "cancelled": cancelled,
    }))
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
        ControlPlaneEvent::FleetAlert {
            lease_id: Some(id), ..
        } => *id == lease_id,
        ControlPlaneEvent::TerminalLine {
            lease_id: Some(id), ..
        } => *id == lease_id,
        ControlPlaneEvent::TargetStatus { .. }
        | ControlPlaneEvent::DockerEvalUpdate { .. }
        | ControlPlaneEvent::TerminalGap { .. }
        | ControlPlaneEvent::ClockDrift { .. }
        | ControlPlaneEvent::FleetAlert { lease_id: None, .. }
        | ControlPlaneEvent::TerminalLine { lease_id: None, .. }
        | ControlPlaneEvent::TargetRemoved { .. } => true,
    }
}

fn event_matches_target(event: &ControlPlaneEvent, target_id: Uuid) -> bool {
    match event {
        ControlPlaneEvent::TargetStatus { target_id: id, .. } => *id == target_id,
        ControlPlaneEvent::FleetAlert {
            target_id: Some(id),
            ..
        } => *id == target_id,
        ControlPlaneEvent::DockerEvalUpdate { target_id: id, .. } => *id == target_id,
        ControlPlaneEvent::TerminalGap { marker } => marker.target_id == target_id,
        ControlPlaneEvent::TerminalLine { target_id: id, .. } => *id == target_id,
        ControlPlaneEvent::ClockDrift { alert } => alert.target_id == target_id,
        ControlPlaneEvent::FleetAlert {
            target_id: None, ..
        } => false,
        ControlPlaneEvent::TargetRemoved { target_id: id } => *id == target_id,
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
