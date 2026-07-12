use crate::config::KriaConfig;
use crate::llm::orchestrator::server_manager::LlamaServerManager;
use crate::llm::provider::{config::ProviderType, ProviderConfig};
use crate::llm::{cloud::CloudBackend, local::LocalBackend, LlmBackend};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Task 0.9 (Requirement 0.9 Rung B): whether a backend genuinely posts a
/// grammar / `json_schema` constraint and can therefore be relied on for a
/// ~100% schema-valid typed plan. This is the strong signal the Planner
/// Capability Ladder uses to pick a LOCAL fallback backend: a `json_object` or
/// `tool_calling` mode (which only *guides* output) does NOT qualify.
pub fn is_grammar_capable(backend: &Arc<dyn LlmBackend>) -> bool {
    use crate::llm::StructuredOutputMode;
    backend.supports_grammar()
        || matches!(
            backend.structured_output_mode(),
            StructuredOutputMode::Grammar | StructuredOutputMode::JsonSchema
        )
}

/// Routing modes for model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingMode {
    Local,
    Colab,
    Gemini,
    External,
}

impl std::str::FromStr for RoutingMode {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mode = match s.to_lowercase().as_str() {
            "colab" => Self::Colab,
            "gemini" => Self::Gemini,
            "external" => Self::External,
            _ => Self::Local,
        };
        Ok(mode)
    }
}

impl RoutingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Colab => "colab",
            Self::Gemini => "gemini",
            Self::External => "external",
        }
    }
}

/// Config-driven model router. Selects which backend to use per request.
pub struct ModelRouter {
    mode: RwLock<RoutingMode>,
    local: Option<Arc<dyn LlmBackend>>,
    /// Concrete typed reference to the local backend (same Arc) for
    /// orchestrator attachment after construction.
    local_concrete: Option<Arc<LocalBackend>>,
    vision_local: Option<Arc<dyn LlmBackend>>,
    /// Concrete typed reference to the vision backend for orchestrator
    /// attachment after construction.
    vision_local_concrete: Option<Arc<LocalBackend>>,
    cloud_clients: RwLock<HashMap<String, Arc<dyn LlmBackend>>>,
    /// Provider selected by Settings/config. This is intentionally separate
    /// from the routing mode so UI/status can show `llama_cpp` instead of the
    /// generic `local` mode after runtime swaps.
    active_provider_id: RwLock<String>,
    /// Local model selected by Settings/config. LocalBackend's model_label is
    /// constructed once, so runtime swaps must update status through this field.
    active_local_model: RwLock<String>,
    /// Local API URL (stored for server probing).
    local_api_url: String,
}

impl ModelRouter {
    /// Create a model router from configuration.
    pub fn from_config(config: &KriaConfig) -> Self {
        let (local, local_concrete) = if !config.llm.local_api_url.is_empty() {
            let backend = Arc::new(LocalBackend::new(
                config.llm.local_api_url.clone(),
                config.llm.active_model.clone(),
                vec!["text".into()],
                config.llm.context_window,
            ));
            (Some(backend.clone() as Arc<dyn LlmBackend>), Some(backend))
        } else {
            (None, None)
        };

        // Create a vision-capable backend if a vision model is explicitly defined.
        // Keep a concrete Arc so orchestrator server manager can be attached.
        let (vision_local, vision_local_concrete) = config
            .llm
            .models
            .iter()
            .find(|m| m.capabilities.contains(&"vision".to_string()) && m.mmproj_file.is_some())
            .map(|vm| {
                let backend = Arc::new(LocalBackend::new(
                    config.llm.local_api_url.clone(),
                    vm.name.clone(),
                    vec!["text".into(), "vision".into()],
                    vm.context_window,
                ));
                (Some(backend.clone() as Arc<dyn LlmBackend>), Some(backend))
            })
            // If no explicit vision model but local backend exists, treat local
            // as vision-capable: the user may have loaded a vision model (e.g.
            // Qwen2.5-VL with --mmproj) on their llama.cpp server.
            .unwrap_or_else(|| {
                if !config.llm.local_api_url.is_empty() {
                    let backend = Arc::new(LocalBackend::new(
                        config.llm.local_api_url.clone(),
                        config.llm.active_model.clone(),
                        vec!["text".into(), "vision".into()],
                        config.llm.context_window,
                    ));
                    (Some(backend.clone() as Arc<dyn LlmBackend>), Some(backend))
                } else {
                    (None, None)
                }
            });

        let mut cloud_clients: HashMap<String, Arc<dyn LlmBackend>> = HashMap::new();

        if !config.llm.cloud_api_key.is_empty() && !config.llm.cloud_endpoint.is_empty() {
            let name = if config.llm.cloud_provider.is_empty() {
                "external".to_string()
            } else {
                config.llm.cloud_provider.clone()
            };
            cloud_clients.insert(
                name.clone(),
                Arc::new(CloudBackend::new(
                    config.llm.cloud_endpoint.clone(),
                    config.llm.cloud_api_key.clone(),
                    config.llm.cloud_model_id.clone(),
                    name,
                    vec!["text".into()],
                    Some(30),
                )),
            );
        }

        for provider_config in &config.providers.providers {
            if !provider_config.enabled || !provider_config.is_configured() {
                continue;
            }
            if provider_config.provider_type == ProviderType::LlamaCpp {
                continue;
            }
            if let Some(backend) = create_provider_backend(provider_config) {
                cloud_clients.insert(provider_config.id.clone(), backend.clone());
                if provider_config.id == config.providers.active_provider {
                    cloud_clients.insert("external".to_string(), backend.clone());
                    if provider_config.provider_type == ProviderType::Gemini {
                        cloud_clients.insert("gemini".to_string(), backend);
                    }
                }
            }
        }

        let mode = config
            .llm
            .routing_mode
            .parse::<RoutingMode>()
            .unwrap_or(RoutingMode::Local);

        Self {
            mode: RwLock::new(mode),
            local,
            local_concrete,
            vision_local,
            vision_local_concrete,
            cloud_clients: RwLock::new(cloud_clients),
            active_provider_id: RwLock::new(config.providers.active_provider.clone()),
            active_local_model: RwLock::new(config.llm.active_model.clone()),
            local_api_url: config.llm.local_api_url.clone(),
        }
    }

    /// Query the local LLM server's `/v1/models` endpoint and return
    /// the model ID if the server is reachable.
    pub async fn detect_server_model(&self) -> Option<String> {
        if self.local_api_url.is_empty() {
            return None;
        }
        let url = format!("{}/models", self.local_api_url);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .ok()?;
        let resp = client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body: serde_json::Value = resp.json().await.ok()?;
        body["data"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|m| m["id"].as_str())
            .map(|s| s.to_string())
    }

    /// Get the current routing mode.
    pub async fn mode(&self) -> RoutingMode {
        *self.mode.read().await
    }

    /// Set the routing mode.
    pub async fn set_mode(&self, mode: RoutingMode) {
        *self.mode.write().await = mode;
    }

    /// Update the selected local model used for status/reporting. The concrete
    /// LocalBackend label is immutable after construction, while llama.cpp can
    /// be restarted with a different GGUF at runtime.
    pub async fn set_active_local_model_label(&self, model_id: impl Into<String>) {
        let model_id = model_id.into();
        if !model_id.trim().is_empty() {
            *self.active_local_model.write().await = model_id;
        }
    }

    /// Sync the live router to a provider selected from Settings.
    ///
    /// Local llama.cpp keeps using the orchestrator-backed local backend.
    /// Every other provider is registered as the current external backend so
    /// the existing route path sees the same runtime the UI shows.
    pub async fn sync_active_provider(&self, provider_config: &ProviderConfig) {
        *self.active_provider_id.write().await = provider_config.id.clone();

        if provider_config.provider_type == ProviderType::LlamaCpp {
            if !provider_config.active_model.trim().is_empty() {
                *self.active_local_model.write().await = provider_config.active_model.clone();
            }
            self.set_mode(RoutingMode::Local).await;
            return;
        }

        if let Some(backend) = create_provider_backend(provider_config) {
            let mut clients = self.cloud_clients.write().await;
            clients.insert(provider_config.id.clone(), backend.clone());
            clients.insert("external".to_string(), backend.clone());
            if provider_config.provider_type == ProviderType::Gemini {
                clients.insert("gemini".to_string(), backend);
                drop(clients);
                self.set_mode(RoutingMode::Gemini).await;
            } else {
                drop(clients);
                self.set_mode(RoutingMode::External).await;
            }
        }
    }

    /// Route a request to the appropriate backend.
    pub async fn route(&self, _intent: &str) -> Option<Arc<dyn LlmBackend>> {
        let mode = self.mode().await;

        match mode {
            RoutingMode::Local => self.local.clone(),
            RoutingMode::Colab => {
                let clients = self.cloud_clients.read().await;
                clients
                    .get("colab")
                    .cloned()
                    .or_else(|| clients.get("external").cloned())
                    .or_else(|| self.local.clone())
            }
            RoutingMode::Gemini => {
                let clients = self.cloud_clients.read().await;
                clients
                    .get("gemini")
                    .cloned()
                    .or_else(|| self.local.clone())
            }
            RoutingMode::External => {
                let clients = self.cloud_clients.read().await;
                clients
                    .get("external")
                    .cloned()
                    .or_else(|| clients.values().next().cloned())
                    .or_else(|| self.local.clone())
            }
        }
    }

    /// Route a request with images to a vision-capable backend.
    /// Falls back to regular routing if no vision backend is available.
    pub async fn route_vision(&self) -> Option<Arc<dyn LlmBackend>> {
        if let Some(ref concrete) = self.vision_local_concrete {
            if concrete.runtime_supports_vision() {
                if let Some(ref v) = self.vision_local {
                    return Some(v.clone());
                }
            } else {
                tracing::warn!(
                    "route_vision: local vision backend configured, but runtime vision is disabled; skipping local multimodal route"
                );
            }
        }

        // Fall back to cloud vision backends if available.
        let clients = self.cloud_clients.read().await;
        if let Some(client) = clients
            .values()
            .find(|client| client.capabilities().iter().any(|cap| cap == "vision"))
        {
            return Some(client.clone());
        }

        // No runtime vision route available.
        None
    }

    /// Check if a vision-capable backend is available.
    pub fn has_vision(&self) -> bool {
        self.vision_local_concrete
            .as_ref()
            .map(|backend| backend.runtime_supports_vision())
            .unwrap_or(false)
    }

    /// Always returns local client (for classification, planning).
    pub fn get_local(&self) -> Option<Arc<dyn LlmBackend>> {
        self.local.clone()
    }

    /// Task 0.9 (Requirement 0.9 Rung B): the configured LOCAL backend, if any.
    /// Used by the GUI-cognition Planner Capability Ladder to obtain a
    /// grammar-capable local backend for the middle rung when the configured
    /// (e.g. cloud) planner backend is NOT itself grammar-capable. Returns the
    /// same `Arc` clone as [`get_local`](Self::get_local).
    pub fn local_backend(&self) -> Option<Arc<dyn LlmBackend>> {
        self.local.clone()
    }

    /// Register a new cloud client at runtime.
    pub async fn register_cloud(
        &self,
        name: String,
        endpoint: String,
        api_key: String,
        model_id: String,
        rpm: Option<u32>,
    ) {
        let client = Arc::new(CloudBackend::new(
            endpoint,
            api_key,
            model_id,
            name.clone(),
            vec!["text".into()],
            rpm,
        ));
        self.cloud_clients.write().await.insert(name, client);
    }

    /// Attach an orchestrator server manager to the local backend.
    /// This wires up dynamic URL resolution and stream cancellation.
    pub fn attach_server_manager(&self, mgr: Arc<LlamaServerManager>) {
        if let Some(ref backend) = self.local_concrete {
            backend.attach_server_manager(mgr.clone());
        }

        if let Some(ref backend) = self.vision_local_concrete {
            backend.attach_server_manager(mgr);
        }
    }

    /// Returns the currently attached orchestrator server manager, if local
    /// backends are orchestrator-managed.
    pub fn orchestrator_server_manager(&self) -> Option<Arc<LlamaServerManager>> {
        self.vision_local_concrete
            .as_ref()
            .and_then(|backend| backend.server_manager())
            .or_else(|| {
                self.local_concrete
                    .as_ref()
                    .and_then(|backend| backend.server_manager())
            })
    }

    /// Get status dict for dashboard.
    pub async fn status(&self) -> serde_json::Value {
        let mode = self.mode().await;
        let local_healthy = match &self.local {
            Some(l) => l.health_check().await,
            None => false,
        };
        let selected_provider = self.active_provider_id.read().await.clone();
        let selected_local_model = self.active_local_model.read().await.clone();
        let local_model = if selected_local_model.trim().is_empty() {
            self.local
                .as_ref()
                .map(|l| l.model_label().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        } else {
            selected_local_model
        };

        // For cloud/external modes, check the active cloud backend instead.
        // For non-local modes: any configured cloud client counts as healthy
        // (CloudBackend::health_check returns is_configured(), no network call).
        let (cloud_healthy, active_provider, active_model) = match mode {
            RoutingMode::Local => (
                false,
                if selected_provider.trim().is_empty() {
                    "local".to_string()
                } else {
                    selected_provider.clone()
                },
                local_model.clone(),
            ),
            _ => {
                let clients = self.cloud_clients.read().await;
                let key = match mode {
                    RoutingMode::Colab => "colab",
                    RoutingMode::Gemini => "gemini",
                    _ => "external",
                };
                let selected = clients
                    .get(key)
                    .map(|client| (key.to_string(), client.clone()))
                    .or_else(|| {
                        clients
                            .iter()
                            .next()
                            .map(|(provider, client)| (provider.clone(), client.clone()))
                    });

                match selected {
                    Some((provider, client)) => (
                        client.health_check().await,
                        provider,
                        client.model_label().to_string(),
                    ),
                    None => (false, mode.as_str().to_string(), "unknown".to_string()),
                }
            }
        };

        // active_healthy = whichever backend the current mode actually uses
        let active_healthy = match mode {
            RoutingMode::Local => local_healthy,
            _ => cloud_healthy,
        };

        let cloud_count = self.cloud_clients.read().await.len();

        serde_json::json!({
            "mode": mode.as_str(),
            "local_healthy": local_healthy,
            "active_healthy": active_healthy,
            "local_model": local_model,
            "active_provider": active_provider,
            "active_model": active_model,
            "cloud_backends": cloud_count,
        })
    }
}

/// Delegates to the single shared provider-type → backend mapping in
/// `llm::provider::registry` (also used by `ProviderRegistry`), so the two
/// no longer drift out of sync.
fn create_provider_backend(config: &ProviderConfig) -> Option<Arc<dyn LlmBackend>> {
    Some(crate::llm::provider::create_backend_for_provider(config))
}
