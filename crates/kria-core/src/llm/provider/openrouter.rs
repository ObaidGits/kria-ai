//! OpenRouter provider backend.
//!
//! OpenRouter is a multi-provider gateway that exposes an OpenAI-compatible API.
//! It adds custom headers for routing and attribution.

use super::config::ProviderConfig;
use super::openai::OpenAIBackend;
use crate::llm::{ChatMessage, LlmBackend, LlmResponse, StructuredOutputMode, ToolSchema};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

/// OpenRouter backend — wraps OpenAI-compatible with OpenRouter-specific headers.
pub struct OpenRouterBackend {
    inner: OpenAIBackend,
    /// App name for OpenRouter attribution.
    _app_name: String,
}

impl OpenRouterBackend {
    pub fn from_config(config: &ProviderConfig) -> Self {
        // OpenRouter uses the same OpenAI-compatible format
        let inner = OpenAIBackend::from_config(config);

        let app_name = config
            .options
            .get("app_name")
            .and_then(|v| v.as_str())
            .unwrap_or("KRIA")
            .to_string();

        Self {
            inner,
            _app_name: app_name,
        }
    }
}

#[async_trait]
impl LlmBackend for OpenRouterBackend {
    fn model_label(&self) -> &str {
        self.inner.model_label()
    }

    fn capabilities(&self) -> &[String] {
        self.inner.capabilities()
    }

    fn is_configured(&self) -> bool {
        self.inner.is_configured()
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        // OpenRouter uses the same format as OpenAI
        self.inner
            .chat(messages, tools, temperature, max_tokens)
            .await
    }

    async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = String> + Send>>> {
        self.inner
            .chat_stream(messages, tools, temperature, max_tokens)
            .await
    }

    async fn health_check(&self) -> bool {
        self.inner.health_check().await
    }

    fn structured_output_mode(&self) -> StructuredOutputMode {
        self.inner.structured_output_mode()
    }

    fn supports_grammar(&self) -> bool {
        self.inner.supports_grammar()
    }

    async fn detect_structured_output_mode(&self) -> StructuredOutputMode {
        self.inner.detect_structured_output_mode().await
    }

    async fn chat_structured(
        &self,
        messages: &[ChatMessage],
        json_schema: serde_json::Value,
        schema_name: &str,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        self.inner
            .chat_structured(messages, json_schema, schema_name, temperature, max_tokens)
            .await
    }
}
