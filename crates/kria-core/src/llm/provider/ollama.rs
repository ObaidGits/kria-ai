//! Ollama provider backend.
//!
//! Communicates with a local Ollama instance via its REST API.
//! Supports model discovery, streaming, and tool calling (Ollama 0.4+).

use super::config::ProviderConfig;
use crate::llm::{
    extract_openai_content_text, extract_openai_message_text, extract_openai_tool_calls,
    ChatMessage, LlmBackend, LlmResponse, TokenUsage, ToolSchema,
};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::time::Duration;

/// Ollama backend using the Ollama REST API.
///
/// Ollama exposes both its native API (`/api/chat`) and an OpenAI-compatible
/// endpoint (`/v1/chat/completions`). We use the OpenAI-compatible endpoint
/// for consistency with the rest of the provider system.
pub struct OllamaBackend {
    base_url: String,
    model_id: String,
    display_name: String,
    capabilities: Vec<String>,
    client: reqwest::Client,
}

impl OllamaBackend {
    pub fn from_config(config: &ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.endpoint.timeout_secs.max(120)))
            .build()
            .unwrap_or_default();

        Self {
            base_url: config.endpoint.base_url.trim_end_matches('/').to_string(),
            model_id: config.active_model.clone(),
            display_name: config.display_name.clone(),
            capabilities: vec!["text".into(), "streaming".into(), "tools".into()],
            client,
        }
    }

    /// List available models from Ollama.
    pub async fn list_models(&self) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/api/tags", self.base_url);
        let resp: serde_json::Value = self.client.get(&url).send().await?.json().await?;
        let models = resp["models"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
            .collect();
        Ok(models)
    }

    /// Pull a model (download if not present).
    pub async fn pull_model(&self, model: &str) -> anyhow::Result<()> {
        let url = format!("{}/api/pull", self.base_url);
        let payload = serde_json::json!({ "name": model, "stream": false });
        self.client
            .post(&url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    fn build_messages(&self, messages: &[ChatMessage]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|msg| {
                if msg.has_images() {
                    serde_json::json!({
                        "role": msg.role,
                        "content": msg.to_multimodal_content(),
                    })
                } else {
                    serde_json::json!({
                        "role": msg.role,
                        "content": msg.content,
                    })
                }
            })
            .collect()
    }
}

#[async_trait]
impl LlmBackend for OllamaBackend {
    fn model_label(&self) -> &str {
        if self.model_id.trim().is_empty() {
            &self.display_name
        } else {
            &self.model_id
        }
    }

    fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    fn is_configured(&self) -> bool {
        !self.base_url.is_empty() && !self.model_id.is_empty()
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        // Use OpenAI-compatible endpoint for consistency
        let url = format!("{}/v1/chat/completions", self.base_url);

        let mut payload = serde_json::json!({
            "model": self.model_id,
            "messages": self.build_messages(messages),
            "temperature": temperature,
            "max_tokens": max_tokens,
            "stream": false,
        });

        if let Some(t) = tools {
            if !t.is_empty() {
                let tool_defs: Vec<serde_json::Value> = t
                    .iter()
                    .map(|ts| {
                        serde_json::json!({
                            "type": "function",
                            "function": {
                                "name": ts.name,
                                "description": ts.description,
                                "parameters": ts.parameters,
                            }
                        })
                    })
                    .collect();
                payload["tools"] = serde_json::Value::Array(tool_defs);
            }
        }

        let resp = self.client.post(&url).json(&payload).send().await?;
        let body: serde_json::Value = resp.error_for_status()?.json().await?;

        let choice = &body["choices"][0];
        let message = &choice["message"];
        let content = extract_openai_message_text(message);
        let tool_calls = extract_openai_tool_calls(message);

        let usage = body["usage"].as_object().map(|u| TokenUsage {
            prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: u["total_tokens"].as_u64().unwrap_or(0) as u32,
        });

        Ok(LlmResponse {
            content,
            model: self.model_id.clone(),
            usage,
            tool_calls,
        })
    }

    async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = String> + Send>>> {
        let url = format!("{}/v1/chat/completions", self.base_url);

        let mut payload = serde_json::json!({
            "model": self.model_id,
            "messages": self.build_messages(messages),
            "temperature": temperature,
            "max_tokens": max_tokens,
            "stream": true,
        });

        if let Some(t) = tools {
            if !t.is_empty() {
                let tool_defs: Vec<serde_json::Value> = t
                    .iter()
                    .map(|ts| {
                        serde_json::json!({
                            "type": "function",
                            "function": {
                                "name": ts.name,
                                "description": ts.description,
                                "parameters": ts.parameters,
                            }
                        })
                    })
                    .collect();
                payload["tools"] = serde_json::Value::Array(tool_defs);
            }
        }

        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;

        let stream = futures::stream::unfold(resp, |mut resp| async move {
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    let text = String::from_utf8_lossy(&chunk).to_string();
                    let mut tokens = String::new();
                    for line in text.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data == "[DONE]" {
                                continue;
                            }
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                                let delta = &v["choices"][0]["delta"]["content"];
                                let tok = extract_openai_content_text(delta);
                                if !tok.is_empty() {
                                    tokens.push_str(&tok);
                                }
                            }
                        }
                    }
                    Some((tokens, resp))
                }
                _ => None,
            }
        });

        Ok(Box::pin(stream))
    }

    async fn health_check(&self) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        self.client
            .get(&url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Ollama's OpenAI-compatible endpoint honors `response_format`
    /// `json_schema` (and native `format` json), so its structured-output mode
    /// is `JsonSchema` (Requirement 0.3). `supports_grammar()` derives `false`
    /// (grammar is the local llama.cpp-only mode) for back-compat.
    fn structured_output_mode(&self) -> crate::llm::StructuredOutputMode {
        crate::llm::StructuredOutputMode::JsonSchema
    }

    /// Structured-output path (Requirement 0.2/0.3): post a non-streaming
    /// `response_format` `json_schema` request and return the JSON object.
    async fn chat_structured(
        &self,
        messages: &[ChatMessage],
        json_schema: serde_json::Value,
        schema_name: &str,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let payload = serde_json::json!({
            "model": self.model_id,
            "messages": self.build_messages(messages),
            "temperature": temperature,
            "max_tokens": max_tokens,
            "stream": false,
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": schema_name, "strict": true, "schema": json_schema }
            }
        });
        let resp = self.client.post(&url).json(&payload).send().await?;
        let body: serde_json::Value = resp.error_for_status()?.json().await?;
        let message = &body["choices"][0]["message"];
        let content = extract_openai_message_text(message);
        let tool_calls = extract_openai_tool_calls(message);
        let usage = body["usage"].as_object().map(|u| TokenUsage {
            prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: u["total_tokens"].as_u64().unwrap_or(0) as u32,
        });
        Ok(LlmResponse {
            content,
            model: self.model_id.clone(),
            usage,
            tool_calls,
        })
    }
}
