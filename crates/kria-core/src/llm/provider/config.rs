//! Provider configuration types.
//!
//! Defines the persistent configuration for each provider, including
//! credentials, endpoints, and model preferences.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Supported provider types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderType {
    /// Local Ollama instance.
    #[serde(rename = "ollama")]
    Ollama,
    /// Local llama.cpp / OpenAI-compatible endpoint.
    #[serde(rename = "llama_cpp")]
    LlamaCpp,
    /// OpenAI API.
    #[serde(rename = "openai")]
    OpenAI,
    /// Google Gemini API.
    #[serde(rename = "gemini")]
    Gemini,
    /// Anthropic Claude API.
    #[serde(rename = "anthropic")]
    Anthropic,
    /// OpenRouter (multi-provider gateway).
    #[serde(rename = "openrouter")]
    OpenRouter,
    /// Generic OpenAI-compatible API.
    #[serde(rename = "openai_compatible")]
    OpenAICompatible,
}

impl ProviderType {
    /// Default endpoint for this provider type.
    pub fn default_endpoint(&self) -> &'static str {
        match self {
            Self::Ollama => "http://localhost:11434",
            Self::LlamaCpp => "http://localhost:8080",
            Self::OpenAI => "https://api.openai.com/v1",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::OpenRouter => "https://openrouter.ai/api/v1",
            Self::OpenAICompatible => "",
        }
    }

    /// Whether this provider type requires an API key.
    pub fn requires_api_key(&self) -> bool {
        matches!(
            self,
            Self::OpenAI | Self::Gemini | Self::Anthropic | Self::OpenRouter
        )
    }

    /// Whether this provider runs locally (affects hardware orchestrator).
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Ollama | Self::LlamaCpp)
    }

    /// Human-readable display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::LlamaCpp => "llama.cpp",
            Self::OpenAI => "OpenAI",
            Self::Gemini => "Google Gemini",
            Self::Anthropic => "Anthropic",
            Self::OpenRouter => "OpenRouter",
            Self::OpenAICompatible => "OpenAI Compatible",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::LlamaCpp => "llama_cpp",
            Self::OpenAI => "openai",
            Self::Gemini => "gemini",
            Self::Anthropic => "anthropic",
            Self::OpenRouter => "openrouter",
            Self::OpenAICompatible => "openai_compatible",
        }
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Configuration for a provider's endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEndpointConfig {
    /// Base URL for the API.
    pub base_url: String,
    /// API key (empty for local providers).
    #[serde(default)]
    pub api_key: String,
    /// Optional organization ID (OpenAI).
    #[serde(default)]
    pub organization_id: Option<String>,
    /// Optional project ID.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Request timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Maximum retries on transient failures.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Rate limit (requests per minute, 0 = unlimited).
    #[serde(default)]
    pub rate_limit_rpm: u32,
    /// Custom headers to include in requests.
    #[serde(default)]
    pub custom_headers: HashMap<String, String>,
}

fn default_timeout() -> u64 {
    60
}

fn default_max_retries() -> u32 {
    3
}

impl Default for ProviderEndpointConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            api_key: String::new(),
            organization_id: None,
            project_id: None,
            timeout_secs: 60,
            max_retries: 3,
            rate_limit_rpm: 0,
            custom_headers: HashMap::new(),
        }
    }
}

/// Full configuration for a single provider instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Unique identifier for this provider instance.
    pub id: String,
    /// Provider type.
    pub provider_type: ProviderType,
    /// Human-readable name (user-customizable).
    pub display_name: String,
    /// Whether this provider is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Endpoint configuration.
    pub endpoint: ProviderEndpointConfig,
    /// Currently selected model ID.
    #[serde(default)]
    pub active_model: String,
    /// Default temperature for this provider.
    #[serde(default = "default_temperature")]
    pub default_temperature: f32,
    /// Default max tokens for this provider.
    #[serde(default = "default_max_tokens")]
    pub default_max_tokens: u32,
    /// Whether to prefer streaming responses.
    #[serde(default = "default_true")]
    pub prefer_streaming: bool,
    /// Provider-specific options.
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

fn default_true() -> bool {
    true
}

fn default_temperature() -> f32 {
    0.7
}

fn default_max_tokens() -> u32 {
    4096
}

impl ProviderConfig {
    /// Create a new provider config with sensible defaults.
    pub fn new(id: impl Into<String>, provider_type: ProviderType) -> Self {
        let id = id.into();
        let display_name = provider_type.display_name().to_string();
        Self {
            id,
            provider_type,
            display_name,
            enabled: true,
            endpoint: ProviderEndpointConfig {
                base_url: provider_type.default_endpoint().to_string(),
                ..Default::default()
            },
            active_model: String::new(),
            default_temperature: 0.7,
            default_max_tokens: 4096,
            prefer_streaming: true,
            options: HashMap::new(),
        }
    }

    /// Check if this provider has valid credentials configured.
    pub fn is_configured(&self) -> bool {
        if self.provider_type.requires_api_key() {
            !self.endpoint.api_key.is_empty() && !self.endpoint.base_url.is_empty()
        } else {
            !self.endpoint.base_url.is_empty()
        }
    }
}

/// Top-level provider settings that persist across sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    /// The currently active provider ID.
    pub active_provider: String,
    /// Fallback provider ID (used when active provider fails).
    pub fallback_provider: Option<String>,
    /// All configured providers.
    pub providers: Vec<ProviderConfig>,
    /// Global streaming preference.
    pub prefer_streaming: bool,
    /// Global temperature override (None = use provider default).
    pub global_temperature: Option<f32>,
    /// Global max tokens override (None = use provider default).
    pub global_max_tokens: Option<u32>,
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        Self {
            active_provider: "llama_cpp".to_string(),
            fallback_provider: None,
            providers: vec![
                // Default local llama.cpp provider
                ProviderConfig::new("llama_cpp", ProviderType::LlamaCpp),
                // Default Ollama provider (disabled until configured)
                {
                    let mut cfg = ProviderConfig::new("ollama", ProviderType::Ollama);
                    cfg.enabled = false;
                    cfg
                },
            ],
            prefer_streaming: true,
            global_temperature: None,
            global_max_tokens: None,
        }
    }
}

impl ProvidersConfig {
    /// Get the active provider config.
    pub fn active(&self) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.id == self.active_provider)
    }

    /// Get a provider config by ID.
    pub fn get(&self, id: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// Get a mutable provider config by ID.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut ProviderConfig> {
        self.providers.iter_mut().find(|p| p.id == id)
    }

    /// Add a new provider configuration.
    pub fn add(&mut self, config: ProviderConfig) {
        // Remove existing with same ID
        self.providers.retain(|p| p.id != config.id);
        self.providers.push(config);
    }

    /// Remove a provider by ID.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.providers.len();
        self.providers.retain(|p| p.id != id);
        self.providers.len() < before
    }

    /// List all enabled providers.
    pub fn enabled_providers(&self) -> Vec<&ProviderConfig> {
        self.providers.iter().filter(|p| p.enabled).collect()
    }
}
