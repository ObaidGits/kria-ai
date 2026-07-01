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

    let speech_gpu_lease = kria_core::resource::gpu_lease::global_gpu_lease();

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
    tokio::sync::broadcast::Receiver<kria_core::voice::v2::VoiceTelemetry>,
)> {
    use kria_core::voice::v2;

    let stt_model_path = resolve_model_file(paths, "stt", &config.voice.stt_model);
    let tts_voice_file = format!("{}.onnx", config.voice.tts_voice);
    let tts_model_path = resolve_model_file(paths, "piper", &tts_voice_file);

    let whisper_bin = which_binary("whisper-cpp").or_else(|| which_binary("main"));
    let piper_bin = which_binary("piper");

    // Issue 4 fix: do NOT attach a GPU lease to the voice speech path. The
    // speech `GpuLeaseManager` is a per-build instance (not shared with image
    // generation), and its recovery state machine needs a `ResourceTelemetry`
    // source that the voice path never configures. The first guard release
    // therefore goes Recovering → (15s) → permanently Degraded, after which
    // every `acquire_speech_lease` fails and TTS stops playing (observed in
    // logs as repeated "tts synth failed: speech GPU lease unavailable").
    // STT and TTS are sequential within a turn, so no real arbitration is lost.
    let mut stt = SpeechToText::new(stt_model_path, whisper_bin);
    stt.set_language(&config.voice.language);
    if config.hardware.threads > 0 {
        stt.set_threads(config.hardware.threads.clamp(1, 12));
    }
    stt.set_command_timeout(std::time::Duration::from_secs(45));
    let tts = TextToSpeech::new(tts_model_path, piper_bin);

    // Issue 3 fix: build the wake detector when wake is enabled OR the mode is
    // "wake_word" (previously the mode silently fell back to continuous unless
    // the separate wake_word.enabled flag was also set — a config foot-gun).
    let wake_requested =
        config.voice.wake_word.enabled || config.voice.mode.eq_ignore_ascii_case("wake_word");
    let wake = if wake_requested {
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
    // Issue 1/2: wire Silero VAD for robust endpoint detection.
    let vad_path = resolve_model_file(paths, "vad", "silero_vad.onnx");
    pipeline.set_vad_model_path(Some(vad_path));
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
    _telemetry_slot: Arc<
        tokio::sync::Mutex<
            Option<tokio::sync::broadcast::Receiver<kria_core::voice::v2::VoiceTelemetry>>,
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

    // Issue 3 fix: terminate any previous session before starting a new one.
    // `force_abort` unblocks a stuck in-flight turn (releasing the turn guard);
    // the epoch bump makes the prior loop + capture forwarder exit on their
    // next epoch check. Guarantees exactly one live session loop.
    v2.force_abort().await;
    let session_epoch = v2.begin_new_session();
    tracing::info!(session_epoch, "voice v2 loop: starting new session");

    // Wave A3.2: warm the faster-whisper STT sidecar at session start so the
    // first utterance does not pay the model cold-load cost. Skipped when the
    // user explicitly selected the in-process whisper-rs rollback engine.
    {
        let cfg = config.read().await;
        let eng = cfg.voice.stt_engine.trim().to_ascii_lowercase();
        let is_whisper_rs = matches!(
            eng.as_str(),
            "whisper-rs" | "whisper-rs-cuda" | "whisper-cuda" | "whisper-rs-vulkan"
        );
        drop(cfg);
        if !is_whisper_rs {
            tokio::spawn(async {
                kria_core::voice::v2::stt_sidecar::warm_up().await;
            });
        }
        // Wave 5: warm Kokoro at session start when selected.
        let tts_eng = {
            let cfg = config.read().await;
            cfg.voice.tts_engine.trim().to_ascii_lowercase()
        };
        if tts_eng == "kokoro" {
            tokio::spawn(async {
                kria_core::voice::v2::tts_sidecar::warm_up().await;
            });
        }
    }

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
        let v2_fwd = v2.clone();
        let fwd_epoch = session_epoch;
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
                if !voice_active_capture.load(std::sync::atomic::Ordering::Relaxed)
                    || v2_fwd.current_session() != fwd_epoch
                {
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
                    tracing::debug!(
                        frame_count,
                        chunk_samples = chunk.samples.len(),
                        "forwarder: received chunk"
                    );
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

                // Wave 8.4: emit a throttled mic-level (RMS, 0..1) so the UI can
                // render a live input meter. Measured from the post-gate samples;
                // ~10 Hz to keep event volume low.
                if frame_count % 5 == 0 && !chunk.samples.is_empty() {
                    let rms = (chunk.samples.iter().map(|s| s * s).sum::<f32>()
                        / chunk.samples.len() as f32)
                        .sqrt();
                    // Map RMS to a 0..1 display level (speech ~0.02–0.2).
                    let level = (rms * 8.0).clamp(0.0, 1.0);
                    let _ = app_capture.emit(
                        "voice:mic_level",
                        serde_json::json!({ "level": level, "rms": rms }),
                    );
                }

                if bt.send(chunk).is_err() {
                    tracing::debug!("v2 capture forwarder: no active turn subscriber yet");
                    continue;
                }
            }
            tracing::info!(total_frames = frame_count, "v2 capture forwarder exited");
        });
    }

    // 3. Pump telemetry events → Tauri UI events. Subscribe a FRESH receiver
    //    each session (broadcast) so restarts keep receiving state (Issue 3).
    //    The pump exits when voice stops or the session epoch advances.
    {
        // Resolved engine labels for diagnostics (Wave 7 observability).
        let (stt_engine_label, tts_engine_label) = {
            let cfg = config.read().await;
            let stt = match cfg.voice.stt_engine.trim().to_ascii_lowercase().as_str() {
                "" | "auto" | "faster-whisper" | "faster_whisper" | "fasterwhisper" | "fw"
                | "sidecar" => "faster-whisper".to_string(),
                other => other.to_string(),
            };
            let tts = match cfg.voice.tts_engine.trim().to_ascii_lowercase().as_str() {
                "kokoro" => "kokoro".to_string(),
                "" | "auto" | "piper-rs" | "piper-cli" => "piper".to_string(),
                other => other.to_string(),
            };
            (stt, tts)
        };
        let mut rx = v2.subscribe_telemetry();
        let app_h = app.clone();
        let va = voice_active.clone();
        let v2_epoch = v2.clone();
        let my_epoch = session_epoch;
        tokio::spawn(async move {
            use kria_core::voice::turn_diagnostics::{
                record as record_turn, TurnOutcome, VoiceTurnRecord,
            };
            use kria_core::voice::v2::{VoiceSessionState, VoiceTelemetry};
            // Per-turn accumulator for structured diagnostics.
            let mut turn_seq: u64 = 0;
            let mut last_final_text: Option<String> = None;
            loop {
                let ev = match rx.recv().await {
                    Ok(ev) => ev,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                // ── Structured turn diagnostics (Wave 7) ──────────────────
                match &ev {
                    VoiceTelemetry::State {
                        state: VoiceSessionState::Listening,
                    } => {
                        // New turn begins; clear the per-turn accumulator.
                        last_final_text = None;
                    }
                    VoiceTelemetry::Final { text, .. } => {
                        last_final_text = Some(text.clone());
                    }
                    VoiceTelemetry::Metrics(m) => {
                        // Turn finalised with measured timings. Empty final =>
                        // EmptyTranscript outcome; otherwise Completed.
                        turn_seq += 1;
                        let empty = last_final_text
                            .as_ref()
                            .map(|t| t.trim().is_empty())
                            .unwrap_or(true);
                        let outcome = if empty {
                            TurnOutcome::EmptyTranscript
                        } else {
                            TurnOutcome::Completed
                        };
                        let mut rec = VoiceTurnRecord::from_metrics(turn_seq, outcome, m);
                        rec.stt_engine = Some(stt_engine_label.clone());
                        rec.tts_engine = Some(tts_engine_label.clone());
                        rec.transcript_len = last_final_text.as_ref().map(|t| t.chars().count());
                        record_turn(rec);
                        last_final_text = None;
                    }
                    VoiceTelemetry::Error { message } => {
                        turn_seq += 1;
                        let lower = message.to_ascii_lowercase();
                        let outcome = if lower.contains("max duration")
                            || lower.contains("watchdog")
                            || lower.contains("timeout")
                        {
                            TurnOutcome::Timeout
                        } else {
                            TurnOutcome::Error
                        };
                        let mut rec =
                            VoiceTurnRecord::from_error(turn_seq, outcome, message.clone());
                        rec.stt_engine = Some(stt_engine_label.clone());
                        rec.tts_engine = Some(tts_engine_label.clone());
                        record_turn(rec);
                        last_final_text = None;
                    }
                    VoiceTelemetry::BusyRejected { .. } => {
                        turn_seq += 1;
                        record_turn(VoiceTurnRecord::from_error(
                            turn_seq,
                            TurnOutcome::Busy,
                            "turn rejected: another turn active",
                        ));
                    }
                    _ => {}
                }

                if let VoiceTelemetry::Final { text, .. } = &ev {
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
                if let Ok(raw) = serde_json::to_value(&ev) {
                    let _ = app_h.emit("voice:v2_telemetry", raw);
                }
                if !va.load(std::sync::atomic::Ordering::Relaxed)
                    || v2_epoch.current_session() != my_epoch
                {
                    break;
                }
            }
        });
    }

    // ─── Wave 4.3/4.4 + 7.2: barge-in watcher (live config hot-reload) ───
    // Runs a lightweight energy VAD over the captured audio broadcast; when
    // sustained speech (≥ min_speech_ms) is detected WHILE the pipeline is
    // Speaking, it requests a barge-in (cancels the turn → stops TTS/playback).
    // The watcher re-reads `voice.barge_in.*` + `energy_threshold` from the
    // live config each Speaking episode, so toggling barge-in or its thresholds
    // takes effect at the next turn boundary without a restart (Wave 7.2).
    // Half-duplex suppression is handled upstream (no voice-barge-in claim
    // without headphone/AEC — Req 5.1/5.3).
    {
        let mut brx = broadcast_tx_arc.subscribe();
        let v2_bi = v2.clone();
        let va_bi = voice_active.clone();
        let epoch_bi = session_epoch;
        let cfg_bi = config.clone();
        tokio::spawn(async move {
            use kria_core::voice::v2::VoiceSessionState;
            let mut voiced_ms: u64 = 0;
            let mut was_speaking = false;
            // Cached live config (refreshed on each Speaking entry).
            let mut barge_enabled = true;
            let mut min_speech_ms: u64 = 180;
            let mut energy_threshold: f32 = 0.02;
            loop {
                if !va_bi.load(std::sync::atomic::Ordering::Relaxed)
                    || v2_bi.current_session() != epoch_bi
                {
                    break;
                }
                match brx.recv().await {
                    Ok(chunk) => {
                        let speaking = v2_bi.state() == VoiceSessionState::Speaking;
                        if !speaking {
                            voiced_ms = 0;
                            was_speaking = false;
                            continue;
                        }
                        // Refresh config at the Speaking-episode boundary (7.2).
                        if !was_speaking {
                            was_speaking = true;
                            voiced_ms = 0;
                            let cfg = cfg_bi.read().await;
                            barge_enabled = cfg.voice.barge_in.enabled;
                            min_speech_ms = cfg.voice.barge_in.min_speech_ms.max(1);
                            energy_threshold = if cfg.voice.energy_threshold > 0.0 {
                                cfg.voice.energy_threshold
                            } else {
                                0.02
                            };
                        }
                        if !barge_enabled {
                            continue;
                        }
                        let sr = chunk.sample_rate.max(1) as u64;
                        let ch = chunk.channels.max(1) as u64;
                        let chunk_ms = (((chunk.samples.len() as u64) * 1000) / (sr * ch)).max(1);
                        let rms = if chunk.samples.is_empty() {
                            0.0
                        } else {
                            (chunk.samples.iter().map(|s| s * s).sum::<f32>()
                                / chunk.samples.len() as f32)
                                .sqrt()
                        };
                        if rms >= energy_threshold {
                            voiced_ms = voiced_ms.saturating_add(chunk_ms);
                        } else {
                            voiced_ms = 0;
                        }
                        if voiced_ms >= min_speech_ms && v2_bi.request_barge_in().await {
                            tracing::info!(
                                voiced_ms,
                                "barge-in watcher: sustained speech during Speaking → barge-in"
                            );
                            voiced_ms = 0;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            tracing::debug!("barge-in watcher exited");
        });
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
        // ─── Wave 3: voice mode dispatch ─────────────────────────────────
        // One loop, three modes. `continuous` auto re-arms each turn;
        // `push_to_talk` runs exactly one turn per activation; `wake_word`
        // blocks each turn until the wake phrase fires (no STT runs while
        // waiting → low idle CPU).
        let voice_mode = {
            let cfg = config_loop.read().await;
            cfg.voice.mode.trim().to_ascii_lowercase()
        };
        tracing::info!(mode = %voice_mode, "voice v2 loop: starting");
        let mut voice_mode = voice_mode; // mutable: hot-reloaded each turn (Wave 7.2)

        let mut wake_rx: Option<
            tokio::sync::mpsc::UnboundedReceiver<kria_core::voice::v2::WakeWordEvent>,
        > = None;
        if voice_mode == "wake_word" {
            match v2_loop.wake.clone() {
                Some(wake) if wake.is_active() => {
                    let (wtx, wrx) = tokio::sync::mpsc::unbounded_channel();
                    wake.spawn(bt_loop.subscribe(), wtx);
                    wake_rx = Some(wrx);
                    tracing::info!("voice v2 loop: wake-word gating active");
                }
                _ => {
                    tracing::warn!(
                        "voice v2 loop: wake_word mode requested but detector is inactive \
                         (model missing or wake disabled); falling back to continuous"
                    );
                }
            }
        }

        let mut turn_index: u64 = 0;
        while voice_active_loop.load(std::sync::atomic::Ordering::Relaxed)
            && v2_loop.current_session() == session_epoch
        {
            // Wave 7.2: turn-boundary mode hot reload. Re-read the configured
            // mode each turn; (de)activate wake gating without a restart.
            {
                let new_mode = {
                    let cfg = config_loop.read().await;
                    cfg.voice.mode.trim().to_ascii_lowercase()
                };
                if new_mode != voice_mode {
                    tracing::info!(from = %voice_mode, to = %new_mode, "voice v2 loop: mode hot-reloaded at turn boundary");
                    voice_mode = new_mode;
                }
                let want_wake = voice_mode == "wake_word";
                if want_wake && wake_rx.is_none() {
                    match v2_loop.wake.clone() {
                        Some(wake) if wake.is_active() => {
                            let (wtx, wrx) = tokio::sync::mpsc::unbounded_channel();
                            wake.spawn(bt_loop.subscribe(), wtx);
                            wake_rx = Some(wrx);
                            tracing::info!("voice v2 loop: wake gating activated (hot reload)");
                        }
                        _ => {
                            tracing::warn!(
                                "voice v2 loop: wake_word mode requested but detector inactive; continuing without wake gate"
                            );
                        }
                    }
                } else if !want_wake && wake_rx.is_some() {
                    wake_rx = None;
                    tracing::info!("voice v2 loop: wake gating deactivated (hot reload)");
                }
            }

            // Wake gate: block until the phrase is detected. While waiting, no
            // capture turn is started, so STT stays idle.
            if let Some(rx) = wake_rx.as_mut() {
                tracing::info!("voice v2 loop: awaiting wake word…");
                // Wave 8: surface a distinct "waiting for wake" UX state.
                let _ = app_loop.emit(
                    "voice:state",
                    serde_json::json!({ "state": "wake_listening" }),
                );
                match rx.recv().await {
                    Some(ev) => {
                        tracing::info!(score = ev.score, source = %ev.source, "voice v2 loop: wake detected");
                        // Wave 8: dedicated wake event for the UI (flash + chime hook).
                        let _ = app_loop.emit(
                            "voice:wake",
                            serde_json::json!({ "score": ev.score, "source": ev.source }),
                        );
                    }
                    None => {
                        tracing::warn!("voice v2 loop: wake channel closed; exiting loop");
                        break;
                    }
                }
                if !voice_active_loop.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
            }

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
                        tracing::warn!(
                            turn = turn_for_llm,
                            "voice v2: no backend routed for voice"
                        );
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
                let hw_tier = hw_turn.tier.as_str();
                let _ = (&config_turn, &tool_reg_turn, hw_tier); // reserved for future voice tool-calling
                let user_name = memory_turn
                    .get_preference("user_name")
                    .unwrap_or(None)
                    .unwrap_or_else(|| "User".to_string());
                let memory_context = match memory_turn.search_facts(&user_text, 5) {
                    Ok(facts) if !facts.is_empty() => {
                        let lines: Vec<String> =
                            facts.iter().map(|f| format!("- {}", f.text)).collect();
                        lines.join("\n")
                    }
                    _ => String::new(),
                };
                // Issue 7/8 fix: voice uses a SLIM conversational prompt with NO
                // tool catalog. The v2 voice path streams raw LLM text straight to
                // TTS (no tool execution), so the full tool list only bloated the
                // context (→ overflow → stuck "Thinking") and risked the model
                // speaking tool-call JSON. Keep it short and speech-friendly.
                let system_prompt = {
                    let mut p = format!(
                        "You are KRIA, a helpful voice assistant speaking with {user_name}. \
                         Reply in a natural, concise, conversational style meant to be heard \
                         aloud. Do not use markdown, code blocks, lists, emojis, or any \
                         tool/function-call syntax. Prefer one or two short sentences unless \
                         more detail is requested."
                    );
                    if !memory_context.is_empty() {
                        p.push_str("\n\nKnown facts about the user:\n");
                        p.push_str(&memory_context);
                    }
                    p
                };
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
                            let mut full_response = String::new();
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
                                                first_token_ms =
                                                    stream_started.elapsed().as_millis() as u64,
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
                                        full_response.push_str(&tok);
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
                                        // Show the assistant's spoken reply in chat
                                        // as a normal message (Issue 3).
                                        let reply = full_response.trim();
                                        if !reply.is_empty() {
                                            let _ = app_for_stream.emit(
                                                "voice:assistant_text",
                                                serde_json::json!({
                                                    "text": reply,
                                                    "turn": turn_for_stream
                                                }),
                                            );
                                        }
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
                                            .send(
                                                "(LLM response timeout — please try again)".into(),
                                            )
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
                            tracing::warn!(
                                turn = turn_for_stream,
                                "voice v2: chat_stream startup timeout"
                            );
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

            // Wave 3: push-to-talk is one utterance per activation. After the
            // turn completes, end the session so the mic is not held open —
            // "only listen while active". The user re-triggers to speak again.
            if voice_mode == "push_to_talk" {
                tracing::info!("voice v2 loop: push_to_talk turn complete; ending session");
                voice_active_loop.store(false, std::sync::atomic::Ordering::Relaxed);
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
            // Wave 8.1: emit the granular FSM state so the UI can distinguish
            // Listening / Transcribing / Thinking / Speaking / Interrupt
            // (additive string values — Property 7). Legacy "processing" is
            // kept as an alias the frontend still understands.
            let s = match state {
                VoiceSessionState::Sleeping => "idle",
                VoiceSessionState::Listening => "listening",
                VoiceSessionState::Transcribing => "transcribing",
                VoiceSessionState::Thinking => "thinking",
                VoiceSessionState::Speaking => "speaking",
                VoiceSessionState::BargeIn => "interrupt",
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

#[cfg(test)]
mod contract_tests {
    //! Wave 0.2: contract test locking the `voice:*` event-name mapping. These
    //! names are a frontend/backend contract (Req 12.1) — changing them is a
    //! breaking change and must be intentional. Granular FSM state strings are
    //! additive (Property 7).
    use super::v2_telemetry_to_event;
    use kria_core::voice::v2::{VoiceSessionState, VoiceTelemetry};

    fn event_name(ev: &VoiceTelemetry) -> &'static str {
        v2_telemetry_to_event(ev).0
    }

    #[test]
    fn state_event_names_are_stable() {
        let cases = [
            (VoiceSessionState::Sleeping, "idle"),
            (VoiceSessionState::Listening, "listening"),
            (VoiceSessionState::Transcribing, "transcribing"),
            (VoiceSessionState::Thinking, "thinking"),
            (VoiceSessionState::Speaking, "speaking"),
            (VoiceSessionState::BargeIn, "interrupt"),
        ];
        for (state, expected) in cases {
            let (name, payload) = v2_telemetry_to_event(&VoiceTelemetry::State { state });
            assert_eq!(name, "voice:state");
            assert_eq!(
                payload.get("state").and_then(|v| v.as_str()),
                Some(expected)
            );
        }
    }

    #[test]
    fn telemetry_event_names_are_stable() {
        assert_eq!(
            event_name(&VoiceTelemetry::Partial {
                text: "hi".into(),
                engine: "faster-whisper".into(),
                seq: 1,
            }),
            "voice:partial_transcript"
        );
        assert_eq!(
            event_name(&VoiceTelemetry::Final {
                text: "hi".into(),
                engine: "faster-whisper".into(),
                confidence: 0.9,
            }),
            "voice:transcript"
        );
        assert_eq!(
            event_name(&VoiceTelemetry::Error {
                message: "boom".into()
            }),
            "voice:error"
        );
        assert_eq!(
            event_name(&VoiceTelemetry::BusyRejected {
                entrypoint: "run_turn".into(),
                state: VoiceSessionState::Speaking,
            }),
            "voice:busy"
        );
        assert_eq!(
            event_name(&VoiceTelemetry::PlaybackFailure {
                message: "x".into()
            }),
            "voice:playback_failure"
        );
        assert_eq!(
            event_name(&VoiceTelemetry::PlaybackRecovered),
            "voice:playback_recovered"
        );
        assert_eq!(
            event_name(&VoiceTelemetry::Interruption {
                reason: "barge".into()
            }),
            "voice:interruption"
        );
    }
}
