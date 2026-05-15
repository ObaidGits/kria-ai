//! Provider Registry — runtime provider management.
//!
//! The registry is the single point of truth for all configured providers.
//! It handles:
//! - Provider lifecycle (create, configure, switch, remove)
//! - Active provider resolution
//! - Fallback logic
//! - Hardware orchestrator notifications on provider switches
//! - Thread-safe concurrent access

use super::capabilities::ModelCapabilities;
use super::config::{ProviderConfig, ProviderType, ProvidersConfig};
use super::connection_test::{test_provider_connection, ConnectionTestResult};
use super::error::{ProviderError, ProviderErrorKind};
use super::types::{ExecutionLocation, ModelInfo, ProviderHealthSnapshot, ProviderStatus};
use crate::llm::LlmBackend;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Callback for hardware orchestrator notifications.
pub type OrchestratorNotifyFn =
    Arc<dyn Fn(ExecutionLocation) -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

/// The provider registry manages all configured providers and routes requests.
pub struct ProviderRegistry {
    /// Provider configurations (persisted).
    config: RwLock<ProvidersConfig>,
    /// Active provider backends (instantiated from config).
    backends: RwLock<HashMap<String, Arc<dyn LlmBackend>>>,
    /// Cached model capabilities per provider.
    capabilities_cache: RwLock<HashMap<String, Vec<ModelCapabilities>>>,
    /// Callback to notify hardware orchestrator of execution location changes.
    orchestrator_notify: RwLock<Option<OrchestratorNotifyFn>>,
    /// Health snapshots per provider.
    health: RwLock<HashMap<String, ProviderHealthSnapshot>>,
}

impl ProviderRegistry {
    /// Create a new registry from persisted configuration.
    pub fn new(config: ProvidersConfig) -> Self {
        Self {
            config: RwLock::new(config),
            backends: RwLock::new(HashMap::new()),
            capabilities_cache: RwLock::new(HashMap::new()),
            orchestrator_notify: RwLock::new(None),
            health: RwLock::new(HashMap::new()),
        }
    }

    /// Set the orchestrator notification callback.
    pub async fn set_orchestrator_notify(&self, notify: OrchestratorNotifyFn) {
        *self.orchestrator_notify.write().await = Some(notify);
    }

    /// Initialize backends from current configuration.
    pub async fn initialize(&self) {
        let config = self.config.read().await;
        let mut backends = self.backends.write().await;

        for provider_config in &config.providers {
            if !provider_config.enabled || !provider_config.is_configured() {
                continue;
            }
            if let Some(backend) = self.create_backend(provider_config) {
                backends.insert(provider_config.id.clone(), backend);
            }
        }
    }

    /// Create a backend instance from provider config.
    fn create_backend(&self, config: &ProviderConfig) -> Option<Arc<dyn LlmBackend>> {
        use super::{anthropic, gemini, ollama, openai, openrouter};

        let backend: Arc<dyn LlmBackend> = match config.provider_type {
            ProviderType::Ollama => Arc::new(ollama::OllamaBackend::from_config(config)),
            ProviderType::LlamaCpp | ProviderType::OpenAICompatible => {
                Arc::new(openai::OpenAIBackend::from_config(config))
            }
            ProviderType::OpenAI => Arc::new(openai::OpenAIBackend::from_config(config)),
            ProviderType::Gemini => Arc::new(gemini::GeminiBackend::from_config(config)),
            ProviderType::Anthropic => Arc::new(anthropic::AnthropicBackend::from_config(config)),
            ProviderType::OpenRouter => Arc::new(openrouter::OpenRouterBackend::from_config(config)),
        };

        Some(backend)
    }

    /// Get the currently active provider backend.
    pub async fn active_backend(&self) -> Option<Arc<dyn LlmBackend>> {
        let config = self.config.read().await;
        let backends = self.backends.read().await;
        backends.get(&config.active_provider).cloned()
    }

    /// Get a specific provider backend by ID.
    pub async fn get_backend(&self, provider_id: &str) -> Option<Arc<dyn LlmBackend>> {
        self.backends.read().await.get(provider_id).cloned()
    }

    /// Switch the active provider.
    ///
    /// This notifies the hardware orchestrator of the execution location change.
    pub async fn switch_provider(&self, provider_id: &str) -> Result<(), ProviderError> {
        // Validate the provider exists and is configured
        let (provider_type, is_configured) = {
            let config = self.config.read().await;
            match config.get(provider_id) {
                Some(p) => (p.provider_type, p.is_configured()),
                None => {
                    return Err(ProviderError::new(
                        ProviderErrorKind::InvalidModel,
                        format!("Provider '{provider_id}' not found"),
                        "registry",
                    ));
                }
            }
        };

        if !is_configured {
            return Err(ProviderError::new(
                ProviderErrorKind::AuthFailure,
                format!("Provider '{provider_id}' is not configured (missing credentials or endpoint)"),
                "registry",
            ));
        }

        // Ensure backend is instantiated
        {
            let backends = self.backends.read().await;
            if !backends.contains_key(provider_id) {
                drop(backends);
                // Create and insert the backend
                let config = self.config.read().await;
                if let Some(provider_config) = config.get(provider_id) {
                    if let Some(backend) = self.create_backend(provider_config) {
                        self.backends.write().await.insert(provider_id.to_string(), backend);
                    }
                }
            }
        }

        // Update active provider
        {
            let mut config = self.config.write().await;
            config.active_provider = provider_id.to_string();
        }

        // Notify hardware orchestrator
        let location = if provider_type.is_local() {
            ExecutionLocation::Local
        } else {
            ExecutionLocation::Cloud
        };

        if let Some(notify) = self.orchestrator_notify.read().await.as_ref() {
            notify(location).await;
        }

        tracing::info!(
            provider = provider_id,
            location = ?location,
            "Switched active provider"
        );

        Ok(())
    }

    /// Switch the active model for the current provider.
    pub async fn switch_model(&self, model_id: &str) -> Result<(), ProviderError> {
        let mut config = self.config.write().await;
        let active_id = config.active_provider.clone();
        match config.get_mut(&active_id) {
            Some(p) => {
                p.active_model = model_id.to_string();
                Ok(())
            }
            None => Err(ProviderError::new(
                ProviderErrorKind::InvalidModel,
                "No active provider",
                "registry",
            )),
        }
    }

    /// Add or update a provider configuration.
    ///
    /// If the provider already exists, it will be updated. The backend will be
    /// re-created with the new configuration.
    pub async fn upsert_provider(&self, provider_config: ProviderConfig) -> Result<(), ProviderError> {
        let id = provider_config.id.clone();

        // Update config
        {
            let mut config = self.config.write().await;
            config.add(provider_config.clone());
        }

        // Re-create backend if configured
        if provider_config.is_configured() && provider_config.enabled {
            if let Some(backend) = self.create_backend(&provider_config) {
                self.backends.write().await.insert(id.clone(), backend);
            }
        } else {
            // Remove backend if no longer configured
            self.backends.write().await.remove(&id);
        }

        Ok(())
    }

    /// Remove a provider.
    pub async fn remove_provider(&self, provider_id: &str) -> Result<(), ProviderError> {
        let mut config = self.config.write().await;
        if config.active_provider == provider_id {
            return Err(ProviderError::new(
                ProviderErrorKind::ProviderSpecific,
                "Cannot remove the active provider. Switch to another provider first.",
                "registry",
            ));
        }
        config.remove(provider_id);
        drop(config);

        self.backends.write().await.remove(provider_id);
        self.capabilities_cache.write().await.remove(provider_id);
        self.health.write().await.remove(provider_id);

        Ok(())
    }

    /// Test a provider's connection.
    pub async fn test_connection(&self, provider_id: &str) -> ConnectionTestResult {
        let config = self.config.read().await;
        match config.get(provider_id) {
            Some(provider_config) => test_provider_connection(provider_config).await,
            None => ConnectionTestResult::failure(
                super::connection_test::ConnectionTestStatus::Error,
                format!("Provider '{provider_id}' not found"),
            ),
        }
    }

    /// Test a provider config without persisting it (for UI "test before save").
    pub async fn test_config(&self, config: &ProviderConfig) -> ConnectionTestResult {
        test_provider_connection(config).await
    }

    /// Discover models available from a provider.
    pub async fn discover_models(&self, provider_id: &str) -> Result<Vec<ModelInfo>, ProviderError> {
        let config = self.config.read().await;
        let provider_config = config.get(provider_id).ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::InvalidModel,
                format!("Provider '{provider_id}' not found"),
                "registry",
            )
        })?;

        // Use connection test to discover models
        let result = test_provider_connection(provider_config).await;
        let models: Vec<ModelInfo> = result
            .discovered_models
            .into_iter()
            .map(|id| ModelInfo {
                id: id.clone(),
                display_name: id.clone(),
                context_window: 0, // Unknown until queried
                max_output_tokens: 0,
                supports_streaming: true,
                supports_tools: false,
                supports_vision: false,
                supports_json_mode: false,
                pricing: None,
                metadata: serde_json::Value::Null,
            })
            .collect();

        Ok(models)
    }

    /// Get status of all providers.
    pub async fn all_status(&self) -> Vec<ProviderStatus> {
        let config = self.config.read().await;
        let _backends = self.backends.read().await;
        let health = self.health.read().await;

        let mut statuses = Vec::new();
        for provider_config in &config.providers {
            let is_active = provider_config.id == config.active_provider;
            let reachable = health
                .get(&provider_config.id)
                .map(|h| h.is_healthy)
                .unwrap_or(false);

            statuses.push(ProviderStatus {
                provider_id: provider_config.id.clone(),
                provider_type: provider_config.provider_type.as_str().to_string(),
                configured: provider_config.is_configured(),
                reachable,
                active_model: if provider_config.active_model.is_empty() {
                    None
                } else {
                    Some(provider_config.active_model.clone())
                },
                available_models: vec![],
                last_error: health.get(&provider_config.id).and_then(|h| {
                    if h.is_healthy {
                        None
                    } else {
                        Some("Last health check failed".to_string())
                    }
                }),
                is_active,
            });
        }

        statuses
    }

    /// Get the current providers configuration (for persistence).
    pub async fn get_config(&self) -> ProvidersConfig {
        self.config.read().await.clone()
    }

    /// Replace the entire providers configuration (e.g., after loading from disk).
    pub async fn load_config(&self, config: ProvidersConfig) {
        *self.config.write().await = config;
        // Re-initialize backends
        self.initialize().await;
    }

    /// Get the execution location of the active provider.
    pub async fn active_execution_location(&self) -> ExecutionLocation {
        let config = self.config.read().await;
        match config.active() {
            Some(p) if p.provider_type.is_local() => ExecutionLocation::Local,
            Some(_) => ExecutionLocation::Cloud,
            None => ExecutionLocation::Local, // Default to local
        }
    }

    /// Update health snapshot for a provider.
    pub async fn update_health(&self, provider_id: &str, healthy: bool, latency_ms: Option<u64>) {
        let config = self.config.read().await;
        let location = config
            .get(provider_id)
            .map(|p| {
                if p.provider_type.is_local() {
                    ExecutionLocation::Local
                } else {
                    ExecutionLocation::Cloud
                }
            })
            .unwrap_or(ExecutionLocation::Local);
        drop(config);

        let snapshot = ProviderHealthSnapshot {
            provider_id: provider_id.to_string(),
            is_healthy: healthy,
            latency_ms,
            error_count: if healthy { 0 } else { 1 },
            last_success_epoch_ms: if healthy {
                Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64,
                )
            } else {
                None
            },
            execution_location: location,
        };

        self.health.write().await.insert(provider_id.to_string(), snapshot);
    }
}
