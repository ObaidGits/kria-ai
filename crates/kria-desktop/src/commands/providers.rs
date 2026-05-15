//! Tauri commands for the Universal Model Provider system.
//!
//! Exposes provider management to the SolidJS frontend via Tauri IPC.

use super::*;
use kria_core::llm::provider::{
    config::ProviderConfig,
    connection_test::test_provider_connection,
};

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

/// Switch the active provider.
#[tauri::command]
pub async fn switch_provider(
    provider_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let mut config = state.config.write().await;

    // Validate provider exists
    if config.providers.get(&provider_id).is_none() {
        return Err(format!("Provider '{}' not found", provider_id));
    }

    // Check if configured
    let is_configured = config
        .providers
        .get(&provider_id)
        .map(|p| p.is_configured())
        .unwrap_or(false);

    if !is_configured {
        return Err(format!(
            "Provider '{}' is not configured (missing credentials or endpoint)",
            provider_id
        ));
    }

    let is_local = config
        .providers
        .get(&provider_id)
        .map(|p| p.provider_type.is_local())
        .unwrap_or(false);

    // Switch active provider
    config.providers.active_provider = provider_id.clone();

    // Update the legacy routing_mode for backward compatibility
    if is_local {
        config.llm.routing_mode = "local".to_string();
    } else {
        config.llm.routing_mode = "external".to_string();
    }

    // Persist
    config.save().map_err(|e| e.to_string())?;

    tracing::info!(
        provider = %provider_id,
        is_local,
        "Provider switched via UI"
    );

    Ok(serde_json::json!({
        "status": "ok",
        "active_provider": provider_id,
        "is_local": is_local,
    }))
}

/// Switch the active model for the current provider.
#[tauri::command]
pub async fn switch_model(
    model_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let mut config = state.config.write().await;
    let active_id = config.providers.active_provider.clone();

    match config.providers.get_mut(&active_id) {
        Some(p) => {
            p.active_model = model_id.clone();
            config.save().map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "status": "ok",
                "provider": active_id,
                "model": model_id,
            }))
        }
        None => Err("No active provider".to_string()),
    }
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
pub async fn test_provider_config(
    config: serde_json::Value,
) -> Result<serde_json::Value, String> {
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

    let new_provider: ProviderConfig =
        serde_json::from_value(provider_config).map_err(|e| format!("Invalid config: {e}"))?;

    let id = new_provider.id.clone();

    let mut config = state.config.write().await;
    config.providers.add(new_provider);
    config.save().map_err(|e| e.to_string())?;

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
        return Err("Cannot remove the active provider. Switch to another provider first.".to_string());
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
            "name": "OpenAI Compatible",
            "description": "Any OpenAI-compatible API endpoint",
            "is_local": false,
            "requires_api_key": false,
            "default_endpoint": "",
        }),
    ];

    Ok(serde_json::json!({ "types": types }))
}
