use super::*;
use std::collections::HashMap;
use std::time::Instant;

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

fn n8n_log_preview_text(value: &str, max_chars: usize) -> String {
    kria_core::infra::pipeline_trace::sanitize_text_for_logs(value, max_chars)
}

fn log_n8n_execution_step(
    correlation_id: &str,
    step: u8,
    total_steps: u8,
    label: &str,
    workflow_id: Option<&str>,
    detail: String,
    elapsed_ms: Option<u128>,
) {
    tracing::info!(
        target: "n8n_execution_trace",
        correlation_id = %correlation_id,
        workflow_id = workflow_id.unwrap_or("-"),
        step,
        total_steps,
        elapsed_ms = ?elapsed_ms,
        detail = %detail,
        "[N8N][{}] Step {}/{} {}",
        correlation_id,
        step,
        total_steps,
        label
    );
}

#[cfg(test)]
mod n8n_chat_bridge_tests {
    use super::*;

    #[test]
    fn parses_explicit_n8n_run_references() {
        assert_eq!(
            parse_local_api_n8n_run_reference("Run test_workflow"),
            Some("test_workflow".to_string())
        );
        assert_eq!(
            parse_local_api_n8n_run_reference("trigger n8n workflow invoice.sync-v1 now"),
            Some("invoice.sync-v1".to_string())
        );
        assert_eq!(
            parse_local_api_n8n_run_reference("run workflow `demo_flow`, please"),
            Some("demo_flow".to_string())
        );
        assert_eq!(
            parse_local_api_n8n_run_reference("Run the test workflow"),
            Some("test workflow".to_string())
        );
        assert_eq!(
            parse_local_api_n8n_run_reference("Retry test_workflow"),
            Some("test_workflow".to_string())
        );
        assert_eq!(
            parse_local_api_n8n_run_reference("Run Test Workflow"),
            Some("Test Workflow".to_string())
        );
    }

    #[test]
    fn ignores_non_workflow_chat_and_invalid_ids() {
        assert_eq!(parse_local_api_n8n_run_reference("hello there"), None);
        assert_eq!(parse_local_api_n8n_run_reference("run"), None);
        assert_eq!(parse_local_api_n8n_run_reference("run ../../secret"), None);
    }

    #[test]
    fn parses_explicit_n8n_confirmation_references() {
        assert_eq!(
            parse_local_api_n8n_confirmation_reference("Confirm workflow gmail_inbox_digest"),
            Some("gmail_inbox_digest".to_string())
        );
        assert_eq!(
            parse_local_api_n8n_confirmation_reference(
                "yes run workflow `slack_post_update`, please"
            ),
            Some("slack_post_update".to_string())
        );
        assert_eq!(
            parse_local_api_n8n_confirmation_reference("confirm workflow ../../secret"),
            None
        );
    }

    #[test]
    fn recognizes_n8n_workflow_inventory_queries() {
        assert!(is_local_api_n8n_workflow_list_query(
            "list of n8n workflows i have"
        ));
        assert!(is_local_api_n8n_workflow_list_query(
            "list of workflows i have"
        ));
        assert!(is_local_api_n8n_workflow_list_query("all workflows list"));
        assert!(is_local_api_n8n_workflow_list_query("n8n discover"));
        assert!(!is_local_api_n8n_workflow_list_query(
            "run gmail_inbox_digest workflow"
        ));
        assert!(!is_local_api_n8n_workflow_list_query(
            "confirm workflow gmail_inbox_digest"
        ));
    }
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

#[derive(Debug, Clone, serde::Deserialize)]
struct LocalApiDesktopChatCommandRequest {
    message: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    manual_profile: Option<super::chat::ManualToolProfileInput>,
    #[serde(default)]
    gui_cognition_test: Option<LocalApiGuiCognitionTestOptions>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct LocalApiGuiCognitionTestOptions {
    #[serde(default)]
    llm_planner_fixture: Option<kria_core::agent::gui_cognition::llm_planner::GuiLlmPlannerFixture>,
    #[serde(default)]
    disable_live_llm_planner: Option<bool>,
    #[serde(default)]
    action_backend_fixture: Option<super::gui_cognition::GuiActionBackendFixture>,
    #[serde(default)]
    perception_fixture: Option<super::gui_cognition::GuiPerceptionFixture>,
    #[serde(default)]
    hitl_decision_fixture: Option<kria_core::agent::gui_cognition::safety_hitl::GuiHitlDecisionFixture>,
    #[serde(default)]
    execution_mode: Option<kria_core::agent::gui_cognition::executor::GuiExecutionMode>,
    #[serde(default)]
    workflow: Option<bool>,
    #[serde(default)]
    workflow_resume: Option<bool>,
    #[serde(default)]
    resume_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct LocalApiN8nPendingSuggestion {
    prompt: String,
    response: kria_core::n8n::WorkflowSuggestionResponse,
    created_at_ms: i64,
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

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalApiN8nRoutePromptRequest {
    prompt: String,
    #[serde(default)]
    previous_user_prompt: Option<String>,
    #[serde(default)]
    manual_n8n_mode: bool,
    #[serde(default)]
    safe_auto_run_enabled: bool,
}

struct NoopLocalApiResponder;

#[async_trait]
impl LocalApiResponder for NoopLocalApiResponder {
    async fn respond(&self, request: &LocalApiChatRequest) -> serde_json::Value {
        serde_json::json!({
            "status": "ignored",
            "message": request.message,
        })
    }
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
    pub(super) n8n_pending_suggestions: Arc<RwLock<HashMap<String, LocalApiN8nPendingSuggestion>>>,
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

async fn local_api_health(
    AxumState(state): AxumState<LocalApiBridgeState>,
) -> Json<serde_json::Value> {
    let n8n_enabled = state.n8n_catalog.read().await.is_some();
    Json(serde_json::json!({
        "status": "healthy",
        "bridge": "desktop",
        "version": env!("CARGO_PKG_VERSION"),
        "features": {
            "n8n_enabled": n8n_enabled,
            "n8n_stage3_confirmation_routing": true,
            "n8n_schema_validation": true,
            "n8n_prompt_context_confirmation": true,
            "n8n_harness_catalog_labels": true,
        },
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

    if let Some(response) = local_api_n8n_pre_fallback_response(&state, &request).await {
        return response;
    }

    let response = state.responder.respond(&request).await;
    (StatusCode::OK, Json(response))
}

async fn local_api_desktop_chat_command(
    AxumState(state): AxumState<LocalApiBridgeState>,
    Json(request): Json<LocalApiDesktopChatCommandRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if request.message.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "error",
                "message": "message is required",
                "reply": "message is required",
            })),
        );
    }

    let app = match state.app_handle.clone() {
        Some(app) => app,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "error",
                    "message": "KRIA app runtime is not attached to the desktop chat command bridge",
                    "reply": "KRIA app runtime is not attached to the desktop chat command bridge",
                })),
            )
        }
    };
    let state_cell = match local_api_app_state(&state) {
        Ok(state_cell) => state_cell,
        Err(error) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "error",
                    "message": error,
                    "reply": error,
                })),
            )
        }
    };
    let Some(app_state) = state_cell.get() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "message": "KRIA is still initializing — please try again in a moment",
                "reply": "KRIA is still initializing — please try again in a moment",
            })),
        );
    };

    if request.manual_profile.as_ref().is_some_and(|profile| {
        profile.mode_id.trim().eq_ignore_ascii_case("gui_cognition")
            || profile
                .app_lock
                .as_deref()
                .map(|value| value.trim().to_ascii_lowercase())
                .is_some_and(|value| {
                    matches!(
                        value.as_str(),
                        "gui" | "gui_cognition" | "gui-cognition" | "desktop_gui"
                    )
                })
    }) {
        match super::gui_cognition::desktop_gui_cognition_command_capture(
            request.message,
            app_state,
            request.session_id,
            "agent",
            request.gui_cognition_test.map(|options| {
                super::gui_cognition::GuiCognitionCommandOptions {
                    llm_planner_fixture: options.llm_planner_fixture,
                    disable_live_llm_planner: options.disable_live_llm_planner.unwrap_or(false),
                    action_backend_fixture: options.action_backend_fixture,
                    perception_fixture: options.perception_fixture,
                    hitl_decision_fixture: options.hitl_decision_fixture,
                    execution_mode: options.execution_mode.unwrap_or_default(),
                    workflow_enabled: options.workflow.unwrap_or(false),
                    workflow_resume: options.workflow_resume.unwrap_or(false),
                    resume_reason: options.resume_reason,
                }
            }),
        )
        .await
        {
            Ok(capture) => {
                let status = StatusCode::from_u16(capture.status_code).unwrap_or(StatusCode::OK);
                return (
                    status,
                    Json(serde_json::json!({
                        "status": capture.status,
                        "reply": capture.reply,
                        "events": capture.events,
                        "desktop_command": {
                            "path": "send_manual_tool_message",
                            "ui_opened": false,
                            "source": "desktop_chat",
                            "mode_id": "gui_cognition"
                        },
                        "response": capture.response,
                    })),
                );
            }
            Err(error) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({
                        "status": "error",
                        "message": error,
                        "reply": error,
                    })),
                );
            }
        }
    }

    match super::chat::desktop_n8n_pre_fallback_command_capture(
        request.message,
        app_state,
        app,
        request.session_id,
        "agent",
    )
    .await
    {
        Some(Ok(capture)) => {
            let status = StatusCode::from_u16(capture.status_code).unwrap_or(StatusCode::OK);
            (
                status,
                Json(serde_json::json!({
                    "status": capture.status,
                    "reply": capture.reply,
                    "events": capture.events,
                    "desktop_command": {
                        "path": "send_message",
                        "ui_opened": false,
                        "source": "desktop_chat"
                    },
                    "response": capture.response,
                })),
            )
        }
        Some(Err(error)) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "status": "error",
                "message": error,
                "reply": error,
            })),
        ),
        None => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "not_handled",
                "reply": "Desktop send_message did not enter deterministic n8n handling for this prompt.",
                "events": [],
                "desktop_command": {
                    "path": "send_message",
                    "ui_opened": false,
                    "source": "desktop_chat"
                },
            })),
        ),
    }
}

async fn local_api_gui_automation_status(
    AxumState(state): AxumState<LocalApiBridgeState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let state_cell = match local_api_app_state(&state) {
        Ok(state_cell) => state_cell,
        Err(error) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "error",
                    "message": error,
                })),
            )
        }
    };
    let Some(app_state) = state_cell.get() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "message": "KRIA is still initializing — please try again in a moment",
            })),
        );
    };

    let action_backend =
        super::gui_cognition::build_gui_action_backend_status(app_state, None).await;
    let capabilities = serde_json::to_value(&action_backend.capabilities)
        .unwrap_or_else(|_| serde_json::json!({}));

    // Task 13 (Issue #11): surface the window-focus/capture/activate backend
    // availability + honest capability notice (additive; flag-OFF omits it).
    let gui_backend_status =
        if kria_core::agent::gui_cognition::window_focus::backend_status_enabled() {
            let is_wayland = action_backend.session_type.eq_ignore_ascii_case("wayland");
            let status = super::gui_cognition::assess_gui_backend_status(
                action_backend.uinput_available,
                action_backend.xdotool_available,
                is_wayland,
            )
            .await;
            serde_json::to_value(&status).unwrap_or(serde_json::Value::Null)
        } else {
            serde_json::Value::Null
        };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "gui_automation": {
                "global_halt_engaged": action_backend.global_halt_engaged,
                "halt_kind": action_backend.halt_kind,
                "halt_reason": action_backend.halt_reason,
                "release_conditions": action_backend.release_conditions,
                "startup_elapsed_ms": action_backend.startup_elapsed_ms,
                "can_observe": action_backend.can_observe,
                "can_plan": action_backend.can_plan,
                "automation_enabled": action_backend.automation_enabled,
                "vision_sidecar": action_backend.vision_sidecar,
                "uinput_daemon": action_backend.uinput_daemon,
                "orchestrator_available": action_backend.orchestrator_available,
                "session_type": action_backend.session_type,
                "xdotool_available": action_backend.xdotool_available,
                "ydotool_available": action_backend.ydotool_available,
                "uinput_available": action_backend.uinput_available,
                "selected_backend": action_backend.selected_backend,
                "backend_selection_reason": action_backend.backend_selection_reason,
                "backend_probe_status": action_backend.backend_probe_status,
                "backend_probe_errors": action_backend.backend_probe_errors,
                "input_backend_kind": action_backend.input_backend_kind,
                "focus_supported": action_backend.focus_supported,
                "typing_supported": action_backend.typing_supported,
                "click_supported": action_backend.click_supported,
                "verification_supported": action_backend.verification_supported,
                "xdotool_usable_for_actions": action_backend.xdotool_usable_for_actions,
                "ydotool_usable_for_actions": action_backend.ydotool_usable_for_actions,
                "uinput_socket_path": action_backend.uinput_socket_path,
                "uinput_socket_accessible": action_backend.uinput_socket_accessible,
                "can_execute_actions": action_backend.can_execute_actions,
                "blockers": action_backend.blockers,
                "capabilities": capabilities,
                "gui_backend_status": gui_backend_status,
            }
        })),
    )
}

pub(super) async fn local_api_n8n_pre_fallback_response_from_app_state(
    app_state: &AppState,
    app_handle: AppHandle,
    request: LocalApiChatRequest,
) -> Option<(StatusCode, Json<serde_json::Value>)> {
    let state = LocalApiBridgeState {
        responder: Arc::new(NoopLocalApiResponder),
        fleet_control_runtime: app_state.fleet_control_runtime.clone(),
        n8n_catalog: app_state.n8n_catalog.clone(),
        n8n_state_store: app_state.n8n_state_store.clone(),
        n8n_inbox_path: app_state.n8n_inbox_path.clone(),
        n8n_audit_path: app_state.n8n_audit_path.clone(),
        n8n_governance_log: app_state.n8n_governance_log.clone(),
        n8n_hitl_responses: app_state.n8n_hitl_responses.clone(),
        n8n_pending_suggestions: Arc::new(RwLock::new(HashMap::new())),
        hitl: app_state.hitl.clone(),
        decision_store: app_state.decision_store.clone(),
        app_handle: Some(app_handle),
    };
    local_api_n8n_pre_fallback_response(&state, &request).await
}

async fn local_api_n8n_pre_fallback_response(
    state: &LocalApiBridgeState,
    request: &LocalApiChatRequest,
) -> Option<(StatusCode, Json<serde_json::Value>)> {
    if let Some(response) = local_api_n8n_prompt_action_response(&state, &request).await {
        return Some(response);
    }

    if let Some(response) = local_api_n8n_info_response(&state, &request).await {
        return Some((StatusCode::OK, Json(response)));
    }

    if let Some(reference) = parse_local_api_n8n_confirmation_reference(&request.message) {
        return Some(
            match invoke_local_api_n8n_confirmed_workflow_reference(&state, &request, &reference)
                .await
            {
                Ok(response) => (StatusCode::OK, Json(response)),
                Err(error) => (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({
                        "status": "error",
                        "error": error.clone(),
                        "message": error.clone(),
                        "reply": error,
                        "source": request.source.clone().unwrap_or_else(|| "api".to_string()),
                        "chat_id": request.chat_id,
                        "from_user": request.from_user,
                        "session_id": local_api_session_id(&request),
                    })),
                ),
            },
        );
    }

    if let Some(reference) = parse_local_api_n8n_run_reference(&request.message) {
        return Some(
            match suggest_local_api_n8n_workflow_reference(&state, &request, &reference).await {
                Ok(response) => (StatusCode::OK, Json(response)),
                Err(error) => (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({
                        "status": "error",
                        "error": error.clone(),
                        "message": error.clone(),
                        "reply": error,
                        "source": request.source.clone().unwrap_or_else(|| "api".to_string()),
                        "chat_id": request.chat_id,
                        "from_user": request.from_user,
                        "session_id": local_api_session_id(&request),
                    })),
                ),
            },
        );
    }

    if let Some(response) = suggest_local_api_n8n_workflow_prompt(&state, &request).await {
        return Some((StatusCode::OK, Json(response)));
    }

    None
}

fn local_api_app_state<'a>(
    state: &'a LocalApiBridgeState,
) -> Result<tauri::State<'a, AppStateCell>, String> {
    state
        .app_handle
        .as_ref()
        .map(|app| app.state::<AppStateCell>())
        .ok_or_else(|| "KRIA app runtime is not attached to the local API bridge".to_string())
}

fn local_api_command_json(
    result: Result<serde_json::Value, String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match result {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "status": "error",
                "error": error,
                "message": error,
                "reply": error,
            })),
        ),
    }
}

fn local_api_route_requested_display_name(
    route: &kria_core::n8n::N8nChatRouteDecision,
) -> Option<String> {
    route
        .input_payload_preview
        .get("requested_workflow_name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn local_api_n8n_card(
    action: &str,
    title: impl Into<String>,
    subtitle: impl Into<String>,
    primary_action: impl Into<String>,
    secondary_actions: Vec<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "title": title.into(),
        "subtitle": subtitle.into(),
        "primary_action": primary_action.into(),
        "secondary_actions": secondary_actions,
        "action": action,
    })
}

fn local_api_prefixed_reference(message: &str, prefixes: &[&str]) -> Option<String> {
    let trimmed = message.trim();
    let lower = trimmed.to_ascii_lowercase();
    let prefix = prefixes.iter().find(|prefix| lower.starts_with(**prefix))?;
    let mut reference = trimmed[prefix.len()..].trim();
    if reference.to_ascii_lowercase().starts_with("the ") {
        reference = reference[4..].trim();
    }
    let mut cleaned = reference
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';'))
        .trim()
        .to_string();
    for separator in [" with ", " using ", " and ", " please", " now"] {
        let lower_cleaned = cleaned.to_ascii_lowercase();
        if let Some(index) = lower_cleaned.find(separator) {
            cleaned.truncate(index);
            cleaned = cleaned
                .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';'))
                .trim()
                .to_string();
        }
    }
    if cleaned.is_empty()
        || cleaned
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\'))
    {
        return None;
    }
    Some(cleaned)
}

fn parse_local_api_n8n_archive_reference(message: &str) -> Option<String> {
    local_api_prefixed_reference(
        message,
        &[
            "archive n8n workflow ",
            "archive workflow ",
            "archive n8n ",
            "archive ",
        ],
    )
}

fn parse_local_api_n8n_restore_reference(message: &str) -> Option<String> {
    local_api_prefixed_reference(
        message,
        &[
            "restore n8n workflow ",
            "restore workflow ",
            "restore n8n ",
            "restore ",
        ],
    )
}

fn parse_local_api_n8n_test_draft_reference(message: &str) -> Option<String> {
    local_api_prefixed_reference(
        message,
        &[
            "test n8n draft ",
            "test workflow draft ",
            "test draft workflow ",
            "test draft ",
        ],
    )
}

fn parse_local_api_n8n_approve_draft_reference(message: &str) -> Option<String> {
    local_api_prefixed_reference(
        message,
        &[
            "approve n8n draft ",
            "approve workflow draft ",
            "approve draft workflow ",
            "approve draft ",
        ],
    )
}

fn parse_local_api_n8n_cleanup_draft_reference(message: &str) -> Option<String> {
    local_api_prefixed_reference(
        message,
        &[
            "cleanup n8n draft ",
            "clean up n8n draft ",
            "cleanup workflow draft ",
            "clean up workflow draft ",
            "cleanup draft ",
            "clean up draft ",
            "reject draft ",
        ],
    )
}

fn local_api_test_input_payload_from_prompt(message: &str) -> serde_json::Value {
    let lower = message.to_ascii_lowercase();
    let title = if lower.contains("inception") {
        "Inception"
    } else if lower.contains("matrix") {
        "The Matrix"
    } else if lower.contains("avatar") {
        "Avatar"
    } else {
        "Inception"
    };
    serde_json::json!({
        "title": title,
        "query": title,
        "body": {
            "title": title,
            "query": title
        },
        "query_params": {
            "title": title,
            "query": title
        }
    })
}

fn local_api_n8n_route_decision(
    state: &LocalApiBridgeState,
    request: &LocalApiChatRequest,
    previous_user_prompt: Option<String>,
    manual_n8n_mode: bool,
    safe_auto_run_enabled: bool,
) -> Option<kria_core::n8n::N8nChatRouteDecision> {
    let catalog = state.n8n_catalog.try_read().ok()?.clone()?;
    let workflows = catalog.workflows();
    Some(
        kria_core::n8n::WorkflowRankingEngine::new(workflows).route_chat(
            kria_core::n8n::N8nChatRouteRequest {
                prompt: request.message.clone(),
                previous_user_prompt,
                manual_n8n_mode,
                safe_auto_run_enabled,
                workflows: Vec::new(),
            },
        ),
    )
}

async fn resolve_local_api_n8n_action_reference(
    state: &LocalApiBridgeState,
    reference: &str,
) -> Option<String> {
    let catalog = state.n8n_catalog.read().await.clone()?;
    let workflows = catalog.workflows();
    match kria_core::n8n::resolve_n8n_workflow_reference(&workflows, reference) {
        kria_core::n8n::N8nWorkflowReferenceMatch::Unique { workflow, .. } => {
            Some(workflow.workflow_id.clone())
        }
        _ => {
            let registry_workflows =
                super::n8n::load_workflow_registry_all_workflows().unwrap_or_default();
            match kria_core::n8n::resolve_n8n_workflow_reference(&registry_workflows, reference) {
                kria_core::n8n::N8nWorkflowReferenceMatch::Unique { workflow, .. } => {
                    Some(workflow.workflow_id.clone())
                }
                _ => Some(reference.trim().to_string()).filter(|value| !value.is_empty()),
            }
        }
    }
}

fn local_api_normalized_reference(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut last_space = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            last_space = false;
        } else if !last_space && !normalized.is_empty() {
            normalized.push(' ');
            last_space = true;
        }
    }
    normalized.trim().to_string()
}

fn local_api_prompt_has_n8n_targeting_intent(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    if [
        "create ",
        "build ",
        "make ",
        "generate ",
        "set up ",
        "setup ",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
        && kria_core::n8n::extract_n8n_authoring_workflow_name(message).is_some()
    {
        return false;
    }
    let has_target_verb = [
        "update ",
        "change ",
        "modify ",
        "edit ",
        "archive ",
        "restore ",
        "delete ",
        "remove ",
        "permanently delete",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase));
    has_target_verb && (lower.contains("workflow") || lower.contains("n8n"))
}

async fn local_api_find_unregistered_n8n_target(
    state: &LocalApiBridgeState,
    prompt_or_reference: &str,
) -> Option<(String, String)> {
    let needle = local_api_normalized_reference(prompt_or_reference);
    if needle.len() < 3 {
        return None;
    }
    let catalog_workflows = state
        .n8n_catalog
        .read()
        .await
        .clone()
        .map(|catalog| catalog.workflows())
        .unwrap_or_default();
    let registry_workflows = super::n8n::load_workflow_registry_all_workflows().unwrap_or_default();
    let state_cell = local_api_app_state(state).ok()?;
    let app_state = state_cell.get()?;
    let config = app_state.config.read().await.n8n.clone();
    let api_key = config.resolve_api_key();
    if api_key.trim().is_empty() {
        return None;
    }
    let url = format!(
        "{}/api/v1/workflows?limit=250",
        config.base_url.trim_end_matches('/')
    );
    let response = reqwest::Client::new()
        .get(url)
        .header("X-N8N-API-KEY", api_key)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let payload = response.json::<serde_json::Value>().await.ok()?;
    let rows = if let Some(rows) = payload.get("data").and_then(serde_json::Value::as_array) {
        rows.clone()
    } else {
        payload
            .get("data")
            .and_then(|value| value.get("data"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    for row in rows {
        let id = row
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim();
        let name = row
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim();
        if id.is_empty() || name.is_empty() {
            continue;
        }
        let normalized_id = local_api_normalized_reference(id);
        let normalized_name = local_api_normalized_reference(name);
        let exact_match = needle == normalized_id
            || needle == normalized_name
            || (normalized_name.len() >= 8 && needle.contains(&normalized_name));
        if !exact_match {
            continue;
        }
        let registered_in_catalog = catalog_workflows.iter().any(|workflow| {
            workflow.n8n_workflow_id == id
                || local_api_normalized_reference(&workflow.workflow_id) == normalized_id
                || local_api_normalized_reference(&workflow.display_name) == normalized_name
        });
        let registered_in_registry = registry_workflows.iter().any(|workflow| {
            workflow.n8n_workflow_id == id
                || local_api_normalized_reference(&workflow.workflow_id) == normalized_id
                || local_api_normalized_reference(&workflow.display_name) == normalized_name
        });
        if !registered_in_catalog && !registered_in_registry {
            return Some((id.to_string(), name.to_string()));
        }
    }
    None
}

fn local_api_n8n_import_required_json(
    request: &LocalApiChatRequest,
    source: String,
    session_id: String,
    n8n_workflow_id: String,
    n8n_workflow_name: String,
) -> serde_json::Value {
    let reply = format!(
        "Workflow \"{n8n_workflow_name}\" exists in n8n but is not registered in KRIA. Import or sync it into KRIA before updating, archiving, restoring, or running it."
    );
    serde_json::json!({
        "status": "import_required",
        "message": request.message,
        "source": source,
        "chat_id": request.chat_id,
        "from_user": request.from_user,
        "session_id": session_id,
        "reply": reply,
        "n8n": {
            "action": "import_required",
            "routing_status": "import_required",
            "workflow_id": serde_json::Value::Null,
            "n8n_workflow_id": n8n_workflow_id,
            "blockers": ["Workflow exists in n8n but is not registered in KRIA."],
            "next_actions": ["Import or sync workflow into KRIA", "Review workflow before CRUD actions"],
            "result": {
                "n8n_workflow_name": n8n_workflow_name,
            },
            "card": local_api_n8n_card(
                "import_required",
                "Import workflow into KRIA",
                "This n8n workflow must be registered before KRIA can manage it safely.",
                "Import workflow",
                vec!["Open n8n"]
            )
        }
    })
}

async fn local_api_n8n_route_prompt(
    AxumState(state): AxumState<LocalApiBridgeState>,
    Json(request): Json<LocalApiN8nRoutePromptRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let chat_request = LocalApiChatRequest {
        message: request.prompt,
        session_id: None,
        source: Some("n8n_prompt_route_eval".into()),
        chat_id: None,
        from_user: Some("prompt-eval".into()),
    };
    let Some(route) = local_api_n8n_route_decision(
        &state,
        &chat_request,
        request.previous_user_prompt,
        request.manual_n8n_mode,
        request.safe_auto_run_enabled,
    ) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "message": "n8n integration is not enabled in KRIA",
                "reply": "n8n integration is not enabled in KRIA",
            })),
        );
    };
    (
        StatusCode::OK,
        Json(serde_json::to_value(route).unwrap_or_else(|_| {
            serde_json::json!({
                "status": "error",
                "message": "failed to serialize n8n route decision",
            })
        })),
    )
}

async fn local_api_n8n_create_authoring_draft(
    AxumState(state): AxumState<LocalApiBridgeState>,
    Json(request): Json<super::n8n::CreateN8nWorkflowDraftInN8nRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let state_cell = match local_api_app_state(&state) {
        Ok(state_cell) => state_cell,
        Err(error) => return local_api_command_json(Err(error)),
    };
    local_api_command_json(super::n8n::create_n8n_workflow_draft_in_n8n(request, state_cell).await)
}

async fn local_api_n8n_create_updated_copy(
    AxumState(state): AxumState<LocalApiBridgeState>,
    Json(request): Json<super::n8n::CreateN8nWorkflowUpdatedCopyRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let state_cell = match local_api_app_state(&state) {
        Ok(state_cell) => state_cell,
        Err(error) => return local_api_command_json(Err(error)),
    };
    local_api_command_json(super::n8n::create_n8n_workflow_updated_copy(request, state_cell).await)
}

async fn local_api_n8n_archive_workflow(
    AxumState(state): AxumState<LocalApiBridgeState>,
    Json(request): Json<super::n8n::ArchiveN8nWorkflowRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let state_cell = match local_api_app_state(&state) {
        Ok(state_cell) => state_cell,
        Err(error) => return local_api_command_json(Err(error)),
    };
    local_api_command_json(super::n8n::archive_n8n_workflow(request, state_cell).await)
}

async fn local_api_n8n_restore_workflow(
    AxumState(state): AxumState<LocalApiBridgeState>,
    Json(request): Json<super::n8n::RestoreN8nWorkflowRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let state_cell = match local_api_app_state(&state) {
        Ok(state_cell) => state_cell,
        Err(error) => return local_api_command_json(Err(error)),
    };
    local_api_command_json(super::n8n::restore_n8n_workflow(request, state_cell).await)
}

async fn local_api_n8n_list_archived_workflows() -> (StatusCode, Json<serde_json::Value>) {
    local_api_command_json(super::n8n::list_archived_n8n_workflows().await)
}

async fn local_api_n8n_test_authoring_draft(
    AxumState(state): AxumState<LocalApiBridgeState>,
    Json(request): Json<super::n8n::TestN8nWorkflowDraftRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let state_cell = match local_api_app_state(&state) {
        Ok(state_cell) => state_cell,
        Err(error) => return local_api_command_json(Err(error)),
    };
    let app = match state.app_handle.clone() {
        Some(app) => app,
        None => {
            return local_api_command_json(Err(
                "KRIA app runtime is not attached to the local API bridge".into(),
            ))
        }
    };
    local_api_command_json(super::n8n::test_n8n_workflow_draft(request, state_cell, app).await)
}

async fn local_api_n8n_approve_authoring_draft(
    AxumState(state): AxumState<LocalApiBridgeState>,
    Json(request): Json<super::n8n::ApproveN8nWorkflowDraftRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let state_cell = match local_api_app_state(&state) {
        Ok(state_cell) => state_cell,
        Err(error) => return local_api_command_json(Err(error)),
    };
    local_api_command_json(super::n8n::approve_n8n_workflow_draft(request, state_cell).await)
}

async fn local_api_n8n_cleanup_authoring_draft(
    AxumState(state): AxumState<LocalApiBridgeState>,
    Json(request): Json<super::n8n::CleanupN8nWorkflowDraftRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let state_cell = match local_api_app_state(&state) {
        Ok(state_cell) => state_cell,
        Err(error) => return local_api_command_json(Err(error)),
    };
    local_api_command_json(super::n8n::cleanup_n8n_workflow_draft(request, state_cell).await)
}

async fn local_api_n8n_prompt_action_response(
    state: &LocalApiBridgeState,
    request: &LocalApiChatRequest,
) -> Option<(StatusCode, Json<serde_json::Value>)> {
    let session_id = local_api_session_id(request);
    let source = request.source.clone().unwrap_or_else(|| "api".to_string());

    if let Some(reference) = parse_local_api_n8n_archive_reference(&request.message) {
        if let Some((n8n_workflow_id, n8n_workflow_name)) =
            local_api_find_unregistered_n8n_target(state, &reference).await
        {
            return Some((
                StatusCode::OK,
                Json(local_api_n8n_import_required_json(
                    request,
                    source.clone(),
                    session_id.clone(),
                    n8n_workflow_id,
                    n8n_workflow_name,
                )),
            ));
        }
        let workflow_id = resolve_local_api_n8n_action_reference(state, &reference).await?;
        let state_cell = match local_api_app_state(state) {
            Ok(state_cell) => state_cell,
            Err(error) => return Some(local_api_command_json(Err(error))),
        };
        let result = super::n8n::archive_n8n_workflow(
            super::n8n::ArchiveN8nWorkflowRequest {
                workflow_id,
                reason: Some("archived from KRIA chat prompt".into()),
                requested_by: request.from_user.clone().or_else(|| Some("local_api_chat".into())),
            },
            state_cell,
        )
        .await
        .map(|value| {
            serde_json::json!({
                "status": value.get("status").and_then(serde_json::Value::as_str).unwrap_or("archived"),
                "message": request.message,
                "source": source,
                "chat_id": request.chat_id,
                "from_user": request.from_user,
                "session_id": session_id,
                "reply": value.get("message").and_then(serde_json::Value::as_str).unwrap_or("Workflow archived in KRIA."),
                "n8n": {
                    "action": "archive_workflow",
                    "routing_status": "archived",
                    "result": value,
                    "card": local_api_n8n_card(
                        "archive_workflow",
                        "Workflow archived",
                        "KRIA will hide this workflow from routing. The n8n workflow remains unchanged.",
                        "Restore workflow",
                        vec!["Open n8n"]
                    )
                }
            })
        });
        return Some(local_api_command_json(result));
    }

    if let Some(reference) = parse_local_api_n8n_restore_reference(&request.message) {
        if let Some((n8n_workflow_id, n8n_workflow_name)) =
            local_api_find_unregistered_n8n_target(state, &reference).await
        {
            return Some((
                StatusCode::OK,
                Json(local_api_n8n_import_required_json(
                    request,
                    source.clone(),
                    session_id.clone(),
                    n8n_workflow_id,
                    n8n_workflow_name,
                )),
            ));
        }
        let workflow_id = resolve_local_api_n8n_action_reference(state, &reference).await?;
        let state_cell = match local_api_app_state(state) {
            Ok(state_cell) => state_cell,
            Err(error) => return Some(local_api_command_json(Err(error))),
        };
        let result = super::n8n::restore_n8n_workflow(
            super::n8n::RestoreN8nWorkflowRequest { workflow_id },
            state_cell,
        )
        .await
        .map(|value| {
            serde_json::json!({
                "status": value.get("status").and_then(serde_json::Value::as_str).unwrap_or("restored"),
                "message": request.message,
                "source": source,
                "chat_id": request.chat_id,
                "from_user": request.from_user,
                "session_id": session_id,
                "reply": value.get("message").and_then(serde_json::Value::as_str).unwrap_or("Workflow restored in KRIA."),
                "n8n": {
                    "action": "restore_workflow",
                    "routing_status": "restored",
                    "result": value,
                    "card": local_api_n8n_card(
                        "restore_workflow",
                        "Workflow restored",
                        "KRIA can review this workflow again before normal routing.",
                        "Review workflow",
                        vec!["Archive workflow"]
                    )
                }
            })
        });
        return Some(local_api_command_json(result));
    }

    if let Some(workflow_id) = parse_local_api_n8n_test_draft_reference(&request.message) {
        let state_cell = match local_api_app_state(state) {
            Ok(state_cell) => state_cell,
            Err(error) => return Some(local_api_command_json(Err(error))),
        };
        let app = match state.app_handle.clone() {
            Some(app) => app,
            None => {
                return Some(local_api_command_json(Err(
                    "KRIA app runtime is not attached to the local API bridge".into(),
                )))
            }
        };
        let result = super::n8n::test_n8n_workflow_draft(
            super::n8n::TestN8nWorkflowDraftRequest {
                workflow_id,
                input_payload: local_api_test_input_payload_from_prompt(&request.message),
                confirmed: true,
            },
            state_cell,
            app,
        )
        .await
        .map(|value| {
            serde_json::json!({
                "status": value.get("status").and_then(serde_json::Value::as_str).unwrap_or("test_started"),
                "message": request.message,
                "source": source,
                "chat_id": request.chat_id,
                "from_user": request.from_user,
                "session_id": session_id,
                "reply": value.get("message").and_then(serde_json::Value::as_str).unwrap_or("Draft test started."),
                "n8n": { "action": "test_authoring_draft", "result": value }
            })
        });
        return Some(local_api_command_json(result));
    }

    if let Some(workflow_id) = parse_local_api_n8n_approve_draft_reference(&request.message) {
        let state_cell = match local_api_app_state(state) {
            Ok(state_cell) => state_cell,
            Err(error) => return Some(local_api_command_json(Err(error))),
        };
        let result = super::n8n::approve_n8n_workflow_draft(
            super::n8n::ApproveN8nWorkflowDraftRequest {
                workflow_id,
                confirmed: true,
            },
            state_cell,
        )
        .await
        .map(|value| {
            serde_json::json!({
                "status": value.get("status").and_then(serde_json::Value::as_str).unwrap_or("approved"),
                "message": request.message,
                "source": source,
                "chat_id": request.chat_id,
                "from_user": request.from_user,
                "session_id": session_id,
                "reply": value.get("message").and_then(serde_json::Value::as_str).unwrap_or("Draft approved."),
                "n8n": { "action": "approve_authoring_draft", "result": value }
            })
        });
        return Some(local_api_command_json(result));
    }

    if let Some(workflow_id) = parse_local_api_n8n_cleanup_draft_reference(&request.message) {
        let state_cell = match local_api_app_state(state) {
            Ok(state_cell) => state_cell,
            Err(error) => return Some(local_api_command_json(Err(error))),
        };
        let delete_n8n_draft = request.message.to_ascii_lowercase().contains("delete n8n");
        let result = super::n8n::cleanup_n8n_workflow_draft(
            super::n8n::CleanupN8nWorkflowDraftRequest {
                workflow_id,
                delete_n8n_draft,
            },
            state_cell,
        )
        .await
        .map(|value| {
            serde_json::json!({
                "status": value.get("status").and_then(serde_json::Value::as_str).unwrap_or("cleaned_up"),
                "message": request.message,
                "source": source,
                "chat_id": request.chat_id,
                "from_user": request.from_user,
                "session_id": session_id,
                "reply": value.get("message").and_then(serde_json::Value::as_str).unwrap_or("Draft cleanup completed."),
                "n8n": { "action": "cleanup_authoring_draft", "result": value }
            })
        });
        return Some(local_api_command_json(result));
    }

    if local_api_prompt_has_n8n_targeting_intent(&request.message) {
        if let Some((n8n_workflow_id, n8n_workflow_name)) =
            local_api_find_unregistered_n8n_target(state, &request.message).await
        {
            return Some((
                StatusCode::OK,
                Json(local_api_n8n_import_required_json(
                    request,
                    source.clone(),
                    session_id.clone(),
                    n8n_workflow_id,
                    n8n_workflow_name,
                )),
            ));
        }
    }

    let route = local_api_n8n_route_decision(state, request, None, true, false)?;
    match route.status {
        kria_core::n8n::N8nChatRouteStatus::CreateWorkflow
        | kria_core::n8n::N8nChatRouteStatus::CreateFromTemplate => {
            let requested_display_name = local_api_route_requested_display_name(&route);
            let state_cell = match local_api_app_state(state) {
                Ok(state_cell) => state_cell,
                Err(error) => return Some(local_api_command_json(Err(error))),
            };
            let result = super::n8n::create_n8n_workflow_draft_in_n8n(
                super::n8n::CreateN8nWorkflowDraftInN8nRequest {
                    prompt: request.message.clone(),
                    workflow_id: None,
                    display_name: requested_display_name,
                    template_id: None,
                },
                state_cell,
            )
            .await
            .map(|value| {
                serde_json::json!({
                    "status": value.get("status").and_then(serde_json::Value::as_str).unwrap_or("draft_created"),
                    "message": request.message,
                    "source": source,
                    "chat_id": request.chat_id,
                    "from_user": request.from_user,
                    "session_id": session_id,
                    "reply": value.get("message").and_then(serde_json::Value::as_str).unwrap_or("Inactive n8n draft created."),
                    "n8n": {
                        "routing": route,
                        "routing_status": "create_workflow",
                        "action": "create_authoring_draft",
                        "result": value,
                        "card": local_api_n8n_card(
                            "create_authoring_draft",
                            "Inactive n8n draft created",
                            "Review, test, and approve this draft before normal routing.",
                            "Review draft",
                            vec!["Test draft", "Cleanup draft"]
                        )
                    }
                })
            });
            Some(local_api_command_json(result))
        }
        kria_core::n8n::N8nChatRouteStatus::UpdateWorkflow => {
            let source_workflow_id = match route
                .selected_workflow
                .as_ref()
                .map(|candidate| candidate.workflow_id.clone())
            {
                Some(workflow_id) => workflow_id,
                None => {
                    return Some((
                        StatusCode::OK,
                        Json(local_api_n8n_suggestion_json(
                            request,
                            source,
                            session_id,
                            route.to_workflow_suggestion_response(),
                        )),
                    ))
                }
            };
            let state_cell = match local_api_app_state(state) {
                Ok(state_cell) => state_cell,
                Err(error) => return Some(local_api_command_json(Err(error))),
            };
            let result = super::n8n::create_n8n_workflow_updated_copy(
                super::n8n::CreateN8nWorkflowUpdatedCopyRequest {
                    source_workflow_id,
                    prompt: request.message.clone(),
                    display_name: None,
                },
                state_cell,
            )
            .await
            .map(|value| {
                serde_json::json!({
                    "status": value.get("status").and_then(serde_json::Value::as_str).unwrap_or("updated_copy_created"),
                    "message": request.message,
                    "source": source,
                    "chat_id": request.chat_id,
                    "from_user": request.from_user,
                    "session_id": session_id,
                    "reply": value.get("message").and_then(serde_json::Value::as_str).unwrap_or("Updated inactive n8n draft copy created."),
                    "n8n": {
                        "routing": route,
                        "routing_status": "update_workflow",
                        "action": "create_updated_copy",
                        "result": value,
                        "card": local_api_n8n_card(
                            "create_updated_copy",
                            "Updated draft copy created",
                            "Original workflow remains unchanged until the copy is reviewed.",
                            "Review updated copy",
                            vec!["Test copy", "Cleanup draft"]
                        )
                    }
                })
            });
            Some(local_api_command_json(result))
        }
        kria_core::n8n::N8nChatRouteStatus::OfferArchive
        | kria_core::n8n::N8nChatRouteStatus::DangerDeleteRequested
        | kria_core::n8n::N8nChatRouteStatus::Blocked => Some((
            StatusCode::OK,
            Json(local_api_n8n_suggestion_json(
                request,
                source,
                session_id,
                route.to_workflow_suggestion_response(),
            )),
        )),
        _ => None,
    }
}

async fn local_api_n8n_info_response(
    state: &LocalApiBridgeState,
    request: &LocalApiChatRequest,
) -> Option<serde_json::Value> {
    let lower = request.message.trim().to_ascii_lowercase();
    let session_id = local_api_session_id(request);
    let source = request.source.clone().unwrap_or_else(|| "api".to_string());

    if is_local_api_n8n_workflow_list_query(&lower) {
        let catalog = state.n8n_catalog.read().await.clone()?;
        let workflows = catalog.workflows();
        let reply = kria_core::n8n::n8n_workflow_inventory_notice(&workflows);
        let total = workflows.len();
        let runnable = workflows
            .iter()
            .filter(|workflow| workflow.is_approved_for_execution())
            .count();
        let workflow_preview = workflows
            .iter()
            .take(12)
            .map(|workflow| {
                serde_json::json!({
                    "display_name": &workflow.display_name,
                    "workflow_id": &workflow.workflow_id,
                    "status": format!("{:?}", workflow.status),
                    "runnable": workflow.is_approved_for_execution(),
                })
            })
            .collect::<Vec<_>>();
        let preview_truncated = total > workflow_preview.len();

        return Some(serde_json::json!({
            "status": "ok",
            "message": request.message,
            "source": source,
            "chat_id": request.chat_id,
            "from_user": request.from_user,
            "session_id": session_id,
            "reply": reply,
            "n8n": {
                "summary": {
                    "total": total,
                    "runnable": runnable,
                    "not_runnable": total.saturating_sub(runnable),
                },
                "workflow_preview": workflow_preview,
                "preview_truncated": preview_truncated,
            },
        }));
    }

    if let Some(workflow_id) = lower.strip_prefix("n8n approve ").map(str::trim) {
        let catalog = state.n8n_catalog.read().await.clone()?;
        let reply = match catalog.get(workflow_id) {
            Some(workflow) if workflow.is_approved_for_execution() => {
                format!("n8n workflow '{workflow_id}' is already approved.")
            }
            Some(workflow) => format!(
                "n8n workflow '{}' is currently {:?}; approval must go through the settings workflow.",
                workflow.workflow_id, workflow.status
            ),
            None => format!("unknown n8n workflow '{workflow_id}'"),
        };

        return Some(serde_json::json!({
            "status": "ok",
            "message": request.message,
            "source": source,
            "chat_id": request.chat_id,
            "from_user": request.from_user,
            "session_id": session_id,
            "reply": reply,
        }));
    }

    if lower == "n8n executions" || lower.contains("n8n execution history") {
        let runs = state.n8n_state_store.runs();
        let reply = format!(
            "n8n execution history: {} KRIA-tracked run state record(s).",
            runs.len()
        );
        return Some(serde_json::json!({
            "status": "ok",
            "message": request.message,
            "source": source,
            "chat_id": request.chat_id,
            "from_user": request.from_user,
            "session_id": session_id,
            "reply": reply,
            "n8n": {
                "source": "kria_state_store",
                "runs": runs,
            },
        }));
    }

    None
}

fn is_local_api_n8n_workflow_list_query(lower: &str) -> bool {
    kria_core::n8n::is_n8n_workflow_inventory_query(lower)
}

fn local_api_session_id(request: &LocalApiChatRequest) -> String {
    request.session_id.clone().unwrap_or_else(|| {
        if request.chat_id.is_some() || request.source.as_deref() == Some("telegram") {
            format!("telegram_{}", request.chat_id.unwrap_or(0))
        } else {
            uuid::Uuid::new_v4().to_string()
        }
    })
}

fn parse_local_api_n8n_run_reference(message: &str) -> Option<String> {
    kria_core::n8n::parse_n8n_workflow_run_reference(message)
}

fn parse_local_api_n8n_confirmation_reference(message: &str) -> Option<String> {
    kria_core::n8n::WorkflowConfirmationFlow::parse_confirmation_reference(message)
}

fn n8n_suggestion_reply(response: &kria_core::n8n::WorkflowSuggestionResponse) -> String {
    if response.candidates.is_empty() {
        return response.message.clone();
    }

    let candidates = response
        .candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            format!(
                "{}. {} ({}) — {} confidence, matched: {}",
                index + 1,
                candidate.display_name,
                candidate.workflow_id,
                candidate.confidence_label,
                if candidate.matched_on.is_empty() {
                    "metadata".to_string()
                } else {
                    candidate.matched_on.join(", ")
                }
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let hint = response
        .confirmation_hint
        .as_deref()
        .unwrap_or("Confirm with: Confirm workflow <workflow_id>");
    format!("{} {candidates}. {hint}.", response.message)
}

fn local_api_n8n_suggestion_json(
    request: &LocalApiChatRequest,
    source: String,
    session_id: String,
    response: kria_core::n8n::WorkflowSuggestionResponse,
) -> serde_json::Value {
    let reply = n8n_suggestion_reply(&response);
    let status = response.status.clone();
    let routing_status = status.clone();
    let action = match status.as_str() {
        "offer_archive" => "offer_archive",
        "danger_delete_requested" => "danger_delete_requested",
        "blocked" => "blocked",
        "needs_clarification" => "ask_clarification",
        "not_found" => "not_found",
        _ => status.as_str(),
    };
    let card = match action {
        "offer_archive" => local_api_n8n_card(
            "offer_archive",
            "Archive instead of delete",
            "KRIA keeps the n8n workflow intact and hides it from routing.",
            "Archive workflow",
            vec!["Open Danger Zone"],
        ),
        "danger_delete_requested" => local_api_n8n_card(
            "danger_delete_requested",
            "Permanent delete requires confirmation",
            "KRIA will not permanently delete directly from chat.",
            "Open Danger Zone",
            vec!["Archive instead"],
        ),
        _ => local_api_n8n_card(
            action,
            "n8n action required",
            "Review the suggested n8n next step.",
            "Review workflow",
            vec![],
        ),
    };
    serde_json::json!({
        "status": status,
        "message": request.message,
        "source": source,
        "chat_id": request.chat_id,
        "from_user": request.from_user,
        "session_id": session_id,
        "reply": reply,
        "n8n": {
            "action": action,
            "routing_status": routing_status,
            "routing": response,
            "card": card,
        },
    })
}

async fn store_local_api_n8n_pending_suggestion(
    state: &LocalApiBridgeState,
    session_id: &str,
    prompt: &str,
    response: &kria_core::n8n::WorkflowSuggestionResponse,
) {
    let mut pending = state.n8n_pending_suggestions.write().await;
    let now = local_api_now_unix_ms();
    pending.retain(|_, item| now.saturating_sub(item.created_at_ms) <= 15 * 60 * 1000);
    pending.insert(
        session_id.to_string(),
        LocalApiN8nPendingSuggestion {
            prompt: prompt.to_string(),
            response: response.clone(),
            created_at_ms: now,
        },
    );
}

async fn local_api_n8n_confirmed_payload(
    state: &LocalApiBridgeState,
    session_id: &str,
    workflow: &kria_core::n8n::N8nWorkflowConfig,
    fallback_prompt: &str,
) -> serde_json::Value {
    let pending = state.n8n_pending_suggestions.read().await;
    let prompt = pending
        .get(session_id)
        .filter(|item| {
            item.response
                .candidates
                .iter()
                .any(|candidate| candidate.workflow_id == workflow.workflow_id)
        })
        .map(|item| item.prompt.as_str())
        .unwrap_or(fallback_prompt);
    kria_core::n8n::build_n8n_suggested_input_payload(workflow, prompt, true)
}

async fn suggest_local_api_n8n_workflow_reference(
    state: &LocalApiBridgeState,
    request: &LocalApiChatRequest,
    reference: &str,
) -> Result<serde_json::Value, String> {
    let catalog = state
        .n8n_catalog
        .read()
        .await
        .clone()
        .ok_or_else(|| "n8n integration is not enabled in KRIA".to_string())?;
    let workflows = catalog.workflows();
    let session_id = local_api_session_id(request);
    let source = request.source.clone().unwrap_or_else(|| "api".to_string());
    log_n8n_execution_step(
        &session_id,
        1,
        9,
        "Prompt Received",
        None,
        format!("prompt=\"{}\"", n8n_log_preview_text(&request.message, 180)),
        None,
    );
    let engine = kria_core::n8n::WorkflowRankingEngine::new(workflows);
    let route = engine.route_chat(kria_core::n8n::N8nChatRouteRequest {
        prompt: request.message.clone(),
        previous_user_prompt: None,
        manual_n8n_mode: false,
        safe_auto_run_enabled: false,
        workflows: Vec::new(),
    });
    let response = route.to_workflow_suggestion_response();
    let candidates = response
        .candidates
        .iter()
        .map(|candidate| candidate.workflow_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    log_n8n_execution_step(
        &session_id,
        2,
        9,
        "Workflow Routing",
        response
            .candidates
            .first()
            .map(|candidate| candidate.workflow_id.as_str()),
        format!(
            "reference=\"{}\", candidates={}, can_auto_run={}",
            n8n_log_preview_text(reference, 80),
            if candidates.is_empty() {
                "-"
            } else {
                &candidates
            },
            response.can_auto_run
        ),
        None,
    );
    store_local_api_n8n_pending_suggestion(&state, &session_id, &request.message, &response).await;
    Ok(local_api_n8n_suggestion_json(
        request, source, session_id, response,
    ))
}

async fn suggest_local_api_n8n_workflow_prompt(
    state: &LocalApiBridgeState,
    request: &LocalApiChatRequest,
) -> Option<serde_json::Value> {
    let catalog = state.n8n_catalog.read().await.clone()?;
    let workflows = catalog.workflows();
    let session_id = local_api_session_id(request);
    let source = request.source.clone().unwrap_or_else(|| "api".to_string());
    let engine = kria_core::n8n::WorkflowRankingEngine::new(workflows);
    let route = engine.route_chat(kria_core::n8n::N8nChatRouteRequest {
        prompt: request.message.clone(),
        previous_user_prompt: None,
        manual_n8n_mode: false,
        safe_auto_run_enabled: false,
        workflows: Vec::new(),
    });
    if route.candidates.is_empty()
        || matches!(
            route.status,
            kria_core::n8n::N8nChatRouteStatus::UseOtherTool
                | kria_core::n8n::N8nChatRouteStatus::ListWorkflows
        )
    {
        return None;
    }
    let response = route.to_workflow_suggestion_response();
    log_n8n_execution_step(
        &session_id,
        1,
        9,
        "Prompt Received",
        None,
        format!("prompt=\"{}\"", n8n_log_preview_text(&request.message, 180)),
        None,
    );
    let candidates = response
        .candidates
        .iter()
        .map(|candidate| candidate.workflow_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    log_n8n_execution_step(
        &session_id,
        2,
        9,
        "Workflow Routing",
        response
            .candidates
            .first()
            .map(|candidate| candidate.workflow_id.as_str()),
        format!("candidates={}, can_auto_run=false", candidates),
        None,
    );
    store_local_api_n8n_pending_suggestion(&state, &session_id, &request.message, &response).await;
    Some(local_api_n8n_suggestion_json(
        request, source, session_id, response,
    ))
}

async fn invoke_local_api_n8n_confirmed_workflow_reference(
    state: &LocalApiBridgeState,
    request: &LocalApiChatRequest,
    reference: &str,
) -> Result<serde_json::Value, String> {
    let catalog = state
        .n8n_catalog
        .read()
        .await
        .clone()
        .ok_or_else(|| "n8n integration is not enabled in KRIA".to_string())?;
    let workflows = catalog.workflows();
    let session_id = local_api_session_id(request);
    let source = request.source.clone().unwrap_or_else(|| "api".to_string());

    match kria_core::n8n::resolve_n8n_workflow_reference(&workflows, reference) {
        kria_core::n8n::N8nWorkflowReferenceMatch::Unique {
            workflow,
            matched_on,
        } => {
            log_n8n_execution_step(
                &session_id,
                3,
                9,
                "Confirmation Check",
                Some(&workflow.workflow_id),
                format!("result=approved, matched_on={:?}", matched_on),
                None,
            );
            let input_payload =
                local_api_n8n_confirmed_payload(state, &session_id, workflow, &request.message)
                    .await;
            invoke_local_api_n8n_workflow(
                state,
                request,
                &workflow.workflow_id,
                matched_on,
                input_payload,
            )
            .await
        }
        kria_core::n8n::N8nWorkflowReferenceMatch::Ambiguous { matches } => Ok(serde_json::json!({
            "status": "needs_clarification",
            "message": request.message,
            "source": source,
            "chat_id": request.chat_id,
            "from_user": request.from_user,
            "session_id": session_id,
            "reply": "That confirmation still matches more than one workflow. Confirm with an exact workflow ID.",
            "n8n": {
                "reference": reference,
                "matches": matches,
            },
        })),
        kria_core::n8n::N8nWorkflowReferenceMatch::NoMatch { available } => Ok(serde_json::json!({
            "status": "not_found",
            "message": request.message,
            "source": source,
            "chat_id": request.chat_id,
            "from_user": request.from_user,
            "session_id": session_id,
            "reply": format!("Workflow \"{}\" was not found. Confirm with an approved workflow ID.", reference),
            "n8n": {
                "reference": reference,
                "available_workflows": available,
            },
        })),
    }
}

async fn invoke_local_api_n8n_workflow(
    state: &LocalApiBridgeState,
    request: &LocalApiChatRequest,
    workflow_id: &str,
    matched_on: Vec<String>,
    input_payload: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let catalog = state
        .n8n_catalog
        .read()
        .await
        .clone()
        .ok_or_else(|| "n8n integration is not enabled in KRIA".to_string())?;
    let session_id = local_api_session_id(request);
    let source = request.source.clone().unwrap_or_else(|| "api".to_string());
    let requested_by = request
        .from_user
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("local_api")
        .to_string();

    state
        .n8n_state_store
        .register_session(&session_id, &session_id);

    let invocation_started = Instant::now();
    if let Some(app_handle) = state.app_handle.as_ref() {
        let _ = app_handle.emit(
            "n8n:workflow_invocation_started",
            serde_json::json!({
                "event_type": "n8n:workflow_invocation_started",
                "workflow_id": workflow_id,
                "correlation_id": session_id,
                "timestamp_ms": local_api_now_unix_ms(),
                "source": "local_api_chat",
            }),
        );
    }

    let runtime = super::n8n::N8nAdapterRuntime {
        catalog,
        catalog_slot: Some(state.n8n_catalog.clone()),
        n8n_state_store: state.n8n_state_store.clone(),
        n8n_inbox_path: state.n8n_inbox_path.clone(),
        n8n_audit_path: state.n8n_audit_path.clone(),
        n8n_governance_log: state.n8n_governance_log.clone(),
        app_handle: state.app_handle.clone(),
        fleet_control_runtime: Some(state.fleet_control_runtime.clone()),
    };
    let result = match super::n8n::run_n8n_workflow_adapter(
        runtime,
        super::n8n::RunN8nWorkflowAdapterRequest {
            workflow_id: workflow_id.to_string(),
            input_payload,
            correlation_id: Some(session_id.clone()),
            workflow_version: None,
            requested_by,
            source: "local_api_chat".into(),
            confirmed: true,
            session_id: Some(session_id.clone()),
            run_mode: String::new(),
        },
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            let error_text = error;
            let error_lower = error_text.to_ascii_lowercase();
            let friendly_error = if error_lower.contains("not registered for post")
                || error_lower.contains("make a get request")
                || (error_lower.contains("post")
                    && error_lower.contains("webhook")
                    && error_lower.contains("get request"))
            {
                "n8n webhook method mismatch. KRIA sends POST requests, but this n8n Webhook node is configured for GET. In n8n, set the Webhook node HTTP Method to POST, save/activate it, then retry.".to_string()
            } else if error_lower.contains("requested webhook")
                && error_lower.contains("not registered")
            {
                "n8n webhook is not active. Open the workflow in n8n, turn it Active, then retry from KRIA. Production webhook URLs only work for active n8n workflows.".to_string()
            } else if error_lower.contains("webhook") && error_lower.contains("not registered") {
                "n8n webhook is not active. Activate the workflow in n8n's editor, then retry from KRIA.".to_string()
            } else {
                error_text
            };
            log_n8n_execution_step(
                &session_id,
                4,
                9,
                "Webhook Invocation Failed",
                Some(workflow_id),
                format!("error={}", n8n_log_preview_text(&friendly_error, 220)),
                Some(invocation_started.elapsed().as_millis()),
            );
            if let Some(app_handle) = state.app_handle.as_ref() {
                let _ = app_handle.emit(
                    "n8n:workflow_invocation_failed",
                    serde_json::json!({
                        "event_type": "n8n:workflow_invocation_failed",
                        "workflow_id": workflow_id,
                        "correlation_id": session_id,
                        "timestamp_ms": local_api_now_unix_ms(),
                        "error_class": "invocation_failed",
                        "message": friendly_error.clone(),
                    }),
                );
            }
            return Err(format!("n8n workflow invocation failed: {friendly_error}"));
        }
    };

    tracing::info!(
        target: "n8n_local_api_chat",
        workflow_id = %result.get("workflow_id").and_then(|value| value.as_str()).unwrap_or(workflow_id),
        workflow_version = %result.get("workflow_version").and_then(|value| value.as_str()).unwrap_or("v1"),
        correlation_id = %result.get("correlation_id").and_then(|value| value.as_str()).unwrap_or(&session_id),
        matched_on = ?matched_on,
        status_code = result.get("status_code").and_then(|value| value.as_u64()).unwrap_or(0),
        "explicit local API chat prompt invoked n8n workflow"
    );

    tracing::info!(
        target: "n8n_execution_trace",
        correlation_id = %result.get("correlation_id").and_then(|value| value.as_str()).unwrap_or(&session_id),
        workflow_id = %result.get("workflow_id").and_then(|value| value.as_str()).unwrap_or(workflow_id),
        status_code = result.get("status_code").and_then(|value| value.as_u64()).unwrap_or(0),
        accepted = result.get("accepted").and_then(|value| value.as_bool()).unwrap_or(true),
        matched_on = ?matched_on,
        elapsed_ms = invocation_started.elapsed().as_millis(),
        "[N8N][{}] Webhook accepted by n8n",
        result.get("correlation_id").and_then(|value| value.as_str()).unwrap_or(&session_id)
    );

    if let Some(app_handle) = state.app_handle.as_ref() {
        let _ = app_handle.emit(
            "n8n:workflow_invocation_accepted",
            serde_json::json!({
                "event_type": "n8n:workflow_invocation_accepted",
                "workflow_id": result.get("workflow_id").cloned().unwrap_or_else(|| serde_json::json!(workflow_id)),
                "workflow_version": result.get("workflow_version").cloned().unwrap_or_else(|| serde_json::json!("v1")),
                "correlation_id": result.get("correlation_id").cloned().unwrap_or_else(|| serde_json::json!(session_id)),
                "timestamp_ms": local_api_now_unix_ms(),
                "status_code": result.get("status_code").cloned().unwrap_or_else(|| serde_json::json!(0)),
                "accepted": result.get("accepted").cloned().unwrap_or_else(|| serde_json::json!(true)),
                "phase": result.get("phase").cloned().unwrap_or_else(|| serde_json::json!("accepted")),
            }),
        );
    }

    let result_workflow_id = result
        .get("workflow_id")
        .and_then(|value| value.as_str())
        .unwrap_or(workflow_id)
        .to_string();
    let result_correlation_id = result
        .get("correlation_id")
        .and_then(|value| value.as_str())
        .unwrap_or(&session_id)
        .to_string();
    let reply = result
        .get("message")
        .and_then(|value| value.as_str())
        .unwrap_or("n8n workflow started. KRIA is tracking the result.")
        .to_string();

    Ok(serde_json::json!({
        "status": "accepted",
        "message": request.message,
        "source": source,
        "chat_id": request.chat_id,
        "from_user": request.from_user,
        "session_id": session_id,
        "reply": reply,
        "n8n": {
            "workflow_id": result_workflow_id,
            "workflow_version": result.get("workflow_version").cloned().unwrap_or_else(|| serde_json::json!("v1")),
            "correlation_id": result_correlation_id,
            "idempotency_key": result.get("idempotency_key").cloned().unwrap_or_default(),
            "matched_on": matched_on,
            "accepted": result.get("accepted").cloned().unwrap_or_else(|| serde_json::json!(true)),
            "status_code": result.get("status_code").cloned().unwrap_or_else(|| serde_json::json!(0)),
            "phase": result.get("phase").cloned().unwrap_or_else(|| serde_json::json!("accepted")),
            "response": result.get("response").cloned().unwrap_or_default(),
        },
    }))
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

    let callback_started = Instant::now();
    let envelope =
        kria_core::n8n::parse_and_verify_callback(&catalog, &body, signature).map_err(|error| {
            tracing::warn!(
                target: "n8n_execution_trace",
                body_bytes = body.len(),
                error = %error,
                "[N8N][-] Callback rejected before state machine"
            );
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "status": "error",
                    "message": error.to_string(),
                })),
            )
        })?;

    log_n8n_execution_step(
        &envelope.correlation_id,
        6,
        9,
        "Callback Received",
        Some(&envelope.workflow_id),
        format!(
            "status={:?}, event_id={}, sequence_number={}",
            envelope.status, envelope.event_id, envelope.sequence_number
        ),
        Some(callback_started.elapsed().as_millis()),
    );

    let decision = state.n8n_state_store.ingest(envelope.clone());
    tracing::info!(
        target: "n8n_callback_trace",
        correlation_id = %envelope.correlation_id,
        event_id = %envelope.event_id,
        workflow_id = %envelope.workflow_id,
        status = ?envelope.status,
        decision = ?decision,
        "HOP-1: State machine ingest complete"
    );
    let governance = state
        .n8n_state_store
        .get(&envelope.correlation_id)
        .map(|run| {
            let workflow = catalog.get(&run.workflow_id);
            kria_core::n8n::evaluate_run(workflow, &run)
        });
    tracing::info!(
        target: "n8n_callback_trace",
        correlation_id = %envelope.correlation_id,
        governance_status = ?governance.as_ref().map(|g| &g.verification_status),
        governance_action = ?governance.as_ref().map(|g| &g.continuation_action),
        "HOP-2: Governance evaluation complete"
    );
    log_n8n_execution_step(
        &envelope.correlation_id,
        7,
        9,
        "Governance",
        Some(&envelope.workflow_id),
        format!(
            "verification={:?}, action={:?}",
            governance.as_ref().map(|g| &g.verification_status),
            governance.as_ref().map(|g| &g.continuation_action)
        ),
        Some(callback_started.elapsed().as_millis()),
    );
    let record = kria_core::n8n::N8nInboxRecord {
        received_at_ms: local_api_now_unix_ms().max(0) as u128,
        decision: decision.clone(),
        envelope: envelope.clone(),
    };
    let inbox_written = match append_n8n_inbox_record(&state.n8n_inbox_path, &record).await {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(error = %error, "failed to persist n8n callback inbox record");
            false
        }
    };
    if let Some(governance) = governance.clone() {
        record_n8n_governance(&state, governance.clone()).await;
        maybe_start_n8n_hitl_bridge(&state, &envelope, &governance);
    }
    log_n8n_execution_step(
        &envelope.correlation_id,
        8,
        9,
        "Persistence",
        Some(&envelope.workflow_id),
        format!(
            "callback_inbox_written={}, governance_recorded={}",
            inbox_written,
            governance.is_some()
        ),
        Some(callback_started.elapsed().as_millis()),
    );

    let response = LocalApiN8nCallbackResponse {
        status: "received".into(),
        decision,
        governance: governance.clone(),
        correlation_id: envelope.correlation_id.clone(),
        event_id: envelope.event_id.clone(),
        workflow_id: envelope.workflow_id.clone(),
        run_status: envelope.status.clone(),
    };

    if let Some(app_handle) = state.app_handle.as_ref() {
        // If this is a terminal callback, emit a chat-visible notification
        if envelope.status.is_terminal() {
            let display_name = state
                .n8n_catalog
                .read()
                .await
                .as_ref()
                .and_then(|c| c.get(&envelope.workflow_id))
                .map(|w| w.display_name.clone())
                .unwrap_or_else(|| envelope.workflow_id.clone());

            // Look up originating session for targeted chat injection
            let session_id = state.n8n_state_store.get_session(&envelope.correlation_id);

            tracing::info!(
                target: "n8n_callback_trace",
                correlation_id = %envelope.correlation_id,
                display_name = %display_name,
                session_id = ?session_id,
                is_terminal = true,
                status = ?envelope.status,
                "HOP-3: Preparing n8n:chat_result event for frontend"
            );

            let chat_result = serde_json::json!({
                "type": "n8n_workflow_complete",
                "workflow_id": &envelope.workflow_id,
                "correlation_id": &envelope.correlation_id,
                "session_id": session_id,
                "status": format!("{:?}", &envelope.status),
                "success": matches!(envelope.status, kria_core::n8n::N8nRunStatus::Completed),
                "evidence": &envelope.evidence,
                "display_name": display_name,
                "governance": &governance,
            });

            tracing::info!(
                target: "n8n_callback_trace",
                correlation_id = %envelope.correlation_id,
                workflow_id = %envelope.workflow_id,
                status = ?envelope.status,
                has_evidence = !envelope.evidence.is_null(),
                governance_status = ?governance.as_ref().map(|g| &g.verification_status),
                governance_action = ?governance.as_ref().map(|g| &g.continuation_action),
                "HOP-4: Emitting n8n:chat_result Tauri event"
            );

            match app_handle.emit("n8n:chat_result", chat_result) {
                Ok(()) => tracing::info!(
                    target: "n8n_callback_trace",
                    correlation_id = %envelope.correlation_id,
                    "HOP-4: n8n:chat_result emitted successfully"
                ),
                Err(e) => tracing::error!(
                    target: "n8n_callback_trace",
                    correlation_id = %envelope.correlation_id,
                    error = %e,
                    "HOP-4: FAILED to emit n8n:chat_result"
                ),
            }
            log_n8n_execution_step(
                &envelope.correlation_id,
                9,
                9,
                "Response Delivery",
                Some(&envelope.workflow_id),
                "terminal chat_result emitted; callback event emission follows".to_string(),
                Some(callback_started.elapsed().as_millis()),
            );
        } else {
            tracing::info!(
                target: "n8n_callback_trace",
                correlation_id = %envelope.correlation_id,
                status = ?envelope.status,
                is_terminal = false,
                "HOP-3: Non-terminal callback — no chat_result event emitted"
            );
            log_n8n_execution_step(
                &envelope.correlation_id,
                9,
                9,
                "Response Delivery",
                Some(&envelope.workflow_id),
                "non-terminal callback accepted; callback event emission follows; chat_result deferred"
                    .to_string(),
                Some(callback_started.elapsed().as_millis()),
            );
        }

        let _ = app_handle.emit("n8n:callback", &response);
    } else {
        tracing::error!(
            target: "n8n_callback_trace",
            correlation_id = %envelope.correlation_id,
            "HOP-3: app_handle is None — cannot emit events to frontend!"
        );
        log_n8n_execution_step(
            &envelope.correlation_id,
            9,
            9,
            "Response Delivery Failed",
            Some(&envelope.workflow_id),
            "app_handle unavailable; frontend events not emitted".to_string(),
            Some(callback_started.elapsed().as_millis()),
        );
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

fn redact_n8n_evidence_shape(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let shape = match value {
                        serde_json::Value::Null => serde_json::json!({"type": "null"}),
                        serde_json::Value::Bool(_) => serde_json::json!({"type": "boolean"}),
                        serde_json::Value::Number(_) => serde_json::json!({"type": "number"}),
                        serde_json::Value::String(text) => {
                            serde_json::json!({"type": "string", "length": text.len(), "redacted": true})
                        }
                        serde_json::Value::Array(values) => {
                            serde_json::json!({"type": "array", "length": values.len(), "redacted": true})
                        }
                        serde_json::Value::Object(values) => {
                            serde_json::json!({"type": "object", "keys": values.keys().cloned().collect::<Vec<_>>(), "redacted": true})
                        }
                    };
                    (key.clone(), shape)
                })
                .collect(),
        ),
        serde_json::Value::Array(values) => {
            serde_json::json!({"type": "array", "length": values.len(), "redacted": true})
        }
        serde_json::Value::String(text) => {
            serde_json::json!({"type": "string", "length": text.len(), "redacted": true})
        }
        serde_json::Value::Number(_) => serde_json::json!({"type": "number"}),
        serde_json::Value::Bool(_) => serde_json::json!({"type": "boolean"}),
        serde_json::Value::Null => serde_json::json!({"type": "null"}),
    }
}

fn redacted_n8n_run_for_sse(run: &kria_core::n8n::N8nWorkflowRunState) -> serde_json::Value {
    serde_json::json!({
        "correlation_id": run.correlation_id,
        "workflow_id": run.workflow_id,
        "workflow_version": run.workflow_version,
        "n8n_run_id": run.n8n_run_id,
        "last_sequence_number": run.last_sequence_number,
        "status": run.status,
        "evidence_log": run.evidence_log.iter().map(redact_n8n_evidence_shape).collect::<Vec<_>>(),
        "side_effects_count": run.side_effects.len(),
        "terminal": run.terminal,
    })
}

fn redacted_n8n_runs_for_sse(
    runs: &[kria_core::n8n::N8nWorkflowRunState],
) -> Vec<serde_json::Value> {
    runs.iter().map(redacted_n8n_run_for_sse).collect()
}

/// SSE endpoint streaming n8n workflow events in real-time.
/// Clients connect and receive events as runs change state.
/// Replaces 5s Dashboard polling with event-driven push.
async fn local_api_n8n_events_sse(
    AxumState(state): AxumState<LocalApiBridgeState>,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    use async_stream::stream;

    let state_store = state.n8n_state_store.clone();
    let governance_log = state.n8n_governance_log.clone();

    let event_stream = stream! {
        // Initial snapshot
        let runs = state_store.runs();
        let gov = governance_log.read().await.clone();
        let snapshot = serde_json::json!({
            "type": "snapshot",
            "runs": redacted_n8n_runs_for_sse(&runs),
            "governance_log": gov,
            "dead_letters_count": state_store.dead_letters().len(),
            "redacted": true,
        });
        yield Ok(Event::default().event("snapshot").data(snapshot.to_string()));

        // Track last known state for delta detection
        let mut last_run_count = runs.len();
        let mut last_gov_count = gov.len();

        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;

            let current_runs = state_store.runs();
            let current_gov = governance_log.read().await.clone();

            // Emit new runs
            if current_runs.len() > last_run_count {
                for run in current_runs.iter().skip(last_run_count) {
                    let event_data = serde_json::json!({
                        "type": "run_update",
                        "run": redacted_n8n_run_for_sse(run),
                        "redacted": true,
                    });
                    yield Ok(Event::default().event("run_update").data(event_data.to_string()));
                }
            } else if current_runs.len() != last_run_count || current_runs.iter().any(|r| {
                // Check if any non-terminal run changed status
                !r.terminal
            }) {
                // Full refresh on structural change
                let refresh = serde_json::json!({
                    "type": "runs_refresh",
                    "runs": redacted_n8n_runs_for_sse(&current_runs),
                    "redacted": true,
                });
                yield Ok(Event::default().event("runs_refresh").data(refresh.to_string()));
            }

            // Emit new governance decisions
            if current_gov.len() > last_gov_count {
                for decision in current_gov.iter().skip(last_gov_count) {
                    let event_data = serde_json::json!({
                        "type": "governance",
                        "decision": decision,
                    });
                    yield Ok(Event::default().event("governance").data(event_data.to_string()));
                }
            }

            last_run_count = current_runs.len();
            last_gov_count = current_gov.len();
        }
    };

    Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
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

fn n8n_docker_callback_bind_addr(host: &str, port: u16) -> Option<String> {
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return None;
    }

    let bind_host = std::env::var("KRIA_N8N_CALLBACK_BIND_HOST")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "172.17.0.1".to_string());

    if bind_host.eq_ignore_ascii_case("disabled") || bind_host == "0" {
        return None;
    }

    Some(format!("{bind_host}:{port}"))
}

async fn start_n8n_docker_callback_bridge(
    bind_addr: String,
    state: LocalApiBridgeState,
) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .map_err(|error| {
            format!("failed to bind n8n Docker callback bridge {bind_addr}: {error}")
        })?;
    let router = Router::new()
        .route("/api/n8n/callback", post(local_api_n8n_callback))
        .layer(axum::extract::DefaultBodyLimit::max(128 * 1024))
        .with_state(state);

    tracing::info!(
        target: "n8n_callback_bridge",
        bind_addr = %bind_addr,
        "n8n Docker callback bridge listening"
    );

    axum::serve(listener, router)
        .await
        .map_err(|error| format!("n8n Docker callback bridge stopped: {error}"))
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
                // Ensure API token exists (generate if first run)
                let _ = super::api_auth::ensure_api_token();

                // Initialize HITL store for API delivery
                let hitl_store = super::api_hitl::HitlStore::new();
                let hitl_state = super::api_hitl::HitlApiState {
                    store: Arc::clone(&hitl_store),
                };

                // Background task: expire old HITL requests every 60s
                let expiry_store = Arc::clone(&hitl_store);
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                    loop {
                        interval.tick().await;
                        // Expire requests older than 5 minutes (300 seconds)
                        let _ = expiry_store.expire_old(300).await;
                    }
                });

                let hitl_router = Router::new()
                    .route(
                        "/api/hitl/pending",
                        get(super::api_hitl::list_pending_handler),
                    )
                    .route("/api/hitl/respond", post(super::api_hitl::respond_handler))
                    .route(
                        "/api/hitl/stream",
                        get(super::api_hitl::hitl_stream_handler),
                    )
                    .with_state(hitl_state);

                let bridge_state = LocalApiBridgeState {
                    responder,
                    fleet_control_runtime,
                    n8n_catalog,
                    n8n_state_store,
                    n8n_inbox_path,
                    n8n_audit_path,
                    n8n_governance_log,
                    n8n_hitl_responses,
                    n8n_pending_suggestions: Arc::new(RwLock::new(HashMap::new())),
                    hitl,
                    decision_store,
                    app_handle: Some(app_handle),
                };

                tracing::info!(
                    target: "local_api_bridge",
                    n8n_stage3_confirmation_routing = true,
                    n8n_schema_validation = true,
                    n8n_prompt_context_confirmation = true,
                    "KRIA local API feature contract loaded"
                );

                if bridge_state.n8n_catalog.read().await.is_some() {
                    if let Some(callback_bind_addr) = n8n_docker_callback_bind_addr(&host, port) {
                        let callback_state = bridge_state.clone();
                        tokio::spawn(async move {
                            if let Err(error) =
                                start_n8n_docker_callback_bridge(callback_bind_addr, callback_state)
                                    .await
                            {
                                tracing::warn!(
                                    target: "n8n_callback_bridge",
                                    error = %error,
                                    "n8n Docker callback bridge unavailable"
                                );
                            }
                        });
                    }
                }

                let router = Router::new()
                    .route("/api/health", get(local_api_health))
                    .route("/api/auth/token", get(super::api_auth::get_token_handler))
                    .route("/api/chat", post(local_api_chat))
                    .route(
                        "/api/testing/desktop-chat-command",
                        post(local_api_desktop_chat_command),
                    )
                    .route(
                        "/api/testing/gui-automation-status",
                        get(local_api_gui_automation_status),
                    )
                    .route("/api/n8n/route", post(local_api_n8n_route_prompt))
                    .route(
                        "/api/n8n/authoring/create-draft",
                        post(local_api_n8n_create_authoring_draft),
                    )
                    .route(
                        "/api/n8n/authoring/create-updated-copy",
                        post(local_api_n8n_create_updated_copy),
                    )
                    .route(
                        "/api/n8n/authoring/test-draft",
                        post(local_api_n8n_test_authoring_draft),
                    )
                    .route(
                        "/api/n8n/authoring/approve-draft",
                        post(local_api_n8n_approve_authoring_draft),
                    )
                    .route(
                        "/api/n8n/authoring/cleanup-draft",
                        post(local_api_n8n_cleanup_authoring_draft),
                    )
                    .route("/api/n8n/archive", post(local_api_n8n_archive_workflow))
                    .route("/api/n8n/restore", post(local_api_n8n_restore_workflow))
                    .route(
                        "/api/n8n/archived",
                        get(local_api_n8n_list_archived_workflows),
                    )
                    .route("/api/n8n/callback", post(local_api_n8n_callback))
                    .route("/api/n8n/hitl-response", get(local_api_n8n_hitl_response))
                    .route("/api/n8n/events", get(local_api_n8n_events_sse))
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
                    .layer(axum::extract::DefaultBodyLimit::max(128 * 1024)) // 128KB max body
                    .layer(axum::middleware::from_fn(super::api_auth::auth_middleware))
                    .layer(CorsLayer::permissive())
                    .with_state(bridge_state);

                // Merge HITL router (separate state) into main router
                let router = router.merge(hitl_router);

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
