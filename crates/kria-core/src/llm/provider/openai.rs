//! OpenAI and OpenAI-compatible provider backend.
//!
//! Handles: OpenAI, llama.cpp, OpenRouter, and any OpenAI-compatible API.

use super::config::ProviderConfig;
use crate::infra::circuit_breaker::CircuitBreaker;
use crate::llm::{
    extract_first_json_object, extract_openai_content_text, extract_openai_message_text,
    extract_openai_tool_calls, ChatMessage, LlmBackend, LlmResponse,
    StructuredOutputMode, TokenUsage, ToolSchema,
};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
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
    /// Explicit per-provider structured-output override (Requirement 0.2). When
    /// `Some`, the runtime probe is skipped and this mode is used as-is. This is
    /// how CI tests inject capability without depending on a live probe, and how
    /// an operator can pin a known-good mode for a proxy.
    structured_override: Option<StructuredOutputMode>,
    /// Cached result of the runtime structured-capability probe, keyed implicitly
    /// by this backend instance (one instance per provider+model). `None` until
    /// the first [`detect_structured_output_mode`](LlmBackend::detect_structured_output_mode).
    structured_cache: Mutex<Option<StructuredOutputMode>>,
}

impl OpenAIBackend {
    /// Safe OpenAI-compatible default structured-output mode when capability is
    /// unknown/unprobed or a probe yielded `None` (Requirement 0.2). Most
    /// OpenAI-compatible endpoints honor `response_format:{type:"json_object"}`;
    /// the planner's strict-validate + bounded re-ask is the safety net. This is
    /// the shared OpenAI-compatible backend for BOTH cloud (opencode/zen) and
    /// local (llama.cpp): the probe correctly upgrades llama.cpp to JsonSchema,
    /// while JsonObject is a safe fallback for unknown providers.
    const DEFAULT_UNKNOWN_STRUCTURED_MODE: StructuredOutputMode = StructuredOutputMode::JsonObject;

    /// Total bounded budget for the runtime structured-capability probe (up to
    /// three sequential calls). Gives the probe its own deadline so a positive
    /// detection has room to complete; on timeout the caller falls back to the
    /// safe default WITHOUT caching `None`.
    const PROBE_BUDGET: Duration = Duration::from_secs(90);

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
            structured_override: None,
            structured_cache: Mutex::new(None),
        }
    }

    /// Pin an explicit structured-output mode for this provider (Requirement
    /// 0.2 per-provider `structured_output` config override). When set, the
    /// runtime probe is skipped. Used by config wiring and by CI tests to inject
    /// capability deterministically without a live probe.
    pub fn with_structured_output_mode(mut self, mode: StructuredOutputMode) -> Self {
        self.structured_override = Some(mode);
        if let Ok(mut cache) = self.structured_cache.lock() {
            *cache = Some(mode);
        }
        self
    }

    fn cached_structured_mode(&self) -> Option<StructuredOutputMode> {
        self.structured_override
            .or_else(|| self.structured_cache.lock().ok().and_then(|c| *c))
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

    /// Synchronous, cached/configured structured-output view. Returns the
    /// per-provider override or the cached POSITIVE probe result; otherwise the
    /// safe OpenAI-compatible default of `json_object` (Requirement 0.2). `None`
    /// is never returned as a default — it is reserved for an explicit
    /// per-provider override of a backend that truly cannot do structured output.
    fn structured_output_mode(&self) -> StructuredOutputMode {
        self.cached_structured_mode()
            .unwrap_or(Self::DEFAULT_UNKNOWN_STRUCTURED_MODE)
    }

    /// Cheap, cached per-provider+model runtime probe (Requirement 0.2). Detects
    /// what the endpoint ACTUALLY honors (a proxy like opencode/zen may strip
    /// `response_format`) and caches it so later planner turns do not re-probe.
    /// When a per-provider override is configured, the probe is skipped entirely.
    ///
    /// Bug-fix (LIVE gate): a transient/None probe result is **never cached**.
    /// Only a POSITIVE detection is cached; if the probe yields `None` (all
    /// attempts errored, timed out, or were cancelled) the value is left uncached
    /// so a later turn can re-probe, AND the safe default (`json_object`) is
    /// returned rather than poisoning the process with `None`. The probe is also
    /// bounded by its own timeout so it is not starved by the caller's deadline.
    async fn detect_structured_output_mode(&self) -> StructuredOutputMode {
        if let Some(mode) = self.cached_structured_mode() {
            return mode;
        }
        let probe = tokio::time::timeout(Self::PROBE_BUDGET, self.probe_structured_output_mode())
            .await
            .unwrap_or(StructuredOutputMode::None);

        if probe.is_structured() {
            // Cache ONLY a positive detection.
            if let Ok(mut cache) = self.structured_cache.lock() {
                *cache = Some(probe);
            }
            probe
        } else {
            // Do NOT cache None — leave uncached so a later turn re-probes, and
            // fall back to the safe OpenAI-compatible default.
            Self::DEFAULT_UNKNOWN_STRUCTURED_MODE
        }
    }

    async fn chat_structured(
        &self,
        messages: &[ChatMessage],
        json_schema: serde_json::Value,
        schema_name: &str,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        // Resolve the mode WITHOUT a blocking multi-call probe in the planner's
        // hot path. Use the per-provider override or a previously-cached POSITIVE
        // probe result; otherwise the safe OpenAI-compatible default
        // (`json_object`). The planner's strict validate + bounded re-ask is the
        // safety net.
        let mode = self
            .cached_structured_mode()
            .unwrap_or(Self::DEFAULT_UNKNOWN_STRUCTURED_MODE);
        self.chat_structured_with_mode(
            messages,
            &json_schema,
            schema_name,
            mode,
            temperature,
            max_tokens,
        )
        .await
    }
}

// ── Shared multi-backend structured-output adapter (Requirement 0.2) ──────────
//
// Ported from `crate::llm::cloud::CloudBackend` onto the shared OpenAI-compatible
// backend used for BOTH cloud (opencode/zen) and local (llama.cpp). Additive
// only — `chat`/`chat_stream` behavior is unchanged.
impl OpenAIBackend {
    /// Parse an OpenAI-compatible chat/completions body into an [`LlmResponse`].
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

    /// Build the OpenAI wire-format messages array. Preserves multimodal
    /// (image) content and `name`, and converts `role: "tool"` messages to
    /// `user` turns (text-pattern tool calls have no `tool_call_id`).
    fn structured_wire_messages(&self, messages: &[ChatMessage]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|m| {
                if m.role.eq_ignore_ascii_case("tool") {
                    let tool_name = m.name.as_deref().unwrap_or("tool");
                    serde_json::json!({
                        "role": "user",
                        "content": format!("[Tool result from '{}']\n{}", tool_name, m.content),
                    })
                } else if m.has_images() {
                    serde_json::json!({
                        "role": m.role,
                        "content": m.to_multimodal_content(),
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
            })
            .collect()
    }

    /// Produce a compact one-level skeleton example object from a JSON schema so
    /// `json_object`-mode requests can carry ONE few-shot of the expected shape
    /// (DeepSeek requires an in-prompt example for `json_object`).
    fn schema_skeleton(schema: &serde_json::Value) -> serde_json::Value {
        match schema.get("type").and_then(|t| t.as_str()) {
            Some("object") => {
                let mut obj = serde_json::Map::new();
                if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
                    for (key, value) in props.iter().take(12) {
                        obj.insert(key.clone(), Self::schema_skeleton(value));
                    }
                }
                serde_json::Value::Object(obj)
            }
            Some("array") => {
                let item = schema
                    .get("items")
                    .map(Self::schema_skeleton)
                    .unwrap_or(serde_json::Value::Null);
                serde_json::Value::Array(vec![item])
            }
            Some("string") => serde_json::Value::String(
                schema
                    .get("enum")
                    .and_then(|e| e.as_array())
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .unwrap_or("text")
                    .to_string(),
            ),
            Some("integer") | Some("number") => serde_json::json!(0),
            Some("boolean") => serde_json::json!(false),
            _ => serde_json::Value::Null,
        }
    }

    /// Append a `json_object`-mode instruction message: the literal word "json"
    /// + the compact schema + one few-shot example (per DeepSeek requirements).
    fn augment_messages_for_json_object(
        &self,
        messages: &[ChatMessage],
        schema: &serde_json::Value,
    ) -> Vec<serde_json::Value> {
        let mut wire = self.structured_wire_messages(messages);
        let example = Self::schema_skeleton(schema);
        let instruction = format!(
            "Respond with a single valid json object — no prose, no markdown fences. \
             The json object MUST conform to this JSON schema:\n{}\n\nExample of the \
             required json shape (values are placeholders):\n{}",
            serde_json::to_string(schema).unwrap_or_default(),
            serde_json::to_string(&example).unwrap_or_default(),
        );
        wire.push(serde_json::json!({ "role": "system", "content": instruction }));
        wire
    }

    fn base_payload(
        &self,
        wire_messages: Vec<serde_json::Value>,
        temperature: f32,
        max_tokens: u32,
    ) -> serde_json::Value {
        serde_json::json!({
            "model": self.model_id,
            "messages": wire_messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "stream": false,
        })
    }

    /// POST a non-streaming chat/completions payload, honoring the existing
    /// bearer-auth + 429 retry conventions of [`OpenAIBackend`]. On 2xx return
    /// the parsed body, otherwise bail with the status + body so callers can
    /// classify (and step past) a non-2xx/error response.
    async fn send_structured_once(
        &self,
        payload: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}/chat/completions", self.endpoint);
        let retries = self.max_retries.max(1);
        for attempt in 0..retries {
            let mut req = self.client.post(&url).json(payload);
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
                    "structured request rate limited, retrying"
                );
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }
            if status.is_success() {
                return Ok(resp.json().await?);
            }
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "structured request failed (status {}): {}",
                status.as_u16(),
                body
            );
        }
        anyhow::bail!(
            "{} structured request failed after {} retries (rate limited)",
            self.provider_id,
            retries
        )
    }

    /// Whether `content` carries a single JSON object (the normalization
    /// target). Tolerant (Requirement 0.4): strips ```json code fences, leading
    /// "thinking"-model preamble, and trailing prose by extracting the FIRST
    /// balanced top-level `{...}` object before validating. Still strict — the
    /// extracted candidate must itself parse as a JSON object.
    fn content_is_json_object(content: &str) -> bool {
        extract_first_json_object(content).is_some()
    }

    /// Runtime probe (Requirement 0.2): detect the strongest structured method
    /// the endpoint genuinely honors. Bounded, best-effort, cached by the caller.
    /// The ladder is json_schema → json_object → tool-calling, and it steps past
    /// a non-2xx/error on json_schema or tool_choice and still tries json_object.
    async fn probe_structured_output_mode(&self) -> StructuredOutputMode {
        let probe_schema = serde_json::json!({
            "type": "object",
            "properties": { "ok": { "type": "boolean" } },
            "required": ["ok"],
            "additionalProperties": false
        });
        let probe_messages = vec![ChatMessage {
            role: "user".into(),
            content: "Reply with the json object {\"ok\": true}".into(),
            name: None,
            images: None,
        }];

        // 1) json_schema
        let wire = self.structured_wire_messages(&probe_messages);
        let mut payload = self.base_payload(wire, 0.0, 64);
        payload["response_format"] = serde_json::json!({
            "type": "json_schema",
            "json_schema": { "name": "probe", "strict": true, "schema": probe_schema }
        });
        if let Ok(body) = self.send_structured_once(&payload).await {
            let content = extract_openai_message_text(&body["choices"][0]["message"]);
            if Self::content_is_json_object(&content) {
                return StructuredOutputMode::JsonSchema;
            }
        }

        // 2) json_object (+ in-prompt schema/example)
        let wire = self.augment_messages_for_json_object(&probe_messages, &probe_schema);
        let mut payload = self.base_payload(wire, 0.0, 64);
        payload["response_format"] = serde_json::json!({ "type": "json_object" });
        if let Ok(body) = self.send_structured_once(&payload).await {
            let content = extract_openai_message_text(&body["choices"][0]["message"]);
            if Self::content_is_json_object(&content) {
                return StructuredOutputMode::JsonObject;
            }
        }

        // 3) tool-calling
        {
            let wire = self.structured_wire_messages(&probe_messages);
            let mut payload = self.base_payload(wire, 0.0, 64);
            payload["tools"] = serde_json::json!([{
                "type": "function",
                "function": {
                    "name": "emit_probe",
                    "description": "Return the structured probe object.",
                    "parameters": probe_schema,
                }
            }]);
            payload["tool_choice"] = serde_json::json!({
                "type": "function",
                "function": { "name": "emit_probe" }
            });
            if let Ok(body) = self.send_structured_once(&payload).await {
                if extract_openai_tool_calls(&body["choices"][0]["message"]).is_some() {
                    return StructuredOutputMode::ToolCalling;
                }
            }
        }

        StructuredOutputMode::None
    }

    /// Issue a structured request using a SPECIFIC mode and normalize the result
    /// to a single JSON object in `LlmResponse::content` (Requirement 0.2). The
    /// caller (planner) still strictly validates + re-asks; this never relaxes
    /// validation. Always NON-streaming.
    async fn chat_structured_with_mode(
        &self,
        messages: &[ChatMessage],
        json_schema: &serde_json::Value,
        schema_name: &str,
        mode: StructuredOutputMode,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        match mode {
            StructuredOutputMode::JsonSchema | StructuredOutputMode::Grammar => {
                let wire = self.structured_wire_messages(messages);
                let mut payload = self.base_payload(wire, temperature, max_tokens);
                payload["response_format"] = serde_json::json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": schema_name,
                        "strict": true,
                        "schema": json_schema,
                    }
                });
                let body = self.send_structured_once(&payload).await?;
                Ok(Self::normalize_structured_response(
                    body,
                    &self.model_id,
                    mode,
                ))
            }
            StructuredOutputMode::JsonObject => {
                let wire = self.augment_messages_for_json_object(messages, json_schema);
                let mut payload = self.base_payload(wire, temperature, max_tokens);
                payload["response_format"] = serde_json::json!({ "type": "json_object" });
                let body = self.send_structured_once(&payload).await?;
                Ok(Self::normalize_structured_response(
                    body,
                    &self.model_id,
                    mode,
                ))
            }
            StructuredOutputMode::ToolCalling => {
                let wire = self.structured_wire_messages(messages);
                let mut payload = self.base_payload(wire, temperature, max_tokens);
                payload["tools"] = serde_json::json!([{
                    "type": "function",
                    "function": {
                        "name": schema_name,
                        "description": "Emit the typed plan as a JSON object.",
                        "parameters": json_schema,
                    }
                }]);
                payload["tool_choice"] = serde_json::json!({
                    "type": "function",
                    "function": { "name": schema_name }
                });
                let body = self.send_structured_once(&payload).await?;
                Ok(Self::normalize_structured_response(
                    body,
                    &self.model_id,
                    mode,
                ))
            }
            StructuredOutputMode::None => {
                // No honored structured method: inject schema + example into the
                // prompt as a best effort and let the planner's strict-validate +
                // bounded re-ask be the guard. Still NON-streaming.
                let wire = self.augment_messages_for_json_object(messages, json_schema);
                let payload = self.base_payload(wire, temperature, max_tokens);
                let body = self.send_structured_once(&payload).await?;
                Ok(Self::normalize_structured_response(
                    body,
                    &self.model_id,
                    mode,
                ))
            }
        }
    }

    /// Normalize any structured response to a single JSON object string in
    /// `content`. For tool-calling, parse `tool_calls[0].function.arguments`.
    /// For all modes, tolerantly extract the first balanced top-level `{...}`
    /// object (stripping ```json fences / thinking-model preamble) so the
    /// planner receives a clean object to strictly validate (Requirement 0.4).
    fn normalize_structured_response(
        body: serde_json::Value,
        model_id: &str,
        mode: StructuredOutputMode,
    ) -> LlmResponse {
        // Read the thinking-model reasoning channel BEFORE `parse_response`
        // consumes the body. Some thinking models (e.g. deepseek-v4-flash) place
        // a valid JSON object in `choices[0].message.reasoning_content` while
        // `content` is empty/truncated (reasoning ate the completion budget).
        let reasoning = Self::extract_reasoning_content(&body);
        let mut resp = Self::parse_response(body, model_id);
        if matches!(mode, StructuredOutputMode::ToolCalling) {
            if let Some(calls) = resp.tool_calls.as_ref() {
                if let Some(first) = calls.first() {
                    let args = &first["function"]["arguments"];
                    let content = match args {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Null => String::new(),
                        other => other.to_string(),
                    };
                    if !content.trim().is_empty() {
                        resp.content = content;
                    }
                }
            }
        }
        // Aggressive extraction for the explicit structured path (Requirement
        // 0.4): the request EXPLICITLY asked for JSON, so pull the first balanced
        // top-level `{...}` object from `content` (handles a leading reasoning
        // preamble / fences). This is legitimate — it is NOT lenient-scraping an
        // arbitrary free-form chat reply (the conservative
        // `sanitize_json_object_content` is preserved for the non-structured
        // flag-OFF `parse_llm_plan` path). If `content` yields no object but the
        // reasoning channel carries one (empty/truncated content on a thinking
        // model), recover it from `reasoning_content` as a best effort; the
        // planner still strict-validates.
        if let Some(obj) = extract_first_json_object(&resp.content) {
            resp.content = obj;
        } else if let Some(obj) = reasoning.as_deref().and_then(extract_first_json_object) {
            resp.content = obj;
        }
        resp
    }

    /// Read `choices[0].message.reasoning_content` (the thinking-model reasoning
    /// channel of the OpenAI-compatible body) as a string, if present. This is a
    /// best-effort recovery source for a truncated/empty `content` on a thinking
    /// model. The value is never logged.
    fn extract_reasoning_content(body: &serde_json::Value) -> Option<String> {
        body["choices"][0]["message"]["reasoning_content"]
            .as_str()
            .map(|s| s.to_string())
    }
}

#[cfg(test)]
mod structured_tests {
    use super::*;
    use crate::llm::provider::config::{ProviderConfig, ProviderType};
    use crate::llm::{ChatMessage, LlmBackend, StructuredOutputMode};
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn backend(endpoint: String) -> OpenAIBackend {
        let mut cfg = ProviderConfig::new("test-openai", ProviderType::OpenAICompatible);
        cfg.endpoint.base_url = endpoint;
        cfg.endpoint.api_key = "test-key".into();
        cfg.active_model = "test-model".into();
        OpenAIBackend::from_config(&cfg)
    }

    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "intent": { "type": "string" } },
            "required": ["intent"],
            "additionalProperties": false
        })
    }

    fn msgs() -> Vec<ChatMessage> {
        vec![ChatMessage {
            role: "user".into(),
            content: "Open the calculator".into(),
            name: None,
            images: None,
        }]
    }

    fn object_body(content: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{ "message": { "content": content } }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })
    }

    fn tool_body(arguments: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "gui_typed_plan", "arguments": arguments }
                    }]
                }
            }]
        })
    }

    #[tokio::test]
    async fn probe_detects_json_schema() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(object_body("{\"ok\":true}")))
            .mount(&server)
            .await;
        let backend = backend(server.uri());
        assert_eq!(
            backend.detect_structured_output_mode().await,
            StructuredOutputMode::JsonSchema
        );
        // Cached: a second call returns the same without re-probing.
        assert_eq!(
            backend.detect_structured_output_mode().await,
            StructuredOutputMode::JsonSchema
        );
    }

    #[tokio::test]
    async fn probe_resolves_json_object_when_schema_errors_with_non_2xx() {
        // Exact LIVE-gate bug: json_schema returns a provider error (non-2xx)
        // but json_object returns a clean object → must resolve JsonObject.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("json_schema"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string("This response_format type is unavailable now"),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("json_object"))
            .respond_with(ResponseTemplate::new(200).set_body_json(object_body("{\"ok\":true}")))
            .mount(&server)
            .await;
        let backend = backend(server.uri());
        assert_eq!(
            backend.detect_structured_output_mode().await,
            StructuredOutputMode::JsonObject
        );
    }

    #[tokio::test]
    async fn probe_falls_back_to_json_object_default_when_nothing_honored() {
        // Bug-fix regression: when ALL structured attempts error/transient, the
        // probe must NOT cache `None`. `detect_structured_output_mode` returns
        // the safe default `JsonObject`, and because nothing was cached a later
        // turn can still re-probe.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("nothing supported"))
            .mount(&server)
            .await;
        let backend = backend(server.uri());
        assert_eq!(
            backend.detect_structured_output_mode().await,
            StructuredOutputMode::JsonObject
        );
        // None was NOT cached → the synchronous view still reports the default.
        assert_eq!(
            backend.structured_output_mode(),
            StructuredOutputMode::JsonObject
        );
    }

    #[tokio::test]
    async fn probe_tool_choice_error_does_not_block_json_object_detection() {
        // The tool-calling step forcing `tool_choice` errors on "thinking"
        // models. Because the ladder tries json_object BEFORE tool-calling, a
        // tool_choice error must never prevent json_object detection.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("json_schema"))
            .respond_with(ResponseTemplate::new(400).set_body_string("json_schema unavailable"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("json_object"))
            .respond_with(ResponseTemplate::new(200).set_body_json(object_body("{\"ok\":true}")))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("tool_choice"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string("Thinking mode does not support this tool_choice"),
            )
            .mount(&server)
            .await;
        let backend = backend(server.uri());
        assert_eq!(
            backend.detect_structured_output_mode().await,
            StructuredOutputMode::JsonObject
        );
    }

    #[tokio::test]
    async fn probe_detects_json_object_through_thinking_preamble_and_fences() {
        // deepseek-v4-flash is a "thinking" model: json_object content may carry
        // a <think> preamble and be wrapped in ```json fences. Detection must be
        // tolerant and still resolve JsonObject.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("json_schema"))
            .respond_with(ResponseTemplate::new(400).set_body_string("json_schema unavailable"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("json_object"))
            .respond_with(ResponseTemplate::new(200).set_body_json(object_body(
                "<think>let me reason about this</think>\n```json\n{\"ok\": true}\n```",
            )))
            .mount(&server)
            .await;
        let backend = backend(server.uri());
        assert_eq!(
            backend.detect_structured_output_mode().await,
            StructuredOutputMode::JsonObject
        );
    }

    #[tokio::test]
    async fn chat_structured_json_object_extracts_object_from_fenced_thinking_output() {
        // Tolerant normalization: a json_object response wrapped in a <think>
        // block + ```json fences is cleaned to a bare object for the planner to
        // strictly validate.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("json_object"))
            .respond_with(ResponseTemplate::new(200).set_body_json(object_body(
                "<think>opening calculator</think>\n```json\n{\"intent\":\"open_app\"}\n```",
            )))
            .mount(&server)
            .await;
        let backend =
            backend(server.uri()).with_structured_output_mode(StructuredOutputMode::JsonObject);
        let resp = backend
            .chat_structured(&msgs(), schema(), "gui_typed_plan", 0.1, 256)
            .await
            .expect("structured chat");
        let parsed: serde_json::Value = serde_json::from_str(&resp.content).expect("json object");
        assert_eq!(parsed["intent"], "open_app");
    }

    #[tokio::test]
    async fn chat_structured_defaults_to_json_object_when_unprobed() {
        // Unknown/unprobed backend (no override, no cached probe): the hot-path
        // structured call must default to json_object and return a valid object —
        // NOT fall through to deterministic fallback.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("json_object"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(object_body("{\"intent\":\"open_app\"}")),
            )
            .mount(&server)
            .await;
        let backend = backend(server.uri());
        let resp = backend
            .chat_structured(&msgs(), schema(), "gui_typed_plan", 0.1, 256)
            .await
            .expect("structured chat");
        let parsed: serde_json::Value = serde_json::from_str(&resp.content).expect("json object");
        assert_eq!(parsed["intent"], "open_app");
    }

    #[tokio::test]
    async fn override_skips_probe() {
        // No mock mounted: any probe request would fail. The override must short
        // circuit so no network call happens.
        let backend = backend("http://127.0.0.1:1/never".into())
            .with_structured_output_mode(StructuredOutputMode::JsonObject);
        assert_eq!(
            backend.detect_structured_output_mode().await,
            StructuredOutputMode::JsonObject
        );
        assert_eq!(
            backend.structured_output_mode(),
            StructuredOutputMode::JsonObject
        );
    }

    #[tokio::test]
    async fn chat_structured_json_schema_returns_object() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_string_contains("json_schema"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(object_body("{\"intent\":\"open_app\"}")),
            )
            .mount(&server)
            .await;
        let backend =
            backend(server.uri()).with_structured_output_mode(StructuredOutputMode::JsonSchema);
        let resp = backend
            .chat_structured(&msgs(), schema(), "gui_typed_plan", 0.1, 256)
            .await
            .expect("structured chat");
        let parsed: serde_json::Value = serde_json::from_str(&resp.content).expect("json object");
        assert_eq!(parsed["intent"], "open_app");
    }

    #[tokio::test]
    async fn chat_structured_json_object_injects_json_word_and_returns_object() {
        let server = MockServer::start().await;
        // Only respond when the request actually contains the literal word
        // "json" + json_object response_format (DeepSeek requirement).
        Mock::given(method("POST"))
            .and(body_string_contains("json_object"))
            .and(body_string_contains("json object"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(object_body("{\"intent\":\"open_app\"}")),
            )
            .mount(&server)
            .await;
        let backend =
            backend(server.uri()).with_structured_output_mode(StructuredOutputMode::JsonObject);
        let resp = backend
            .chat_structured(&msgs(), schema(), "gui_typed_plan", 0.1, 256)
            .await
            .expect("structured chat");
        let parsed: serde_json::Value = serde_json::from_str(&resp.content).expect("json object");
        assert_eq!(parsed["intent"], "open_app");
    }

    #[tokio::test]
    async fn chat_structured_tool_calling_normalizes_arguments() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("tool_choice"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(tool_body("{\"intent\":\"open_app\"}")),
            )
            .mount(&server)
            .await;
        let backend =
            backend(server.uri()).with_structured_output_mode(StructuredOutputMode::ToolCalling);
        let resp = backend
            .chat_structured(&msgs(), schema(), "gui_typed_plan", 0.1, 256)
            .await
            .expect("structured chat");
        // The tool_calls[0].function.arguments string is normalized into content.
        let parsed: serde_json::Value = serde_json::from_str(&resp.content).expect("json object");
        assert_eq!(parsed["intent"], "open_app");
    }

    #[tokio::test]
    async fn supports_grammar_is_false_for_openai_compatible_backcompat() {
        // Back-compat: the default structured mode is json_object (the safe
        // OpenAI-compatible default), so supports_grammar() derives false
        // (grammar is the local-only mode).
        let backend = backend("http://127.0.0.1:1/never".into());
        assert!(!backend.supports_grammar());
        assert_eq!(
            backend.structured_output_mode(),
            StructuredOutputMode::JsonObject
        );
    }

    /// Build a json_object body whose `content` carries a leading reasoning
    /// preamble (no code fences) followed by the JSON object — the deepseek
    /// thinking-model shape the aggressive structured extraction must handle.
    fn reasoning_preamble_body(preamble: &str, object: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{ "message": { "content": format!("{preamble}{object}") } }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })
    }

    /// Build a body with EMPTY `content` but a JSON object in the thinking-model
    /// `reasoning_content` channel (the truncated-completion shape).
    fn reasoning_content_body(object: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{ "message": { "content": "", "reasoning_content": object } }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })
    }

    #[tokio::test]
    async fn normalize_structured_extracts_object_after_reasoning_preamble() {
        // Task 0 live blocker: a thinking model emits a reasoning preamble (no
        // fences) BEFORE the JSON object in `content`. The structured path uses
        // aggressive extraction (extract_first_json_object) to pull the first
        // balanced top-level object — NOT the conservative sanitizer (which would
        // reject the surrounding prose).
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("json_object"))
            .respond_with(ResponseTemplate::new(200).set_body_json(reasoning_preamble_body(
                "Let me think about which app to open. The user wants the calculator.\n",
                "{\"intent\":\"open_app\"}",
            )))
            .mount(&server)
            .await;
        let backend =
            backend(server.uri()).with_structured_output_mode(StructuredOutputMode::JsonObject);
        let resp = backend
            .chat_structured(&msgs(), schema(), "gui_typed_plan", 0.1, 256)
            .await
            .expect("structured chat");
        let parsed: serde_json::Value = serde_json::from_str(&resp.content).expect("json object");
        assert_eq!(parsed["intent"], "open_app");
    }

    #[tokio::test]
    async fn normalize_structured_recovers_object_from_reasoning_content() {
        // Task 0 live blocker: a thinking model spent its completion budget on
        // reasoning, leaving `content` empty (`finish_reason="length"`) but the
        // JSON object present in `reasoning_content`. The reasoning_content
        // fallback recovers the object so the planner can strict-validate.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("json_object"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(reasoning_content_body("{\"intent\":\"open_app\"}")),
            )
            .mount(&server)
            .await;
        let backend =
            backend(server.uri()).with_structured_output_mode(StructuredOutputMode::JsonObject);
        let resp = backend
            .chat_structured(&msgs(), schema(), "gui_typed_plan", 0.1, 256)
            .await
            .expect("structured chat");
        let parsed: serde_json::Value = serde_json::from_str(&resp.content).expect("json object");
        assert_eq!(parsed["intent"], "open_app");
    }
}
