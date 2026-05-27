use super::*;

#[tauri::command]
pub async fn send_image_message(
    image_data: Vec<u8>,
    mime_type: String,
    text: Option<String>,
    state: State<'_, AppStateCell>,
    app: AppHandle,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    emit_agent_stage(
        &app,
        "input_received",
        "Image prompt received from UI",
        Some(serde_json::json!({
            "mime_type": mime_type.clone(),
            "bytes": image_data.len(),
            "has_text": text.is_some(),
        })),
    );

    // Validate MIME type
    let allowed = [
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "image/bmp",
    ];
    if !allowed.contains(&mime_type.as_str()) {
        return Err(format!("unsupported image type: {}", mime_type));
    }

    // Validate image size (max 10 MB)
    if image_data.len() > 10 * 1024 * 1024 {
        return Err("image too large (max 10 MB)".into());
    }

    touch_orchestrator_activity(&state.orchestrator_last_activity_at).await;
    let orchestrator_img = state.orchestrator.read().await.clone();
    if orchestrator_img.is_some() {
        emit_agent_stage(
            &app,
            "ensuring_local_runtime",
            "Ensuring local LLM runtime is ready for image analysis",
            None,
        );
    }
    if let Err(e) =
        ensure_orchestrator_ready_for_turn(orchestrator_img.as_ref(), "image_turn").await
    {
        emit_agent_stage(
            &app,
            "failed",
            "Local runtime preflight failed",
            Some(serde_json::json!({ "error": e.clone() })),
        );
        return Err(e);
    }

    // Store image to ~/.kria/attachments/ with hash-based filename
    let config = state.config.read().await;
    let paths = config.resolve_paths().map_err(|e| e.to_string())?;
    drop(config);
    let attach_dir = paths.data_dir.join("attachments");
    std::fs::create_dir_all(&attach_dir).map_err(|e| e.to_string())?;

    let hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        image_data.hash(&mut h);
        Utc::now().timestamp_nanos_opt().unwrap_or(0).hash(&mut h);
        format!("{:016x}", h.finish())
    };
    let ext = match mime_type.as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        _ => "bin",
    };
    let filename = format!("{}.{}", hash, ext);
    let filepath = attach_dir.join(&filename);
    std::fs::write(&filepath, &image_data).map_err(|e| e.to_string())?;

    tracing::info!(path = %filepath.display(), size = image_data.len(), "image attachment saved");

    emit_agent_stage(
        &app,
        "image_saved",
        "Image attachment saved to local storage",
        Some(serde_json::json!({
            "filename": filename.clone(),
        })),
    );

    let user_text = text.unwrap_or_else(|| "What's in this image?".into());
    let image_intent = infer_image_intent_from_text(&user_text).to_string();
    let _ = app.emit(
        "agent:thinking",
        serde_json::json!({"status": "processing"}),
    );

    let image_path_for_llm = filepath.to_string_lossy().to_string();

    let agent_loop = state.agent_loop.clone();
    let memory_store = state.memory_store.clone();
    let tool_registry = state.tool_registry.clone();
    let event_bus = state.event_bus.clone();
    let config = state.config.read().await;
    let hw_tier = state.hardware_info.tier.as_str();

    emit_agent_stage(
        &app,
        "preparing_tool_context",
        "Collecting tool descriptions for image request",
        Some(serde_json::json!({ "hardware_tier": hw_tier })),
    );

    let tool_defs = tool_registry.list_for_tier(hw_tier);
    let tool_descriptions = build_tool_descriptions_for_prompt(&tool_defs);

    emit_agent_stage(
        &app,
        "tool_context_ready",
        "Tool descriptions prepared",
        Some(serde_json::json!({ "tool_count": tool_defs.len() })),
    );

    let llm_context_window = config.llm.context_window.max(1024);
    let visual_token_cap = image_visual_token_cap_for_context(llm_context_window);
    let response_reserve = if llm_context_window <= 2048 { 480 } else { 640 };
    let system_reserve = if llm_context_window <= 2048 { 320 } else { 480 };
    let history_reserve = if llm_context_window <= 2048 { 320 } else { 700 };
    let ocr_token_cap = if llm_context_window <= 2048 { 256 } else { 320 };

    let (preanalysis_summary, llm_images): (Option<String>, Vec<ImageAttachment>) = if let Some(
        handler,
    ) =
        tool_registry.get_handler("analyze_image")
    {
        emit_agent_stage(
            &app,
            "preanalyzing_image",
            "Running automatic image pre-analysis",
            None,
        );

        let preanalysis_params = serde_json::json!({
            "path": image_path_for_llm.clone(),
            "operations": ["metadata", "ocr", "features", "thumbnail"],
            "intent": image_intent.clone(),
            "context_window": llm_context_window,
            "response_reserve": response_reserve,
            "system_reserve": system_reserve,
            "history_reserve": history_reserve,
            "ocr_token_cap": ocr_token_cap,
            "metadata_token_cap": 72,
            "hard_visual_token_cap": visual_token_cap,
            "max_images_per_turn": IMAGE_SAFE_MAX_ATTACHMENTS_PER_TURN,
        });

        match tokio::time::timeout(
            std::time::Duration::from_secs(IMAGE_PREANALYSIS_TIMEOUT_SECS),
            handler.execute(preanalysis_params),
        )
        .await
        {
            Ok(result) if result.success => {
                let summary = extract_image_preanalysis_summary(&result.data);
                let extracted_images =
                    extract_preprocessed_image_attachments(&result.data, &mime_type)
                        .unwrap_or_default();
                let mut images =
                    constrain_runtime_image_attachments(extracted_images, llm_context_window);
                if images.is_empty() {
                    if let Some(native) =
                        build_native_preprocessed_attachment_with_max(&image_path_for_llm, 640)
                    {
                        images.push(native);
                    }
                }
                let step_status = build_preprocessing_step_status(&result.data, &image_intent);
                emit_agent_stage(
                    &app,
                    "preanalysis_ready",
                    "Image pre-analysis completed",
                    Some(serde_json::json!({
                        "has_summary": summary.is_some(),
                        "llm_image_count": images.len(),
                        "context_window": llm_context_window,
                        "visual_token_cap": visual_token_cap,
                        "step_status": step_status,
                    })),
                );

                if images.is_empty() {
                    emit_agent_stage(
                        &app,
                        "preanalysis_invalid",
                        "Pre-analysis returned no image payload; aborting request",
                        None,
                    );
                    return Err("Image preprocessing produced no usable image payload. Please check sidecar OCR/vision dependencies and try again.".into());
                }

                (summary, images)
            }
            Ok(result) => {
                emit_agent_stage(
                    &app,
                    "preanalysis_failed",
                    "Image pre-analysis failed; aborting before LLM call",
                    Some(serde_json::json!({
                        "error": result.error,
                    })),
                );
                return Err("Image preprocessing failed before LLM dispatch. Please verify sidecar/OCR dependencies and try again.".into());
            }
            Err(_) => {
                emit_agent_stage(
                    &app,
                    "preanalysis_timeout",
                    "Image pre-analysis timed out; aborting before LLM call",
                    Some(serde_json::json!({
                        "timeout_secs": IMAGE_PREANALYSIS_TIMEOUT_SECS,
                    })),
                );
                return Err("Image preprocessing timed out before LLM dispatch. Please retry after the sidecar is healthy.".into());
            }
        }
    } else {
        emit_agent_stage(
            &app,
            "preanalysis_unavailable",
            "Image pre-analysis tool is unavailable; aborting request",
            None,
        );
        return Err(
            "Image preprocessing tool is unavailable. Please restart KRIA and try again.".into(),
        );
    };

    emit_agent_stage(
        &app,
        "image_encoded",
        "Preprocessed image payload encoded for multimodal LLM input",
        Some(serde_json::json!({
            "image_count": llm_images.len(),
        })),
    );

    let user_name = memory_store
        .get_preference("user_name")
        .unwrap_or(None)
        .unwrap_or_else(|| "User".to_string());
    let os_name = std::env::consts::OS;

    // Detect package managers for image message context
    let pm_string_img = {
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

    let memory_context = match memory_store.search_facts(&user_text, 5) {
        Ok(facts) if !facts.is_empty() => {
            let fact_lines: Vec<String> = facts.iter().map(|f| format!("- {}", f.text)).collect();
            format!("Known facts about the user:\n{}", fact_lines.join("\n"))
        }
        _ => String::new(),
    };

    emit_agent_stage(
        &app,
        "memory_context_ready",
        "Memory context prepared for image prompt",
        Some(serde_json::json!({
            "has_context": !memory_context.is_empty(),
        })),
    );

    let system_prompt = kria_core::agent::prompts::build_system_prompt(
        &tool_descriptions,
        &user_name,
        os_name,
        hw_tier,
        &pm_string_img,
        &memory_context,
    );

    emit_agent_stage(
        &app,
        "system_prompt_ready",
        "System prompt prepared for image request",
        Some(serde_json::json!({
            "prompt_chars": system_prompt.chars().count(),
        })),
    );
    drop(config);

    let session_id = state.current_session_id.read().await.clone();
    let memory_writer: Arc<dyn MemoryManager> = memory_store.clone();

    emit_agent_stage(
        &app,
        "building_message_history",
        "Building multimodal conversation history",
        Some(serde_json::json!({
            "session_id": session_id.clone(),
        })),
    );

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

    if let Some(summary) = preanalysis_summary
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        messages.push(ChatMessage {
            role: "system".into(),
            content: format!(
                "Automatic pre-analysis context (already validated):\n{}",
                summary
            ),
            name: None,
            images: None,
        });
    }

    append_recent_turns_for_llm(&mut messages, &recent_turns);
    messages.push(ChatMessage {
        role: "user".into(),
        content: build_image_llm_user_content(&user_text, &image_path_for_llm, &image_intent, None),
        name: None,
        images: Some(llm_images),
    });

    // Persist user turn (content only, images stored in attachments/)
    let _ = memory_writer.store_turn(&memory_turn_write(
        session_id.clone(),
        format!("{}\n[image: {}]", user_text, filename),
        String::new(),
        None,
        None,
        None,
    ));

    emit_agent_stage(
        &app,
        "user_turn_saved",
        "Image prompt stored in session memory",
        Some(serde_json::json!({
            "history_turns": recent_turns.len() + 1,
        })),
    );

    // Auto-title
    {
        let title_key = format!("session_title:{}", session_id);
        if memory_store
            .get_preference(&title_key)
            .unwrap_or(None)
            .is_none()
        {
            let title = if user_text.len() > 50 {
                format!("{}...", &user_text[..50])
            } else {
                user_text.clone()
            };
            let _ = memory_writer
                .set_preference(&preference_record(title_key, format!("📷 {}", title)));
        }
    }

    event_bus.publish(kria_core::infra::event_bus::KriaEvent::MessageReceived {
        session_id: session_id.clone(),
        content: user_text.clone(),
    });

    emit_agent_stage(
        &app,
        "dispatching_to_llm",
        "Dispatching multimodal prompt to agent loop",
        None,
    );

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();
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
    let user_message_clone = user_text.clone();
    let preanalysis_summary_fallback = preanalysis_summary.clone();
    let stale_guard_agent = agent_loop.clone();

    let agent = agent_loop.clone();
    let sid = session_id.clone();
    tauri::async_runtime::spawn(async move {
        agent.run(&sid, &mut messages, event_tx).await;
    });

    emit_agent_stage(
        &app,
        "agent_loop_started",
        "Agent loop started for image request",
        None,
    );

    // Event consumer (same as send_message)
    tauri::async_runtime::spawn(async move {
        let mut full_response = String::new();
        let mut saw_first_token = false;
        let mut active_turn_id: Option<String> = None;
        let mut pending_tool_params: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();

        emit_agent_stage(
            &app_handle,
            "awaiting_llm_output",
            "Image prompt sent to LLM; waiting for first response token",
            None,
        );

        loop {
            let event = match tokio::time::timeout(
                std::time::Duration::from_secs(AGENT_EVENT_IDLE_TIMEOUT_SECS),
                event_rx.recv(),
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
                        "agent:token",
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
                    match &event {
                        StreamEvent::Done(_) | StreamEvent::Error(_) => {
                            tracing::debug!(
                                session_id = %session_id_clone,
                                turn_id = %turn_id,
                                "Forwarding terminal stream event for stale turn in image consumer"
                            );
                        }
                        _ => {
                            tracing::debug!(
                                session_id = %session_id_clone,
                                turn_id = %turn_id,
                                "Dropping stale stream event in image consumer"
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
                    let _ = app_handle.emit("agent:token", serde_json::json!({ "text": text }));
                }
                StreamEvent::ToolStart { name, params } => {
                    pending_tool_params.insert(name.clone(), params.clone());
                    emit_agent_stage(
                        &app_handle,
                        "tool_started",
                        "Tool execution started",
                        Some(serde_json::json!({ "tool": name.clone() })),
                    );
                    let _ = app_handle.emit(
                        "agent:tool_call",
                        serde_json::json!({ "name": name, "params": params }),
                    );
                }
                StreamEvent::ToolEnd {
                    name,
                    result,
                    success,
                    ..
                } => {
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
                    let payload = build_tool_result_event_payload(&name, &result, success);
                    let metadata = payload
                        .get("metadata")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let _ = app_handle.emit("agent:tool_result", payload);

                    let persisted_payload = serde_json::json!({
                        "name": name,
                        "args": args,
                        "success": success,
                        "result": result,
                        "metadata": metadata,
                    });
                    let tool_name = persisted_payload
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("tool");
                    let _ = memory_writer_clone.store_turn(&memory_turn_write(
                        session_id_clone.clone(),
                        String::new(),
                        summarize_tool_turn_for_history(
                            tool_name,
                            success,
                            persisted_payload
                                .get("result")
                                .unwrap_or(&serde_json::Value::Null),
                            persisted_payload
                                .get("metadata")
                                .unwrap_or(&serde_json::Value::Null),
                        ),
                        Some(tool_name.to_string()),
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
                    let _ = app_handle.emit("agent:approval_required", serde_json::json!({ "requestId": request_id, "toolName": action, "riskLevel": risk_level, "args": parameters, "reason": "" }));
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
                        "agent:approval_result",
                        serde_json::json!({ "action": action, "approved": approved }),
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
                        "agent:tool_choice_required",
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
                        Some(serde_json::json!({ "plan": plan.clone() })),
                    );
                    let _ = app_handle.emit(
                        "agent:thinking",
                        serde_json::json!({ "status": "planning", "plan": plan }),
                    );
                }
                StreamEvent::Error(err) => {
                    let lower_err = err.to_ascii_lowercase();
                    let is_transport_failure = lower_err.contains("error sending request for url")
                        || lower_err.contains("connection refused")
                        || lower_err.contains("tcp connect")
                        || lower_err.contains("dns error")
                        || lower_err.contains("timed out");

                    if (lower_err.contains("circuit open")
                        || lower_err.contains("local llm unavailable")
                        || is_transport_failure)
                        && full_response.is_empty()
                    {
                        if let Some(summary) = preanalysis_summary_fallback.as_ref() {
                            let fallback_text = format!(
                                "⚠️ Local vision model is temporarily unavailable. Here is the image pre-analysis:\n\n{}",
                                summary
                            );
                            full_response = fallback_text.clone();
                            emit_agent_stage(
                                &app_handle,
                                "llm_unavailable_preanalysis_fallback",
                                "LLM unavailable; returning pre-analysis summary fallback",
                                None,
                            );
                            let _ = app_handle
                                .emit("agent:token", serde_json::json!({ "text": fallback_text }));
                            continue;
                        }
                    }

                    let user_visible_error = format!("⚠️ {err}");
                    if full_response.is_empty() {
                        full_response = user_visible_error.clone();
                    }
                    emit_agent_stage(
                        &app_handle,
                        "failed",
                        "Agent stream reported an error",
                        Some(serde_json::json!({ "error": err.clone() })),
                    );
                    let _ = app_handle.emit(
                        "agent:token",
                        serde_json::json!({ "text": user_visible_error }),
                    );
                }
                StreamEvent::Done(final_text) => {
                    if !final_text.is_empty() && full_response.is_empty() {
                        full_response = final_text;
                    }
                    emit_agent_stage(
                        &app_handle,
                        "llm_done",
                        "LLM stream completed",
                        Some(serde_json::json!({
                            "response_chars": full_response.chars().count(),
                        })),
                    );
                }
                StreamEvent::RecoveryOptions { .. } => {
                    // image_chat does not use fleet tools; ignore
                }
                StreamEvent::TaskStep(_) => {
                    // image_chat does not emit task steps; ignore
                }
            }
        }

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

            let fact_mgr = kria_core::memory::facts::FactManager::new(
                memory_store_clone.as_ref(),
                &vectors_clone,
                &embeddings_clone,
            );
            match fact_mgr.extract_from_turn(&user_message_clone, &full_response) {
                Ok(ids) if !ids.is_empty() => {
                    tracing::info!(
                        count = ids.len(),
                        "auto-extracted facts from image conversation"
                    );
                    emit_agent_stage(
                        &app_handle,
                        "facts_extracted",
                        "New user facts extracted from the image conversation",
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

        let _ = app_handle.emit("agent:done", serde_json::json!({}));
        decrement_active_turn_counter(&active_turns_for_tracking);
        touch_orchestrator_activity(&last_activity_for_tracking).await;
    });

    Ok(serde_json::json!({
        "status": "processing",
        "attachment": filename,
    }))
}
