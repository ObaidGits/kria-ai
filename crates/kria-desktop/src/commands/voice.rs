use super::*;

#[tauri::command]
pub async fn start_voice(state: State<'_, AppStateCell>, app: AppHandle) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    if state
        .voice_active
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return Ok(()); // Already active
    }

    // Pre-flight checks: verify required binaries and models exist
    let whisper_available = which_binary("whisper-cpp")
        .or_else(|| which_binary("main"))
        .is_some();
    if !whisper_available {
        return Err("Voice requires whisper-cpp (or 'main' binary from whisper.cpp) on your PATH. Install it with: sudo apt install whisper.cpp OR build from https://github.com/ggerganov/whisper.cpp".into());
    }

    let piper_available = which_binary("piper").is_some();
    if !piper_available {
        return Err("Voice requires Piper TTS binary on your PATH. Install it from: https://github.com/rhasspy/piper/releases".into());
    }

    // Refresh config from disk on every voice start so external edits in
    // ~/.kria/config.toml are not stuck behind stale in-memory state.
    let mut effective_config = match KriaConfig::load(None) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!(error = %e, "failed to reload config from disk for voice start; using in-memory config");
            state.config.read().await.clone()
        }
    };
    {
        let mut cfg_guard = state.config.write().await;
        *cfg_guard = effective_config.clone();
    }

    // Resolve tier-aware defaults that init_runtime applies but are lost on reload.
    if effective_config
        .voice
        .stt_model
        .eq_ignore_ascii_case("auto")
    {
        effective_config.voice.stt_model = state.hardware_info.tier.stt_model().to_string();
    }

    // Verify required models and rebuild pipeline from latest saved settings.
    let voice_pipeline = {
        let paths = effective_config
            .resolve_paths()
            .map_err(|e| e.to_string())?;

        let stt_model = resolve_model_file(&paths, "stt", &effective_config.voice.stt_model);
        if !stt_model.exists() {
            return Err(format!(
                "STT model not found at: {}. Run 'python scripts/download_models.py' to download models.",
                stt_model.display()
            ));
        }

        let tts_voice_file = format!("{}.onnx", effective_config.voice.tts_voice);
        let tts_model = resolve_model_file(&paths, "piper", &tts_voice_file);
        if !tts_model.exists() {
            return Err(format!(
                "TTS voice model not found at: {}. Run 'python scripts/download_models.py' to download models.",
                tts_model.display()
            ));
        }

        build_voice_pipeline(&effective_config, &paths)
    };

    {
        let mut vp_guard = state.voice_pipeline.write().await;
        *vp_guard = voice_pipeline.clone();
    }

    // ── v2 hot-swap ───────────────────────────────────────────────────────
    // If the (freshly reloaded) config requests v2 but active_voice is still
    // Legacy (e.g. config changed from v1→v2 after init_runtime ran, or the
    // v2 build failed at startup and the user fixed the prerequisites), try
    // to build the v2 pipeline now and swap it in atomically.
    if effective_config.voice.engine.eq_ignore_ascii_case("v2")
        && !state.active_voice.read().await.is_streaming()
    {
        match effective_config
            .resolve_paths()
            .ok()
            .and_then(|p| build_v2_pipeline(&effective_config, &p, state.hardware_info.tier).ok())
        {
            Some((v2_pipeline, _state_rx, telemetry_rx)) => {
                tracing::info!("voice: hot-swapping active_voice → v2");
                *state.active_voice.write().await =
                    kria_core::voice::v2::ActivePipeline::Streaming(v2_pipeline);
                *state.voice_v2_telemetry.lock().await = Some(telemetry_rx);
            }
            None => {
                tracing::warn!("voice: v2 hot-swap failed; continuing with v1 pipeline");
            }
        }
    }

    state
        .voice_active
        .store(true, std::sync::atomic::Ordering::Relaxed);

    // ── v2 continuous mic-capture loop ────────────────────────────────────
    // When the pipeline is the v2 streaming FSM, bypass the v1 event loop
    // entirely and spin up a self-contained capture→run_turn loop. All v1
    // validation above is still performed (binary/model checks) so the
    // same config requirements apply.
    if let Some(v2) = state.active_voice.read().await.streaming() {
        start_voice_v2_loop(
            v2,
            state.voice_active.clone(),
            state.voice_v2_telemetry.clone(),
            state.model_router.clone(),
            state.current_session_id.clone(),
            state.config.clone(),
            state.hardware_info.clone(),
            state.memory_store.clone(),
            state.tool_registry.clone(),
            app.clone(),
        )
        .await;
        return Ok(());
    }

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<VoicePipelineEvent>();

    if let Err(e) = voice_pipeline.start(event_tx).await {
        state
            .voice_active
            .store(false, std::sync::atomic::Ordering::Relaxed);
        return Err(e.to_string());
    }

    let _ = app.emit("voice:state", serde_json::json!({ "state": "listening" }));

    // Spawn a task that listens for voice pipeline events and forwards them
    let app_handle = app.clone();
    let voice_pipeline = voice_pipeline.clone();
    let memory_store = state.memory_store.clone();
    let memory_writer: Arc<dyn MemoryManager> = memory_store.clone();
    let agent_loop = state.agent_loop.clone();
    let tool_registry = state.tool_registry.clone();
    let event_bus = state.event_bus.clone();
    let config = state.config.clone();
    let session_id_lock = state.current_session_id.clone();
    let embeddings = state.embeddings.clone();
    let vectors = state.vectors.clone();
    let hw_info_voice = state.hardware_info.clone();
    let orchestrator_voice = state.orchestrator.read().await.clone();
    let active_turns_voice = state.orchestrator_active_turns.clone();
    let last_activity_voice = state.orchestrator_last_activity_at.clone();

    tauri::async_runtime::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            match event {
                VoicePipelineEvent::StateChanged(new_state) => {
                    let state_str = match new_state {
                        VoicePipelineState::Idle => "idle",
                        VoicePipelineState::Listening => "listening",
                        VoicePipelineState::Processing => "processing",
                        VoicePipelineState::Speaking => "speaking",
                    };
                    let _ =
                        app_handle.emit("voice:state", serde_json::json!({ "state": state_str }));
                }
                VoicePipelineEvent::PartialTranscript(frame) => {
                    let _ = app_handle.emit(
                        "voice:partial_transcript",
                        serde_json::json!({
                            "text": frame.text,
                            "confidence": frame.confidence,
                            "language": frame.language,
                            "stability": frame.stability,
                            "partial": true,
                        }),
                    );
                }
                VoicePipelineEvent::Transcript(frame) => {
                    let text = frame.text;
                    let language = frame.language;
                    let confidence = frame.confidence;

                    tracing::info!(
                        language = %language,
                        confidence,
                        chars = text.chars().count(),
                        "voice transcript received"
                    );
                    if kria_core::infra::pipeline_trace::pipeline_debug_enabled() {
                        tracing::debug!(
                            target: "kria_pipeline",
                            transcript = %kria_core::infra::pipeline_trace::sanitize_text_for_logs(&text, 320),
                            language = %language,
                            confidence,
                            "voice transcript preview"
                        );
                    }
                    let _ = app_handle.emit(
                        "voice:transcript",
                        serde_json::json!({
                            "text": text.clone(),
                            "confidence": confidence,
                            "language": language.clone(),
                            "stability": 1.0,
                        }),
                    );

                    touch_orchestrator_activity(&last_activity_voice).await;
                    if let Err(e) = ensure_orchestrator_ready_for_turn(
                        orchestrator_voice.as_ref(),
                        "voice_turn",
                    )
                    .await
                    {
                        tracing::warn!(?e, "voice turn preflight failed");
                        let _ = app_handle.emit(
                            "agent:token",
                            serde_json::json!({ "text": format!("⚠️ {e}") }),
                        );
                        let _ = app_handle.emit("agent:done", serde_json::json!({}));
                        continue;
                    }
                    active_turns_voice.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                    // Feed transcript through the agent loop (same as send_message)
                    let session_id = session_id_lock.read().await.clone();
                    let config_guard = config.read().await;
                    let hw_tier = hw_info_voice.tier.as_str();

                    let tool_defs = tool_registry.list_for_tier(hw_tier);
                    let tool_descriptions = build_tool_descriptions_for_prompt(&tool_defs);

                    let user_name = memory_store
                        .get_preference("user_name")
                        .unwrap_or(None)
                        .unwrap_or_else(|| "User".to_string());
                    let os_name = std::env::consts::OS;
                    let pm_string_voice = {
                        let pms = get_available_package_managers();
                        match pms.as_slice() {
                            [] => "unknown".to_string(),
                            [only] => only.as_str().to_string(),
                            [primary, rest @ ..] => {
                                let alts: Vec<&str> = rest.iter().map(|p| p.as_str()).collect();
                                format!(
                                    "{} (also available: {})",
                                    primary.as_str(),
                                    alts.join(", ")
                                )
                            }
                        }
                    };
                    let memory_context = match memory_store.search_facts(&text, 5) {
                        Ok(facts) if !facts.is_empty() => {
                            let lines: Vec<String> =
                                facts.iter().map(|f| format!("- {}", f.text)).collect();
                            format!("Known facts about the user:\n{}", lines.join("\n"))
                        }
                        _ => String::new(),
                    };

                    let system_prompt = kria_core::agent::prompts::build_system_prompt(
                        &tool_descriptions,
                        &user_name,
                        os_name,
                        hw_tier,
                        &pm_string_voice,
                        &memory_context,
                    );
                    drop(config_guard);

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
                    append_recent_turns_for_llm(&mut messages, &recent_turns);
                    messages.push(ChatMessage {
                        role: "user".into(),
                        content: text.clone(),
                        name: None,
                        images: None,
                    });

                    let _ = memory_writer.store_turn(&memory_turn_write(
                        session_id.clone(),
                        format!("🎤 {}", text),
                        String::new(),
                        None,
                        None,
                        None,
                    ));

                    event_bus.publish(kria_core::infra::event_bus::KriaEvent::MessageReceived {
                        session_id: session_id.clone(),
                        content: text.clone(),
                    });

                    let _ = app_handle.emit(
                        "agent:thinking",
                        serde_json::json!({"status": "processing"}),
                    );

                    let (agent_tx, mut agent_rx) =
                        tokio::sync::mpsc::unbounded_channel::<StreamEvent>();

                    let agent = agent_loop.clone();
                    let stale_guard_agent = agent_loop.clone();
                    let sid = session_id.clone();
                    tokio::spawn(async move {
                        agent.run(&sid, &mut messages, agent_tx).await;
                    });

                    // Collect agent response for TTS
                    let mut full_response = String::new();
                    let mut pending_tool_params: std::collections::HashMap<
                        String,
                        serde_json::Value,
                    > = std::collections::HashMap::new();
                    let app2 = app_handle.clone();
                    let ms2 = memory_store.clone();
                    let mw2 = memory_writer.clone();
                    let sid2 = session_id.clone();
                    let emb2 = embeddings.clone();
                    let vec2 = vectors.clone();
                    let text2 = text.clone();
                    let vp = voice_pipeline.clone();
                    let mut active_turn_id: Option<String> = None;

                    while let Some(ev) = agent_rx.recv().await {
                        if let StreamEvent::TurnAccepted {
                            session_id,
                            turn_id,
                        } = &ev
                        {
                            if session_id == &sid2 {
                                active_turn_id = Some(turn_id.clone());
                            }
                            continue;
                        }

                        if let Some(turn_id) = active_turn_id.as_deref() {
                            if !stale_guard_agent.is_turn_active(&sid2, turn_id) {
                                tracing::debug!(
                                    session_id = %sid2,
                                    turn_id = %turn_id,
                                    "Dropping stale stream event in voice consumer"
                                );
                                continue;
                            }
                        }

                        match ev {
                            StreamEvent::TurnAccepted { .. } => {}
                            StreamEvent::Token(t) => {
                                full_response.push_str(&t);
                                let _ = app2.emit("agent:token", serde_json::json!({"text": t}));
                            }
                            StreamEvent::ToolStart { name, params } => {
                                pending_tool_params.insert(name.clone(), params.clone());
                                let _ = app2.emit(
                                    "agent:tool_call",
                                    serde_json::json!({"name": name, "params": params}),
                                );
                            }
                            StreamEvent::ToolEnd {
                                name,
                                result,
                                success,
                            } => {
                                let args = pending_tool_params
                                    .remove(&name)
                                    .unwrap_or_else(|| serde_json::json!({}));
                                let payload =
                                    build_tool_result_event_payload(&name, &result, success);
                                let metadata = payload
                                    .get("metadata")
                                    .cloned()
                                    .unwrap_or(serde_json::Value::Null);
                                let _ = app2.emit("agent:tool_result", payload);

                                let persisted_payload = serde_json::json!({
                                    "name": name,
                                    "args": args,
                                    "success": success,
                                    "result": result,
                                    "metadata": metadata,
                                });

                                let _ = mw2.store_turn(&memory_turn_write(
                                    sid2.clone(),
                                    String::new(),
                                    summarize_tool_turn_for_history(
                                        persisted_payload
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("tool"),
                                        success,
                                        persisted_payload
                                            .get("result")
                                            .unwrap_or(&serde_json::Value::Null),
                                        persisted_payload
                                            .get("metadata")
                                            .unwrap_or(&serde_json::Value::Null),
                                    ),
                                    persisted_payload
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string()),
                                    Some(persisted_payload.to_string()),
                                    None,
                                ));

                                // Persist image metadata in chat_media table when generate_image succeeds
                                if name == "generate_image" && success {
                                    if let Some(imgs) =
                                        result.get("images").and_then(|v| v.as_array())
                                    {
                                        for img in imgs {
                                            let file_path = img
                                                .get("path")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            if file_path.is_empty() {
                                                continue;
                                            }
                                            let _ = mw2.store_media(&ChatMediaRecord {
                                                session_id: sid2.clone(),
                                                media_type: "generated".into(),
                                                file_path,
                                                sha256: img
                                                    .get("sha256")
                                                    .and_then(|v| v.as_str())
                                                    .map(|s| s.to_string()),
                                                prompt: result
                                                    .get("prompt")
                                                    .and_then(|v| v.as_str())
                                                    .map(|s| s.to_string()),
                                                width: img
                                                    .get("width")
                                                    .and_then(|v| v.as_u64())
                                                    .map(|v| v as u32),
                                                height: img
                                                    .get("height")
                                                    .and_then(|v| v.as_u64())
                                                    .map(|v| v as u32),
                                                style: img
                                                    .get("style")
                                                    .and_then(|v| v.as_str())
                                                    .map(|s| s.to_string()),
                                                provenance: img
                                                    .get("provenance")
                                                    .and_then(|v| v.as_str())
                                                    .map(|s| s.to_string()),
                                            });
                                        }
                                    }
                                }
                            }
                            StreamEvent::ToolProgress {
                                call_id,
                                message,
                                percent,
                            } => {
                                let _ = app2.emit(
                                    "kria:tool-progress",
                                    serde_json::json!({
                                        "call_id": call_id,
                                        "message": message,
                                        "percent": percent,
                                        "session_id": sid2,
                                    }),
                                );
                            }
                            StreamEvent::ToolPayloadChunk {
                                call_id,
                                seq,
                                is_final,
                                data,
                            } => {
                                let _ = app2.emit(
                                    "kria:tool-payload-chunk",
                                    serde_json::json!({
                                        "call_id": call_id,
                                        "seq": seq,
                                        "is_final": is_final,
                                        "data": data,
                                        "session_id": sid2,
                                    }),
                                );
                            }
                            StreamEvent::ApprovalRequired {
                                request_id,
                                action,
                                risk_level,
                                parameters,
                            } => {
                                let _ = app2.emit("agent:approval_required", serde_json::json!({"requestId": request_id, "toolName": action, "riskLevel": risk_level, "args": parameters, "reason": ""}));
                            }
                            StreamEvent::ApprovalResult { action, approved } => {
                                let _ = app2.emit(
                                    "agent:approval_result",
                                    serde_json::json!({"action": action, "approved": approved}),
                                );
                            }
                            StreamEvent::ToolChoiceRequired {
                                query,
                                confidence,
                                min_confidence,
                                candidates,
                            } => {
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
                                let _ = app2.emit(
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
                                let _ = app2.emit(
                                    "agent:thinking",
                                    serde_json::json!({"status": "planning", "plan": plan}),
                                );
                            }
                            StreamEvent::Error(err) => {
                                let _ = app2.emit(
                                    "agent:token",
                                    serde_json::json!({"text": format!("⚠️ {err}")}),
                                );
                            }
                            StreamEvent::Done(final_text) => {
                                if !final_text.is_empty() && full_response.is_empty() {
                                    full_response = final_text;
                                }
                            }
                        }
                    }

                    // Persist assistant response
                    if !full_response.is_empty() && !is_transient_llm_error_text(&full_response) {
                        let _ = mw2.store_turn(&memory_turn_write(
                            sid2.clone(),
                            String::new(),
                            full_response.clone(),
                            None,
                            None,
                            None,
                        ));
                        let fact_mgr =
                            kria_core::memory::facts::FactManager::new(ms2.as_ref(), &vec2, &emb2);
                        let _ = fact_mgr.extract_from_turn(&text2, &full_response);

                        // Speak the response via TTS
                        if let Err(e) = vp.speak(&full_response).await {
                            tracing::warn!("TTS playback failed: {e}");
                        }
                    }

                    let _ = app2.emit("agent:done", serde_json::json!({}));
                    decrement_active_turn_counter(&active_turns_voice);
                    touch_orchestrator_activity(&last_activity_voice).await;
                }
                VoicePipelineEvent::SpeakingStarted => {
                    let _ =
                        app_handle.emit("voice:state", serde_json::json!({ "state": "speaking" }));
                }
                VoicePipelineEvent::SpeakingDone => {
                    let _ =
                        app_handle.emit("voice:state", serde_json::json!({ "state": "listening" }));
                }
                VoicePipelineEvent::Error(err) => {
                    tracing::warn!("voice pipeline error: {err}");
                    let _ = app_handle.emit("voice:error", serde_json::json!({ "error": err }));
                }
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn stop_voice(state: State<'_, AppStateCell>, app: AppHandle) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    state
        .voice_active
        .store(false, std::sync::atomic::Ordering::Relaxed);
    // Abort any in-flight v2 turn immediately so barge-in / stop is instant.
    if let Some(v2) = state.active_voice.read().await.streaming() {
        v2.force_abort().await;
    }
    let voice_pipeline = state.voice_pipeline.read().await.clone();
    voice_pipeline.stop().await;
    let _ = app.emit("voice:state", serde_json::json!({ "state": "idle" }));
    Ok(())
}

#[tauri::command]
pub async fn get_voice_status(state: State<'_, AppStateCell>) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let voice_pipeline = state.voice_pipeline.read().await.clone();
    let pipeline_state = voice_pipeline.state().await;
    Ok(serde_json::json!({
        "active": state.voice_active.load(std::sync::atomic::Ordering::Relaxed),
        "state": pipeline_state,
    }))
}

// ───────────────── voice v2 commands (additive) ──────────────────────────
//
// `voice_v2_speak` runs ONE end-to-end v2 turn from a text prompt:
// LLM token stream → SentenceSplitter → CliPiperTts → PlaybackSink with
// hard barge-in. Used by the UI when `voice.engine = "v2"` is set; the v1
// `start_voice` flow is untouched. `voice_v2_abort` cancels the active
// turn (also exposed for the "KRIA stop now" emergency phrase).

#[tauri::command]
pub async fn voice_v2_speak(
    prompt: String,
    state: State<'_, AppStateCell>,
    app: AppHandle,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let v2 = {
        let active = state.active_voice.read().await;
        active.streaming().ok_or_else(|| {
            "voice v2 is not active — set `voice.engine = \"v2\"` in config.toml".to_string()
        })?
    };

    // Lazy-wire the AudioPlayer the first time we speak so the playback
    // sink can open a real session via `begin_session`.
    let player = {
        let cfg = state.config.read().await;
        let speaker = cfg.voice.speaker_device.clone();
        let follow = cfg.voice.follow_system_default_speaker;
        Arc::new(
            kria_core::voice::AudioPlayer::new()
                .with_output_device(Some(speaker))
                .follow_system_default(follow),
        )
    };
    v2.set_audio_player(player).await;

    // Drain telemetry into UI events for the duration of this turn.
    let telemetry_rx = state.voice_v2_telemetry.lock().await.take();
    if let Some(mut rx) = telemetry_rx {
        let app_handle = app.clone();
        let slot = state.voice_v2_telemetry.clone();
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                let payload = serde_json::to_value(&ev).unwrap_or_default();
                let _ = app_handle.emit("voice:v2_telemetry", payload);
            }
            // Receiver closed — put None back (channel can't be revived
            // without rebuilding the pipeline).
            *slot.lock().await = None;
        });
    }

    // Build the LLM closure: takes the user prompt, streams tokens off the
    // routed LlmBackend, returns an mpsc::Receiver<String>. The closure
    // owns the stream so cancellation simply drops it.
    let router = state.model_router.clone();
    let llm = move |prompt: String| async move {
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);
        let backend = match router.route(&prompt).await {
            Some(b) => b,
            None => {
                let _ = tx
                    .send("(no LLM backend available — check `voice.engine` / model config)".into())
                    .await;
                return rx;
            }
        };
        tokio::spawn(async move {
            let messages = vec![ChatMessage {
                role: "user".into(),
                content: prompt,
                name: None,
                images: None,
            }];
            match backend.chat_stream(&messages, None, 0.7, 512).await {
                Ok(mut stream) => {
                    use futures::StreamExt;
                    while let Some(tok) = stream.next().await {
                        if tx.send(tok).await.is_err() {
                            break;
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(format!("(LLM error: {e})")).await;
                }
            }
        });
        rx
    };

    // Drive the turn. Errors surface back to the UI.
    v2.clone()
        .run_speak_turn(prompt, llm)
        .await
        .map_err(|e| e.to_string())?;
    let _ = app.emit("voice:state", serde_json::json!({ "state": "idle" }));
    Ok(())
}

#[tauri::command]
pub async fn voice_v2_abort(state: State<'_, AppStateCell>) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    if let Some(v2) = state.active_voice.read().await.streaming() {
        v2.force_abort().await;
    }
    Ok(())
}
