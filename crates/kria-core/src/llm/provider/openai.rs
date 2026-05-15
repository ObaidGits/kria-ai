//! OpenAI and OpenAI-compatible provider backend.
//!
//! Handles: OpenAI, llama.cpp, OpenRouter, and any OpenAI-compatible API.

use super::config::ProviderConfig;
use crate::infra::circuit_breaker::CircuitBreaker;
use crate::llm::{
    extract_openai_content_text, extract_openai_message_text, extract_openai_tool_calls,
    ChatMessage, LlmBackend, LlmResponse, TokenUsage, ToolSchema,
};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// OpenAI-compatible backend that works with OpenAI, llama.cpp, and other
/// compatible APIs.
pub struct OpenAIBackend {
    endpoint: String,
    api_key: String,
    model_id: String,
    display_name: String,
    provider_id: String,
    capabilities: Vec<String>,
    client: reqwest::Client,
    _circuit: Arc<CircuitBreaker>,
    max_retries: u32,
}

impl OpenAIBackend {
    pub fn from_config(config: &ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.endpoint.timeout_secs))
            .build()
            .unwrap_or_default();

        Self {
            endpoint: config.endpoint.base_url.trim_end_matches('/').to_string(),
            api_key: config.endpoint.api_key.clone(),
            model_id: config.active_model.clone(),
            display_name: config.display_name.clone(),
            provider_id: config.id.clone(),
            capabilities: vec!["text".into(), "streaming".into(), "tools".into()],
            client,
            _circuit: Arc::new(CircuitBreaker::with_defaults(&config.id)),
            max_retries: config.endpoint.max_retries,
        }
    }

    fn build_messages_payload(&self, messages: &[ChatMessage]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|msg| {
                if msg.has_images() {
                    serde_json::json!({
                        "role": msg.role,
                        "content": msg.to_multimodal_content(),
                    })
                } else {
                    let mut m = serde_json::json!({
                        "role": msg.role,
                        "content": msg.content,
                    });
                    if let Some(ref name) = msg.name {
                        m["name"] = serde_json::json!(name);
                    }
                    m
                }
            })
            .collect()
    }

    fn build_tools_payload(&self, tools: Option<&[ToolSchema]>) -> Option<serde_json::Value> {
        tools.and_then(|t| {
            if t.is_empty() {
                return None;
            }
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
            Some(serde_json::Value::Array(tool_defs))
        })
    }
}

#[async_trait]
impl LlmBackend for OpenAIBackend {
    fn model_label(&self) -> &str {
        &self.display_name
    }

    fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    fn is_configured(&self) -> bool {
        !self.endpoint.is_empty()
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        let mut payload = serde_json::json!({
            "model": self.model_id,
            "messages": self.build_messages_payload(messages),
            "temperature": temperature,
            "max_tokens": max_tokens,
        });

        if let Some(tools_val) = self.build_tools_payload(tools) {
            payload["tools"] = tools_val;
        }

        let url = format!("{}/chat/completions", self.endpoint);

        for attempt in 0..self.max_retries {
            let mut req = self.client.post(&url).json(&payload);
            if !self.api_key.is_empty() {
                req = req.bearer_auth(&self.api_key);
            }

            let resp = req.send().await?;
            let status = resp.status();

            if status.as_u16() == 429 {
                let wait = 2u64.pow(attempt);
                tracing::warn!(
                    provider = %self.provider_id,
                    attempt,
                    wait_secs = wait,
                    "rate limited, retrying"
                );
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }

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

            return Ok(LlmResponse {
                content,
                model: self.model_id.clone(),
                usage,
                tool_calls,
            });
        }

        anyhow::bail!(
            "{} failed after {} retries (rate limited)",
            self.provider_id,
            self.max_retries
        )
    }

    async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = String> + Send>>> {
        let mut payload = serde_json::json!({
            "model": self.model_id,
            "messages": self.build_messages_payload(messages),
            "temperature": temperature,
            "max_tokens": max_tokens,
            "stream": true,
        });

        if let Some(tools_val) = self.build_tools_payload(tools) {
            payload["tools"] = tools_val;
        }

        let url = format!("{}/chat/completions", self.endpoint);
        let mut req = self.client.post(&url).json(&payload);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }

        let resp = req.send().await?.error_for_status()?;

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
                                let delta_content = &v["choices"][0]["delta"]["content"];
                                let tok = extract_openai_content_text(delta_content);
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
        let url = if self.endpoint.ends_with("/v1") {
            format!("{}/models", self.endpoint)
        } else {
            format!("{}/v1/models", self.endpoint)
        };

        let mut req = self.client.get(&url);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }

        req.send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}
