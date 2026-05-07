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
    // Log v2 engine selection — the v2 stack is scaffolded under
    // `kria_core::voice::v2` (sentence splitter, post-edit, playback sink,
    // AEC + wake skeletons, FSM). Running v2 end-to-end is gated behind
    // additional cargo features (`voice-whisper-rs`, `voice-piper-rs`, …)
    // and is not yet the default runtime path. Until then we always build
    // the v1 pipeline; v2 is exercised through unit tests + the
    // `voice_v2_status` command.
    let engine = config.voice.engine.to_ascii_lowercase();
    if engine == "v2" {
        tracing::warn!(
            "voice.engine = \"v2\" requested; v2 stack is scaffold-only in this build, \
             falling back to v1. Enable the relevant cargo features and complete the \
             VoicePipelineV2 runtime loop to switch over."
        );
    } else if engine != "v1" && !engine.is_empty() {
        tracing::warn!(engine = %engine, "unknown voice.engine value; using v1");
    }

    let stt_model_path = paths.models_dir.join("stt").join(&config.voice.stt_model);
    let tts_voice_file = format!("{}.onnx", config.voice.tts_voice);
    let tts_model_path = paths.models_dir.join("piper").join(&tts_voice_file);

    // Resolve + log wake-word model wiring so v2 readiness is visible even
    // while the runtime path is still v1. Construction is cheap (no model
    // load when disabled) and any failure falls back silently.
    if config.voice.wake_word.enabled {
        let wake_dir = paths.models_dir.join("wake");
        let wake_path = if config.voice.wake_word.model_path.is_empty() {
            wake_dir.join("hey_ria.onnx")
        } else {
            let p = std::path::PathBuf::from(&config.voice.wake_word.model_path);
            if p.is_absolute() {
                p
            } else {
                wake_dir.join(p.file_name().unwrap_or_default())
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
    let vad_model_path = paths.models_dir.join("vad").join("silero_vad.onnx");

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

    let stt_model_path = paths.models_dir.join("stt").join(&config.voice.stt_model);
    let tts_voice_file = format!("{}.onnx", config.voice.tts_voice);
    let tts_model_path = paths.models_dir.join("piper").join(&tts_voice_file);

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
        let wake_dir = paths.models_dir.join("wake");
        let wake_path = if config.voice.wake_word.model_path.is_empty() {
            wake_dir.join("hey_ria.onnx")
        } else {
            let p = std::path::PathBuf::from(&config.voice.wake_word.model_path);
            if p.is_absolute() {
                p
            } else {
                wake_dir.join(p.file_name().unwrap_or_default())
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
        let follow_mic = capture_cfg.voice.follow_system_default_mic
            || mic_device.trim().is_empty()
            || mic_device.eq_ignore_ascii_case("auto");
        let noise_mode = capture_cfg.voice.noise_suppression_mode.clone();
        drop(capture_cfg);

        let capture = AudioCapture::new(16_000)
            .with_input_device(mic_device)
            .follow_system_default(follow_mic)
            .with_noise_suppression_mode(noise_mode);

        let (mut capture_rx, _capture_handle) = match capture.start() {
            Ok(pair) => pair,
            Err(e) => {
                tracing::error!("v2 audio capture failed to start: {e}");
                let _ = app.emit(
                    "voice:error",
                    serde_json::json!({ "error": format!("Mic start failed: {e}") }),
                );
                voice_active.store(false, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        };

        let bt = broadcast_tx_arc.clone();
        let v2_state = v2.subscribe_state();
        // Forward mpsc → broadcast, gating when Speaking/Thinking/BargeIn to
        // prevent recording KRIA's own TTS output (echo cancellation gate).
        tokio::spawn(async move {
            // Keep capture_handle alive for the duration of this task.
            // (_capture_handle is moved here to prevent premature drop.)
            while let Some(chunk) = capture_rx.recv().await {
                let st = *v2_state.borrow();
                if matches!(
                    st,
                    VoiceSessionState::Speaking
                        | VoiceSessionState::Thinking
                        | VoiceSessionState::BargeIn
                ) {
                    // Discard — KRIA is generating/speaking; skip to prevent echo.
                    continue;
                }
                if bt.send(chunk).is_err() {
                    break;
                }
            }
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
        while voice_active_loop.load(std::sync::atomic::Ordering::Relaxed) {
            // Transition to Listening before each turn.
            v2_loop.force_wake("auto");

            let audio_rx = bt_loop.subscribe();
            let router_turn = router_loop.clone();
            let config_turn = config_loop.clone();
            let session_id_turn = session_id_loop.clone();
            let memory_turn = memory_store_loop.clone();
            let tool_reg_turn = tool_registry_loop.clone();
            let hw_turn = hw_info_loop.clone();

            let llm = move |user_text: String| async move {
                let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);
                let backend = match router_turn.route("voice").await {
                    Some(b) => b,
                    None => {
                        let _ = tx
                            .send("(No LLM backend — check model config)".into())
                            .await;
                        return rx;
                    }
                };
                // Build messages with system prompt + recent context (mirrors v1 flow).
                let session_id = session_id_turn.read().await.clone();
                let cfg = config_turn.read().await;
                let hw_tier = hw_turn.tier.as_str();
                let tool_defs = tool_reg_turn.list_for_tier(hw_tier);
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
                tokio::spawn(async move {
                    use futures::StreamExt;
                    match backend.chat_stream(&messages, None, 0.7, 512).await {
                        Ok(mut stream) => {
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

            if let Err(e) = v2_loop.clone().run_turn(audio_rx, llm).await {
                tracing::warn!("v2 run_turn error: {e}");
                let _ = app_loop.emit("voice:error", serde_json::json!({ "error": e.to_string() }));
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
        VoiceTelemetry::Partial { text, engine } => (
            "voice:partial_transcript",
            serde_json::json!({ "text": text, "confidence": 0.7, "language": "auto", "stability": 0.5, "engine": engine }),
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
        _ => (
            "voice:v2_telemetry",
            serde_json::to_value(ev).unwrap_or_default(),
        ),
    }
}
