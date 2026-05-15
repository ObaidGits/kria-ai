//! Core types for the Universal Model Provider system.

use serde::{Deserialize, Serialize};

/// Information about a model available from a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Provider-specific model identifier (e.g., "gpt-4o", "gemini-2.0-flash").
    pub id: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Maximum context window in tokens.
    pub context_window: usize,
    /// Maximum output tokens.
    pub max_output_tokens: usize,
    /// Whether the model supports streaming.
    pub supports_streaming: bool,
    /// Whether the model supports tool/function calling.
    pub supports_tools: bool,
    /// Whether the model supports vision/image input.
    pub supports_vision: bool,
    /// Whether the model supports structured output / JSON mode.
    pub supports_json_mode: bool,
    /// Pricing info (per million tokens), if known.
    pub pricing: Option<ModelPricing>,
    /// Provider-specific metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Pricing information for a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    /// Cost per million input tokens (USD).
    pub input_per_million: f64,
    /// Cost per million output tokens (USD).
    pub output_per_million: f64,
}

/// Runtime status of a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStatus {
    /// Provider identifier.
    pub provider_id: String,
    /// Provider type.
    pub provider_type: String,
    /// Whether the provider is currently configured (has credentials/endpoint).
    pub configured: bool,
    /// Whether the provider is currently reachable.
    pub reachable: bool,
    /// Currently selected model ID.
    pub active_model: Option<String>,
    /// Available models (may be empty if not yet discovered).
    pub available_models: Vec<ModelInfo>,
    /// Last error message, if any.
    pub last_error: Option<String>,
    /// Whether this is the currently active provider.
    pub is_active: bool,
}

/// Execution location classification for hardware orchestrator integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionLocation {
    /// Model runs locally (GPU/CPU resources needed).
    Local,
    /// Model runs in the cloud (no local GPU needed).
    Cloud,
    /// Hybrid: some processing local, some cloud.
    Hybrid,
}

/// Provider health snapshot for telemetry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealthSnapshot {
    pub provider_id: String,
    pub is_healthy: bool,
    pub latency_ms: Option<u64>,
    pub error_count: u32,
    pub last_success_epoch_ms: Option<u64>,
    pub execution_location: ExecutionLocation,
}
