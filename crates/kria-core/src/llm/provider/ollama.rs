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
    max_retries: u32,
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
            max_retries: config.endpoint.max_retries.max(1),
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

    /// Build the wire-format messages array. Converts bare `role: "tool"`
    /// messages into `role: "user"` turns with the tool result embedded as
    /// text — text-pattern tool calls (KRIA's default) have no `tool_call_id`,
    /// and a bare `role: "tool"` message can be rejected by stricter
    /// OpenAI-compatible endpoints/proxies fronting Ollama with a 400 "tool
    /// result's tool id() not found" error. This mirrors the same fix applied
    /// to `OpenAIBackend::structured_wire_messages`.
    fn build_messages(&self, messages: &[ChatMessage]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|msg| {
                if msg.role.eq_ignore_ascii_case("tool") {
                    let tool_name = msg.name.as_deref().unwrap_or("tool");
                    serde_json::json!({
                        "role": "user",
                        "content": format!("[Tool result from '{}']\n{}", tool_name, msg.content),
                    })
                } else if msg.has_images() {
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

        for attempt in 0..self.max_retries {
            let resp = self.client.post(&url).json(&payload).send().await?;
            let status = resp.status();

            if status.as_u16() == 429 {
                let wait = 2u64.pow(attempt);
                tracing::warn!(attempt, wait_secs = wait, "Ollama rate limited, retrying");
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
            "Ollama failed after {} retries (rate limited)",
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

        // Retry on 429 before opening the stream (mirrors the non-streaming
        // `chat` path). Once the stream is opened successfully, in-flight SSE
        // reads are not retried.
        let resp = {
            let mut opened = None;
            for attempt in 0..self.max_retries {
                let r = self.client.post(&url).json(&payload).send().await?;
                if r.status().as_u16() == 429 {
                    let wait = 2u64.pow(attempt);
                    tracing::warn!(
                        attempt,
                        wait_secs = wait,
                        "Ollama stream rate limited, retrying"
                    );
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    continue;
                }
                opened = Some(r.error_for_status()?);
                break;
            }
            match opened {
                Some(r) => r,
                None => anyhow::bail!(
                    "Ollama stream failed after {} retries (rate limited)",
                    self.max_retries
                ),
            }
        };

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

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> OllamaBackend {
        OllamaBackend {
            base_url: "http://localhost:11434".into(),
            model_id: "llama3".into(),
            display_name: "Ollama".into(),
            capabilities: vec!["text".into()],
            client: reqwest::Client::new(),
            max_retries: 1,
        }
    }

    /// Regression test: a bare `role: "tool"` message (KRIA's text-pattern tool
    /// calling has no `tool_call_id`) must be converted to a `role: "user"`
    /// turn, not sent as-is — stricter OpenAI-compatible endpoints/proxies
    /// fronting Ollama reject an untethered `role: "tool"` message with a 400.
    #[test]
    fn build_messages_converts_bare_tool_role_to_user() {
        let b = backend();
        let messages = vec![
            ChatMessage {
                role: "user".into(),
                content: "list skills".into(),
                name: None,
                images: None,
            },
            ChatMessage {
                role: "tool".into(),
                content: "{\"skills\":[]}".into(),
                name: Some("list_installed_skills".into()),
                images: None,
            },
        ];

        let wire = b.build_messages(&messages);

        assert_eq!(wire[0]["role"], "user");
        assert_eq!(wire[1]["role"], "user");
        assert!(wire[1]["content"]
            .as_str()
            .unwrap()
            .contains("list_installed_skills"));
        assert!(wire[1]["content"].as_str().unwrap().contains("skills"));
    }

    #[test]
    fn build_messages_passes_through_non_tool_roles_unchanged() {
        let b = backend();
        let messages = vec![ChatMessage {
            role: "assistant".into(),
            content: "hello".into(),
            name: None,
            images: None,
        }];

        let wire = b.build_messages(&messages);
        assert_eq!(wire[0]["role"], "assistant");
        assert_eq!(wire[0]["content"], "hello");
    }
}
