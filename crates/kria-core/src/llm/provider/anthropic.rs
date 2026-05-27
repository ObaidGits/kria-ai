//! Anthropic Claude provider backend.
//!
//! Uses the Anthropic Messages API with x-api-key authentication.
//! Supports streaming, tool calling, and vision.

use super::config::ProviderConfig;
use crate::llm::{ChatMessage, LlmBackend, LlmResponse, TokenUsage, ToolSchema};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::time::Duration;

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic Claude backend.
pub struct AnthropicBackend {
    base_url: String,
    api_key: String,
    model_id: String,
    display_name: String,
    capabilities: Vec<String>,
    client: reqwest::Client,
    max_retries: u32,
}

impl AnthropicBackend {
    pub fn from_config(config: &ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.endpoint.timeout_secs))
            .build()
            .unwrap_or_default();

        Self {
            base_url: config.endpoint.base_url.trim_end_matches('/').to_string(),
            api_key: config.endpoint.api_key.clone(),
            model_id: if config.active_model.is_empty() {
                "claude-sonnet-4-20250514".to_string()
            } else {
                config.active_model.clone()
            },
            display_name: config.display_name.clone(),
            capabilities: vec![
                "text".into(),
                "streaming".into(),
                "tools".into(),
                "vision".into(),
            ],
            client,
            max_retries: config.endpoint.max_retries,
        }
    }

    /// Convert KRIA messages to Anthropic format.
    /// Anthropic uses a separate `system` parameter, not a system message in the array.
    fn build_messages(&self, messages: &[ChatMessage]) -> (Option<String>, Vec<serde_json::Value>) {
        let mut system = String::new();
        let mut anthropic_messages = Vec::new();

        for msg in messages {
            if msg.role == "system" {
                if !system.is_empty() {
                    system.push('\n');
                }
                system.push_str(&msg.content);
                continue;
            }

            let role = match msg.role.as_str() {
                "assistant" => "assistant",
                _ => "user",
            };

            // Build content blocks
            let content = if msg.has_images() {
                let mut blocks = Vec::new();
                if let Some(ref images) = msg.images {
                    for img in images {
                        blocks.push(serde_json::json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": img.mime_type,
                                "data": img.data,
                            }
                        }));
                    }
                }
                if !msg.content.is_empty() {
                    blocks.push(serde_json::json!({
                        "type": "text",
                        "text": msg.content,
                    }));
                }
                serde_json::Value::Array(blocks)
            } else {
                serde_json::json!(msg.content)
            };

            anthropic_messages.push(serde_json::json!({
                "role": role,
                "content": content,
            }));
        }

        let system_opt = if system.is_empty() {
            None
        } else {
            Some(system)
        };

        (system_opt, anthropic_messages)
    }

    /// Convert tool schemas to Anthropic format.
    fn build_tools_payload(&self, tools: Option<&[ToolSchema]>) -> Option<Vec<serde_json::Value>> {
        tools.and_then(|t| {
            if t.is_empty() {
                return None;
            }
            let tool_defs: Vec<serde_json::Value> = t
                .iter()
                .map(|ts| {
                    serde_json::json!({
                        "name": ts.name,
                        "description": ts.description,
                        "input_schema": ts.parameters,
                    })
                })
                .collect();
            Some(tool_defs)
        })
    }

    /// Extract text and tool calls from Anthropic response content blocks.
    fn extract_response(content: &[serde_json::Value]) -> (String, Option<Vec<serde_json::Value>>) {
        let mut text = String::new();
        let mut tool_calls: Vec<serde_json::Value> = Vec::new();

        for block in content {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(t) = block["text"].as_str() {
                        text.push_str(t);
                    }
                }
                Some("tool_use") => {
                    let name = block["name"].as_str().unwrap_or("");
                    let input = &block["input"];
                    tool_calls.push(serde_json::json!({
                        "type": "function",
                        "id": block["id"].as_str().unwrap_or(""),
                        "function": {
                            "name": name,
                            "arguments": input.to_string(),
                        }
                    }));
                }
                _ => {}
            }
        }

        let tool_calls_opt = if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        };

        (text, tool_calls_opt)
    }
}

#[async_trait]
impl LlmBackend for AnthropicBackend {
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
        !self.api_key.is_empty()
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        let url = format!("{}/messages", self.base_url);
        let (system, anthropic_messages) = self.build_messages(messages);

        let mut payload = serde_json::json!({
            "model": self.model_id,
            "messages": anthropic_messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
        });

        if let Some(sys) = system {
            payload["system"] = serde_json::json!(sys);
        }

        if let Some(tools_val) = self.build_tools_payload(tools) {
            payload["tools"] = serde_json::Value::Array(tools_val);
        }

        for attempt in 0..self.max_retries {
            let resp = self
                .client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&payload)
                .send()
                .await?;

            let status = resp.status();
            if status.as_u16() == 429 {
                let wait = 2u64.pow(attempt);
                tracing::warn!(
                    attempt,
                    wait_secs = wait,
                    "Anthropic rate limited, retrying"
                );
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }

            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Anthropic API error ({}): {}", status, body);
            }

            let body: serde_json::Value = resp.json().await?;

            let content_blocks = body["content"].as_array().cloned().unwrap_or_default();

            let (content, tool_calls) = Self::extract_response(&content_blocks);

            let usage = body["usage"].as_object().map(|u| TokenUsage {
                prompt_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
                completion_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
                total_tokens: (u["input_tokens"].as_u64().unwrap_or(0)
                    + u["output_tokens"].as_u64().unwrap_or(0))
                    as u32,
            });

            return Ok(LlmResponse {
                content,
                model: self.model_id.clone(),
                usage,
                tool_calls,
            });
        }

        anyhow::bail!("Anthropic failed after {} retries", self.max_retries)
    }

    async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = String> + Send>>> {
        let url = format!("{}/messages", self.base_url);
        let (system, anthropic_messages) = self.build_messages(messages);

        let mut payload = serde_json::json!({
            "model": self.model_id,
            "messages": anthropic_messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "stream": true,
        });

        if let Some(sys) = system {
            payload["system"] = serde_json::json!(sys);
        }

        if let Some(tools_val) = self.build_tools_payload(tools) {
            payload["tools"] = serde_json::Value::Array(tools_val);
        }

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
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
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                                let event_type = v["type"].as_str().unwrap_or("");
                                match event_type {
                                    "content_block_delta" => {
                                        if let Some(delta_text) = v["delta"]["text"].as_str() {
                                            tokens.push_str(delta_text);
                                        }
                                    }
                                    "message_stop" => {}
                                    _ => {}
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
        self.is_configured()
    }
}
