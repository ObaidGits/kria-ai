use kria_core::config::KriaConfig;

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
        "tier": profile.tier.as_str(),
        "ttfa_budget_ms": profile.ttfa_budget_ms,
        "post_edit_timeout_ms": profile.post_edit_timeout_ms,
        "stt_engine": profile.stt_engine,
        "stt_model": profile.stt_model,
        "tts_engine": profile.tts_engine,
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
        "note": "v2 runtime loop pending; engine='v2' currently falls back to v1.",
    }))
}
