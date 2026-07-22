//! Tauri commands for the Universal Model Provider system.
//!
//! Exposes provider management to the SolidJS frontend via Tauri IPC.

use super::*;
use kria_core::config::{KriaConfig, LocalModelDef, OrchestratorConfig};
use kria_core::llm::orchestrator::tier_strategy::derive_model_profile;
use kria_core::llm::provider::{
    config::{ProviderConfig, ProviderType},
    connection_test::{test_provider_connection, ConnectionTestStatus},
};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter};

fn is_llama_cpp_runtime(provider_type: ProviderType) -> bool {
    provider_type == ProviderType::LlamaCpp
}

fn provider_mode(provider_type: ProviderType) -> &'static str {
    match provider_type {
        ProviderType::LlamaCpp => "local",
        ProviderType::Gemini => "gemini",
        _ => "external",
    }
}

fn sync_legacy_llm_from_provider(
    config: &mut KriaConfig,
    provider_id: &str,
) -> Result<ProviderConfig, String> {
    let provider = config
        .providers
        .get(provider_id)
        .cloned()
        .ok_or_else(|| format!("Provider '{provider_id}' not found"))?;

    config.providers.active_provider = provider.id.clone();
    config.llm.routing_mode = provider_mode(provider.provider_type).to_string();

    match provider.provider_type {
        ProviderType::LlamaCpp => {
            if !provider.endpoint.base_url.trim().is_empty() {
                config.llm.local_api_url = provider.endpoint.base_url.clone();
            }
            if !provider.active_model.trim().is_empty() {
                config.llm.active_model = provider.active_model.clone();
            }
        }
        _ => {
            config.llm.cloud_provider = provider.id.clone();
            config.llm.cloud_endpoint = provider.endpoint.base_url.clone();
            if !provider.active_model.trim().is_empty() {
                config.llm.cloud_model_id = provider.active_model.clone();
            }
        }
    }

    Ok(provider)
}

fn env_override_summary() -> serde_json::Value {
    let names = [
        "KRIA_ACTIVE_PROVIDER",
        "KRIA_ACTIVE_MODEL",
        "KRIA_PROVIDER_API_KEY",
        "KRIA_LLM_MODE",
        "KRIA_CLOUD_API_KEY",
        "KRIA_OPENAI_API_KEY",
        "KRIA_GEMINI_API_KEY",
        "KRIA_ANTHROPIC_API_KEY",
        "KRIA_OPENROUTER_API_KEY",
        "KRIA_OPENCODE_API_KEY",
    ];

    let active: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| std::env::var(name).is_ok())
        .collect();

    serde_json::json!({
        "env_wins": !active.is_empty(),
        "active_env_vars": active,
        "precedence": ["environment", "user_settings", "default_config"],
    })
}

fn active_runtime_payload(
    config: &KriaConfig,
    provider: &ProviderConfig,
    router_status: Option<serde_json::Value>,
) -> serde_json::Value {
    let active_model = match provider.provider_type {
        ProviderType::LlamaCpp if provider.active_model.trim().is_empty() => {
            config.llm.active_model.clone()
        }
        _ => provider.active_model.clone(),
    };

    serde_json::json!({
        "provider_id": provider.id,
        "provider_type": provider.provider_type.as_str(),
        "display_name": provider.display_name,
        "active_model": active_model,
        "endpoint": provider.endpoint.base_url,
        "enabled": provider.enabled,
        "configured": provider.is_configured(),
        "is_local": provider.provider_type.is_local(),
        "is_llama_cpp_runtime": is_llama_cpp_runtime(provider.provider_type),
        "requires_api_key": provider.provider_type.requires_api_key(),
        "routing_mode": config.llm.routing_mode,
        "legacy_active_model": config.llm.active_model,
        "restart_required_for_local_model_change": false,
        "runtime_apply_handles_local_model_change": is_llama_cpp_runtime(provider.provider_type),
        "router_status": router_status,
        "config_source": env_override_summary(),
    })
}

async fn active_runtime_payload_for_state(state: &AppState) -> Result<serde_json::Value, String> {
    let config = state.config.read().await;
    let provider = config
        .providers
        .active()
        .cloned()
        .ok_or_else(|| "No active provider configured".to_string())?;
    let router_status = state.model_router.status().await;
    let apply_status = state.llm_runtime_apply_status.read().await.clone();
    let mut payload = active_runtime_payload(&config, &provider, Some(router_status.clone()));
    payload["apply_status"] = serde_json::json!(apply_status);
    payload["router_status"] = router_status;
    Ok(payload)
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

async fn publish_apply_status(
    state: &AppState,
    app: &AppHandle,
    state_name: &str,
    phase: &str,
    provider_id: Option<String>,
    model_id: Option<String>,
    message: impl Into<String>,
    last_error: Option<String>,
) {
    let snapshot = LlmRuntimeApplySnapshot {
        state: state_name.to_string(),
        phase: phase.to_string(),
        provider_id,
        model_id,
        message: message.into(),
        last_error,
        updated_unix_ms: unix_ms(),
    };

    *state.llm_runtime_apply_status.write().await = snapshot.clone();
    let _ = app.emit("llm-runtime:apply", serde_json::json!(snapshot));
}

fn file_stem(value: &str) -> Option<String> {
    Path::new(value)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
}

fn model_filename_candidates(filename: &str) -> Vec<String> {
    let filename = filename.trim();
    if filename.is_empty() {
        return Vec::new();
    }

    let mut candidates = vec![filename.to_string()];
    let path = Path::new(filename);
    if path.extension().is_none() && !filename.to_ascii_lowercase().ends_with(".gguf") {
        candidates.push(format!("{filename}.gguf"));
    }
    candidates
}

fn add_workspace_model_dirs(start: Option<PathBuf>, dirs: &mut Vec<PathBuf>) {
    let Some(start) = start else { return };
    let mut cursor = Some(start.as_path());
    while let Some(dir) = cursor {
        dirs.push(dir.join("models").join("llm"));
        cursor = dir.parent();
        if cursor.map(|path| path == Path::new("/")).unwrap_or(true) {
            break;
        }
    }
}

fn local_model_search_dirs() -> Vec<PathBuf> {
    let paths = kria_core::platform::paths::KriaPaths::resolve();
    let mut dirs = vec![paths.llm_models];
    add_workspace_model_dirs(std::env::current_dir().ok(), &mut dirs);
    add_workspace_model_dirs(
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf)),
        &mut dirs,
    );

    let mut seen = std::collections::HashSet::new();
    dirs.into_iter()
        .filter(|dir| seen.insert(dir.clone()))
        .collect()
}

fn resolve_model_file_path(filename: &str) -> PathBuf {
    let direct = PathBuf::from(filename);
    if direct.is_absolute() {
        if direct.exists() {
            return direct;
        }
        if direct.extension().is_none() {
            let with_gguf = direct.with_extension("gguf");
            if with_gguf.exists() {
                return with_gguf;
            }
        }
    }

    for dir in local_model_search_dirs() {
        for candidate_name in model_filename_candidates(filename) {
            let candidate = dir.join(candidate_name);
            if candidate.exists() {
                return candidate;
            }
        }
    }

    direct
}

fn find_configured_local_model(config: &KriaConfig, model_id: &str) -> Option<LocalModelDef> {
    let model_id = model_id.trim();
    config
        .llm
        .models
        .iter()
        .find(|model| {
            model.name.eq_ignore_ascii_case(model_id)
                || model.file.eq_ignore_ascii_case(model_id)
                || file_stem(&model.file)
                    .map(|stem| stem.eq_ignore_ascii_case(model_id))
                    .unwrap_or(false)
        })
        .cloned()
}

fn ad_hoc_local_model_from_file(model_id: &str, path: &Path, config: &KriaConfig) -> LocalModelDef {
    let file = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(model_id)
        .to_string();
    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(model_id)
        .to_string();
    let size_bytes = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    let size_gb = (size_bytes as f32 / 1024.0 / 1024.0 / 1024.0).max(1.0);

    LocalModelDef {
        name: name.clone(),
        file,
        display_name: name,
        context_window: config.llm.context_window.max(2048),
        max_tokens: config.llm.max_tokens.max(512),
        vram_estimate_gb: size_gb,
        capabilities: vec!["chat".to_string()],
        mmproj_file: None,
    }
}

fn resolve_local_model_for_runtime(
    config: &KriaConfig,
    model_id: &str,
) -> Result<LocalModelDef, String> {
    if let Some(model) = find_configured_local_model(config, model_id) {
        return Ok(model);
    }

    let path = resolve_model_file_path(model_id);
    if path.exists() {
        return Ok(ad_hoc_local_model_from_file(model_id, &path, config));
    }

    Err(format!(
        "Local model '{model_id}' is not defined in config and no matching GGUF file was found"
    ))
}

fn prepare_local_runtime(
    state: &AppState,
    config: &KriaConfig,
    provider: &ProviderConfig,
) -> Result<(String, Option<String>, OrchestratorConfig, LocalModelDef), String> {
    let model_id = provider
        .active_model
        .trim()
        .if_empty_then(config.llm.active_model.trim());
    if model_id.is_empty() {
        return Err("No local model selected".to_string());
    }

    let model = resolve_local_model_for_runtime(config, model_id)?;
    let model_path = resolve_model_file_path(&model.file);
    if !model_path.exists() {
        return Err(format!(
            "Configured local model file was not found: {}",
            model_path.display()
        ));
    }

    let mmproj_path = model
        .mmproj_file
        .as_ref()
        .map(|file| resolve_model_file_path(file))
        .filter(|path| path.exists())
        .map(|path| path.to_string_lossy().to_string());

    let mut orch_cfg = config.orchestrator.clone();
    orch_cfg.model_profile = derive_model_profile(&model, &config.orchestrator.model_profile);

    let model_size_mb = std::fs::metadata(&model_path)
        .map(|metadata| metadata.len() / (1024 * 1024))
        .unwrap_or((model.vram_estimate_gb as u64) * 1024);
    orch_cfg.tune_for_tier(
        state.hardware_info.tier,
        state.hardware_info.total_ram_mb,
        state.hardware_info.vram_mb,
        model_size_mb,
    );

    Ok((
        model_path.to_string_lossy().to_string(),
        mmproj_path,
        orch_cfg,
        model,
    ))
}

trait EmptyFallback<'a> {
    fn if_empty_then(self, fallback: &'a str) -> &'a str;
}

impl<'a> EmptyFallback<'a> for &'a str {
    fn if_empty_then(self, fallback: &'a str) -> &'a str {
        if self.is_empty() {
            fallback
        } else {
            self
        }
    }
}

fn mutate_selection_config(
    mut config: KriaConfig,
    provider_id: &str,
    model_id: Option<String>,
) -> Result<(KriaConfig, ProviderConfig, bool), String> {
    let old_active_provider = config.providers.active_provider.clone();
    let old_active_model = config
        .providers
        .get(&old_active_provider)
        .map(|provider| provider.active_model.clone())
        .unwrap_or_default();

    let provider_type = config
        .providers
        .get(provider_id)
        .map(|provider| provider.provider_type)
        .ok_or_else(|| format!("Provider '{provider_id}' not found"))?;

    let requested_model = model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let normalized_model = match (provider_type, requested_model.as_deref()) {
        (ProviderType::LlamaCpp, Some(model)) => {
            Some(resolve_local_model_for_runtime(&config, model)?.name)
        }
        (_, Some(model)) => Some(model.to_string()),
        _ => None,
    };

    {
        let provider = config
            .providers
            .get_mut(provider_id)
            .ok_or_else(|| format!("Provider '{provider_id}' not found"))?;

        if !provider.enabled {
            return Err(format!(
                "Provider '{}' is disabled. Enable it before activating it.",
                provider.display_name
            ));
        }

        if let Some(model) = normalized_model {
            provider.active_model = model;
        }

        if !provider.is_configured() {
            return Err(format!(
                "Provider '{}' is not configured (missing credentials or endpoint)",
                provider.display_name
            ));
        }
    }

    let provider = sync_legacy_llm_from_provider(&mut config, provider_id)?;
    let changed = old_active_provider != provider.id || old_active_model != provider.active_model;

    Ok((config, provider, changed))
}

pub(super) async fn start_local_orchestrator(
    state: &AppState,
    config: &KriaConfig,
    provider: &ProviderConfig,
) -> Result<Arc<Orchestrator>, String> {
    let (model_path, mmproj_path, orch_cfg, model) =
        prepare_local_runtime(state, config, provider)?;

    tracing::info!(
        model = %model.name,
        file = %model.file,
        resolved = %model_path,
        mmproj = ?mmproj_path,
        "LLM runtime apply: starting local orchestrator"
    );

    let orchestrator = Orchestrator::start(
        orch_cfg,
        model_path,
        mmproj_path,
        state.event_bus.clone(),
        state.health.clone(),
    )
    .await
    .map_err(|error| error.to_string())?;

    if let Err(error) = configure_orchestrator_fleet_bridge(&orchestrator, &state.fleet_runtime) {
        tracing::warn!(
            error = %error,
            "LLM runtime apply: failed to wire fleet bridge for swapped orchestrator"
        );
    } else {
        pulse_target_pool_telemetry(&state.fleet_runtime.target_pool).await;
    }

    Ok(orchestrator)
}

async fn restore_previous_runtime(
    state: &AppState,
    app: &AppHandle,
    previous_config: KriaConfig,
) -> Result<(), String> {
    let previous_provider = previous_config
        .providers
        .active()
        .cloned()
        .ok_or_else(|| "Previous provider is missing; rollback cannot continue".to_string())?;

    if previous_provider.provider_type == ProviderType::LlamaCpp
        && previous_config.orchestrator.enabled
    {
        publish_apply_status(
            state,
            app,
            "switching",
            "rollback_local_runtime",
            Some(previous_provider.id.clone()),
            Some(previous_provider.active_model.clone()),
            "Restoring previous local runtime",
            None,
        )
        .await;
        let rollback_orchestrator =
            start_local_orchestrator(state, &previous_config, &previous_provider).await?;
        state
            .model_router
            .attach_server_manager(rollback_orchestrator.server_manager.clone());
        *state.orchestrator.write().await = Some(rollback_orchestrator.clone());
        super::feature_controls::start_orchestrator_tasks(state, app, rollback_orchestrator).await;
    } else {
        state
            .model_router
            .sync_active_provider(&previous_provider)
            .await;
        *state.orchestrator.write().await = None;
    }

    previous_config.save().map_err(|error| error.to_string())?;
    *state.config.write().await = previous_config;
    Ok(())
}

async fn apply_provider_selection(
    state: &AppState,
    app: &AppHandle,
    provider_id: String,
    model_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let apply_guard = state
        .llm_runtime_apply_lock
        .try_lock()
        .map_err(|_| "Another model/provider switch is already in progress".to_string())?;
    let lifecycle_guard = state.feature_controls.lifecycle_guard().await;

    if state.orchestrator_active_turns.load(Ordering::SeqCst) > 0 {
        return Err(
            "Cannot switch AI runtime while a local model turn is running. Wait for the turn to finish and try again."
                .to_string(),
        );
    }

    let previous_config = state.config.read().await.clone();
    let (desired_config, provider, changed_model) =
        mutate_selection_config(previous_config.clone(), &provider_id, model_id)?;

    publish_apply_status(
        state,
        app,
        "switching",
        "validating",
        Some(provider.id.clone()),
        Some(provider.active_model.clone()),
        "Validating selected AI runtime",
        None,
    )
    .await;

    let result = if provider.provider_type == ProviderType::LlamaCpp {
        apply_local_runtime_selection(
            state,
            app,
            previous_config,
            desired_config.clone(),
            provider.clone(),
        )
        .await
    } else {
        apply_external_provider_selection(state, app, desired_config.clone(), provider.clone())
            .await
    };

    drop(lifecycle_guard);
    drop(apply_guard);

    if let Err(error) = result {
        publish_apply_status(
            state,
            app,
            "failed",
            "failed",
            Some(provider.id.clone()),
            Some(provider.active_model.clone()),
            "AI runtime switch failed",
            Some(error.clone()),
        )
        .await;
        return Err(error);
    }

    tracing::info!(
        provider = %provider.id,
        provider_type = %provider.provider_type.as_str(),
        model = %provider.active_model,
        changed_model,
        "LLM provider selection applied"
    );

    let router_status = state.model_router.status().await;
    let mut payload =
        active_runtime_payload(&desired_config, &provider, Some(router_status.clone()));
    payload["apply_status"] =
        serde_json::json!(state.llm_runtime_apply_status.read().await.clone());
    payload["router_status"] = router_status;
    payload["status"] = serde_json::json!("ok");
    payload["restart_required"] = serde_json::json!(false);
    payload["runtime_swapped"] =
        serde_json::json!(changed_model && is_llama_cpp_runtime(provider.provider_type));

    Ok(payload)
}

async fn apply_external_provider_selection(
    state: &AppState,
    app: &AppHandle,
    desired_config: KriaConfig,
    provider: ProviderConfig,
) -> Result<(), String> {
    publish_apply_status(
        state,
        app,
        "switching",
        "testing_provider",
        Some(provider.id.clone()),
        Some(provider.active_model.clone()),
        "Testing provider connection",
        None,
    )
    .await;

    let test_result = test_provider_connection(&provider).await;
    if !matches!(
        test_result.status,
        ConnectionTestStatus::Success | ConnectionTestStatus::Degraded
    ) {
        return Err(format!(
            "Provider validation failed: {}",
            test_result.message
        ));
    }

    publish_apply_status(
        state,
        app,
        "switching",
        "rebinding_router",
        Some(provider.id.clone()),
        Some(provider.active_model.clone()),
        "Binding chat router to selected provider",
        None,
    )
    .await;

    desired_config.save().map_err(|error| error.to_string())?;
    *state.config.write().await = desired_config;
    state.model_router.sync_active_provider(&provider).await;

    super::feature_controls::stop_orchestrator_tasks(state).await;
    if let Some(orchestrator) = state.orchestrator.write().await.take() {
        publish_apply_status(
            state,
            app,
            "switching",
            "releasing_local_runtime",
            Some(provider.id.clone()),
            Some(provider.active_model.clone()),
            "Releasing local llama.cpp runtime because an external provider is active",
            None,
        )
        .await;
        orchestrator.shutdown().await;
    }

    publish_apply_status(
        state,
        app,
        "ready",
        "ready",
        Some(provider.id),
        Some(provider.active_model),
        "AI runtime is ready",
        None,
    )
    .await;

    Ok(())
}

async fn apply_local_runtime_selection(
    state: &AppState,
    app: &AppHandle,
    previous_config: KriaConfig,
    desired_config: KriaConfig,
    provider: ProviderConfig,
) -> Result<(), String> {
    let _validated_runtime = prepare_local_runtime(state, &desired_config, &provider)?;

    if !desired_config.orchestrator.enabled {
        publish_apply_status(
            state,
            app,
            "switching",
            "disabling_local_runtime",
            Some(provider.id.clone()),
            Some(provider.active_model.clone()),
            "Saving local model selection without starting the disabled orchestrator",
            None,
        )
        .await;
        super::feature_controls::stop_orchestrator_tasks(state).await;
        if let Some(existing) = state.orchestrator.write().await.take() {
            existing.shutdown().await;
        }
        desired_config.save().map_err(|error| error.to_string())?;
        *state.config.write().await = desired_config;
        state.model_router.sync_active_provider(&provider).await;
        publish_apply_status(
            state,
            app,
            "ready",
            "disabled",
            Some(provider.id),
            Some(provider.active_model),
            "Local model selected; model orchestrator remains disabled",
            None,
        )
        .await;
        return Ok(());
    }

    publish_apply_status(
        state,
        app,
        "switching",
        "stopping_previous_runtime",
        Some(provider.id.clone()),
        Some(provider.active_model.clone()),
        "Stopping previous local runtime",
        None,
    )
    .await;

    super::feature_controls::stop_orchestrator_tasks(state).await;
    if let Some(existing) = state.orchestrator.write().await.take() {
        existing.shutdown().await;
    }

    publish_apply_status(
        state,
        app,
        "switching",
        "starting_local_runtime",
        Some(provider.id.clone()),
        Some(provider.active_model.clone()),
        "Starting selected local model",
        None,
    )
    .await;

    match start_local_orchestrator(state, &desired_config, &provider).await {
        Ok(orchestrator) => {
            state
                .model_router
                .attach_server_manager(orchestrator.server_manager.clone());
            state.model_router.sync_active_provider(&provider).await;
            *state.orchestrator.write().await = Some(orchestrator.clone());
            super::feature_controls::start_orchestrator_tasks(state, app, orchestrator).await;
            desired_config.save().map_err(|error| error.to_string())?;
            *state.config.write().await = desired_config;

            publish_apply_status(
                state,
                app,
                "ready",
                "ready",
                Some(provider.id),
                Some(provider.active_model),
                "Local AI runtime is ready",
                None,
            )
            .await;
            Ok(())
        }
        Err(error) => {
            tracing::error!(
                error = %error,
                "LLM runtime apply: selected local model failed; attempting rollback"
            );
            publish_apply_status(
                state,
                app,
                "switching",
                "rollback",
                Some(provider.id.clone()),
                Some(provider.active_model.clone()),
                "Selected local model failed; restoring previous runtime",
                Some(error.clone()),
            )
            .await;

            if let Err(rollback_error) =
                restore_previous_runtime(state, app, previous_config.clone()).await
            {
                let combined = format!(
                    "Selected local model failed: {error}. Rollback also failed: {rollback_error}"
                );
                publish_apply_status(
                    state,
                    app,
                    "rollback_required",
                    "rollback_failed",
                    Some(provider.id),
                    Some(provider.active_model),
                    "Manual restart required after failed model switch",
                    Some(combined.clone()),
                )
                .await;
                return Err(combined);
            }

            Err(format!(
                "Selected local model failed and KRIA rolled back: {error}"
            ))
        }
    }
}

/// List all configured providers with their status.
#[tauri::command]
pub async fn list_providers(state: State<'_, AppStateCell>) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let config = state.config.read().await;
    let providers = &config.providers;

    let statuses: Vec<serde_json::Value> = providers
        .providers
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "provider_type": p.provider_type.as_str(),
                "display_name": p.display_name,
                "enabled": p.enabled,
                "configured": p.is_configured(),
                "active_model": p.active_model,
                "endpoint": p.endpoint.base_url,
                "is_active": p.id == providers.active_provider,
                "is_local": p.provider_type.is_local(),
                "requires_api_key": p.provider_type.requires_api_key(),
            })
        })
        .collect();

    Ok(serde_json::json!({
        "providers": statuses,
        "active_provider": providers.active_provider,
        "prefer_streaming": providers.prefer_streaming,
        "config_source": env_override_summary(),
    }))
}

/// Get the active provider details.
#[tauri::command]
pub async fn get_active_provider(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let config = state.config.read().await;
    let providers = &config.providers;

    match providers.active() {
        Some(p) => Ok(serde_json::json!({
            "id": p.id,
            "provider_type": p.provider_type.as_str(),
            "display_name": p.display_name,
            "active_model": p.active_model,
            "configured": p.is_configured(),
            "endpoint": p.endpoint.base_url,
            "is_local": p.provider_type.is_local(),
            "temperature": p.default_temperature,
            "max_tokens": p.default_max_tokens,
            "prefer_streaming": p.prefer_streaming,
        })),
        None => Ok(serde_json::json!({
            "error": "No active provider configured"
        })),
    }
}

/// Get the canonical active LLM runtime shown by Settings → Models.
#[tauri::command]
pub async fn get_active_llm_runtime(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    active_runtime_payload_for_state(state).await
}

/// Get the latest runtime apply/swap status shown by Settings → Models.
#[tauri::command]
pub async fn get_llm_runtime_apply_status(
    state: State<'_, AppStateCell>,
) -> Result<LlmRuntimeApplySnapshot, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    Ok(state.llm_runtime_apply_status.read().await.clone())
}

/// Atomically select provider + model and sync legacy runtime config.
#[tauri::command]
pub async fn set_active_llm_selection(
    provider_id: String,
    model_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    apply_provider_selection(state, &app, provider_id, model_id).await
}

/// Switch the active provider.
#[tauri::command]
pub async fn switch_provider(
    provider_id: String,
    app: AppHandle,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    apply_provider_selection(state, &app, provider_id, None).await
}

/// Switch the active model for the current provider.
#[tauri::command]
pub async fn switch_model(
    model_id: String,
    app: AppHandle,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let active_id = state.config.read().await.providers.active_provider.clone();
    apply_provider_selection(state, &app, active_id, Some(model_id)).await
}

/// Test a provider's connection.
#[tauri::command]
pub async fn test_provider_connection_cmd(
    provider_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let config = state.config.read().await;
    let provider_config = config
        .providers
        .get(&provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?
        .clone();
    drop(config);

    let result = test_provider_connection(&provider_config).await;
    serde_json::to_value(&result).map_err(|e| e.to_string())
}

/// Test a provider configuration without persisting it (for "test before save" UX).
#[tauri::command]
pub async fn test_provider_config(config: serde_json::Value) -> Result<serde_json::Value, String> {
    let provider_config: ProviderConfig =
        serde_json::from_value(config).map_err(|e| format!("Invalid config: {e}"))?;
    let result = test_provider_connection(&provider_config).await;
    serde_json::to_value(&result).map_err(|e| e.to_string())
}

/// Discover models available from a provider.
#[tauri::command]
pub async fn discover_provider_models(
    provider_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let config = state.config.read().await;
    let provider_config = config
        .providers
        .get(&provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?
        .clone();
    drop(config);

    let result = test_provider_connection(&provider_config).await;
    Ok(serde_json::json!({
        "models": result.discovered_models,
        "status": format!("{:?}", result.status),
        "message": result.message,
    }))
}

/// Add or update a provider configuration.
#[tauri::command]
pub async fn upsert_provider(
    provider_config: serde_json::Value,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let mut new_provider: ProviderConfig =
        serde_json::from_value(provider_config).map_err(|e| format!("Invalid config: {e}"))?;

    let id = new_provider.id.clone();

    let mut config = state.config.write().await;
    if new_provider.endpoint.api_key.trim().is_empty() {
        if let Some(existing) = config.providers.get(&id) {
            new_provider.endpoint.api_key = existing.endpoint.api_key.clone();
        }
    }
    config.providers.add(new_provider);
    config.save().map_err(|e| e.to_string())?;

    if config.providers.active_provider == id {
        if let Some(provider) = config.providers.get(&id).cloned() {
            drop(config);
            state.model_router.sync_active_provider(&provider).await;
        }
    }

    Ok(serde_json::json!({
        "status": "ok",
        "provider_id": id,
        "message": "Provider configuration saved",
    }))
}

/// Remove a provider.
#[tauri::command]
pub async fn remove_provider(
    provider_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let mut config = state.config.write().await;

    if config.providers.active_provider == provider_id {
        return Err(
            "Cannot remove the active provider. Switch to another provider first.".to_string(),
        );
    }

    let removed = config.providers.remove(&provider_id);
    if removed {
        config.save().map_err(|e| e.to_string())?;
    }

    Ok(serde_json::json!({
        "status": if removed { "ok" } else { "not_found" },
        "provider_id": provider_id,
    }))
}

/// Get available provider types (for the "Add Provider" UI).
#[tauri::command]
pub async fn get_provider_types() -> Result<serde_json::Value, String> {
    let types = vec![
        serde_json::json!({
            "id": "ollama",
            "name": "Ollama",
            "description": "Local Ollama instance for running open-source models",
            "is_local": true,
            "requires_api_key": false,
            "default_endpoint": "http://localhost:11434",
        }),
        serde_json::json!({
            "id": "llama_cpp",
            "name": "llama.cpp",
            "description": "Local llama.cpp server (OpenAI-compatible)",
            "is_local": true,
            "requires_api_key": false,
            "default_endpoint": "http://localhost:8080",
        }),
        serde_json::json!({
            "id": "openai",
            "name": "OpenAI",
            "description": "OpenAI GPT models (GPT-4o, GPT-4, etc.)",
            "is_local": false,
            "requires_api_key": true,
            "default_endpoint": "https://api.openai.com/v1",
        }),
        serde_json::json!({
            "id": "gemini",
            "name": "Google Gemini",
            "description": "Google Gemini models (Gemini 2.0 Flash, Pro, etc.)",
            "is_local": false,
            "requires_api_key": true,
            "default_endpoint": "https://generativelanguage.googleapis.com/v1beta",
        }),
        serde_json::json!({
            "id": "anthropic",
            "name": "Anthropic",
            "description": "Anthropic Claude models (Claude Sonnet, Opus, Haiku)",
            "is_local": false,
            "requires_api_key": true,
            "default_endpoint": "https://api.anthropic.com/v1",
        }),
        serde_json::json!({
            "id": "openrouter",
            "name": "OpenRouter",
            "description": "Multi-provider gateway with access to 100+ models",
            "is_local": false,
            "requires_api_key": true,
            "default_endpoint": "https://openrouter.ai/api/v1",
        }),
        serde_json::json!({
            "id": "openai_compatible",
            "name": "Custom API",
            "description": "Custom OpenAI-compatible endpoint such as vLLM, LM Studio, LiteLLM, Groq, or a private gateway",
            "is_local": false,
            "requires_api_key": false,
            "default_endpoint": "",
        }),
    ];

    Ok(serde_json::json!({ "types": types }))
}
