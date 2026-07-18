//! WebSocket endpoint — streams the real KRIA agent loop to clients.
//!
//! Protocol (JSON text frames):
//!   client → server:
//!     `{"type":"chat","session_id":"…","message":"…"}`
//!     `{"type":"approve","request_id":"…"}` / `{"type":"deny","request_id":"…"}`
//!     `{"type":"cancel","session_id":"…"}`
//!     `{"type":"ping"}`
//!   server → client:
//!     `{"type":"connected","version":"…"}`
//!     `{"type":"token","text":"…"}`, `tool_start`, `tool_end`, `task_step`,
//!     `tool_progress`, `approval_required`, `plan`, `error`, `done`, …
//!
//! HITL: the agent emits `approval_required`; the client answers with
//! `approve`/`deny` carrying the `request_id`, resolved via the HITL gateway.
//! The receiver loop keeps reading control frames while a turn streams, so
//! approvals work mid-turn.

use crate::gateway::PhoneGatewayState;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use kria_core::agent::loop_engine::StreamEvent;
use kria_core::agent::AgentLoop;
use kria_core::infra::pipeline_trace::{log_pipeline_step, sanitize_text_for_logs};
use kria_core::llm::ChatMessage;
use kria_core::memory::conversation::{ConversationStore, ConversationTurn};
use kria_core::notify::{NtfyClient, NtfyMessage, NtfyPriority};
use kria_core::safety::hitl::ApprovalResponse;
use std::sync::Arc;
use tokio::sync::Mutex;

type WsSink = Arc<Mutex<SplitSink<WebSocket, Message>>>;

/// Max prior turns loaded as history for session continuity (Phase 4.5.6).
const HISTORY_LIMIT: usize = 10;

pub fn ws_routes() -> Router<Arc<PhoneGatewayState>> {
    Router::new().route("/ws", get(ws_handler))
}

#[derive(Debug, serde::Deserialize)]
pub struct WsQuery {
    /// Device token (Phase 4.5.4). Required when `mobile.require_device_auth`.
    #[serde(default)]
    pub token: Option<String>,
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<PhoneGatewayState>>,
    Query(query): Query<WsQuery>,
) -> impl IntoResponse {
    // ── Device-token gate (Phase 4.5.4) ──────────────────────────────
    // When mobile auth is required, a valid per-device signed token must be
    // presented before the socket upgrades. This keeps the agent WS off-limits
    // to anything on the mesh that has not been explicitly paired + not revoked.
    let mut device_id: Option<String> = None;
    if state.config.mobile.require_device_auth {
        if let Some(registry) = state.device_registry.as_ref() {
            match query.token.as_deref().map(|t| registry.verify_token(t)) {
                Some(Ok(id)) => {
                    log_pipeline_step(
                        "ws",
                        "mobile_device_authenticated",
                        "Mobile device token accepted on /ws",
                        Some(serde_json::json!({ "device_id": id })),
                    );
                    device_id = Some(id);
                }
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "rejecting /ws connection: invalid device token");
                    return (StatusCode::UNAUTHORIZED, "invalid or revoked device token")
                        .into_response();
                }
                None => {
                    tracing::warn!("rejecting /ws connection: device token required but missing");
                    return (StatusCode::UNAUTHORIZED, "device token required").into_response();
                }
            }
        }
    }
    ws.on_upgrade(move |socket| handle_socket(socket, state, device_id))
        .into_response()
}

/// Send a JSON frame over the shared sink. Returns Err if the socket closed.
async fn send_json(sink: &WsSink, value: serde_json::Value) -> Result<(), ()> {
    let mut guard = sink.lock().await;
    guard
        .send(Message::Text(value.to_string().into()))
        .await
        .map_err(|_| ())
}

async fn handle_socket(
    socket: WebSocket,
    state: Arc<PhoneGatewayState>,
    device_id: Option<String>,
) {
    let (sink, mut receiver) = socket.split();
    let sink: WsSink = Arc::new(Mutex::new(sink));

    let _ = send_json(
        &sink,
        serde_json::json!({ "type": "connected", "version": env!("CARGO_PKG_VERSION") }),
    )
    .await;

    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => {
                let val: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = send_json(
                            &sink,
                            serde_json::json!({ "type": "error", "message": format!("invalid JSON: {e}") }),
                        )
                        .await;
                        continue;
                    }
                };

                let msg_type = val
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
                let session_id = val
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("ws")
                    .to_string();

                match msg_type {
                    "chat" => {
                        let user_msg = val
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("")
                            .to_string();

                        log_pipeline_step(
                            &session_id,
                            "server_ws_chat_received",
                            "WebSocket chat message received",
                            Some(serde_json::json!({
                                "message_preview": sanitize_text_for_logs(&user_msg, 220),
                            })),
                        );

                        match state.agent_loop.clone() {
                            Some(agent) => {
                                // Stream the turn in a spawned task so the
                                // receiver loop stays free to handle approve/
                                // deny/cancel frames while the agent runs.
                                spawn_chat_turn(
                                    agent,
                                    sink.clone(),
                                    session_id,
                                    user_msg,
                                    state.session_store.clone(),
                                    state.notifier.clone(),
                                    device_id.clone(),
                                );
                            }
                            None => {
                                let _ = send_json(
                                    &sink,
                                    serde_json::json!({
                                        "type": "error",
                                        "message": "agent runtime not initialized on this server build",
                                    }),
                                )
                                .await;
                            }
                        }
                    }
                    "approve" | "deny" => {
                        let request_id = val
                            .get("request_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let decision = if msg_type == "approve" {
                            ApprovalResponse::Approved
                        } else {
                            ApprovalResponse::Denied
                        };
                        let resolved = match &state.agent_loop {
                            Some(agent) => {
                                agent.hitl_gateway().respond(&request_id, decision).await
                            }
                            None => false,
                        };
                        let _ = send_json(
                            &sink,
                            serde_json::json!({
                                "type": "hitl_ack",
                                "action": msg_type,
                                "request_id": request_id,
                                "resolved": resolved,
                            }),
                        )
                        .await;
                    }
                    "cancel" => {
                        let cancelled = state.turn_admission.cancel_session(&session_id);
                        let _ = send_json(
                            &sink,
                            serde_json::json!({
                                "type": "cancel_ack",
                                "session_id": session_id,
                                "cancelled": cancelled,
                            }),
                        )
                        .await;
                    }
                    "ping" => {
                        let _ = send_json(&sink, serde_json::json!({ "type": "pong" })).await;
                    }
                    _ => {
                        let _ = send_json(
                            &sink,
                            serde_json::json!({
                                "type": "error",
                                "message": format!("unknown message type: {msg_type}"),
                            }),
                        )
                        .await;
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}

/// Spawn a task that runs one agent turn and streams its events to the client.
///
/// Adds Phase 4.5 behaviour on top of the raw stream:
///   * loads prior turns for the session so phone + desktop resume the same
///     conversation (4.5.6 session continuity);
///   * persists the user + assistant turns after the stream completes;
///   * fires ntfy push notifications on approval-needed / task-done (4.5.5).
#[allow(clippy::too_many_arguments)]
fn spawn_chat_turn(
    agent: Arc<AgentLoop>,
    sink: WsSink,
    session_id: String,
    user_msg: String,
    session_store: Option<Arc<ConversationStore>>,
    notifier: Option<Arc<NtfyClient>>,
    device_id: Option<String>,
) {
    tokio::spawn(async move {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();

        // ── Session continuity: prepend recent history (4.5.6) ──────────
        let mut messages: Vec<ChatMessage> = Vec::new();
        if let Some(store) = session_store.as_ref() {
            if let Ok(turns) = store.get_recent_turns(&session_id, HISTORY_LIMIT) {
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
            content: user_msg.clone(),
            name: None,
            images: None,
        });

        let agent_run = agent.clone();
        let sid = session_id.clone();
        tokio::spawn(async move {
            agent_run.run(&sid, &mut messages, event_tx).await;
        });

        let mut assistant_text = String::new();
        while let Some(event) = event_rx.recv().await {
            // Capture assistant output for persistence + push summaries.
            match &event {
                StreamEvent::Token(t) => assistant_text.push_str(t),
                StreamEvent::Done(final_text) if !final_text.is_empty() => {
                    if assistant_text.is_empty() {
                        assistant_text = final_text.clone();
                    }
                }
                StreamEvent::ApprovalRequired {
                    action, risk_level, ..
                } => {
                    if let Some(n) = notifier.as_ref() {
                        push_notify(
                            n.clone(),
                            NtfyMessage::new(
                                "KRIA: approval needed",
                                format!("{action} ({risk_level}) is waiting for your approval."),
                            )
                            .with_priority(NtfyPriority::High)
                            .with_tags(vec!["warning".to_string()]),
                        );
                    }
                }
                _ => {}
            }

            let frame = event_to_frame(&event);
            if send_json(&sink, frame).await.is_err() {
                break; // client disconnected
            }
        }

        // ── Persist the completed turn (4.5.6) ──────────────────────────
        if let Some(store) = session_store.as_ref() {
            persist_turn(store, &session_id, "user", &user_msg);
            if !assistant_text.is_empty() {
                persist_turn(store, &session_id, "assistant", &assistant_text);
            }
        }

        // ── Task-done push (4.5.5) ──────────────────────────────────────
        if let Some(n) = notifier.as_ref() {
            if !assistant_text.is_empty() {
                push_notify(
                    n.clone(),
                    NtfyMessage::new(
                        "KRIA: task done",
                        sanitize_text_for_logs(&assistant_text, 160),
                    )
                    .with_tags(vec!["white_check_mark".to_string()]),
                );
            }
        }

        log_pipeline_step(
            &session_id,
            "server_ws_chat_done",
            "WebSocket agent turn stream finished",
            device_id
                .as_ref()
                .map(|id| serde_json::json!({ "device_id": id })),
        );
    });
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
        tracing::warn!(error = %e, "failed to persist ws conversation turn");
    }
}

/// Fire a push notification without blocking the stream loop.
fn push_notify(notifier: Arc<NtfyClient>, msg: NtfyMessage) {
    tokio::spawn(async move {
        if let Err(e) = notifier.publish(&msg).await {
            tracing::warn!(error = %e, "ntfy push failed");
        }
    });
}

/// Map a `StreamEvent` to the client JSON frame convention (`{"type": …}`).
fn event_to_frame(event: &StreamEvent) -> serde_json::Value {
    use serde_json::json;
    match event {
        StreamEvent::TurnAccepted {
            session_id,
            turn_id,
        } => json!({
            "type": "turn_accepted", "session_id": session_id, "turn_id": turn_id,
        }),
        StreamEvent::Token(t) => json!({ "type": "token", "text": t }),
        StreamEvent::ToolStart { name, params } => json!({
            "type": "tool_start", "name": name, "params": params,
        }),
        StreamEvent::ToolEnd {
            name,
            result,
            success,
            human_readable,
            conversational_summary,
            execution_metadata,
        } => json!({
            "type": "tool_end",
            "name": name,
            "success": success,
            "result": result,
            "human_readable": human_readable,
            "conversational_summary": conversational_summary,
            "execution_metadata": execution_metadata,
        }),
        StreamEvent::RecoveryOptions {
            context,
            detail,
            options,
        } => json!({
            "type": "recovery_options",
            "context": context,
            "detail": detail,
            "options": serde_json::to_value(options).unwrap_or(serde_json::Value::Null),
        }),
        StreamEvent::TaskStep(step) => json!({
            "type": "task_step",
            "step": serde_json::to_value(step).unwrap_or(serde_json::Value::Null),
        }),
        StreamEvent::ToolProgress {
            call_id,
            message,
            percent,
        } => json!({
            "type": "tool_progress", "call_id": call_id, "message": message, "percent": percent,
        }),
        StreamEvent::ToolPayloadChunk {
            call_id,
            seq,
            is_final,
            data,
        } => json!({
            "type": "tool_payload_chunk",
            "call_id": call_id,
            "seq": seq,
            "is_final": is_final,
            "data": data,
        }),
        StreamEvent::ApprovalRequired {
            request_id,
            action,
            risk_level,
            parameters,
        } => json!({
            "type": "approval_required",
            "request_id": request_id,
            "action": action,
            "risk_level": risk_level,
            "parameters": parameters,
        }),
        StreamEvent::ApprovalResult { action, approved } => json!({
            "type": "approval_result", "action": action, "approved": approved,
        }),
        StreamEvent::ToolChoiceRequired {
            query,
            confidence,
            min_confidence,
            candidates,
        } => json!({
            "type": "tool_choice_required",
            "query": query,
            "confidence": confidence,
            "min_confidence": min_confidence,
            "candidates": serde_json::to_value(candidates).unwrap_or(serde_json::Value::Null),
        }),
        StreamEvent::Plan(p) => json!({ "type": "plan", "text": p }),
        StreamEvent::Error(e) => json!({ "type": "error", "message": e }),
        StreamEvent::Done(t) => json!({ "type": "done", "text": t }),
    }
}
