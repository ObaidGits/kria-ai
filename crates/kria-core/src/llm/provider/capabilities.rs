//! Provider capability negotiation and model metadata normalization.
//!
//! Different providers expose different capabilities. This module provides
//! a normalized capability contract so the orchestration layer never needs
//! to know which provider is active.

use serde::{Deserialize, Serialize};

/// Capabilities that a provider or model may support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderCapability {
    /// Basic chat completion.
    ChatCompletion,
    /// Server-sent event streaming.
    Streaming,
    /// Tool/function calling.
    ToolCalling,
    /// Vision/image input.
    Vision,
    /// Text embeddings generation.
    Embeddings,
    /// Structured output / JSON mode.
    JsonMode,
    /// Reasoning/chain-of-thought (e.g., o1-style models).
    Reasoning,
    /// Code execution sandbox.
    CodeExecution,
    /// Audio input/output.
    Audio,
    /// Multi-turn conversation with system messages.
    SystemMessages,
    /// Batch/async completions.
    BatchCompletion,
}

/// Normalized model capabilities for a specific model on a specific provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilities {
    /// The model identifier.
    pub model_id: String,
    /// Set of supported capabilities.
    pub capabilities: Vec<ProviderCapability>,
    /// Maximum context window (tokens).
    pub context_window: usize,
    /// Maximum output tokens.
    pub max_output_tokens: usize,
    /// Whether the model supports parallel tool calls.
    pub parallel_tool_calls: bool,
    /// Maximum number of images per request (0 = no vision).
    pub max_images: usize,
    /// Supported image formats (e.g., "png", "jpeg", "webp").
    pub supported_image_formats: Vec<String>,
    /// Maximum tokens for reasoning/thinking (0 = not supported).
    pub max_reasoning_tokens: usize,
}

impl ModelCapabilities {
    /// Check if a specific capability is supported.
    pub fn supports(&self, cap: ProviderCapability) -> bool {
        self.capabilities.contains(&cap)
    }

    /// Check if the model can handle tool calling.
    pub fn can_use_tools(&self) -> bool {
        self.supports(ProviderCapability::ToolCalling)
    }

    /// Check if the model can handle vision input.
    pub fn can_see(&self) -> bool {
        self.supports(ProviderCapability::Vision)
    }

    /// Check if the model supports streaming.
    pub fn can_stream(&self) -> bool {
        self.supports(ProviderCapability::Streaming)
    }

    /// Create a minimal capability set (chat only).
    pub fn chat_only(model_id: String, context_window: usize, max_output: usize) -> Self {
        Self {
            model_id,
            capabilities: vec![ProviderCapability::ChatCompletion],
            context_window,
            max_output_tokens: max_output,
            parallel_tool_calls: false,
            max_images: 0,
            supported_image_formats: vec![],
            max_reasoning_tokens: 0,
        }
    }

    /// Create a full-featured capability set.
    pub fn full_featured(model_id: String, context_window: usize, max_output: usize) -> Self {
        Self {
            model_id,
            capabilities: vec![
                ProviderCapability::ChatCompletion,
                ProviderCapability::Streaming,
                ProviderCapability::ToolCalling,
                ProviderCapability::Vision,
                ProviderCapability::JsonMode,
                ProviderCapability::SystemMessages,
            ],
            context_window,
            max_output_tokens: max_output,
            parallel_tool_calls: true,
            max_images: 10,
            supported_image_formats: vec![
                "png".into(),
                "jpeg".into(),
                "webp".into(),
                "gif".into(),
            ],
            max_reasoning_tokens: 0,
        }
    }
}
