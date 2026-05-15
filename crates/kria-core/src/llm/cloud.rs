use crate::llm::{
    extract_openai_content_text, extract_openai_message_text, extract_openai_tool_calls,
    ChatMessage, LlmBackend, LlmResponse, TokenUsage, ToolSchema,
};
use async_trait::async_trait;
use futures::Stream;
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Cloud LLM backend via OpenAI-compatible API (Gemini, GPT, Claude, Groq, OpenRouter).
pub struct CloudBackend {
    endpoint: String,
    api_key: String,
    model_id: String,
    display_name: String,
    capabilities: Vec<String>,
    client: reqwest::Client,
    rate_limiter: Option<RateLimiter>,
    /// Whether this backend supports tool/function calling.
    /// Set to false for models that return 400 on tool payloads.
    supports_tools: std::sync::atomic::AtomicBool,
}

struct RateLimiter {
    rpm: u32,
    timestamps: Mutex<VecDeque<Instant>>,
}

impl RateLimiter {
    fn new(rpm: u32) -> Self {
        Self {
            rpm,
            timestamps: Mutex::new(VecDeque::new()),
        }
    }

    async fn acquire(&self) {
        loop {
            let should_wait = {
                let mut ts = self.timestamps.lock().unwrap();
                let now = Instant::now();
                let window = Duration::from_secs(60);
                ts.retain(|t| now.duration_since(*t) < window);
                if (ts.len() as u32) < self.rpm {
                    ts.push_back(now);
                    false
                } else {
                    true
                }
            };
            if should_wait {
                tokio::time::sleep(Duration::from_millis(500)).await;
            } else {
                break;
            }
        }
    }
}

impl CloudBackend {
    pub fn new(
        endpoint: String,
        api_key: String,
        model_id: String,
        display_name: String,
        capabilities: Vec<String>,
        rate_limit_rpm: Option<u32>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        Self {
            endpoint,
            api_key,
            model_id,
            display_name,
            capabilities,
            client,
            rate_limiter: rate_limit_rpm.map(RateLimiter::new),
            supports_tools: std::sync::atomic::AtomicBool::new(true),
        }
    }
}

#[async_trait]
impl LlmBackend for CloudBackend {
    fn model_label(&self) -> &str {
        &self.display_name
    }

    fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    fn is_configured(&self) -> bool {
        !self.api_key.is_empty() && !self.endpoint.is_empty()
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        use std::sync::atomic::Ordering;

        if let Some(ref rl) = self.rate_limiter {
            rl.acquire().await;
        }

        // Build the wire-format messages array.
        //
        // IMPORTANT: `role: "tool"` messages require a matching `tool_call_id`
        // from the assistant's prior tool_calls array. When KRIA uses text-pattern
        // tool calls (not native function calling), there is no tool_call_id.
        // Sending bare `role: "tool"` messages to providers like Minimax/Anthropic
        // causes a 400 "tool result's tool id() not found" error.
        //
        // Fix: convert `role: "tool"` messages to `role: "user"` messages with
        // the tool result embedded as text. This is safe — the LLM still sees
        // the tool output, just formatted as a user turn.
        let build_messages = |msgs: &[ChatMessage]| {
            msgs.iter().map(|m| {
                if m.role.eq_ignore_ascii_case("tool") {
                    // Wrap tool result as a user message so providers without
                    // native tool-result support don't reject the conversation.
                    let tool_name = m.name.as_deref().unwrap_or("tool");
                    serde_json::json!({
                        "role": "user",
                        "content": format!("[Tool result from '{}']\n{}", tool_name, m.content),
                    })
                } else {
                    let mut msg = serde_json::json!({
                        "role": m.role,
                        "content": m.content,
                    });
                    if let Some(ref name) = m.name {
                        msg["name"] = serde_json::json!(name);
                    }
                    msg
                }
            }).collect::<Vec<_>>()
        };

        let build_tools = |t: &[ToolSchema]| -> Vec<serde_json::Value> {
            t.iter().map(|ts| serde_json::json!({
                "type": "function",
                "function": {
                    "name": ts.name,
                    "description": ts.description,
                    "parameters": ts.parameters,
                }
            })).collect()
        };

        let url = format!("{}/chat/completions", self.endpoint);

        // Determine whether to include tools in this request.
        // If a previous call got a 400 from this backend, we learned it
        // doesn't support tools and skip them permanently for this session.
        let include_tools = self.supports_tools.load(Ordering::Relaxed)
            && tools.map(|t| !t.is_empty()).unwrap_or(false);

        let make_payload = |with_tools: bool| {
            let mut p = serde_json::json!({
                "model": self.model_id,
                "messages": build_messages(messages),
                "temperature": temperature,
                "max_tokens": max_tokens,
            });
            if with_tools {
                if let Some(t) = tools {
                    if !t.is_empty() {
                        p["tools"] = serde_json::Value::Array(build_tools(t));
                    }
                }
            }
            p
        };

        for attempt in 0..3u32 {
            let payload = make_payload(include_tools);

            let resp = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&payload)
                .send()
                .await?;

            let status = resp.status().as_u16();

            match status {
                429 => {
                    let wait = 2u64.pow(attempt);
                    tracing::warn!(attempt, wait_secs = wait, "cloud LLM rate limited, retrying");
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    continue;
                }
                401 | 403 => {
                    let body = resp.text().await.unwrap_or_default();
                    anyhow::bail!(
                        "Authentication failed ({status}) for endpoint '{}'. \
                         Check your API key. Details: {body}",
                        self.endpoint
                    );
                }
                400 => {
                    let body = resp.text().await.unwrap_or_default();
                    tracing::warn!(
                        endpoint = %self.endpoint,
                        model = %self.model_id,
                        body = %body,
                        "cloud LLM 400 Bad Request"
                    );
                    // If we sent tools and the model rejected them, disable
                    // tools for this backend and retry without them.
                    if include_tools && (body.contains("tool") || body.contains("function")) {
                        tracing::warn!(
                            model = %self.model_id,
                            "model does not support tool calling — disabling tools for this session"
                        );
                        self.supports_tools.store(false, Ordering::Relaxed);
                        // Retry immediately without tools
                        let payload_no_tools = make_payload(false);
                        let resp2 = self
                            .client
                            .post(&url)
                            .bearer_auth(&self.api_key)
                            .json(&payload_no_tools)
                            .send()
                            .await?;
                        let body2: serde_json::Value =
                            resp2.error_for_status()?.json().await?;
                        return Ok(Self::parse_response(body2, &self.model_id));
                    }
                    anyhow::bail!(
                        "Bad request (400) to '{}': {body}",
                        self.endpoint
                    );
                }
                200..=299 => {
                    let body: serde_json::Value = resp.json().await?;
                    return Ok(Self::parse_response(body, &self.model_id));
                }
                _ => {
                    let body = resp.text().await.unwrap_or_default();
                    anyhow::bail!(
                        "Cloud LLM error ({status}) from '{}': {body}",
                        self.endpoint
                    );
                }
            }
        }

        anyhow::bail!("cloud LLM failed after 3 retries")
    }

    async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = String> + Send>>> {
        use std::sync::atomic::Ordering;

        if let Some(ref rl) = self.rate_limiter {
            rl.acquire().await;
        }

        let include_tools = self.supports_tools.load(Ordering::Relaxed)
            && tools.map(|t| !t.is_empty()).unwrap_or(false);

        // Build wire-format messages with tool→user conversion (same as chat())
        let wire_messages: Vec<serde_json::Value> = messages.iter().map(|m| {
            if m.role.eq_ignore_ascii_case("tool") {
                let tool_name = m.name.as_deref().unwrap_or("tool");
                serde_json::json!({
                    "role": "user",
                    "content": format!("[Tool result from '{}']\n{}", tool_name, m.content),
                })
            } else {
                let mut msg = serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                });
                if let Some(ref name) = m.name {
                    msg["name"] = serde_json::json!(name);
                }
                msg
            }
        }).collect();

        let mut payload = serde_json::json!({
            "model": self.model_id,
            "messages": wire_messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "stream": true,
        });

        if include_tools {
            if let Some(t) = tools {
                if !t.is_empty() {
                    let tool_defs: Vec<serde_json::Value> = t
                        .iter()
                        .map(|ts| serde_json::json!({
                            "type": "function",
                            "function": {
                                "name": ts.name,
                                "description": ts.description,
                                "parameters": ts.parameters,
                            }
                        }))
                        .collect();
                    payload["tools"] = serde_json::Value::Array(tool_defs);
                }
            }
        }

        let url = format!("{}/chat/completions", self.endpoint);

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await?;

        let status = resp.status().as_u16();

        // Handle 400 before consuming the stream — strip tools and retry.
        if status == 400 {
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(
                endpoint = %self.endpoint,
                model = %self.model_id,
                body = %body,
                "cloud LLM stream 400 Bad Request"
            );
            if include_tools {
                tracing::warn!(
                    model = %self.model_id,
                    "model does not support tool calling — disabling tools and retrying stream"
                );
                self.supports_tools.store(false, Ordering::Relaxed);
                // Remove tools and retry
                payload.as_object_mut().map(|o| o.remove("tools"));
                let resp2 = self
                    .client
                    .post(&url)
                    .bearer_auth(&self.api_key)
                    .json(&payload)
                    .send()
                    .await?
                    .error_for_status()?;
                return Ok(Self::make_sse_stream(resp2));
            }
            anyhow::bail!("Bad request (400) to '{}': {body}", self.endpoint);
        }

        if status == 401 || status == 403 {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Authentication failed ({status}) for endpoint '{}'. \
                 Check your API key. Details: {body}",
                self.endpoint
            );
        }

        let resp = resp.error_for_status()?;
        Ok(Self::make_sse_stream(resp))
    }

    async fn health_check(&self) -> bool {
        self.is_configured()
    }
}

impl CloudBackend {
    fn parse_response(body: serde_json::Value, model_id: &str) -> LlmResponse {
        let choice = &body["choices"][0];
        let message = &choice["message"];
        let content = extract_openai_message_text(message);
        let tool_calls = extract_openai_tool_calls(message);
        let usage = body["usage"].as_object().map(|u| TokenUsage {
            prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: u["total_tokens"].as_u64().unwrap_or(0) as u32,
        });
        LlmResponse {
            content,
            model: model_id.to_string(),
            usage,
            tool_calls,
        }
    }

    fn make_sse_stream(resp: reqwest::Response) -> Pin<Box<dyn Stream<Item = String> + Send>> {
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
        Box::pin(stream)
    }
}
