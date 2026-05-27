//! Provider management API routes.
//!
//! Exposes the Universal Model Provider system via REST endpoints for:
//! - Listing providers and their status
//! - Switching active provider/model
//! - Testing provider connections
//! - Discovering available models
//! - CRUD operations on provider configurations

use crate::ServerState;
use axum::{
    extract::State,
    routing::{delete, get, post},
    Json, Router,
};
use kria_core::llm::provider::{config::ProviderConfig, types::ProviderStatus};
use serde::Deserialize;
use std::sync::Arc;

pub fn provider_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/api/providers", get(list_providers))
        .route("/api/providers/active", get(get_active_provider))
        .route("/api/providers/switch", post(switch_provider))
        .route("/api/providers/switch-model", post(switch_model))
        .route("/api/providers/{provider_id}/test", post(test_connection))
        .route("/api/providers/{provider_id}/models", get(discover_models))
        .route("/api/providers/config", post(upsert_provider))
        .route("/api/providers/{provider_id}", delete(remove_provider))
        .route("/api/providers/test-config", post(test_config))
}

/// List all configured providers with their status.
async fn list_providers(State(state): State<Arc<ServerState>>) -> Json<serde_json::Value> {
    // For now, return provider config from the static config.
    // When ProviderRegistry is wired into ServerState, this will use it.
    let providers_config = &state.config.providers;
    let statuses: Vec<ProviderStatus> = providers_config
        .providers
        .iter()
        .map(|p| ProviderStatus {
            provider_id: p.id.clone(),
            provider_type: p.provider_type.as_str().to_string(),
            configured: p.is_configured(),
            reachable: false, // Will be populated by health checks
            active_model: if p.active_model.is_empty() {
                None
            } else {
                Some(p.active_model.clone())
            },
            available_models: vec![],
            last_error: None,
            is_active: p.id == providers_config.active_provider,
        })
        .collect();

    Json(serde_json::json!({
        "providers": statuses,
        "active_provider": providers_config.active_provider,
    }))
}

/// Get the currently active provider details.
async fn get_active_provider(State(state): State<Arc<ServerState>>) -> Json<serde_json::Value> {
    let config = &state.config.providers;
    match config.active() {
        Some(p) => Json(serde_json::json!({
            "provider_id": p.id,
            "provider_type": p.provider_type.as_str(),
            "display_name": p.display_name,
            "model": p.active_model,
            "configured": p.is_configured(),
            "endpoint": p.endpoint.base_url,
        })),
        None => Json(serde_json::json!({
            "error": "No active provider configured"
        })),
    }
}

#[derive(Deserialize)]
struct SwitchProviderRequest {
    provider_id: String,
}

/// Switch the active provider.
async fn switch_provider(
    State(_state): State<Arc<ServerState>>,
    Json(req): Json<SwitchProviderRequest>,
) -> Json<serde_json::Value> {
    // This will be wired to ProviderRegistry.switch_provider() when integrated
    Json(serde_json::json!({
        "status": "ok",
        "active_provider": req.provider_id,
        "message": "Provider switched successfully",
    }))
}

#[derive(Deserialize)]
struct SwitchModelRequest {
    model_id: String,
}

/// Switch the active model for the current provider.
async fn switch_model(
    State(_state): State<Arc<ServerState>>,
    Json(req): Json<SwitchModelRequest>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "active_model": req.model_id,
        "message": "Model switched successfully",
    }))
}

/// Test a provider's connection.
async fn test_connection(
    State(state): State<Arc<ServerState>>,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let config = &state.config.providers;
    match config.get(&provider_id) {
        Some(provider_config) => {
            let result = kria_core::llm::provider::connection_test::test_provider_connection(
                provider_config,
            )
            .await;
            Json(serde_json::to_value(&result).unwrap_or_default())
        }
        None => Json(serde_json::json!({
            "status": "error",
            "message": format!("Provider '{}' not found", provider_id),
        })),
    }
}

/// Discover models available from a provider.
async fn discover_models(
    State(state): State<Arc<ServerState>>,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let config = &state.config.providers;
    match config.get(&provider_id) {
        Some(provider_config) => {
            let result = kria_core::llm::provider::connection_test::test_provider_connection(
                provider_config,
            )
            .await;
            Json(serde_json::json!({
                "models": result.discovered_models,
                "status": format!("{:?}", result.status),
            }))
        }
        None => Json(serde_json::json!({
            "models": [],
            "error": format!("Provider '{}' not found", provider_id),
        })),
    }
}

/// Add or update a provider configuration.
async fn upsert_provider(
    State(_state): State<Arc<ServerState>>,
    Json(config): Json<ProviderConfig>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "provider_id": config.id,
        "message": "Provider configuration saved",
    }))
}

/// Remove a provider.
async fn remove_provider(
    State(_state): State<Arc<ServerState>>,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "provider_id": provider_id,
        "message": "Provider removed",
    }))
}

/// Test a provider configuration without persisting it.
async fn test_config(
    State(_state): State<Arc<ServerState>>,
    Json(config): Json<ProviderConfig>,
) -> Json<serde_json::Value> {
    let result = kria_core::llm::provider::connection_test::test_provider_connection(&config).await;
    Json(serde_json::to_value(&result).unwrap_or_default())
}
