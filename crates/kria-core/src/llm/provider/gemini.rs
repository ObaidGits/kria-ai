//! Google Gemini provider backend.
//!
//! Uses the Gemini REST API with API key authentication.
//! Supports streaming, tool calling, and vision.

use super::config::ProviderConfig;
use crate::llm::{ChatMessage, LlmBackend, LlmResponse, TokenUsage, ToolSchema};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::time::Duration;

/// Google Gemini backend.
pub struct GeminiBackend {
    base_url: String,
    api_key: String,
    model_id: String,
    display_name: String,
    capabilities: Vec<String>,
    client: reqwest::Client,
    max_retries: u32,
}

impl GeminiBackend {
    pub fn from_config(config: &ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.endpoint.timeout_secs))
            .build()
            .unwrap_or_default();

        Self {
            base_url: config.endpoint.base_url.trim_end_matches('/').to_string(),
            api_key: config.endpoint.api_key.clone(),
            model_id: if config.active_model.is_empty() {
                "gemini-2.0-flash".to_string()
            } else {
                config
                    .active_model
                    .strip_prefix("models/")
                    .unwrap_or(&config.active_model)
                    .to_string()
            },
            display_name: config.display_name.clone(),
            capabilities: vec![
                "text".into(),
                "streaming".into(),
                "tools".into(),
                "vision".into(),
            ],
            client,
            max_retries: config.endpoint.max_retries.max(1),
        }
    }

    /// Convert KRIA messages to Gemini format.
    ///
    /// `role: "tool"` messages are converted to labeled `role: "user"` text
    /// turns rather than Gemini's native `functionResponse` part. Gemini's
    /// `functionResponse` must reference a `functionCall` (and, for Gemini 3,
    /// its `id`) that the model itself emitted in the previous turn — but
    /// KRIA's default tool-calling is text-pattern based (`<tool_call>{...}
    /// </tool_call>` as plain assistant text), so no real `functionCall` part
    /// exists to reference. Emitting an unpaired `functionResponse` risks a
    /// 400. A labeled `user` turn is honest, safe, and consistent with the
    /// same fix applied to the OpenAI-compatible, Ollama, and Anthropic
    /// backends. This also avoids two consecutive `tool` results (multiple
    /// tool calls in one round) becoming untagged back-to-back `user` turns
    /// that some Gemini model configs reject.
    fn build_contents(&self, messages: &[ChatMessage]) -> Vec<serde_json::Value> {
        let mut contents = Vec::new();
        let mut system_instruction = String::new();

        for msg in messages {
            if msg.role == "system" {
                system_instruction.push_str(&msg.content);
                system_instruction.push('\n');
                continue;
            }

            if msg.role.eq_ignore_ascii_case("tool") {
                let tool_name = msg.name.as_deref().unwrap_or("tool");
                contents.push(serde_json::json!({
                    "role": "user",
                    "parts": [{
                        "text": format!("[Tool result from '{}']\n{}", tool_name, msg.content),
                    }],
                }));
                continue;
            }

            let role = match msg.role.as_str() {
                "assistant" => "model",
                _ => "user",
            };

            let mut parts = Vec::new();

            if !msg.content.is_empty() {
                parts.push(serde_json::json!({"text": msg.content}));
            }

            // Add images if present
            if let Some(ref images) = msg.images {
                for img in images {
                    parts.push(serde_json::json!({
                        "inline_data": {
                            "mime_type": img.mime_type,
                            "data": img.data,
                        }
                    }));
                }
            }

            contents.push(serde_json::json!({
                "role": role,
                "parts": parts,
            }));
        }

        contents
    }

    /// Convert tool schemas to Gemini function declarations.
    fn build_tools_payload(&self, tools: Option<&[ToolSchema]>) -> Option<serde_json::Value> {
        tools.and_then(|t| {
            if t.is_empty() {
                return None;
            }
            let declarations: Vec<serde_json::Value> = t
                .iter()
                .map(|ts| {
                    serde_json::json!({
                        "name": ts.name,
                        "description": ts.description,
                        "parameters": ts.parameters,
                    })
                })
                .collect();
            Some(serde_json::json!([{
                "function_declarations": declarations
            }]))
        })
    }
}

#[async_trait]
impl LlmBackend for GeminiBackend {
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
        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, self.model_id, self.api_key
        );

        let contents = self.build_contents(messages);

        let mut payload = serde_json::json!({
            "contents": contents,
            "generationConfig": {
                "temperature": temperature,
                "maxOutputTokens": max_tokens,
            }
        });

        // Add system instruction if present
        let system_text: String = messages
            .iter()
            .filter(|m| m.role == "system")
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !system_text.is_empty() {
            payload["systemInstruction"] = serde_json::json!({
                "parts": [{"text": system_text}]
            });
        }

        if let Some(tools_val) = self.build_tools_payload(tools) {
            payload["tools"] = tools_val;
        }

        for attempt in 0..self.max_retries {
            let resp = self.client.post(&url).json(&payload).send().await?;
            let status = resp.status();

            if status.as_u16() == 429 {
                let wait = 2u64.pow(attempt);
                tracing::warn!(attempt, wait_secs = wait, "Gemini rate limited, retrying");
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }

            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Gemini API error ({}): {}", status, body);
            }

            let body: serde_json::Value = resp.json().await?;

            // Extract content from Gemini response
            let candidate = &body["candidates"][0];
            let parts = &candidate["content"]["parts"];

            let mut content = String::new();
            let mut tool_calls: Option<Vec<serde_json::Value>> = None;

            if let Some(parts_arr) = parts.as_array() {
                for part in parts_arr {
                    if let Some(text) = part["text"].as_str() {
                        content.push_str(text);
                    }
                    if let Some(fc) = part.get("functionCall") {
                        let name = fc["name"].as_str().unwrap_or("");
                        let args = &fc["args"];
                        let call = serde_json::json!({
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": args.to_string(),
                            }
                        });
                        tool_calls.get_or_insert_with(Vec::new).push(call);
                    }
                }
            }

            // Extract usage
            let usage = body["usageMetadata"].as_object().map(|u| TokenUsage {
                prompt_tokens: u["promptTokenCount"].as_u64().unwrap_or(0) as u32,
                completion_tokens: u["candidatesTokenCount"].as_u64().unwrap_or(0) as u32,
                total_tokens: u["totalTokenCount"].as_u64().unwrap_or(0) as u32,
            });

            return Ok(LlmResponse {
                content,
                model: self.model_id.clone(),
                usage,
                tool_calls,
            });
        }

        anyhow::bail!(
            "Gemini failed after {} retries (rate limited)",
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
        let url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse&key={}",
            self.base_url, self.model_id, self.api_key
        );

        let contents = self.build_contents(messages);

        let mut payload = serde_json::json!({
            "contents": contents,
            "generationConfig": {
                "temperature": temperature,
                "maxOutputTokens": max_tokens,
            }
        });

        let system_text: String = messages
            .iter()
            .filter(|m| m.role == "system")
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !system_text.is_empty() {
            payload["systemInstruction"] = serde_json::json!({
                "parts": [{"text": system_text}]
            });
        }

        if let Some(tools_val) = self.build_tools_payload(tools) {
            payload["tools"] = tools_val;
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
                        "Gemini stream rate limited, retrying"
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
                    "Gemini stream failed after {} retries (rate limited)",
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
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                                if let Some(parts) =
                                    v["candidates"][0]["content"]["parts"].as_array()
                                {
                                    for part in parts {
                                        if let Some(t) = part["text"].as_str() {
                                            tokens.push_str(t);
                                        }
                                    }
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
        if self.api_key.is_empty() {
            return false;
        }
        let url = format!("{}/models?key={}", self.base_url, self.api_key);
        self.client
            .get(&url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> GeminiBackend {
        GeminiBackend {
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            api_key: "test-key".into(),
            model_id: "gemini-2.0-flash".into(),
            display_name: "Gemini".into(),
            capabilities: vec!["text".into()],
            client: reqwest::Client::new(),
            max_retries: 1,
        }
    }

    /// Regression test: a bare `role: "tool"` message (KRIA's text-pattern tool
    /// calling has no `functionCall` id to pair with) must NOT be sent as a
    /// native `functionResponse` part, and must not silently collapse into an
    /// untagged `user` turn either — it is wrapped with a clear "[Tool result
    /// from ...]" prefix, avoiding both an unpaired-functionResponse rejection
    /// and ambiguous back-to-back untagged `user` turns.
    #[test]
    fn build_contents_wraps_bare_tool_role_as_labeled_user_turn() {
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

        let contents = b.build_contents(&messages);

        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[1]["role"], "user");
        let text = contents[1]["parts"][0]["text"].as_str().unwrap();
        assert!(text.contains("list_installed_skills"));
        assert!(text.contains("skills"));
        // Must NOT be a native functionResponse part.
        assert!(contents[1]["parts"][0].get("functionResponse").is_none());
    }

    #[test]
    fn build_contents_maps_assistant_role_to_model() {
        let b = backend();
        let messages = vec![ChatMessage {
            role: "assistant".into(),
            content: "hi".into(),
            name: None,
            images: None,
        }];

        let contents = b.build_contents(&messages);
        assert_eq!(contents[0]["role"], "model");
    }
}
