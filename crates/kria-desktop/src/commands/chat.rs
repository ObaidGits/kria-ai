use super::*;

/// Parse "X of Y step(s)" patterns from completion messages.
/// Returns (completed, total). Defaults to (0, 1) if no match.
fn parse_partial_step_count(text: &str) -> (u32, u32) {
    if let Some(re) = regex::Regex::new(r"(\d+)\s+of\s+(\d+)\s+step").ok() {
        if let Some(cap) = re.captures(text) {
            let completed = cap
                .get(1)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            let total = cap
                .get(2)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            return (completed, total);
        }
    }
    (0, 1)
}

/// Classify a final response message into a typed verdict for the frontend.
/// This lets the frontend render typed badges (Complete, Partial, Failed, Blocked)
/// without parsing the response text — addresses P10-1 from GUI_VULS.md.
///
/// The frontend should prefer this typed verdict over string parsing.
fn classify_response_verdict(text: &str) -> &'static str {
    let lower = text.to_lowercase();

    // Failure signals (highest priority)
    if lower.contains("did not fully complete")
        || lower.contains("task failed")
        || lower.contains("workflow failed")
        || lower.contains("error:")
        || lower.contains("⚠")
    {
        // Distinguish blocked (HITL/login required) from failed
        if lower.contains("requires approval")
            || lower.contains("login required")
            || lower.contains("not installed")
            || lower.contains("hitl")
        {
            return "blocked";
        }
        // Partial completion (some steps succeeded)
        if lower.contains("verified") && lower.contains("of") && lower.contains("step") {
            return "partial";
        }
        return "failed";
    }

    // Structural completion (succeeded but visibility unverified)
    if lower.contains("structurally complete") || lower.contains("structural success") {
        return "structurally_complete";
    }

    // Already satisfied
    if lower.contains("already")
        && (lower.contains("done") || lower.contains("complete") || lower.contains("running"))
    {
        return "already_satisfied";
    }

    // Standard completion
    if lower.contains("task completed") || lower.contains("✓") || lower.contains("completed") {
        return "complete";
    }

    // Cancelled
    if lower.contains("cancel") || lower.contains("stopped") {
        return "cancelled";
    }

    // Default: assume conversation (no workflow verdict applies)
    "conversation"
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct DesktopChatCommandEvent {
    pub name: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct DesktopChatCommandCapture {
    pub status_code: u16,
    pub status: String,
    pub reply: String,
    pub response: serde_json::Value,
    pub events: Vec<DesktopChatCommandEvent>,
}

fn desktop_chat_event(
    name: impl Into<String>,
    payload: serde_json::Value,
) -> DesktopChatCommandEvent {
    DesktopChatCommandEvent {
        name: name.into(),
        payload,
    }
}

fn desktop_chat_stage_event(
    step: &str,
    message: &str,
    detail: Option<serde_json::Value>,
) -> DesktopChatCommandEvent {
    desktop_chat_event(
        "agent:stage",
        serde_json::json!({
            "step": step,
            "message": message,
            "detail": detail.unwrap_or(serde_json::Value::Null),
            "ts": Utc::now().to_rfc3339(),
        }),
    )
}

pub(super) async fn desktop_n8n_pre_fallback_command_capture(
    message: String,
    app_state: &AppState,
    app: AppHandle,
    session_id_override: Option<String>,
    event_scope_prefix: &str,
) -> Option<Result<DesktopChatCommandCapture, String>> {
    let session_id = match session_id_override.filter(|value| !value.trim().is_empty()) {
        Some(value) => value,
        None => app_state.current_session_id.read().await.clone(),
    };
    let request = super::local_api::LocalApiChatRequest {
        message: message.clone(),
        session_id: Some(session_id.clone()),
        source: Some("desktop_chat".into()),
        chat_id: None,
        from_user: Some("Desktop".into()),
    };
    let Some((status_code, Json(n8n_response))) =
        super::local_api::local_api_n8n_pre_fallback_response_from_app_state(
            app_state, app, request,
        )
        .await
    else {
        return None;
    };

    let reply = n8n_response
        .get("reply")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            n8n_response
                .get("message")
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or("n8n request handled by KRIA.")
        .to_string();
    let n8n_action = n8n_response
        .pointer("/n8n/action")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    let memory_writer: Arc<dyn MemoryManager> = app_state.memory_store.clone();
    let _ = memory_writer.store_turn(&memory_turn_write(
        session_id.clone(),
        message,
        String::new(),
        None,
        None,
        None,
    ));
    let _ = memory_writer.store_turn(&memory_turn_write(
        session_id,
        String::new(),
        reply.clone(),
        Some("n8n".into()),
        Some(
            serde_json::json!({
                "action": n8n_action.clone(),
            })
            .to_string(),
        ),
        None,
    ));

    let events = vec![
        desktop_chat_event(
            format!("{event_scope_prefix}:thinking"),
            serde_json::json!({"status": "processing"}),
        ),
        desktop_chat_stage_event(
            "n8n_prompt_handled",
            "n8n prompt handled by deterministic desktop route",
            Some(serde_json::json!({
                "status": status_code.as_u16(),
                "n8n_action": n8n_action,
            })),
        ),
        desktop_chat_event(
            format!("{event_scope_prefix}:token"),
            serde_json::json!({ "text": reply.clone() }),
        ),
        desktop_chat_event(
            format!("{event_scope_prefix}:tool_result"),
            serde_json::json!({
                "tool": "n8n",
                "result": n8n_response.clone(),
            }),
        ),
        desktop_chat_event(format!("{event_scope_prefix}:done"), serde_json::json!({})),
    ];

    let status = if status_code.is_success() {
        "processing"
    } else {
        "error"
    }
    .to_string();
    Some(Ok(DesktopChatCommandCapture {
        status_code: status_code.as_u16(),
        status,
        reply,
        response: n8n_response,
        events,
    }))
}

#[cfg(test)]
mod chat_verdict_tests {
    use super::*;

    #[test]
    fn classify_complete() {
        assert_eq!(
            classify_response_verdict("Task completed. KRIA verified 1 step."),
            "complete"
        );
        assert_eq!(
            classify_response_verdict("✓ browser at https://example.com"),
            "complete"
        );
    }

    #[test]
    fn classify_partial() {
        assert_eq!(
            classify_response_verdict(
                "⚠️ Task did not fully complete. KRIA verified 1 of 2 steps."
            ),
            "partial"
        );
    }

    #[test]
    fn classify_failed() {
        // "app not found" doesn't trigger blocked patterns
        assert_eq!(
            classify_response_verdict("⚠️ Workflow failed: timeout"),
            "failed"
        );
        // But "not installed" DOES trigger blocked
        assert_eq!(
            classify_response_verdict("⚠️ Error: app is not installed"),
            "blocked"
        );
    }

    #[test]
    fn classify_blocked_login() {
        assert_eq!(
            classify_response_verdict("⚠️ Error: HITL approval timed out"),
            "blocked"
        );
    }

    #[test]
    fn classify_conversation() {
        assert_eq!(
            classify_response_verdict("Hello! How can I help you today?"),
            "conversation"
        );
    }
}

async fn send_message_with_profile(
    message: String,
    execution_profile: TurnExecutionProfile,
    state: State<'_, AppStateCell>,
    app: AppHandle,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let event_scope_prefix = match execution_profile.mode {
        TurnExecutionMode::Assistant => "agent",
        TurnExecutionMode::PromptLab => "prompt_lab",
    };
    let ev_thinking = format!("{event_scope_prefix}:thinking");
    let ev_token = format!("{event_scope_prefix}:token");
    let ev_done = format!("{event_scope_prefix}:done");
    let ev_tool_call = format!("{event_scope_prefix}:tool_call");
    let ev_tool_result = format!("{event_scope_prefix}:tool_result");
    let ev_approval_required = format!("{event_scope_prefix}:approval_required");
    let ev_approval_result = format!("{event_scope_prefix}:approval_result");
    let ev_tool_choice_required = format!("{event_scope_prefix}:tool_choice_required");

    if let Some(capture) = desktop_n8n_pre_fallback_command_capture(
        message.clone(),
        state,
        app.clone(),
        None,
        event_scope_prefix,
    )
    .await
    {
        let capture = capture?;
        for event in &capture.events {
            let _ = app.emit(&event.name, event.payload.clone());
        }
        if capture.status_code < 400 {
            return Ok(serde_json::json!({
                "status": "processing",
            }));
        }
        let error = capture
            .response
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&capture.reply)
            .to_string();
        return Err(error);
    }

    enforce_colab_dispatch_requirements(state, &app).await?;

    touch_orchestrator_activity(&state.orchestrator_last_activity_at).await;
    let orchestrator_snapshot = state.orchestrator.read().await.clone();
    if orchestrator_snapshot.is_some() {
        emit_agent_stage(
            &app,
            "ensuring_local_runtime",
            "Ensuring local LLM runtime is ready",
            None,
        );
    }
    if let Err(e) =
        ensure_orchestrator_ready_for_turn(orchestrator_snapshot.as_ref(), "ui_turn").await
    {
        let user_visible_error = format!("⚠️ {e}");
        emit_agent_stage(
            &app,
            "failed",
            "Local runtime preflight failed",
            Some(serde_json::json!({ "error": e.clone() })),
        );
        // Also emit error as token so it shows in the chat UI
        let _ = app.emit(
            &ev_token,
            serde_json::json!({ "text": &user_visible_error }),
        );
        let _ = app.emit(&ev_done, serde_json::json!({}));
        return Err(e);
    }

    tracing::info!(chars = message.chars().count(), "user prompt received");
    if kria_core::infra::pipeline_trace::pipeline_debug_enabled() {
        tracing::debug!(
            target: "kria_pipeline",
            prompt = %kria_core::infra::pipeline_trace::sanitize_text_for_logs(&message, 320),
            "send_message prompt preview"
        );
    }

    emit_agent_stage(
        &app,
        "input_received",
        "Prompt received from UI",
        Some(serde_json::json!({
            "chars": message.chars().count(),
        })),
    );

    let _ = app.emit(&ev_thinking, serde_json::json!({"status": "processing"}));

    let agent_loop = state.agent_loop.clone();
    let memory_store = state.memory_store.clone();
    let tool_registry = state.tool_registry.clone();
    let event_bus = state.event_bus.clone();
    let config = state.config.read().await;
    let hw_tier = state.hardware_info.tier.as_str();

    emit_agent_stage(
        &app,
        "preparing_tool_context",
        "Collecting tool descriptions for this hardware tier",
        Some(serde_json::json!({ "hardware_tier": hw_tier })),
    );

    // Build the system prompt with tool descriptions and user context
    let tool_defs = tool_registry.list_for_tier(hw_tier);
    let tool_descriptions = build_tool_descriptions_for_prompt(&tool_defs);

    emit_agent_stage(
        &app,
        "tool_context_ready",
        "Tool descriptions prepared",
        Some(serde_json::json!({ "tool_count": tool_defs.len() })),
    );

    // Retrieve user context from memory
    let user_name = memory_store
        .get_preference("user_name")
        .unwrap_or(None)
        .unwrap_or_else(|| "User".to_string());
    let os_name = std::env::consts::OS;

    // Detect all available package managers and format as "primary (also: alt1, alt2)"
    let pm_string = {
        let pms = get_available_package_managers();
        match pms.as_slice() {
            [] => "unknown".to_string(),
            [only] => only.as_str().to_string(),
            [primary, rest @ ..] => {
                let alts: Vec<&str> = rest.iter().map(|p| p.as_str()).collect();
                format!("{} (also available: {})", primary.as_str(), alts.join(", "))
            }
        }
    };

    // Get recent memory facts for context injection
    emit_agent_stage(
        &app,
        "loading_memory_context",
        "Searching memory for relevant user facts",
        None,
    );

    let memory_context = match memory_store.search_facts(&message, 5) {
        Ok(facts) if !facts.is_empty() => {
            let fact_lines: Vec<String> = facts.iter().map(|f| format!("- {}", f.text)).collect();
            format!("Known facts about the user:\n{}", fact_lines.join("\n"))
        }
        _ => String::new(),
    };

    emit_agent_stage(
        &app,
        "memory_context_ready",
        "Memory context prepared",
        Some(serde_json::json!({
            "has_context": !memory_context.is_empty(),
        })),
    );

    let system_prompt = kria_core::agent::prompts::build_system_prompt(
        &tool_descriptions,
        &user_name,
        os_name,
        hw_tier,
        &pm_string,
        &memory_context,
    );

    emit_agent_stage(
        &app,
        "system_prompt_ready",
        "System prompt prepared and ready for LLM",
        Some(serde_json::json!({
            "prompt_chars": system_prompt.chars().count(),
        })),
    );

    drop(config);

    // Use the persistent session ID from AppState
    let session_id = state.current_session_id.read().await.clone();
    let memory_writer: Arc<dyn MemoryManager> = memory_store.clone();

    emit_agent_stage(
        &app,
        "building_message_history",
        "Building conversation history for LLM input",
        Some(serde_json::json!({
            "session_id": session_id.clone(),
        })),
    );

    // Build conversation messages (system + recent history + current message)
    let recent_turns = memory_store
        .get_recent_turns(&session_id, 5)
        .unwrap_or_default();

    let mut messages = Vec::with_capacity(recent_turns.len() + 2);
    messages.push(ChatMessage {
        role: "system".into(),
        content: system_prompt,
        name: None,
        images: None,
    });

    // Add recent conversation history (with compact shaping for 4K context servers)
    append_recent_turns_for_llm(&mut messages, &recent_turns);

    // Add current user message
    messages.push(ChatMessage {
        role: "user".into(),
        content: message.clone(),
        name: None,
        images: None,
    });

    // Persist user turn
    let _ = memory_writer.store_turn(&memory_turn_write(
        session_id.clone(),
        message.clone(),
        String::new(),
        None,
        None,
        None,
    ));

    emit_agent_stage(
        &app,
        "user_turn_saved",
        "User prompt stored in session memory",
        Some(serde_json::json!({
            "history_turns": recent_turns.len() + 1,
        })),
    );

    // Auto-title: if this is the first message in the session, generate a title
    {
        let title_key = format!("session_title:{}", session_id);
        if memory_store
            .get_preference(&title_key)
            .unwrap_or(None)
            .is_none()
        {
            let title = if message.len() > 50 {
                format!("{}...", &message[..50])
            } else {
                message.clone()
            };
            let _ = memory_writer.set_preference(&preference_record(title_key, title));
        }
    }

    // Publish event
    event_bus.publish(kria_core::infra::event_bus::KriaEvent::MessageReceived {
        session_id: session_id.clone(),
        content: message.clone(),
    });

    emit_agent_stage(
        &app,
        "dispatching_to_llm",
        "Dispatching prepared prompt to agent loop",
        None,
    );

    // Create event channel and run agent loop
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();
    state
        .orchestrator_active_turns
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let active_turns_for_tracking = state.orchestrator_active_turns.clone();
    let last_activity_for_tracking = state.orchestrator_last_activity_at.clone();

    let app_handle = app.clone();
    let session_id_clone = session_id.clone();
    let memory_store_clone = memory_store.clone();
    let memory_writer_clone = memory_writer.clone();
    let embeddings_clone = state.embeddings.clone();
    let vectors_clone = state.vectors.clone();
    let user_message_clone = message.clone();
    let orchestrator_for_recovery = state.orchestrator.read().await.clone();
    let ironclad_orchestrator_cell_for_stream = state.orchestrator.clone();
    let ironclad_reset_for_stream = state.ironclad_reset.clone();
    let ironclad_forensic_for_stream = state.ironclad_forensic_log.clone();
    let retry_agent = agent_loop.clone();
    let stale_guard_agent = agent_loop.clone();
    let retry_session_id = session_id.clone();
    let retry_execution_profile = execution_profile.clone();
    let retry_messages_seed = messages.clone();

    // Spawn agent loop in background
    let agent = agent_loop.clone();
    let sid = session_id.clone();
    let run_profile = execution_profile.clone();
    tauri::async_runtime::spawn(async move {
        agent
            .run_with_profile(&sid, &mut messages, event_tx, Some(run_profile))
            .await;
    });

    emit_agent_stage(
        &app,
        "agent_loop_started",
        "Agent loop started; waiting for streamed events",
        None,
    );

    // Spawn event consumer that forwards to frontend
    tauri::async_runtime::spawn(async move {
        let mut full_response = String::new();
        let mut saw_first_token = false;
        let mut successful_tool_count = 0usize;
        let mut last_successful_tool: Option<(String, serde_json::Value)> = None;
        let mut recovery_attempted = false;
        let mut active_rx = event_rx;
        let mut active_turn_id: Option<String> = None;
        let mut pending_tool_params: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();

        emit_agent_stage(
            &app_handle,
            "awaiting_llm_output",
            "Prompt sent to LLM; waiting for first response token",
            None,
        );

        loop {
            let event = match tokio::time::timeout(
                std::time::Duration::from_secs(AGENT_EVENT_IDLE_TIMEOUT_SECS),
                active_rx.recv(),
            )
            .await
            {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(_) => {
                    emit_agent_stage(
                        &app_handle,
                        "timed_out_waiting_for_llm",
                        "No agent events received within timeout window",
                        Some(serde_json::json!({
                            "timeout_secs": AGENT_EVENT_IDLE_TIMEOUT_SECS,
                        })),
                    );
                    full_response = AGENT_TIMEOUT_MESSAGE.to_string();
                    let _ = app_handle.emit(
                        &ev_token,
                        serde_json::json!({
                            "text": AGENT_TIMEOUT_MESSAGE,
                        }),
                    );
                    break;
                }
            };

            if let StreamEvent::TurnAccepted {
                session_id,
                turn_id,
            } = &event
            {
                if session_id == &session_id_clone {
                    active_turn_id = Some(turn_id.clone());
                }
                continue;
            }

            if let Some(turn_id) = active_turn_id.as_deref() {
                if !stale_guard_agent.is_turn_active(&session_id_clone, turn_id) {
                    // Always forward Done/Error events even for stale turns so the
                    // frontend receives the `agent:done` signal and clears isThinking.
                    // Other events (Token, ToolStart, etc.) are safe to drop.
                    match &event {
                        StreamEvent::Done(_) | StreamEvent::Error(_) => {
                            tracing::debug!(
                                session_id = %session_id_clone,
                                turn_id = %turn_id,
                                "Forwarding terminal stream event for stale turn"
                            );
                            // fall through to the match below
                        }
                        _ => {
                            tracing::debug!(
                                session_id = %session_id_clone,
                                turn_id = %turn_id,
                                "Dropping stale stream event in send_message consumer"
                            );
                            continue;
                        }
                    }
                }
            }

            match event {
                StreamEvent::TurnAccepted { .. } => {}
                StreamEvent::Token(text) => {
                    if !saw_first_token {
                        saw_first_token = true;
                        emit_agent_stage(
                            &app_handle,
                            "llm_streaming",
                            "LLM started streaming tokens",
                            None,
                        );
                    }
                    full_response.push_str(&text);
                    let _ = app_handle.emit(
                        &ev_token,
                        serde_json::json!({
                            "text": text,
                        }),
                    );
                }
                StreamEvent::ToolStart { name, params } => {
                    if kria_core::infra::pipeline_trace::pipeline_debug_enabled() {
                        tracing::debug!(
                            target: "kria_pipeline",
                            tool = %name,
                            params = ?kria_core::infra::pipeline_trace::sanitize_json_for_logs(&params, 280, 8),
                            "tool call event"
                        );
                    }
                    pending_tool_params.insert(name.clone(), params.clone());
                    emit_agent_stage(
                        &app_handle,
                        "tool_started",
                        "Tool execution started",
                        Some(serde_json::json!({
                            "tool": name.clone(),
                        })),
                    );
                    let _ = app_handle.emit(
                        &ev_tool_call,
                        serde_json::json!({
                            "name": name,
                            "params": params,
                        }),
                    );
                }
                StreamEvent::ToolEnd {
                    name,
                    result,
                    success,
                    conversational_summary,
                    execution_metadata,
                    human_readable,
                } => {
                    if success {
                        successful_tool_count = successful_tool_count.saturating_add(1);
                        last_successful_tool = Some((name.clone(), result.clone()));
                    }

                    if kria_core::infra::pipeline_trace::pipeline_debug_enabled() {
                        tracing::debug!(
                            target: "kria_pipeline",
                            tool = %name,
                            success,
                            result = ?kria_core::infra::pipeline_trace::sanitize_json_for_logs(&result, 280, 8),
                            conversational_summary = ?conversational_summary.as_ref().map(|s| kria_core::infra::pipeline_trace::sanitize_text_for_logs(s, 120)),
                            "tool result event"
                        );
                    }
                    emit_agent_stage(
                        &app_handle,
                        "tool_finished",
                        "Tool execution completed",
                        Some(serde_json::json!({
                            "tool": name.clone(),
                            "success": success,
                        })),
                    );
                    let args = pending_tool_params
                        .remove(&name)
                        .unwrap_or_else(|| serde_json::json!({}));
                    let mut payload = build_tool_result_event_payload(&name, &result, success);

                    // Add synthesized fields to payload
                    if let Some(summary) = conversational_summary {
                        if let Some(obj) = payload.as_object_mut() {
                            obj.insert(
                                "conversational_summary".to_string(),
                                serde_json::Value::String(summary),
                            );
                        }
                    }
                    if let Some(hr) = human_readable {
                        if let Some(obj) = payload.as_object_mut() {
                            obj.insert("human_readable".to_string(), serde_json::Value::String(hr));
                        }
                    }
                    if let Some(metadata) = execution_metadata {
                        if let Some(obj) = payload.as_object_mut() {
                            obj.insert("execution_metadata".to_string(), metadata);
                        }
                    }

                    let metadata = payload
                        .get("metadata")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let _ = app_handle.emit(&ev_tool_result, payload);

                    let persisted_payload = serde_json::json!({
                        "name": name,
                        "args": args,
                        "success": success,
                        "result": result,
                        "metadata": metadata,
                    });
                    let _ = memory_writer_clone.store_turn(&memory_turn_write(
                        session_id_clone.clone(),
                        String::new(),
                        summarize_tool_turn_for_history(
                            &name,
                            success,
                            &result,
                            persisted_payload
                                .get("metadata")
                                .unwrap_or(&serde_json::Value::Null),
                        ),
                        Some(name),
                        Some(persisted_payload.to_string()),
                        None,
                    ));
                }
                StreamEvent::ToolProgress {
                    call_id,
                    message,
                    percent,
                } => {
                    let _ = app_handle.emit(
                        "kria:tool-progress",
                        serde_json::json!({
                            "call_id": call_id,
                            "message": message,
                            "percent": percent,
                            "session_id": session_id_clone,
                        }),
                    );
                }
                StreamEvent::ToolPayloadChunk {
                    call_id,
                    seq,
                    is_final,
                    data,
                } => {
                    let _ = app_handle.emit(
                        "kria:tool-payload-chunk",
                        serde_json::json!({
                            "call_id": call_id,
                            "seq": seq,
                            "is_final": is_final,
                            "data": data,
                            "session_id": session_id_clone,
                        }),
                    );
                }
                StreamEvent::ApprovalRequired {
                    request_id,
                    action,
                    risk_level,
                    parameters,
                } => {
                    emit_agent_stage(
                        &app_handle,
                        "approval_required",
                        "Agent requested user approval",
                        Some(serde_json::json!({
                            "action": action.clone(),
                            "risk_level": risk_level.clone(),
                        })),
                    );
                    let _ = app_handle.emit(
                        &ev_approval_required,
                        serde_json::json!({
                            "requestId": request_id,
                            "toolName": action,
                            "riskLevel": risk_level,
                            "args": parameters,
                            "reason": "",
                        }),
                    );
                }
                StreamEvent::ApprovalResult { action, approved } => {
                    emit_agent_stage(
                        &app_handle,
                        "approval_result",
                        "User approval decision received",
                        Some(serde_json::json!({
                            "action": action.clone(),
                            "approved": approved,
                        })),
                    );
                    let _ = app_handle.emit(
                        &ev_approval_result,
                        serde_json::json!({
                            "action": action,
                            "approved": approved,
                        }),
                    );
                }
                StreamEvent::ToolChoiceRequired {
                    query,
                    confidence,
                    min_confidence,
                    candidates,
                } => {
                    emit_agent_stage(
                        &app_handle,
                        "tool_choice_required",
                        "Low-confidence routing requires user tool selection",
                        Some(serde_json::json!({
                            "confidence": confidence,
                            "min_confidence": min_confidence,
                            "candidate_count": candidates.len(),
                        })),
                    );
                    let list: Vec<serde_json::Value> = candidates
                        .into_iter()
                        .map(|c| {
                            serde_json::json!({
                                "name": c.name,
                                "label": c.label,
                                "reason": c.reason,
                                "confidence": c.confidence,
                            })
                        })
                        .collect();
                    let _ = app_handle.emit(
                        &ev_tool_choice_required,
                        serde_json::json!({
                            "query": query,
                            "confidence": confidence,
                            "minConfidence": min_confidence,
                            "candidates": list,
                        }),
                    );
                }
                StreamEvent::Plan(plan) => {
                    emit_agent_stage(
                        &app_handle,
                        "planning",
                        "Agent is updating execution plan",
                        Some(serde_json::json!({
                            "plan": plan.clone(),
                        })),
                    );
                    let _ = app_handle.emit(
                        &ev_thinking,
                        serde_json::json!({
                            "status": "planning",
                            "plan": plan,
                        }),
                    );
                }
                StreamEvent::RecoveryOptions {
                    context,
                    detail,
                    options,
                } => {
                    tracing::info!(
                        session_id = %session_id_clone,
                        context = %context,
                        options_count = options.len(),
                        "recovery_options: emitting to UI"
                    );
                    let _ = app_handle.emit(
                        &format!("{session_id_clone}:recovery_options"),
                        serde_json::json!({
                            "context": context,
                            "detail": detail,
                            "options": options.iter().map(|o| serde_json::json!({
                                "label": o.label,
                                "action_prompt": o.action_prompt,
                                "style": o.style,
                            })).collect::<Vec<_>>(),
                        }),
                    );
                }
                StreamEvent::TaskStep(step) => {
                    let _ = app_handle.emit(
                        &format!("{session_id_clone}:task_step"),
                        serde_json::json!({
                            "index": step.index,
                            "total": step.total,
                            "description": step.description,
                            "status": step.status,
                        }),
                    );
                    // ── Phase: Canonical Telemetry Bridge ──────────────────────────
                    // Emit structured workflow telemetry alongside legacy task_step.
                    // Frontend progressively migrates to consuming this instead.
                    let telemetry_event = match step.status {
                        kria_core::agent::loop_engine::TaskStepStatus::Running => {
                            Some(serde_json::json!({
                                "version": 1,
                                "seq": step.index,
                                "event": {
                                    "type": "step_started",
                                    "workflow_id": session_id_clone,
                                    "step_index": step.index,
                                    "description": step.description,
                                    "step_type": "command_execution"
                                },
                                "timestamp_ms": 0,
                                "source": "legacy_shim"
                            }))
                        }
                        kria_core::agent::loop_engine::TaskStepStatus::Done => {
                            Some(serde_json::json!({
                                "version": 1,
                                "seq": step.index,
                                "event": {
                                    "type": "step_completed",
                                    "workflow_id": session_id_clone,
                                    "step_index": step.index,
                                    "structural_success": true,
                                    "visibility_confidence": { "level": "not_applicable" },
                                    "artifacts": []
                                },
                                "timestamp_ms": 0,
                                "source": "legacy_shim"
                            }))
                        }
                        kria_core::agent::loop_engine::TaskStepStatus::Failed => {
                            Some(serde_json::json!({
                                "version": 1,
                                "seq": step.index,
                                "event": {
                                    "type": "step_completed",
                                    "workflow_id": session_id_clone,
                                    "step_index": step.index,
                                    "structural_success": false,
                                    "visibility_confidence": { "level": "not_applicable" },
                                    "artifacts": []
                                },
                                "timestamp_ms": 0,
                                "source": "legacy_shim"
                            }))
                        }
                        _ => None,
                    };
                    if let Some(telemetry) = telemetry_event {
                        let _ = app_handle.emit("workflow:telemetry", telemetry);
                    }
                }
                StreamEvent::Error(err) => {
                    tracing::error!("Agent error: {}", err);
                    let is_transport_failure = is_likely_local_llm_transport_error(&err);

                    append_ironclad_forensic_record(
                        &ironclad_forensic_for_stream,
                        &app_handle,
                        "agent_stream_error",
                        if is_transport_failure {
                            "warning"
                        } else {
                            "error"
                        },
                        if is_transport_failure {
                            "Agent stream transport failure detected"
                        } else {
                            "Agent stream error detected"
                        },
                        err.clone(),
                        "agent.stream",
                    )
                    .await;

                    let status_payload = collect_ironclad_status_from_parts(
                        &ironclad_orchestrator_cell_for_stream,
                        &ironclad_reset_for_stream,
                        &ironclad_forensic_for_stream,
                    )
                    .await;
                    let _ = app_handle.emit("ironclad:status", status_payload);

                    if is_transport_failure
                        && full_response.is_empty()
                        && successful_tool_count == 0
                        && !recovery_attempted
                    {
                        recovery_attempted = true;
                        emit_agent_stage(
                            &app_handle,
                            "llm_transport_error_recovery_started",
                            "LLM transport failed early; attempting orchestrator recovery and single retry",
                            Some(serde_json::json!({
                                "mode": match retry_execution_profile.mode {
                                    TurnExecutionMode::Assistant => "assistant",
                                    TurnExecutionMode::PromptLab => "prompt_lab",
                                },
                            })),
                        );

                        if let Some(orchestrator) = orchestrator_for_recovery.as_ref() {
                            let mut recovered = false;

                            if orchestrator.server_manager.is_swapping() {
                                emit_agent_stage(
                                    &app_handle,
                                    "llm_transport_error_waiting_for_swap",
                                    "LLM transport failed during active swap; waiting for runtime to become ready",
                                    None,
                                );
                                let _ = orchestrator
                                    .server_manager
                                    .wait_for_swap_done(std::time::Duration::from_secs(45))
                                    .await;
                                match ensure_orchestrator_ready_for_turn(
                                    Some(orchestrator),
                                    "transport_failure_wait_for_swap",
                                )
                                .await
                                {
                                    Ok(()) => {
                                        recovered = true;
                                    }
                                    Err(wait_err) => {
                                        tracing::warn!(
                                            error = %wait_err,
                                            "runtime still not ready after swap wait; escalating to restart"
                                        );
                                    }
                                }
                            }

                            if !recovered {
                                match orchestrator.restart("transport_failure").await {
                                    Ok(()) => {
                                        recovered = true;
                                    }
                                    Err(restart_err) => {
                                        tracing::error!(
                                            ?restart_err,
                                            "orchestrator restart failed after transport error"
                                        );
                                        emit_agent_stage(
                                            &app_handle,
                                            "llm_transport_error_recovery_failed",
                                            "Orchestrator recovery failed; falling back to error handling",
                                            Some(serde_json::json!({
                                                "error": restart_err.to_string(),
                                            })),
                                        );
                                    }
                                }
                            }

                            if recovered {
                                emit_agent_stage(
                                    &app_handle,
                                    "llm_transport_error_recovery_succeeded",
                                    "Orchestrator recovered; retrying this turn once",
                                    None,
                                );

                                let (retry_tx, retry_rx) =
                                    tokio::sync::mpsc::unbounded_channel::<StreamEvent>();
                                let mut retry_messages = retry_messages_seed.clone();
                                let retry_agent_clone = retry_agent.clone();
                                let retry_sid_clone = retry_session_id.clone();
                                let retry_profile_clone = retry_execution_profile.clone();

                                tauri::async_runtime::spawn(async move {
                                    retry_agent_clone
                                        .run_with_profile(
                                            &retry_sid_clone,
                                            &mut retry_messages,
                                            retry_tx,
                                            Some(retry_profile_clone),
                                        )
                                        .await;
                                });

                                active_rx = retry_rx;
                                continue;
                            }
                        } else {
                            emit_agent_stage(
                                &app_handle,
                                "llm_transport_error_recovery_unavailable",
                                "No orchestrator active; skipping auto-recovery",
                                None,
                            );
                        }
                    }

                    if is_transport_failure && full_response.is_empty() && successful_tool_count > 0
                    {
                        if let Some((tool_name, tool_result)) = last_successful_tool.as_ref() {
                            let fallback_text =
                                build_tool_only_fallback_message(tool_name, true, tool_result);
                            full_response = fallback_text.clone();
                            emit_agent_stage(
                                &app_handle,
                                "llm_transport_error_tool_fallback",
                                "LLM transport failed after tool success; returning tool-only fallback",
                                Some(serde_json::json!({
                                    "tool": tool_name,
                                    "successful_tool_count": successful_tool_count,
                                })),
                            );
                            let _ = app_handle.emit(
                                &ev_token,
                                serde_json::json!({
                                    "text": fallback_text,
                                }),
                            );
                            continue;
                        }
                    }

                    if is_transport_failure && !full_response.is_empty() {
                        emit_agent_stage(
                            &app_handle,
                            "llm_transport_error_after_partial_output",
                            "LLM transport failed after partial response; preserving generated content",
                            Some(serde_json::json!({
                                "response_chars": full_response.chars().count(),
                            })),
                        );
                        continue;
                    }

                    let user_visible_error = format!("⚠️ {err}");
                    if full_response.is_empty() {
                        full_response = user_visible_error.clone();
                    }
                    emit_agent_stage(
                        &app_handle,
                        "failed",
                        "Agent stream reported an error",
                        Some(serde_json::json!({
                            "error": err.clone(),
                        })),
                    );
                    let _ = app_handle.emit(
                        &ev_token,
                        serde_json::json!({
                            "text": user_visible_error,
                        }),
                    );
                }
                StreamEvent::Done(final_text) => {
                    if !final_text.is_empty() && full_response.is_empty() {
                        full_response = final_text.clone();
                        // Ensure terminal-only responses (Done without prior Token events)
                        // still render as an assistant message in the UI.
                        let _ = app_handle.emit(
                            &ev_token,
                            serde_json::json!({
                                "text": final_text,
                            }),
                        );
                    }

                    // ── P10-1 / P12-4: Emit structured verdict telemetry ─────
                    // Frontend can consume this typed envelope instead of parsing
                    // the natural-language `final_text` content.
                    let verdict_kind = classify_response_verdict(&final_text);

                    // Build the typed verdict object matching the frontend type
                    // discriminator pattern: { type: "<kind>", ...kind-specific-fields }
                    let verdict_obj = match verdict_kind {
                        "complete" => serde_json::json!({ "type": "complete" }),
                        "structurally_complete" => serde_json::json!({
                            "type": "structurally_complete",
                            "unverified_outcomes": Vec::<String>::new(),
                        }),
                        "already_satisfied" => serde_json::json!({
                            "type": "already_satisfied",
                            "evidence": final_text.chars().take(200).collect::<String>(),
                        }),
                        "partial" => {
                            // Try to extract "X of Y steps" from text
                            let (completed, total) = parse_partial_step_count(&final_text);
                            serde_json::json!({
                                "type": "partial",
                                "completed": completed,
                                "total": total,
                                "reason": final_text.chars().take(200).collect::<String>(),
                            })
                        }
                        "blocked" => serde_json::json!({
                            "type": "blocked",
                            "reason": final_text.chars().take(200).collect::<String>(),
                        }),
                        "failed" => serde_json::json!({
                            "type": "failed",
                            "step": 0,
                            "reason": final_text.chars().take(200).collect::<String>(),
                        }),
                        "cancelled" => serde_json::json!({
                            "type": "failed",
                            "step": 0,
                            "reason": "cancelled",
                        }),
                        _ => serde_json::json!({ "type": "complete" }), // Default for conversation
                    };

                    let _ = app_handle.emit(
                        "workflow:telemetry",
                        serde_json::json!({
                            "version": 1,
                            "seq": 0,
                            "event": {
                                "type": "completed",
                                "workflow_id": session_id_clone,
                                "verdict": verdict_obj,
                                "summary": final_text.chars().take(300).collect::<String>(),
                                "artifacts": [],
                                "continuation": []
                            },
                            "timestamp_ms": 0,
                            "source": "react_loop"
                        }),
                    );

                    emit_agent_stage(
                        &app_handle,
                        "llm_done",
                        "LLM stream completed",
                        Some(serde_json::json!({
                            "response_chars": full_response.chars().count(),
                            "verdict": verdict_kind,
                        })),
                    );
                }
            }
        }

        // Persist assistant response (skip transient runtime errors so they don't bloat future context)
        if !full_response.is_empty() && !is_transient_llm_error_text(&full_response) {
            let _ = memory_writer_clone.store_turn(&memory_turn_write(
                session_id_clone,
                String::new(),
                full_response.clone(),
                None,
                None,
                None,
            ));

            emit_agent_stage(
                &app_handle,
                "assistant_turn_saved",
                "Assistant response stored in session memory",
                Some(serde_json::json!({
                    "response_chars": full_response.chars().count(),
                })),
            );

            // Automatic fact extraction from user message + assistant response
            let fact_mgr = kria_core::memory::facts::FactManager::new(
                memory_store_clone.as_ref(),
                &vectors_clone,
                &embeddings_clone,
            );
            match fact_mgr.extract_from_turn(&user_message_clone, &full_response) {
                Ok(ids) if !ids.is_empty() => {
                    tracing::info!(count = ids.len(), "auto-extracted facts from conversation");
                    emit_agent_stage(
                        &app_handle,
                        "facts_extracted",
                        "New user facts extracted from the conversation",
                        Some(serde_json::json!({
                            "fact_count": ids.len(),
                        })),
                    );
                }
                Ok(_) => {}
                Err(e) => tracing::warn!("fact extraction failed: {}", e),
            }
        }

        emit_agent_stage(
            &app_handle,
            "completed",
            "Pipeline completed and UI will finalize rendering",
            None,
        );

        let _ = app_handle.emit(&ev_done, serde_json::json!({}));
        decrement_active_turn_counter(&active_turns_for_tracking);
        touch_orchestrator_activity(&last_activity_for_tracking).await;
    });

    Ok(serde_json::json!({
        "status": "processing",
    }))
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct LabExecutionProfileInput {
    pub app_lock: Option<String>,
    pub tool_lock: Option<String>,
    pub strategy: Option<String>,
}

impl LabExecutionProfileInput {
    fn tool_selection_strategy(&self) -> PromptLabToolSelectionStrategy {
        match self
            .strategy
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
        {
            Some(value)
                if value == "direct"
                    || value == "direct_locked_tool"
                    || value == "direct-locked-tool" =>
            {
                PromptLabToolSelectionStrategy::DirectLockedTool
            }
            _ => PromptLabToolSelectionStrategy::RoutedWithinLock,
        }
    }

    fn to_core_profile(&self) -> TurnExecutionProfile {
        TurnExecutionProfile::prompt_lab(
            self.app_lock.clone(),
            self.tool_lock.clone(),
            self.tool_selection_strategy(),
        )
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct ManualToolProfileInput {
    pub mode_id: String,
    pub label: Option<String>,
    pub app_lock: Option<String>,
    pub tool_lock: Option<String>,
    pub strategy: Option<String>,
}

impl ManualToolProfileInput {
    fn tool_selection_strategy(&self) -> PromptLabToolSelectionStrategy {
        match self
            .strategy
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
        {
            Some(value)
                if value == "direct"
                    || value == "direct_locked_tool"
                    || value == "direct-locked-tool" =>
            {
                PromptLabToolSelectionStrategy::DirectLockedTool
            }
            _ => PromptLabToolSelectionStrategy::RoutedWithinLock,
        }
    }

    fn to_core_profile(&self) -> TurnExecutionProfile {
        TurnExecutionProfile::manual_tool(
            self.app_lock.clone(),
            self.tool_lock.clone(),
            self.tool_selection_strategy(),
        )
    }

    fn selected_label(&self) -> String {
        self.label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.mode_id.trim())
            .to_string()
    }
}

#[tauri::command]
pub async fn send_message(
    message: String,
    state: State<'_, AppStateCell>,
    app: AppHandle,
) -> Result<serde_json::Value, String> {
    send_message_with_profile(message, TurnExecutionProfile::assistant(), state, app).await
}

#[tauri::command]
pub async fn send_manual_tool_message(
    message: String,
    profile: ManualToolProfileInput,
    state: State<'_, AppStateCell>,
    app: AppHandle,
) -> Result<serde_json::Value, String> {
    if profile.mode_id.trim().eq_ignore_ascii_case("auto")
        || (profile.app_lock.as_deref().unwrap_or("").trim().is_empty()
            && profile.tool_lock.as_deref().unwrap_or("").trim().is_empty())
    {
        return send_message_with_profile(message, TurnExecutionProfile::assistant(), state, app)
            .await;
    }

    let execution_profile = profile.to_core_profile();
    let execution_id = Uuid::new_v4().to_string();
    let prompt_preview = kria_core::infra::pipeline_trace::sanitize_text_for_logs(&message, 220);
    let selected_tool = profile.selected_label();
    let matched_tools = {
        let state_ref = state.get().ok_or_else(|| {
            "KRIA is still initializing — please try again in a moment".to_string()
        })?;
        let hw_tier = state_ref.hardware_info.tier.as_str();
        state_ref
            .tool_registry
            .list_for_tier(hw_tier)
            .into_iter()
            .filter(|tool| execution_profile.allows_tool_name(&tool.name))
            .map(|tool| tool.name)
            .collect::<Vec<_>>()
    };

    if matched_tools.is_empty() {
        return Err(format!(
            "Manual tool mode '{}' has no available tools for this runtime",
            selected_tool
        ));
    }

    tracing::info!(
        target: "tool_mode",
        mode = "manual",
        selected_tool = %selected_tool,
        app_lock = ?execution_profile.app_lock,
        tool_lock = ?execution_profile.tool_lock,
        routing = "manual_override",
        semantic_routing = "bypassed",
        execution_id = %execution_id,
        prompt_preview = %prompt_preview,
        matched_tool_count = matched_tools.len(),
        matched_tools = ?matched_tools,
        "[TOOL_MODE] manual tool selection activated"
    );

    let telemetry = serde_json::json!({
        "event": "ManualToolSelectionActivated",
        "selected_tool": selected_tool,
        "mode_id": profile.mode_id,
        "timestamp_ms": unix_now_ms(),
        "execution_id": execution_id,
        "prompt_preview": prompt_preview,
        "routing": "manual_override",
        "semantic_routing": "bypassed",
        "matched_tools": matched_tools,
    });
    let _ = app.emit("ManualToolSelectionActivated", telemetry.clone());
    let _ = app.emit("tool_mode:telemetry", telemetry.clone());
    emit_agent_stage(
        &app,
        "manual_tool_selection_activated",
        "Manual tool mode selected by user",
        Some(telemetry),
    );

    send_message_with_profile(message, execution_profile, state, app).await
}

#[tauri::command]
pub async fn send_lab_message(
    message: String,
    profile: Option<LabExecutionProfileInput>,
    state: State<'_, AppStateCell>,
    app: AppHandle,
) -> Result<serde_json::Value, String> {
    let execution_profile = profile
        .map(|value| value.to_core_profile())
        .unwrap_or_else(|| {
            TurnExecutionProfile::prompt_lab(
                None,
                None,
                PromptLabToolSelectionStrategy::RoutedWithinLock,
            )
        });
    send_message_with_profile(message, execution_profile, state, app).await
}
