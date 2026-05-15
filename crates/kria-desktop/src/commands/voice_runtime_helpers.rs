use super::*;

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn load_cached_hardware_info(cache_path: &std::path::Path) -> Option<HardwareInfo> {
    let text = std::fs::read_to_string(cache_path).ok()?;
    serde_json::from_str::<HardwareInfo>(&text).ok()
}

/// Resolve a model filename against multiple candidate directories.
///
/// Resolution order (mirrors the LLM `resolve_model_file` helper in runtime.rs):
/// 1. `KRIA_MODELS_DIR/<subdir>/` or `~/.kria/models/<subdir>/` (managed location)
/// 2. Workspace `models/<subdir>/` (walk up from CWD — covers Tauri dev runs)
/// 3. Return the primary path even if missing, so callers can emit a clear error.
pub(crate) fn resolve_model_file(
    paths: &kria_core::platform::paths::KriaPaths,
    subdir: &str,
    filename: &str,
) -> std::path::PathBuf {
    let primary = paths.models_dir.join(subdir).join(filename);
    if primary.exists() {
        return primary;
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = Some(cwd.as_path());
        while let Some(d) = dir {
            let candidate = d.join("models").join(subdir).join(filename);
            if candidate.exists() {
                return candidate;
            }
            dir = d.parent();
        }
    }
    primary
}

pub(crate) fn resolve_hardware_info(
    config: &KriaConfig,
    cache_path: &std::path::Path,
) -> (HardwareInfo, String) {
    // Highest precedence: explicit env override.
    if let Ok(env_tier) = std::env::var("KRIA_TIER") {
        let env_tier = env_tier.trim();
        if !env_tier.is_empty() {
            let mut hw = detect_hardware();
            hw.tier = env_tier
                .parse::<HardwareTier>()
                .unwrap_or(HardwareTier::Standard);
            return (hw, format!("env:KRIA_TIER={env_tier}"));
        }
    }

    // Next precedence: config override.
    if !config.hardware.tier.trim().is_empty() {
        let mut hw = detect_hardware();
        hw.tier = config
            .hardware
            .tier
            .parse::<HardwareTier>()
            .unwrap_or(HardwareTier::Standard);
        return (hw, format!("config.hardware.tier={}", config.hardware.tier));
    }

    let force_redetect = env_truthy("KRIA_REDETECT") || env_truthy("KRIA_REDETECT_HARDWARE");

    // Next precedence: cached detection result.
    if !force_redetect {
        if let Some(cached) = load_cached_hardware_info(cache_path) {
            return (cached, "cache:hardware_tier.json".to_string());
        }
    }

    // Fallback: fresh detection.
    (detect_hardware(), "detect_hardware()".to_string())
}

pub(crate) fn build_voice_pipeline(
    config: &KriaConfig,
    paths: &kria_core::platform::paths::KriaPaths,
) -> Arc<VoicePipeline> {
    // Build the baseline v1 pipeline for compatibility. `start_voice` may
    // hot-swap `active_voice` to v2 immediately after this when v2 runtime
    // dependencies are available.
    let engine = config.voice.engine.to_ascii_lowercase();
    if engine == "v2" {
        tracing::info!("voice.engine = \"v2\" requested; constructing compatibility v1 pipeline (v2 hot-swap attempted during start_voice)");
    } else if engine != "v1" && !engine.is_empty() {
        tracing::warn!(engine = %engine, "unknown voice.engine value; using v1");
    }

    let stt_model_path = resolve_model_file(paths, "stt", &config.voice.stt_model);
    let tts_voice_file = format!("{}.onnx", config.voice.tts_voice);
    let tts_model_path = resolve_model_file(paths, "piper", &tts_voice_file);

    // Resolve + log wake-word model wiring so v2 readiness is visible even
    // while the runtime path is still v1. Construction is cheap (no model
    // load when disabled) and any failure falls back silently.
    if config.voice.wake_word.enabled {
        let wake_path = if config.voice.wake_word.model_path.is_empty() {
            resolve_model_file(paths, "wake", "hey_ria.onnx")
        } else {
            let p = std::path::PathBuf::from(&config.voice.wake_word.model_path);
            if p.is_absolute() {
                p
            } else {
                resolve_model_file(paths, "wake", p.to_string_lossy().as_ref())
            }
        };
        let detector = kria_core::voice::v2::WakeWordDetector::try_load(
            wake_path.clone(),
            config.voice.wake_word.sensitivity,
            "hey ria",
            config.voice.wake_word.aliases.clone(),
        );
        tracing::info!(
            keyword_path = %wake_path.display(),
            sensitivity = config.voice.wake_word.sensitivity,
            active = detector.is_active(),
            "wake-word detector resolved"
        );
    }

    let whisper_bin = which_binary("whisper-cpp").or_else(|| which_binary("main"));
    let piper_bin = which_binary("piper");

    // Surface a clear warning if the configured STT model file is missing,
    // so the user knows to run `python scripts/download_models.py`.
    if !stt_model_path.exists() {
        tracing::warn!(
            model = %stt_model_path.display(),
            "configured STT model file not found — run `python scripts/download_models.py --tier lite` to fetch it"
        );
    }

    let speech_gpu_lease = GpuLeaseManager::shared(
        std::time::Duration::from_secs(120),
        std::time::Duration::from_secs(15),
    );

    let mut stt = SpeechToText::new(stt_model_path.clone(), whisper_bin.clone());
    stt.set_gpu_lease(speech_gpu_lease.clone());
    stt.set_language(&config.voice.language);
    if config.hardware.threads > 0 {
        stt.set_threads(config.hardware.threads.clamp(1, 12));
    }
    stt.set_command_timeout(std::time::Duration::from_secs(45));
    let mut tts = TextToSpeech::new(tts_model_path, piper_bin);
    tts.set_gpu_lease(speech_gpu_lease.clone());
    let vad_model_path = resolve_model_file(paths, "vad", "silero_vad.onnx");

    let pipeline =
        Arc::new(VoicePipeline::new(config.voice.clone(), stt, tts).with_vad_model(vad_model_path));

    // Pre-warm whisper at startup: page-cache the model file + (on CUDA/metal)
    // trigger the one-time GPU layer init *before* the first user utterance.
    // Without this, the first transcription pays the full cold-load cost (the
    // "optimizing GPU layer…" pause) and often exceeds the STT timeout.
    // Best-effort — errors are logged and ignored.
    if stt_model_path.exists() && whisper_bin.is_some() {
        let warm_model = stt_model_path.clone();
        let warm_bin = whisper_bin.clone();
        let warm_lang = config.voice.language.clone();
        let warm_threads = config.hardware.threads;
        let warm_gpu_lease = speech_gpu_lease.clone();
        tokio::spawn(async move {
            let mut warm_stt = SpeechToText::new(warm_model, warm_bin);
            warm_stt.set_gpu_lease(warm_gpu_lease);
            warm_stt.set_language(&warm_lang);
            if warm_threads > 0 {
                warm_stt.set_threads(warm_threads.clamp(1, 12));
            }
            warm_stt.set_command_timeout(std::time::Duration::from_secs(120));
            // 1 second of silence at 16 kHz — just enough to load the model.
            let silence = vec![0.0f32; 16_000];
            let started = std::time::Instant::now();
            match warm_stt.transcribe_samples(&silence, 16_000).await {
                Ok(_) => tracing::info!(
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "whisper warmup complete"
                ),
                Err(e) => tracing::warn!(error = %e, "whisper warmup failed (non-fatal)"),
            }
        });
    }

    pipeline
}

/// Build the v2 in-process streaming pipeline using the v1 CLI engines as
/// the underlying backends. Adds the streaming sentence playback + hard
/// barge-in concurrency model on top, without requiring native deps
/// (whisper-rs, sonata, webrtc-apm). When the user later enables
/// `voice-whisper-rs` / `voice-piper-rs` features at build time the swap
/// is local to this builder.
pub(crate) fn build_v2_pipeline(
    config: &KriaConfig,
    paths: &kria_core::platform::paths::KriaPaths,
    hw_tier: kria_core::platform::detect::HardwareTier,
) -> anyhow::Result<(
    Arc<kria_core::voice::v2::VoicePipelineV2>,
    tokio::sync::watch::Receiver<kria_core::voice::v2::VoiceSessionState>,
    tokio::sync::mpsc::UnboundedReceiver<kria_core::voice::v2::VoiceTelemetry>,
)> {
    use kria_core::voice::v2;

    let stt_model_path = resolve_model_file(paths, "stt", &config.voice.stt_model);
    let tts_voice_file = format!("{}.onnx", config.voice.tts_voice);
    let tts_model_path = resolve_model_file(paths, "piper", &tts_voice_file);

    let whisper_bin = which_binary("whisper-cpp").or_else(|| which_binary("main"));
    let piper_bin = which_binary("piper");

    let speech_gpu_lease = GpuLeaseManager::shared(
        std::time::Duration::from_secs(120),
        std::time::Duration::from_secs(15),
    );

    let mut stt = SpeechToText::new(stt_model_path, whisper_bin);
    stt.set_gpu_lease(speech_gpu_lease.clone());
    stt.set_language(&config.voice.language);
    if config.hardware.threads > 0 {
        stt.set_threads(config.hardware.threads.clamp(1, 12));
    }
    stt.set_command_timeout(std::time::Duration::from_secs(45));
    let mut tts = TextToSpeech::new(tts_model_path, piper_bin);
    tts.set_gpu_lease(speech_gpu_lease);

    let wake = if config.voice.wake_word.enabled {
        let wake_path = if config.voice.wake_word.model_path.is_empty() {
            resolve_model_file(paths, "wake", "hey_ria.onnx")
        } else {
            let p = std::path::PathBuf::from(&config.voice.wake_word.model_path);
            if p.is_absolute() {
                p
            } else {
                resolve_model_file(paths, "wake", p.to_string_lossy().as_ref())
            }
        };
        Some(v2::WakeWordDetector::try_load(
            wake_path,
            config.voice.wake_word.sensitivity,
            "hey ria",
            config.voice.wake_word.aliases.clone(),
        ))
    } else {
        None
    };

    let (pipeline, state_rx, telemetry_rx) =
        v2::build_v2_with_cli_engines(&config.voice, hw_tier, Arc::new(stt), Arc::new(tts), wake);
    Ok((pipeline, state_rx, telemetry_rx))
}

// ─── v2 continuous voice loop ─────────────────────────────────────────────
//
// Called from `start_voice` when the engine is "v2". Starts an `AudioCapture`
// thread, broadcasts chunks into the v2 pipeline's `run_turn` loop, and pumps
// telemetry events to the UI. Runs entirely in a background task; `stop_voice`
// signals it to exit via `voice_active = false` + `force_abort`.

#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_voice_v2_loop(
    v2: Arc<kria_core::voice::v2::VoicePipelineV2>,
    voice_active: Arc<std::sync::atomic::AtomicBool>,
    telemetry_slot: Arc<
        tokio::sync::Mutex<
            Option<tokio::sync::mpsc::UnboundedReceiver<kria_core::voice::v2::VoiceTelemetry>>,
        >,
    >,
    router: Arc<ModelRouter>,
    session_id_lock: Arc<RwLock<String>>,
    config: Arc<RwLock<KriaConfig>>,
    hw_info: Arc<HardwareInfo>,
    memory_store: Arc<dyn MemoryRuntime>,
    tool_registry: Arc<kria_core::tools::registry::ToolRegistry>,
    app: AppHandle,
) {
    use kria_core::voice::capture::AudioCapture;
    use kria_core::voice::v2::VoiceSessionState;

    // 1. Wire the AudioPlayer to the pipeline.
    {
        let cfg = config.read().await;
        let player = Arc::new(
            kria_core::voice::AudioPlayer::new()
                .with_output_device(Some(cfg.voice.speaker_device.clone()))
                .follow_system_default(cfg.voice.follow_system_default_speaker),
        );
        v2.set_audio_player(player).await;
    }

    // 2. Start AudioCapture and forward to a broadcast channel, gating chunks
    //    when the pipeline is Speaking so the mic doesn't pick up KRIA's voice.
    let (broadcast_tx, _) =
        tokio::sync::broadcast::channel::<kria_core::voice::capture::AudioChunk>(128);
    let broadcast_tx_arc = Arc::new(broadcast_tx);
    {
        let capture_cfg = config.read().await;
        let mic_device = capture_cfg.voice.mic_device.clone();
        let headphone_mode = capture_cfg.voice.mode.eq_ignore_ascii_case("headphone");
        let follow_mic = capture_cfg.voice.follow_system_default_mic
            || mic_device.trim().is_empty()
            || mic_device.eq_ignore_ascii_case("auto");
        let noise_mode = capture_cfg.voice.noise_suppression_mode.clone();
        let noise_gate_enabled = !noise_mode.eq_ignore_ascii_case("off");
        drop(capture_cfg);

        let bt = broadcast_tx_arc.clone();
        let v2_state = v2.subscribe_state();
        let voice_active_capture = voice_active.clone();
        let app_capture = app.clone();
        let _ = app.emit(
            "voice:io_mode",
            serde_json::json!({
                "mode": if headphone_mode { "headphone" } else { "half_duplex" },
                "headphone": headphone_mode
            }),
        );

        // Run capture on a blocking worker so the non-Send CPAL stream handle
        // never has to cross async task thread boundaries.
        tauri::async_runtime::spawn_blocking(move || {
            use kria_core::voice::audio_enhance::{EchoGate, SpectralGate};

            let capture = AudioCapture::new(16_000)
                .with_input_device(mic_device)
                .follow_system_default(follow_mic)
                .with_noise_suppression_mode(noise_mode);

            let (mut capture_rx, _capture_handle) = match capture.start() {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::error!("v2 audio capture failed to start: {e}");
                    let _ = app_capture.emit(
                        "voice:error",
                        serde_json::json!({ "error": format!("Mic start failed: {e}") }),
                    );
                    voice_active_capture.store(false, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
            };

            // Tier 2: Live audio enhancement pipeline
            let mut noise_gate = if noise_gate_enabled {
                SpectralGate::new()
            } else {
                SpectralGate::disabled()
            };
            let mut echo_gate = if headphone_mode {
                EchoGate::headphone_mode()
            } else {
                EchoGate::speaker_mode()
            };

            let mut frame_count = 0u64;
            loop {
                if !voice_active_capture.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }

                let mut chunk = match capture_rx.try_recv() {
                    Ok(c) => c,
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                };

                frame_count += 1;
                if frame_count % 50 == 1 {
                    tracing::debug!(frame_count, chunk_samples = chunk.samples.len(), "forwarder: received chunk");
                }

                // Tier 2: Echo gate — suppress mic during/after TTS playback
                let st = *v2_state.borrow();
                let is_playing = matches!(
                    st,
                    VoiceSessionState::Speaking | VoiceSessionState::Thinking
                );
                if is_playing && !echo_gate.is_suppressing() {
                    echo_gate.playback_started();
                } else if !is_playing && echo_gate.is_suppressing() {
                    echo_gate.playback_stopped();
                }
                echo_gate.process(&mut chunk.samples);

                // In half-duplex mode, still drop chunks during active playback
                // (echo gate provides soft suppression, this is the hard gate)
                if !headphone_mode && is_playing {
                    continue;
                }

                // Tier 2: Spectral noise gate — suppress fan/room noise
                noise_gate.process(&mut chunk.samples);

                if bt.send(chunk).is_err() {
                    tracing::debug!("v2 capture forwarder: no active turn subscriber yet");
                    continue;
                }
            }
            tracing::info!(total_frames = frame_count, "v2 capture forwarder exited");
        });
    }

    // 3. Pump telemetry events → Tauri UI events.
    {
        let mut rx_opt = telemetry_slot.lock().await.take();
        if let Some(mut rx) = rx_opt.take() {
            let app_h = app.clone();
            let va = voice_active.clone();
            let slot = telemetry_slot.clone();
            tokio::spawn(async move {
                while let Some(ev) = rx.recv().await {
                    if let kria_core::voice::v2::VoiceTelemetry::Final { text, .. } = &ev {
                        let preview = text.chars().take(120).collect::<String>();
                        let _ = app_h.emit(
                            "voice:debug",
                            serde_json::json!({
                                "stage": "stt_final",
                                "text_len": text.chars().count(),
                                "text_preview": preview
                            }),
                        );
                    }
                    let (tauri_event, payload) = v2_telemetry_to_event(&ev);
                    let _ = app_h.emit(tauri_event, payload);
                    // Also forward raw telemetry for debug/UI extensions.
                    if let Ok(raw) = serde_json::to_value(&ev) {
                        let _ = app_h.emit("voice:v2_telemetry", raw);
                    }
                    if !va.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                }
                *slot.lock().await = None;
            });
        }
    }

    let _ = app.emit("voice:state", serde_json::json!({ "state": "listening" }));

    // 4. Main run_turn loop. Each call to run_turn executes one full
    //    wake → capture → STT → LLM → TTS cycle.
    let v2_loop = v2.clone();
    let voice_active_loop = voice_active.clone();
    let router_loop = router.clone();
    let config_loop = config.clone();
    let session_id_loop = session_id_lock.clone();
    let memory_store_loop = memory_store.clone();
    let tool_registry_loop = tool_registry.clone();
    let hw_info_loop = hw_info.clone();
    let app_loop = app.clone();
    let bt_loop = broadcast_tx_arc.clone();

    tauri::async_runtime::spawn(async move {
        let mut turn_index: u64 = 0;
        while voice_active_loop.load(std::sync::atomic::Ordering::Relaxed) {
            turn_index = turn_index.saturating_add(1);
            tracing::info!(turn_index, "voice v2: starting turn loop");
            let _ = app_loop.emit(
                "voice:debug",
                serde_json::json!({ "stage": "turn_start", "turn": turn_index }),
            );
            // Transition to Listening before each turn.
            v2_loop.force_wake("auto");

            let audio_rx = bt_loop.subscribe();
            let router_turn = router_loop.clone();
            let config_turn = config_loop.clone();
            let session_id_turn = session_id_loop.clone();
            let memory_turn = memory_store_loop.clone();
            let tool_reg_turn = tool_registry_loop.clone();
            let hw_turn = hw_info_loop.clone();
            let transcript_only_mode = env_truthy("KRIA_VOICE_TRANSCRIPT_ONLY");

            let app_for_llm = app_loop.clone();
            let turn_for_llm = turn_index;
            let llm = move |user_text: String| async move {
                let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);
                let user_text_len = user_text.chars().count();
                if transcript_only_mode {
                    tracing::info!(
                        turn = turn_for_llm,
                        user_text_len,
                        "voice v2: transcript-only debug mode enabled (LLM bypass)"
                    );
                    let _ = app_for_llm.emit(
                        "voice:debug",
                        serde_json::json!({
                            "stage": "llm_bypass_transcript_only",
                            "turn": turn_for_llm,
                            "text_len": user_text_len
                        }),
                    );
                    let _ = tx.send(format!("(Transcript only) {}", user_text)).await;
                    return rx;
                }
                tracing::info!(
                    turn = turn_for_llm,
                    user_text_len,
                    "voice v2: invoking LLM route"
                );
                let _ = app_for_llm.emit(
                    "voice:debug",
                    serde_json::json!({
                        "stage": "llm_route_start",
                        "turn": turn_for_llm,
                        "text_len": user_text_len
                    }),
                );
                let backend = match tokio::time::timeout(
                    std::time::Duration::from_secs(12),
                    router_turn.route("voice"),
                )
                .await
                {
                    Ok(Some(b)) => b,
                    Ok(None) => {
                        tracing::warn!(turn = turn_for_llm, "voice v2: no backend routed for voice");
                        let _ = app_for_llm.emit(
                            "voice:debug",
                            serde_json::json!({ "stage": "llm_route_none", "turn": turn_for_llm }),
                        );
                        let _ = tx
                            .send("(No LLM backend — check model config)".into())
                            .await;
                        return rx;
                    }
                    Err(_) => {
                        tracing::warn!(turn = turn_for_llm, "voice v2: backend routing timed out");
                        let _ = app_for_llm.emit(
                            "voice:debug",
                            serde_json::json!({ "stage": "llm_route_timeout", "turn": turn_for_llm }),
                        );
                        let _ = tx.send("(LLM routing timeout)".into()).await;
                        return rx;
                    }
                };
                tracing::info!(turn = turn_for_llm, "voice v2: backend routed");
                let _ = app_for_llm.emit(
                    "voice:debug",
                    serde_json::json!({ "stage": "llm_route_ok", "turn": turn_for_llm }),
                );
                // Build messages with system prompt + recent context (mirrors v1 flow).
                let session_id = session_id_turn.read().await.clone();
                let cfg = config_turn.read().await;
                let hw_tier = hw_turn.tier.as_str();
                let mut tool_defs = tool_reg_turn.list_for_tier(hw_tier);
                // Disable search tools for voice interactions to prevent aggressive searching
                tool_defs.retain(|t| !t.name.contains("search") && t.name != "search_news");
                let tool_descriptions = build_tool_descriptions_for_prompt(&tool_defs);
                let user_name = memory_turn
                    .get_preference("user_name")
                    .unwrap_or(None)
                    .unwrap_or_else(|| "User".to_string());
                let memory_context = match memory_turn.search_facts(&user_text, 5) {
                    Ok(facts) if !facts.is_empty() => {
                        let lines: Vec<String> =
                            facts.iter().map(|f| format!("- {}", f.text)).collect();
                        format!("Known facts:\n{}", lines.join("\n"))
                    }
                    _ => String::new(),
                };
                let system_prompt = kria_core::agent::prompts::build_system_prompt(
                    &tool_descriptions,
                    &user_name,
                    std::env::consts::OS,
                    hw_tier,
                    "auto",
                    &memory_context,
                );
                drop(cfg);
                let recent_turns = memory_turn
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
                    content: user_text,
                    name: None,
                    images: None,
                });
                let app_for_stream = app_for_llm.clone();
                let turn_for_stream = turn_for_llm;
                tokio::spawn(async move {
                    use futures::StreamExt;
                    let stream_started = std::time::Instant::now();
                    tracing::info!(
                        turn = turn_for_stream,
                        message_count = messages.len(),
                        "voice v2: requesting chat_stream"
                    );
                    let _ = app_for_stream.emit(
                        "voice:debug",
                        serde_json::json!({
                            "stage": "llm_stream_request",
                            "turn": turn_for_stream,
                            "message_count": messages.len()
                        }),
                    );
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(20),
                        backend.chat_stream(&messages, None, 0.7, 512),
                    )
                    .await
                    {
                        Ok(Ok(mut stream)) => {
                            let mut seen_token = false;
                            let mut token_count: usize = 0;
                            loop {
                                let wait = if seen_token { 20 } else { 15 };
                                match tokio::time::timeout(
                                    std::time::Duration::from_secs(wait),
                                    stream.next(),
                                )
                                .await
                                {
                                    Ok(Some(tok)) => {
                                        token_count = token_count.saturating_add(1);
                                        if !seen_token {
                                            tracing::info!(
                                                turn = turn_for_stream,
                                                first_token_ms = stream_started.elapsed().as_millis() as u64,
                                                "voice v2: llm first token"
                                            );
                                            let _ = app_for_stream.emit(
                                                "voice:debug",
                                                serde_json::json!({
                                                    "stage": "llm_first_token",
                                                    "turn": turn_for_stream,
                                                    "latency_ms": stream_started.elapsed().as_millis() as u64
                                                }),
                                            );
                                        }
                                        seen_token = true;
                                        if tx.send(tok).await.is_err() {
                                            break;
                                        }
                                    }
                                    Ok(None) => {
                                        tracing::info!(
                                            turn = turn_for_stream,
                                            token_count,
                                            total_ms = stream_started.elapsed().as_millis() as u64,
                                            "voice v2: llm stream completed"
                                        );
                                        let _ = app_for_stream.emit(
                                            "voice:debug",
                                            serde_json::json!({
                                                "stage": "llm_stream_done",
                                                "turn": turn_for_stream,
                                                "token_count": token_count
                                            }),
                                        );
                                        break;
                                    }
                                    Err(_) => {
                                        tracing::warn!(
                                            turn = turn_for_stream,
                                            token_count,
                                            "voice v2: llm stream stalled waiting for token"
                                        );
                                        let _ = app_for_stream.emit(
                                            "voice:debug",
                                            serde_json::json!({
                                                "stage": "llm_stream_token_timeout",
                                                "turn": turn_for_stream,
                                                "token_count": token_count
                                            }),
                                        );
                                        let _ = tx
                                            .send("(LLM response timeout — please try again)".into())
                                            .await;
                                        break;
                                    }
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(turn = turn_for_stream, error = %e, "voice v2: chat_stream failed");
                            let _ = app_for_stream.emit(
                                "voice:debug",
                                serde_json::json!({
                                    "stage": "llm_stream_error",
                                    "turn": turn_for_stream,
                                    "error": e.to_string()
                                }),
                            );
                            let _ = tx.send(format!("(LLM error: {e})")).await;
                        }
                        Err(_) => {
                            tracing::warn!(turn = turn_for_stream, "voice v2: chat_stream startup timeout");
                            let _ = app_for_stream.emit(
                                "voice:debug",
                                serde_json::json!({ "stage": "llm_stream_start_timeout", "turn": turn_for_stream }),
                            );
                            let _ = tx
                                .send("(LLM startup timeout — model server busy)".into())
                                .await;
                        }
                    }
                });
                rx
            };

            if let Err(e) = v2_loop.clone().run_turn(audio_rx, llm).await {
                let err_text = e.to_string();
                let stopped = !voice_active_loop.load(std::sync::atomic::Ordering::Relaxed);
                if stopped
                    && (err_text.contains("turn cancelled before transcription")
                        || err_text.contains("stt stream cancelled"))
                {
                    tracing::info!(turn = turn_index, error = %err_text, "voice v2: suppressing expected cancel error after stop");
                } else {
                    tracing::warn!(turn = turn_index, error = %err_text, "v2 run_turn error");
                    let _ = app_loop.emit("voice:error", serde_json::json!({ "error": err_text }));
                }
            }

            // Post-turn silence gap: prevents the next turn's STT from picking
            // up residual echo from the speaker (≥300 ms is enough for room echo).
            if voice_active_loop.load(std::sync::atomic::Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
        }
        let _ = app_loop.emit("voice:state", serde_json::json!({ "state": "idle" }));
        tracing::info!("v2 voice loop exited");
    });
}

/// Map a `VoiceTelemetry` variant to the canonical Tauri event name + JSON
/// payload that the existing UI listeners already handle.
fn v2_telemetry_to_event(
    ev: &kria_core::voice::v2::VoiceTelemetry,
) -> (&'static str, serde_json::Value) {
    use kria_core::voice::v2::{VoiceSessionState, VoiceTelemetry};
    match ev {
        VoiceTelemetry::State { state } => {
            let s = match state {
                VoiceSessionState::Sleeping => "idle",
                VoiceSessionState::Listening => "listening",
                VoiceSessionState::Transcribing | VoiceSessionState::Thinking => "processing",
                VoiceSessionState::Speaking => "speaking",
                VoiceSessionState::BargeIn => "listening",
            };
            ("voice:state", serde_json::json!({ "state": s }))
        }
        VoiceTelemetry::Partial { text, engine, seq } => (
            "voice:partial_transcript",
            serde_json::json!({ "text": text, "confidence": 0.7, "language": "auto", "stability": 0.5, "engine": engine, "seq": seq }),
        ),
        VoiceTelemetry::Final {
            text,
            confidence,
            engine,
        } => (
            "voice:transcript",
            serde_json::json!({ "text": text, "confidence": confidence, "language": "auto", "stability": 1.0, "engine": engine }),
        ),
        VoiceTelemetry::Error { message } => {
            ("voice:error", serde_json::json!({ "error": message }))
        }
        VoiceTelemetry::BusyRejected { entrypoint, state } => (
            "voice:busy",
            serde_json::json!({
                "entrypoint": entrypoint,
                "state": format!("{state:?}").to_lowercase(),
                "message": "Voice runtime is busy with an active turn"
            }),
        ),
        VoiceTelemetry::PlaybackFailure { message } => (
            "voice:playback_failure",
            serde_json::json!({ "error": message }),
        ),
        VoiceTelemetry::PlaybackRecovered => (
            "voice:playback_recovered",
            serde_json::json!({ "ok": true }),
        ),
        VoiceTelemetry::Interruption { reason } => (
            "voice:interruption",
            serde_json::json!({ "reason": reason }),
        ),
        _ => (
            "voice:v2_telemetry",
            serde_json::to_value(ev).unwrap_or_default(),
        ),
    }
}
