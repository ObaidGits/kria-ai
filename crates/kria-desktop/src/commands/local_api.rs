use super::*;
use std::collections::HashMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct LocalApiN8nCallbackResponse {
    status: String,
    decision: kria_core::n8n::N8nIngestDecision,
    governance: Option<kria_core::n8n::N8nGovernanceDecision>,
    correlation_id: String,
    event_id: String,
    workflow_id: String,
    run_status: kria_core::n8n::N8nRunStatus,
}

#[derive(Debug, serde::Deserialize)]
struct LocalApiN8nHitlQuery {
    request_id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct LocalApiChatRequest {
    pub(super) message: String,
    pub(super) session_id: Option<String>,
    #[serde(default)]
    pub(super) source: Option<String>,
    #[serde(default)]
    pub(super) chat_id: Option<i64>,
    #[serde(default)]
    pub(super) from_user: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct LocalApiFleetEventsQuery {
    #[serde(default)]
    lease_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct LocalApiFleetTerminalQuery {
    target_id: String,
    #[serde(default)]
    lease_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct LocalApiFleetHeartbeatRequest {
    #[serde(default)]
    lease_id: Option<String>,
    #[serde(default)]
    sent_at_unix_ms: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
struct LocalApiFleetDockerEvalRequest {
    lease_id: String,
    target_id: String,
    #[serde(default)]
    suite_name: Option<String>,
}

#[async_trait]
pub(super) trait LocalApiResponder: Send + Sync {
    async fn respond(&self, request: &LocalApiChatRequest) -> serde_json::Value;
}

#[derive(Clone)]
pub(super) struct LocalApiBridgeState {
    pub(super) responder: Arc<dyn LocalApiResponder>,
    pub(super) fleet_control_runtime: Arc<DesktopFleetControlRuntime>,
    pub(super) n8n_catalog: Arc<RwLock<Option<Arc<kria_core::n8n::N8nCatalog>>>>,
    pub(super) n8n_state_store: Arc<kria_core::n8n::N8nWorkflowStateStore>,
    pub(super) n8n_inbox_path: PathBuf,
    pub(super) n8n_audit_path: PathBuf,
    pub(super) n8n_governance_log: Arc<RwLock<Vec<kria_core::n8n::N8nGovernanceDecision>>>,
    pub(super) n8n_hitl_responses: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    pub(super) hitl: Arc<HitlGateway>,
    pub(super) decision_store: Arc<kria_core::agent::collaborative_decision::DecisionStore>,
    pub(super) app_handle: Option<AppHandle>,
}

#[derive(Clone)]
pub(super) struct AgentLoopLocalApiResponder {
    pub(super) agent_loop: Arc<AgentLoop>,
    pub(super) memory_store: Arc<dyn MemoryRuntime>,
    pub(super) tool_registry: Arc<ToolRegistry>,
    pub(super) embeddings: Arc<EmbeddingModel>,
    pub(super) vectors: Arc<VectorIndex>,
    pub(super) hw_tier: String,
    pub(super) orchestrator: Arc<tokio::sync::RwLock<Option<Arc<Orchestrator>>>>,
}

#[async_trait]
impl LocalApiResponder for AgentLoopLocalApiResponder {
    async fn respond(&self, request: &LocalApiChatRequest) -> serde_json::Value {
        let chat_id = request.chat_id.unwrap_or(0);
        let from_user = request.from_user.as_deref().unwrap_or("User");
        let orc_snapshot = self.orchestrator.read().await.clone();
        let reply = kria_core::platform::telegram::process_message(
            &request.message,
            chat_id,
            from_user,
            &self.agent_loop,
            &self.memory_store,
            &self.tool_registry,
            &self.embeddings,
            &self.vectors,
            &self.hw_tier,
            orc_snapshot.as_ref(),
            // Local API bridge is always the owner — it runs inside the desktop
            // process and is not accessible to external callers.
            true,
        )
        .await;

        let session_id = request.session_id.clone().unwrap_or_else(|| {
            if request.chat_id.is_some() || request.source.as_deref() == Some("telegram") {
                format!("telegram_{chat_id}")
            } else {
                uuid::Uuid::new_v4().to_string()
            }
        });

        serde_json::json!({
            "status": "received",
            "message": request.message,
            "source": request.source.clone().unwrap_or_else(|| "api".to_string()),
            "chat_id": request.chat_id,
            "from_user": request.from_user,
            "session_id": session_id,
            "reply": reply,
        })
    }
}

async fn local_api_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "bridge": "desktop",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

pub(super) async fn local_api_chat(
    AxumState(state): AxumState<LocalApiBridgeState>,
    Json(request): Json<LocalApiChatRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if request.message.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "error",
                "message": "message is required",
            })),
        );
    }

    let response = state.responder.respond(&request).await;
    (StatusCode::OK, Json(response))
}

async fn local_api_n8n_callback(
    AxumState(state): AxumState<LocalApiBridgeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<LocalApiN8nCallbackResponse>, (StatusCode, Json<serde_json::Value>)> {
    let signature = headers
        .get("x-kria-signature")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    let catalog = state.n8n_catalog.read().await.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "message": "n8n integration is not enabled in KRIA",
            })),
        )
    })?;

    let envelope =
        kria_core::n8n::parse_and_verify_callback(&catalog, &body, signature).map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "status": "error",
                    "message": error.to_string(),
                })),
            )
        })?;

    let decision = state.n8n_state_store.ingest(envelope.clone());
    let governance = state
        .n8n_state_store
        .get(&envelope.correlation_id)
        .map(|run| {
            let workflow = catalog.get(&run.workflow_id);
            kria_core::n8n::evaluate_run(workflow, &run)
        });
    let record = kria_core::n8n::N8nInboxRecord {
        received_at_ms: local_api_now_unix_ms().max(0) as u128,
        decision: decision.clone(),
        envelope: envelope.clone(),
    };
    if let Err(error) = append_n8n_inbox_record(&state.n8n_inbox_path, &record).await {
        tracing::warn!(error = %error, "failed to persist n8n callback inbox record");
    }
    if let Some(governance) = governance.clone() {
        record_n8n_governance(&state, governance.clone()).await;
        maybe_start_n8n_hitl_bridge(&state, &envelope, &governance);
    }

    let response = LocalApiN8nCallbackResponse {
        status: "received".into(),
        decision,
        governance,
        correlation_id: envelope.correlation_id,
        event_id: envelope.event_id,
        workflow_id: envelope.workflow_id,
        run_status: envelope.status,
    };

    if let Some(app_handle) = state.app_handle.as_ref() {
        let _ = app_handle.emit("n8n:callback", &response);
    }
    Ok(Json(response))
}

async fn local_api_n8n_hitl_response(
    AxumState(state): AxumState<LocalApiBridgeState>,
    Query(query): Query<LocalApiN8nHitlQuery>,
) -> Json<serde_json::Value> {
    let response = state
        .n8n_hitl_responses
        .read()
        .await
        .get(&query.request_id)
        .cloned();

    Json(serde_json::json!({
        "status": if response.is_some() { "ready" } else { "pending" },
        "request_id": query.request_id,
        "response": response,
    }))
}

async fn local_api_fleet_events(
    AxumState(state): AxumState<LocalApiBridgeState>,
    Query(query): Query<LocalApiFleetEventsQuery>,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let mut rx = state.fleet_control_runtime.manager.subscribe_events();
    let snapshot_payload = serde_json::json!({
        "type": "snapshot",
        "targets": state.fleet_control_runtime.snapshot_targets().await,
    })
    .to_string();
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
                        if !local_api_event_matches_lease(&event, filter) {
                            continue;
                        }
                    }

                    let payload = local_api_control_plane_event_json(&event).to_string();
                    yield Ok(Event::default().data(payload));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "local fleet SSE consumer lagged");
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

async fn local_api_fleet_terminal_ws(
    ws: WebSocketUpgrade,
    AxumState(state): AxumState<LocalApiBridgeState>,
    Query(query): Query<LocalApiFleetTerminalQuery>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| local_api_handle_fleet_terminal_socket(socket, state, query))
}

async fn local_api_handle_fleet_terminal_socket(
    socket: WebSocket,
    state: LocalApiBridgeState,
    query: LocalApiFleetTerminalQuery,
) {
    let target_id = match Uuid::parse_str(query.target_id.trim()) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(error = %error, target_id = %query.target_id, "invalid target_id for local terminal ws");
            return;
        }
    };

    let lease_id = query
        .lease_id
        .as_deref()
        .and_then(|raw| Uuid::parse_str(raw).ok());

    let session_id = Uuid::new_v4().to_string();
    if let Err(error) = state
        .fleet_control_runtime
        .manager
        .register_terminal_session(target_id, session_id.clone(), None)
        .await
    {
        tracing::warn!(error = %error, target_id = %target_id, "failed to register local terminal session");
        return;
    }

    let mut rx = state.fleet_control_runtime.manager.subscribe_events();
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
                        tracing::warn!(error = %error, target_id = %target_id, "local terminal websocket receive error");
                        let _ = state
                            .fleet_control_runtime
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
                        if !local_api_event_matches_target(&event, target_id) {
                            continue;
                        }

                        let payload = local_api_control_plane_event_json(&event).to_string();
                        if sender.send(Message::Text(payload.into())).await.is_err() {
                            let _ = state
                                .fleet_control_runtime
                                .manager
                                .report_terminal_ws_failure(
                                    target_id,
                                    session_id.clone(),
                                    None,
                                    "local terminal websocket closed while sending event",
                                    true,
                                )
                                .await;
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, target_id = %target_id, "local terminal websocket event stream lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn local_api_fleet_lease_heartbeat(
    AxumPath(lease_id): AxumPath<Uuid>,
    AxumState(state): AxumState<LocalApiBridgeState>,
    Json(payload): Json<LocalApiFleetHeartbeatRequest>,
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

    match state
        .fleet_control_runtime
        .manager
        .heartbeat(lease_id)
        .await
    {
        Ok(()) => Ok(Json(serde_json::json!({
            "type": "heartbeat_ack",
            "lease_id": lease_id,
            "received_sent_at_unix_ms": payload.sent_at_unix_ms,
            "ts_unix_ms": Utc::now().timestamp_millis(),
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

async fn local_api_fleet_docker_evals(
    AxumState(state): AxumState<LocalApiBridgeState>,
    Json(request): Json<LocalApiFleetDockerEvalRequest>,
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
        .fleet_control_runtime
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
            "status": local_api_docker_health_label(summary.status),
            "passed_count": summary.passed_count,
            "failed_count": summary.failed_count,
            "started_at_unix_ms": summary.started_at_unix_ms,
            "finished_at_unix_ms": summary.finished_at_unix_ms,
            "cases": summary.cases,
        }
    })))
}

fn local_api_event_matches_lease(event: &ControlPlaneEvent, lease_id: Uuid) -> bool {
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

fn local_api_event_matches_target(event: &ControlPlaneEvent, target_id: Uuid) -> bool {
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

fn local_api_control_plane_event_json(event: &ControlPlaneEvent) -> serde_json::Value {
    match event {
        ControlPlaneEvent::TargetStatus {
            target_id,
            display_name,
            mode,
            state,
            tainted,
            reason,
            health_score,
            latency_ewma_ms,
            recent_failure_rate,
            docker_health,
            docker_pass_count,
            docker_fail_count,
            docker_last_run_at_unix_ms,
        } => serde_json::json!({
            "type": "target_status",
            "target_id": target_id,
            "display_name": display_name,
            "mode": local_api_target_mode_label(*mode),
            "state": local_api_target_state_label(*state),
            "tainted": tainted,
            "reason": reason,
            "health_score": health_score,
            "latency_ewma_ms": latency_ewma_ms,
            "recent_failure_rate": recent_failure_rate,
            "docker_health": local_api_docker_health_label(*docker_health),
            "docker_pass_count": docker_pass_count,
            "docker_fail_count": docker_fail_count,
            "docker_last_run_at_unix_ms": docker_last_run_at_unix_ms,
            "updated_at_unix_ms": local_api_now_unix_ms(),
        }),
        ControlPlaneEvent::FleetAlert {
            target_id,
            lease_id,
            category,
            message,
        } => serde_json::json!({
            "type": "fleet_alert",
            "target_id": target_id,
            "lease_id": lease_id,
            "category": category,
            "message": message,
            "created_at_unix_ms": local_api_now_unix_ms(),
        }),
        ControlPlaneEvent::DockerEvalUpdate {
            target_id,
            run_id,
            docker_health,
            docker_pass_count,
            docker_fail_count,
            updated_at_unix_ms,
        } => serde_json::json!({
            "type": "docker_eval_update",
            "target_id": target_id,
            "run_id": run_id,
            "docker_health": local_api_docker_health_label(*docker_health),
            "docker_pass_count": docker_pass_count,
            "docker_fail_count": docker_fail_count,
            "docker_last_run_at_unix_ms": updated_at_unix_ms,
            "updated_at_unix_ms": updated_at_unix_ms,
        }),
        ControlPlaneEvent::TerminalGap { marker } => serde_json::json!({
            "type": "terminal_gap",
            "target_id": marker.target_id,
            "session_id": marker.session_id,
            "since_offset": marker.since_offset,
            "message": marker.message,
            "created_at_unix_ms": marker.created_at_unix_ms,
        }),
        ControlPlaneEvent::TerminalLine {
            target_id,
            lease_id,
            offset,
            stream,
            text,
            ts_unix_ms,
        } => serde_json::json!({
            "type": "terminal_line",
            "target_id": target_id,
            "lease_id": lease_id,
            "offset": offset,
            "stream": local_api_terminal_stream_label(*stream),
            "text": text,
            "ts_unix_ms": ts_unix_ms,
        }),
        ControlPlaneEvent::ClockDrift { alert } => serde_json::json!({
            "type": "clock_drift",
            "alert": {
                "target_id": alert.target_id,
                "previous_buffer_ms": alert.previous_buffer_ms,
                "next_buffer_ms": alert.next_buffer_ms,
                "rejection_count": alert.rejection_count,
                "created_at_unix_ms": alert.created_at_unix_ms,
            }
        }),
        ControlPlaneEvent::TargetRemoved { target_id } => serde_json::json!({
            "type": "target_removed",
            "target_id": target_id,
        }),
    }
}

fn local_api_target_mode_label(mode: TargetMode) -> &'static str {
    match mode {
        TargetMode::SshBootstrap => "ssh_bootstrap",
        TargetMode::ReverseWs => "reverse_ws",
        TargetMode::UnixSocket => "unix_socket",
    }
}

fn local_api_target_state_label(state: TargetState) -> &'static str {
    match state {
        TargetState::Ready => "ready",
        TargetState::Leased => "leased",
        TargetState::Quarantine => "quarantine",
        TargetState::Tainted => "tainted",
        TargetState::Disabled => "disabled",
    }
}

fn local_api_docker_health_label(status: DockerHealthStatus) -> &'static str {
    match status {
        DockerHealthStatus::Unknown => "unknown",
        DockerHealthStatus::Running => "running",
        DockerHealthStatus::Pass => "pass",
        DockerHealthStatus::Fail => "fail",
    }
}

fn local_api_terminal_stream_label(stream: TerminalStream) -> &'static str {
    match stream {
        TerminalStream::Stdout => "stdout",
        TerminalStream::Stderr => "stderr",
        TerminalStream::System => "system",
    }
}

fn local_api_now_unix_ms() -> i64 {
    Utc::now().timestamp_millis()
}

async fn append_n8n_inbox_record(
    path: &Path,
    record: &kria_core::n8n::N8nInboxRecord,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(&line).await?;
    Ok(())
}

async fn append_n8n_audit_record(
    path: &Path,
    decision: &kria_core::n8n::N8nGovernanceDecision,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let record = serde_json::json!({
        "ts_unix_ms": local_api_now_unix_ms(),
        "type": "n8n_governance_decision",
        "decision": decision,
    });
    let mut line = serde_json::to_vec(&record)?;
    line.push(b'\n');
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(&line).await?;
    Ok(())
}

async fn record_n8n_governance(
    state: &LocalApiBridgeState,
    decision: kria_core::n8n::N8nGovernanceDecision,
) {
    {
        let mut log = state.n8n_governance_log.write().await;
        log.push(decision.clone());
        let overflow = log.len().saturating_sub(100);
        if overflow > 0 {
            log.drain(0..overflow);
        }
    }

    if let Err(error) = append_n8n_audit_record(&state.n8n_audit_path, &decision).await {
        tracing::warn!(error = %error, "failed to persist n8n governance audit record");
    }

    if let Some(app_handle) = state.app_handle.as_ref() {
        let _ = app_handle.emit("n8n:governance", &decision);
        if decision.continuation_action == kria_core::n8n::N8nContinuationAction::ContinueWorkflow {
            let _ = app_handle.emit("n8n:continuation", &decision);
        }
    }
}

fn maybe_start_n8n_hitl_bridge(
    state: &LocalApiBridgeState,
    envelope: &kria_core::n8n::N8nCallbackEnvelope,
    decision: &kria_core::n8n::N8nGovernanceDecision,
) {
    if decision.continuation_action != kria_core::n8n::N8nContinuationAction::PauseForHitl {
        return;
    }

    let evidence = envelope.evidence.clone();
    let request_id = evidence
        .get("hitl_request_id")
        .or_else(|| evidence.get("request_id"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(kria_core::safety::hitl::HitlGateway::generate_request_id);
    let description = evidence
        .get("question")
        .or_else(|| evidence.get("description"))
        .and_then(|value| value.as_str())
        .unwrap_or("n8n workflow needs human approval before it can continue")
        .to_string();
    let workflow_id = envelope.workflow_id.clone();
    let correlation_id = envelope.correlation_id.clone();

    let hitl = state.hitl.clone();
    let decision_store = state.decision_store.clone();
    let responses = state.n8n_hitl_responses.clone();
    let app_handle = state.app_handle.clone();
    let params = serde_json::json!({
        "source": "n8n",
        "workflow_id": envelope.workflow_id,
        "workflow_version": envelope.workflow_version,
        "correlation_id": envelope.correlation_id,
        "n8n_run_id": envelope.n8n_run_id,
        "evidence": evidence,
    });

    tokio::spawn(async move {
        let collaborative_decision_id = {
            use kria_core::agent::collaborative_decision::{DecisionCandidate, Rollbackability};

            let affected_resources = vec![
                format!("n8n:workflow:{workflow_id}"),
                format!("n8n:correlation:{correlation_id}"),
            ];
            let candidate = DecisionCandidate::approval(
                "n8n workflow continuation",
                description.clone(),
                RiskLevel::Red,
                Rollbackability::Unknown,
                affected_resources,
                Some("n8n.pause_for_hitl".to_string()),
            );

            match decision_store.create_decision(
                workflow_id.clone(),
                Some(correlation_id.clone()),
                candidate,
            ) {
                Ok(decision) => {
                    if let Some(app_handle) = app_handle.as_ref() {
                        let _ = app_handle.emit("interaction_decision:created", &decision);
                    }
                    Some(decision.id)
                }
                Err(error) => {
                    tracing::warn!(error = %error, "failed to persist n8n collaborative decision");
                    None
                }
            }
        };

        let response = hitl
            .request_approval_with_id(
                &request_id,
                "n8n_workflow_approval",
                params,
                RiskLevel::Red,
                &description,
                false,
            )
            .await;

        let response_payload = serde_json::json!({
            "request_id": request_id,
            "approved": matches!(response, ApprovalResponse::Approved),
            "response": match response {
                ApprovalResponse::Approved => "approved",
                ApprovalResponse::Denied => "denied",
                ApprovalResponse::Timeout => "timeout",
            },
            "interaction_decision_id": collaborative_decision_id,
            "decided_at_unix_ms": local_api_now_unix_ms(),
        });
        if let Some(decision_id) = collaborative_decision_id.as_deref() {
            let result = match response {
                ApprovalResponse::Approved => {
                    decision_store.resolve(decision_id, "approve", "hitl_gateway")
                }
                ApprovalResponse::Denied => {
                    decision_store.resolve(decision_id, "deny", "hitl_gateway")
                }
                ApprovalResponse::Timeout => decision_store.expire(decision_id, "hitl_gateway"),
            };
            if let Err(error) = result {
                tracing::warn!(error = %error, decision_id, "failed to update n8n collaborative decision");
            }
        }
        responses
            .write()
            .await
            .insert(request_id.clone(), response_payload.clone());
        if let Some(app_handle) = app_handle.as_ref() {
            let _ = app_handle.emit("n8n:hitl_response", response_payload);
        }
    });
}

async fn probe_existing_local_api_bridge(health_url: &str) -> bool {
    match reqwest::Client::new()
        .get(health_url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

pub(super) fn start_local_api_bridge(
    host: String,
    port: u16,
    responder: Arc<dyn LocalApiResponder>,
    fleet_control_runtime: Arc<DesktopFleetControlRuntime>,
    n8n_catalog: Arc<RwLock<Option<Arc<kria_core::n8n::N8nCatalog>>>>,
    n8n_state_store: Arc<kria_core::n8n::N8nWorkflowStateStore>,
    n8n_inbox_path: PathBuf,
    n8n_audit_path: PathBuf,
    n8n_governance_log: Arc<RwLock<Vec<kria_core::n8n::N8nGovernanceDecision>>>,
    n8n_hitl_responses: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    hitl: Arc<HitlGateway>,
    decision_store: Arc<kria_core::agent::collaborative_decision::DecisionStore>,
    app_handle: AppHandle,
    health: Arc<HealthRegistry>,
) {
    let bind_addr = format!("{host}:{port}");
    let health_url = format!("{}/api/health", local_api_base_url(&host, port));
    health.register("local_api_bridge");
    health.update(
        "local_api_bridge",
        ServiceStatus::Starting,
        Some(format!("binding {bind_addr}")),
    );

    tokio::spawn(async move {
        match tokio::net::TcpListener::bind(&bind_addr).await {
            Ok(listener) => {
                let router = Router::new()
                    .route("/api/health", get(local_api_health))
                    .route("/api/chat", post(local_api_chat))
                    .route("/api/n8n/callback", post(local_api_n8n_callback))
                    .route("/api/n8n/hitl-response", get(local_api_n8n_hitl_response))
                    .route("/api/fleet/events", get(local_api_fleet_events))
                    .route("/api/fleet/terminal", get(local_api_fleet_terminal_ws))
                    .route(
                        "/api/fleet/leases/{lease_id}/heartbeat",
                        post(local_api_fleet_lease_heartbeat),
                    )
                    .route(
                        "/api/fleet/docker-evals",
                        post(local_api_fleet_docker_evals),
                    )
                    .layer(CorsLayer::permissive())
                    .with_state(LocalApiBridgeState {
                        responder,
                        fleet_control_runtime,
                        n8n_catalog,
                        n8n_state_store,
                        n8n_inbox_path,
                        n8n_audit_path,
                        n8n_governance_log,
                        n8n_hitl_responses,
                        hitl,
                        decision_store,
                        app_handle: Some(app_handle),
                    });

                health.update(
                    "local_api_bridge",
                    ServiceStatus::Healthy,
                    Some(format!("listening on {health_url}")),
                );

                if let Err(e) = axum::serve(listener, router).await {
                    health.update(
                        "local_api_bridge",
                        ServiceStatus::Degraded,
                        Some(format!("bridge stopped: {e}")),
                    );
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                if probe_existing_local_api_bridge(&health_url).await {
                    health.update(
                        "local_api_bridge",
                        ServiceStatus::Healthy,
                        Some(format!("reusing existing listener at {health_url}")),
                    );
                } else {
                    health.update(
                        "local_api_bridge",
                        ServiceStatus::Degraded,
                        Some(format!(
                            "{bind_addr} already in use, but {health_url} is not responding"
                        )),
                    );
                }
            }
            Err(e) => {
                health.update(
                    "local_api_bridge",
                    ServiceStatus::Degraded,
                    Some(format!("failed to bind {bind_addr}: {e}")),
                );
            }
        }
    });
}
