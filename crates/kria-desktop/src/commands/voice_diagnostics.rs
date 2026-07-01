use kria_core::config::KriaConfig;

/// Wave 7 observability: structured per-turn voice diagnostics + aggregate
/// health. Answers "why did a turn fail / time out / return empty?" using the
/// in-memory ring buffer populated by the telemetry pump. Only measured values
/// are present; missing milestones are `null`.
#[tauri::command]
pub async fn voice_turn_diagnostics(limit: Option<usize>) -> Result<serde_json::Value, String> {
    use kria_core::voice::turn_diagnostics::{aggregate, snapshot};
    let lim = limit.unwrap_or(20).clamp(1, 64);
    let turns = snapshot(lim);
    let agg = aggregate();
    Ok(serde_json::json!({
        "turns": turns,
        "aggregate": agg,
    }))
}

/// Inspect the v2 voice stack: which engines are compiled in (cargo features)
/// and what the resolved [`VoiceTierProfile`] would look like for the current
/// config + detected hardware. Used during the v2 rollout to verify that
/// builds + downloads + tier resolution are all consistent before flipping
/// `voice.engine` to `"v2"` in production.
#[tauri::command]
pub async fn voice_v2_status() -> Result<serde_json::Value, String> {
    use kria_core::voice::tier::VoiceTierProfile;
    use kria_core::voice::v2::wake::{WakeWordDetector, WakeWordModels};
    use kria_core::voice::v2::CompiledFeatures;

    let config = KriaConfig::load(None).map_err(|e| e.to_string())?;
    let paths = config.resolve_paths().map_err(|e| e.to_string())?;
    let hw = kria_core::platform::detect::detect_hardware();
    let profile = VoiceTierProfile::build(&config.voice, hw.tier);
    let features = CompiledFeatures::current();

    // Resolve the wake-word model path against KriaPaths. Treat the config
    // value as either an absolute path or a name relative to
    // `<models>/wake/`. Probe both paths so the UI can tell the user which
    // file is missing without installing one.
    let wake_cfg = &config.voice.wake_word;
    let wake_dir = paths.models_dir.join("wake");
    let wake_keyword_path = if wake_cfg.model_path.is_empty() {
        wake_dir.join("hey_ria.onnx")
    } else {
        let p = std::path::PathBuf::from(&wake_cfg.model_path);
        if p.is_absolute() {
            p
        } else if p.components().count() > 1 {
            paths
                .models_dir
                .join(p.strip_prefix("models").unwrap_or(&p))
        } else {
            wake_dir.join(p)
        }
    };
    let wake_models = WakeWordModels::from_keyword_path(wake_keyword_path.clone());

    // Probe the faster-whisper STT sidecar + Kokoro TTS sidecar health so the
    // UI reflects runtime truth (Req 8.4 / 9.3 health indicator). Short
    // timeouts; a missing sidecar simply reports healthy=false.
    let probe_client = reqwest::Client::new();
    let stt_base = kria_core::voice::v2::stt_sidecar::base_url();
    let tts_base = kria_core::voice::v2::tts_sidecar::base_url();
    let stt_healthy = kria_core::voice::v2::stt_sidecar::is_healthy(&probe_client, &stt_base).await;
    let tts_healthy = kria_core::voice::v2::tts_sidecar::is_healthy(&probe_client, &tts_base).await;

    // Try to load the detector; falls back to disabled when the feature is
    // off or model files are missing. Either outcome surfaces in the JSON.
    let wake_detector = if wake_cfg.enabled {
        WakeWordDetector::try_load(
            wake_keyword_path.clone(),
            wake_cfg.sensitivity,
            "hey ria",
            wake_cfg.aliases.clone(),
        )
    } else {
        WakeWordDetector::disabled()
    };

    Ok(serde_json::json!({
        "engine_setting": config.voice.engine,
        "mode": config.voice.mode,
        "tier": profile.tier.as_str(),
        "ttfa_budget_ms": profile.ttfa_budget_ms,
        "post_edit_timeout_ms": profile.post_edit_timeout_ms,
        "stt_engine": profile.stt_engine,
        "stt_model": profile.stt_model,
        "tts_engine": profile.tts_engine,
        "config_warnings": config.voice.validate(),
        "stt_sidecar": {
            "url": stt_base,
            "healthy": stt_healthy,
            "kind": "faster-whisper",
        },
        "tts_sidecar": {
            "url": tts_base,
            "healthy": tts_healthy,
            "kind": "kokoro",
            "selected": profile.tts_engine.eq_ignore_ascii_case("kokoro"),
        },
        "aec_aggressiveness": profile.aec_aggressiveness,
        "post_edit_always": profile.post_edit_always,
        "hardware_tier": hw.tier.as_str(),
        "compiled_features": features,
        "any_native_backend": features.any_native(),
        "wake_word": {
            "enabled_in_config": wake_cfg.enabled,
            "feature_compiled": features.voice_wake_oww,
            "active": wake_detector.is_active(),
            "sensitivity": wake_cfg.sensitivity,
            "aliases": wake_cfg.aliases,
            "models_dir": wake_dir.display().to_string(),
            "keyword_path": wake_models.keyword.display().to_string(),
            "embedding_path": wake_models.embedding.display().to_string(),
            "melspectrogram_path": wake_models.melspectrogram.display().to_string(),
            "all_models_present": wake_models.all_present(),
        },
        "note": "Voice v2 runtime active. Default STT = faster-whisper sidecar (GPU INT8 small, CPU fallback); default TTS = Piper (Kokoro opt-in via tts_engine='kokoro'). whisper-rs retained as explicit rollback.",
    }))
}

/// Debug helper: transcribe a user-provided audio file (WAV) using the same
/// whisper path configured for voice turns. This isolates STT correctness from
/// capture/turn orchestration.
#[tauri::command]
pub async fn voice_transcribe_audio_file(path: String) -> Result<serde_json::Value, String> {
    use crate::commands::command_helpers::which_binary;
    use crate::commands::voice_runtime_helpers::resolve_model_file;
    use kria_core::voice::stt::SpeechToText;

    let file_path = std::path::PathBuf::from(path.trim());
    if !file_path.exists() {
        return Err("Audio file does not exist".into());
    }
    if !file_path.is_file() {
        return Err("Path must point to a file".into());
    }
    let ext = file_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext != "wav" {
        return Err("Only .wav files are supported for now".into());
    }

    let config = KriaConfig::load(None).map_err(|e| e.to_string())?;
    let paths = config.resolve_paths().map_err(|e| e.to_string())?;
    let stt_model_path = resolve_model_file(&paths, "stt", &config.voice.stt_model);
    let whisper_bin = which_binary("whisper-cpp").or_else(|| which_binary("main"));
    let Some(whisper_bin) = whisper_bin else {
        return Err("whisper-cpp binary not found in PATH".into());
    };

    let mut stt = SpeechToText::new(stt_model_path, Some(whisper_bin));
    stt.set_language(&config.voice.language);
    if config.hardware.threads > 0 {
        stt.set_threads(config.hardware.threads.clamp(1, 12));
    }
    let result = stt
        .transcribe_file(&file_path)
        .await
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "text": result.text,
        "language": result.language,
        "confidence": result.confidence,
        "duration_ms": result.duration_ms,
        "engine": "whisper-cli",
        "path": file_path.display().to_string()
    }))
}

/// Debug helper: transcribe uploaded audio bytes from UI without requiring a
/// direct filesystem path from the webview.
#[tauri::command]
pub async fn voice_transcribe_uploaded_audio(
    name: String,
    bytes: Vec<u8>,
) -> Result<serde_json::Value, String> {
    use crate::commands::command_helpers::which_binary;
    use crate::commands::voice_runtime_helpers::resolve_model_file;
    use kria_core::voice::stt::SpeechToText;

    if bytes.is_empty() {
        return Err("Uploaded audio is empty".into());
    }

    let ext = std::path::Path::new(name.trim())
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext.is_empty() {
        return Err("Audio filename must include an extension".into());
    }
    let allowed = ["wav", "mp3", "m4a", "flac", "ogg", "webm"];
    if !allowed.contains(&ext.as_str()) {
        return Err(format!(
            "Unsupported audio extension .{ext}. Supported: {}",
            allowed.join(", ")
        ));
    }

    let temp_name = format!(
        "kria_voice_upload_{}_{}.{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis(),
        ext
    );
    let temp_path = std::env::temp_dir().join(temp_name);
    std::fs::write(&temp_path, &bytes)
        .map_err(|e| format!("Failed to write temp audio file: {e}"))?;

    let run = async {
        let config = KriaConfig::load(None).map_err(|e| e.to_string())?;
        let paths = config.resolve_paths().map_err(|e| e.to_string())?;
        let stt_model_path = resolve_model_file(&paths, "stt", &config.voice.stt_model);
        let whisper_bin = which_binary("whisper-cpp").or_else(|| which_binary("main"));
        let Some(whisper_bin) = whisper_bin else {
            return Err("whisper-cpp binary not found in PATH".into());
        };

        let mut stt = SpeechToText::new(stt_model_path, Some(whisper_bin));
        stt.set_language(&config.voice.language);
        if config.hardware.threads > 0 {
            stt.set_threads(config.hardware.threads.clamp(1, 12));
        }
        let result = stt
            .transcribe_file(&temp_path)
            .await
            .map_err(|e| e.to_string())?;

        Ok::<serde_json::Value, String>(serde_json::json!({
            "text": result.text,
            "language": result.language,
            "confidence": result.confidence,
            "duration_ms": result.duration_ms,
            "engine": "whisper-cli",
            "name": name
        }))
    }
    .await;

    let _ = std::fs::remove_file(&temp_path);
    run
}
