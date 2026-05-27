//! Tests for the Universal Model Provider system.

#[cfg(test)]
mod tests {
    use super::super::capabilities;
    use super::super::config::{
        ProviderConfig, ProviderEndpointConfig, ProviderType, ProvidersConfig,
    };
    use super::super::connection_test::ConnectionTestStatus;
    use super::super::error::{ProviderError, ProviderErrorKind};
    use super::super::registry::ProviderRegistry;
    use super::super::types::ExecutionLocation;
    use super::super::*;

    // ─── Config Tests ────────────────────────────────────────────────────────

    #[test]
    fn test_provider_type_properties() {
        assert!(ProviderType::Ollama.is_local());
        assert!(ProviderType::LlamaCpp.is_local());
        assert!(!ProviderType::OpenAI.is_local());
        assert!(!ProviderType::Gemini.is_local());
        assert!(!ProviderType::Anthropic.is_local());
        assert!(!ProviderType::OpenRouter.is_local());

        assert!(!ProviderType::Ollama.requires_api_key());
        assert!(!ProviderType::LlamaCpp.requires_api_key());
        assert!(ProviderType::OpenAI.requires_api_key());
        assert!(ProviderType::Gemini.requires_api_key());
        assert!(ProviderType::Anthropic.requires_api_key());
        assert!(ProviderType::OpenRouter.requires_api_key());
    }

    #[test]
    fn test_provider_config_is_configured() {
        // Local provider: only needs endpoint
        let mut local = ProviderConfig::new("test_local", ProviderType::LlamaCpp);
        local.endpoint.base_url = "http://localhost:8080".to_string();
        assert!(local.is_configured());

        // Cloud provider: needs endpoint + API key
        let mut cloud = ProviderConfig::new("test_cloud", ProviderType::OpenAI);
        cloud.endpoint.base_url = "https://api.openai.com/v1".to_string();
        assert!(!cloud.is_configured()); // No API key

        cloud.endpoint.api_key = "sk-test123".to_string();
        assert!(cloud.is_configured());
    }

    #[test]
    fn test_providers_config_crud() {
        let mut config = ProvidersConfig::default();

        // Default has llama_cpp and ollama
        assert_eq!(config.providers.len(), 2);
        assert_eq!(config.active_provider, "llama_cpp");

        // Add a new provider
        let openai = ProviderConfig::new("openai", ProviderType::OpenAI);
        config.add(openai);
        assert_eq!(config.providers.len(), 3);

        // Get by ID
        assert!(config.get("openai").is_some());
        assert!(config.get("nonexistent").is_none());

        // Remove
        assert!(config.remove("openai"));
        assert_eq!(config.providers.len(), 2);
        assert!(!config.remove("openai")); // Already removed
    }

    #[test]
    fn test_providers_config_active() {
        let config = ProvidersConfig::default();
        let active = config.active();
        assert!(active.is_some());
        assert_eq!(active.unwrap().id, "llama_cpp");
    }

    #[test]
    fn test_providers_config_enabled_filter() {
        let mut config = ProvidersConfig::default();
        // llama_cpp is enabled, ollama is disabled by default
        let enabled = config.enabled_providers();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].id, "llama_cpp");

        // Enable ollama
        if let Some(ollama) = config.get_mut("ollama") {
            ollama.enabled = true;
        }
        let enabled = config.enabled_providers();
        assert_eq!(enabled.len(), 2);
    }

    // ─── Error Tests ─────────────────────────────────────────────────────────

    #[test]
    fn test_error_classification() {
        assert!(ProviderErrorKind::RateLimited.is_retryable());
        assert!(ProviderErrorKind::Timeout.is_retryable());
        assert!(ProviderErrorKind::NetworkError.is_retryable());
        assert!(ProviderErrorKind::ServiceUnavailable.is_retryable());

        assert!(!ProviderErrorKind::AuthFailure.is_retryable());
        assert!(!ProviderErrorKind::InvalidModel.is_retryable());
        assert!(!ProviderErrorKind::QuotaExceeded.is_retryable());
        assert!(!ProviderErrorKind::ContentFiltered.is_retryable());
    }

    #[test]
    fn test_error_from_http_status() {
        let err = ProviderError::from_http_status(401, "Unauthorized", "openai");
        assert_eq!(err.kind, ProviderErrorKind::AuthFailure);
        assert_eq!(err.status_code, Some(401));
        assert!(!err.retryable);

        let err = ProviderError::from_http_status(429, "Too many requests", "openai");
        assert_eq!(err.kind, ProviderErrorKind::RateLimited);
        assert!(err.retryable);

        let err = ProviderError::from_http_status(500, "Internal error", "gemini");
        assert_eq!(err.kind, ProviderErrorKind::ServiceUnavailable);
        assert!(err.retryable);
    }

    #[test]
    fn test_error_display() {
        let err = ProviderError::new(ProviderErrorKind::AuthFailure, "Invalid API key", "openai");
        let display = format!("{}", err);
        assert!(display.contains("openai"));
        assert!(display.contains("Authentication Error"));
        assert!(display.contains("Invalid API key"));
    }

    // ─── Capabilities Tests ──────────────────────────────────────────────────

    #[test]
    fn test_model_capabilities() {
        let caps =
            capabilities::ModelCapabilities::full_featured("gpt-4o".to_string(), 128000, 16384);

        assert!(caps.can_use_tools());
        assert!(caps.can_see());
        assert!(caps.can_stream());
        assert!(caps.supports(capabilities::ProviderCapability::JsonMode));
        assert_eq!(caps.context_window, 128000);
        assert_eq!(caps.max_output_tokens, 16384);
    }

    #[test]
    fn test_chat_only_capabilities() {
        let caps = capabilities::ModelCapabilities::chat_only("phi-4-mini".to_string(), 4096, 2048);

        assert!(!caps.can_use_tools());
        assert!(!caps.can_see());
        assert!(!caps.can_stream());
        assert_eq!(caps.context_window, 4096);
    }

    // ─── Registry Tests ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_registry_initialization() {
        let config = ProvidersConfig::default();
        let registry = ProviderRegistry::new(config);
        registry.initialize().await;

        // Default config has llama_cpp with localhost endpoint
        // Backend should be created (even if not reachable)
        let location = registry.active_execution_location().await;
        assert_eq!(location, ExecutionLocation::Local);
    }

    #[tokio::test]
    async fn test_registry_switch_provider() {
        let mut config = ProvidersConfig::default();

        // Add a configured cloud provider
        let mut openai = ProviderConfig::new("openai", ProviderType::OpenAI);
        openai.endpoint.api_key = "sk-test".to_string();
        openai.endpoint.base_url = "https://api.openai.com/v1".to_string();
        openai.active_model = "gpt-4o".to_string();
        config.add(openai);

        let registry = ProviderRegistry::new(config);
        registry.initialize().await;

        // Switch to OpenAI
        let result = registry.switch_provider("openai").await;
        assert!(result.is_ok());

        let location = registry.active_execution_location().await;
        assert_eq!(location, ExecutionLocation::Cloud);
    }

    #[tokio::test]
    async fn test_registry_switch_nonexistent_provider() {
        let config = ProvidersConfig::default();
        let registry = ProviderRegistry::new(config);

        let result = registry.switch_provider("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_registry_switch_unconfigured_provider() {
        let mut config = ProvidersConfig::default();
        // Add unconfigured provider (no API key)
        let openai = ProviderConfig::new("openai", ProviderType::OpenAI);
        config.add(openai);

        let registry = ProviderRegistry::new(config);

        let result = registry.switch_provider("openai").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_registry_upsert_provider() {
        let config = ProvidersConfig::default();
        let registry = ProviderRegistry::new(config);

        let mut new_provider = ProviderConfig::new("anthropic", ProviderType::Anthropic);
        new_provider.endpoint.api_key = "sk-ant-test".to_string();
        new_provider.active_model = "claude-sonnet-4-20250514".to_string();

        let result = registry.upsert_provider(new_provider).await;
        assert!(result.is_ok());

        let all_status = registry.all_status().await;
        assert!(all_status.iter().any(|s| s.provider_id == "anthropic"));
    }

    #[tokio::test]
    async fn test_registry_remove_active_provider_fails() {
        let config = ProvidersConfig::default();
        let registry = ProviderRegistry::new(config);

        // Cannot remove the active provider
        let result = registry.remove_provider("llama_cpp").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_registry_remove_inactive_provider() {
        let config = ProvidersConfig::default();
        let registry = ProviderRegistry::new(config);

        let result = registry.remove_provider("ollama").await;
        assert!(result.is_ok());

        let all_status = registry.all_status().await;
        assert!(!all_status.iter().any(|s| s.provider_id == "ollama"));
    }

    #[tokio::test]
    async fn test_registry_all_status() {
        let config = ProvidersConfig::default();
        let registry = ProviderRegistry::new(config);

        let statuses = registry.all_status().await;
        assert_eq!(statuses.len(), 2); // llama_cpp + ollama

        let active = statuses.iter().find(|s| s.is_active);
        assert!(active.is_some());
        assert_eq!(active.unwrap().provider_id, "llama_cpp");
    }

    #[tokio::test]
    async fn test_registry_switch_model() {
        let config = ProvidersConfig::default();
        let registry = ProviderRegistry::new(config);

        let result = registry.switch_model("phi-4-mini").await;
        assert!(result.is_ok());

        let cfg = registry.get_config().await;
        assert_eq!(cfg.active().unwrap().active_model, "phi-4-mini");
    }

    #[tokio::test]
    async fn test_registry_orchestrator_notify() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let config = ProvidersConfig::default();
        let registry = ProviderRegistry::new(config);

        let notified = Arc::new(AtomicBool::new(false));
        let notified_clone = notified.clone();

        registry
            .set_orchestrator_notify(Arc::new(move |_location| {
                let n = notified_clone.clone();
                Box::pin(async move {
                    n.store(true, Ordering::SeqCst);
                })
            }))
            .await;

        // Add and switch to a configured cloud provider
        let mut openai = ProviderConfig::new("openai_test", ProviderType::OpenAI);
        openai.endpoint.api_key = "sk-test".to_string();
        openai.endpoint.base_url = "https://api.openai.com/v1".to_string();
        registry.upsert_provider(openai).await.unwrap();
        registry.switch_provider("openai_test").await.unwrap();

        assert!(notified.load(Ordering::SeqCst));
    }

    // ─── Serialization Tests ─────────────────────────────────────────────────

    #[test]
    fn test_provider_config_serialization() {
        let config = ProvidersConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ProvidersConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.active_provider, config.active_provider);
        assert_eq!(deserialized.providers.len(), config.providers.len());
    }

    #[test]
    fn test_provider_config_toml_roundtrip() {
        let config = ProvidersConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let deserialized: ProvidersConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(deserialized.active_provider, "llama_cpp");
        assert_eq!(deserialized.providers.len(), 2);
    }

    // ─── Integration with existing types ─────────────────────────────────────

    #[test]
    fn test_execution_location_from_provider_type() {
        let local_types = [ProviderType::Ollama, ProviderType::LlamaCpp];
        let cloud_types = [
            ProviderType::OpenAI,
            ProviderType::Gemini,
            ProviderType::Anthropic,
            ProviderType::OpenRouter,
        ];

        for t in local_types {
            assert!(t.is_local());
        }
        for t in cloud_types {
            assert!(!t.is_local());
        }
    }

    use std::sync::Arc;
}
