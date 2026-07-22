use crate::infra::circuit_breaker::{CircuitBreaker, CircuitBreakerError, CircuitState};
use crate::llm::orchestrator::server_manager::{
    LlamaServerManager, StreamActivityGuard, STATE_READY,
};
use crate::llm::{
    extract_openai_content_text, extract_openai_message_text, extract_openai_tool_calls,
    trim_messages_for_context, ChatMessage, ContextTooLargeError, LlmBackend, LlmResponse,
    ToolSchema,
};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Consecutive transport-level `/health` failures required before the local backend
/// is reported "not reachable". Debounces transient blips during generation.
const HEALTH_FAILURE_THRESHOLD: u32 = 3;

/// Outcome of a single `/health` probe against the local llama-server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalHealthProbe {
    /// HTTP 200 — serving and ready.
    Ready,
    /// Reachable, non-2xx, not loading (e.g. all slots busy). Server is ALIVE.
    Busy,
    /// Reachable, non-2xx, model loading. Server is ALIVE.
    Loading,
    /// Transport error or timeout — no HTTP response at all.
    Unreachable,
}

/// Pre-flight memory check — estimates whether the requested context will fit in
/// available RAM. Returns `Some(warning_message)` if risky, `None` otherwise.
///
/// This is a best-effort heuristic, not a hard refusal. Llama-server's actual
/// memory usage depends on model size + KV cache + tokens. We assume:
/// - 4 bytes per char ~ 1 token
/// - Each token uses ~ 256KB of KV cache (varies by model)
/// - Available RAM threshold: 70% of total system RAM
fn check_memory_budget(context_tokens: usize, max_response_tokens: usize) -> Option<String> {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_memory();
    let available_bytes = sys.available_memory();
    let total_bytes = sys.total_memory();

    // Rough estimate: each token uses ~256KB for KV cache + activations
    // (this varies wildly by model size, but is a useful upper bound for 7B-13B models)
    const BYTES_PER_TOKEN_ESTIMATE: u64 = 256 * 1024;
    let total_tokens = (context_tokens + max_response_tokens) as u64;
    let estimated_memory_needed = total_tokens * BYTES_PER_TOKEN_ESTIMATE;

    // Warn if estimated memory exceeds 70% of available
    let threshold = (available_bytes as f64 * 0.7) as u64;
    if estimated_memory_needed > threshold {
        return Some(format!(
            "Estimated context memory ({:.1} GB) exceeds 70% of available RAM ({:.1} GB free of {:.1} GB total). \
             May cause OOM in llama-server. Consider reducing context or using a smaller model.",
            estimated_memory_needed as f64 / 1_073_741_824.0,
            available_bytes as f64 / 1_073_741_824.0,
            total_bytes as f64 / 1_073_741_824.0,
        ));
    }
    None
}

/// Local LLM backend using llama.cpp via HTTP API.
///
/// When an orchestrator `LlamaServerManager` is attached, the API URL and
/// context window are resolved dynamically from the server manager, and
/// in-flight streams can be cancelled via `CancellationToken` during swaps.
pub struct LocalBackend {
    /// Fallback API URL (used when no server manager is attached).
    api_url: String,
    model_label: String,
    capabilities: Vec<String>,
    /// Dynamic context window (updated by orchestrator swaps).
    context_window: Arc<AtomicUsize>,
    client: reqwest::Client,
    /// Dedicated short-timeout client for `/health` probes. Kept separate from the
    /// 120s inference `client` so a saturated inference path can neither delay nor
    /// mask a reachability probe (Issue-1 root cause: shared client + long timeout).
    health_client: reqwest::Client,
    /// Consecutive transport-level health failures. Debounces the user-facing
    /// "not reachable" signal so a single transient blip (or a busy/loading `/health`
    /// response during an in-flight generation) never flips the banner.
    consecutive_health_failures: AtomicU32,
    circuit: Arc<CircuitBreaker>,
    /// Optional server manager for orchestrator-managed mode.
    /// Replaceable so Settings-driven local model swaps can attach the new
    /// server manager after a successful orchestrator restart.
    server_manager: RwLock<Option<Arc<LlamaServerManager>>>,
}

impl LocalBackend {
    pub fn new(
        api_url: String,
        model_label: String,
        capabilities: Vec<String>,
        context_window: usize,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();

        // Short, dedicated probe client: fast connect + overall deadline so the
        // reachability signal is never coupled to long-running inference requests.
        let health_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(2))
            .build()
            .unwrap_or_default();

        Self {
            api_url,
            model_label,
            capabilities,
            context_window: Arc::new(AtomicUsize::new(context_window)),
            client,
            health_client,
            consecutive_health_failures: AtomicU32::new(0),
            circuit: Arc::new(CircuitBreaker::with_defaults("local-llm")),
            server_manager: RwLock::new(None),
        }
    }

    /// Attach a server manager from the orchestrator.
    /// Enables dynamic URL resolution and stream cancellation.
    /// Safe to call on `&self`; a later local model swap replaces the manager.
    pub fn attach_server_manager(&self, mgr: Arc<LlamaServerManager>) {
        if let Ok(mut guard) = self.server_manager.write() {
            *guard = Some(mgr);
        }
    }

    /// Detach a stopped managed runtime so requests cannot retain its lifecycle owner.
    pub fn detach_server_manager(&self) {
        if let Ok(mut guard) = self.server_manager.write() {
            *guard = None;
        }
    }

    /// Returns the attached orchestrator server manager, if any.
    pub fn server_manager(&self) -> Option<Arc<LlamaServerManager>> {
        self.server_manager
            .read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// Resolve the current API URL — from server manager if attached, else fallback.
    fn resolve_api_url(&self) -> String {
        if let Some(mgr) = self.server_manager() {
            let url = mgr.api_url();
            if !url.is_empty() {
                return url;
            }
        }
        self.api_url.clone()
    }

    /// Update the context window (called by orchestrator after swap).
    pub fn update_context_window(&self, ctx: usize) {
        self.context_window.store(ctx, Ordering::Release);
    }

    /// Get a cancellation token if orchestrator is attached.
    fn cancel_token(&self) -> Option<CancellationToken> {
        self.server_manager().map(|mgr| mgr.cancel_token())
    }

    /// Whether the orchestrator is in a controlled transition (swap OR intended
    /// restart) where `/health` is briefly offline while the SAME model comes
    /// back. Reachability must report this as warming, never "unreachable".
    fn is_warming(&self) -> bool {
        self.server_manager
            .read()
            .ok()
            .and_then(|guard| guard.clone())
            .map(|mgr| mgr.is_warming())
            .unwrap_or(false)
    }

    /// Wait for any in-progress swap to finish, returning `false` on timeout.
    /// Replaces the busy-poll loops used before the Notify refactor (Phase 5).
    async fn wait_for_swap(&self, timeout_secs: u64) -> bool {
        let Some(mgr) = self.server_manager() else {
            return true;
        };
        mgr.wait_for_swap_done(Duration::from_secs(timeout_secs))
            .await
    }

    /// Query the llama.cpp `/v1/models` endpoint to detect the actually loaded model.
    /// Returns the model ID string if the server responds, or None.
    pub async fn detect_server_model(&self) -> Option<String> {
        let url = format!("{}/models", self.resolve_api_url());
        let resp = self.client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body: serde_json::Value = resp.json().await.ok()?;
        // llama.cpp returns { "data": [{ "id": "model-name", ... }] }
        body["data"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|m| m["id"].as_str())
            .map(|s| s.to_string())
    }

    /// Update the model label dynamically (e.g. after detecting from server).
    pub fn set_model_label(&mut self, label: String) {
        self.model_label = label;
    }

    fn looks_like_context_overflow_response(_status: reqwest::StatusCode, body: &str) -> bool {
        let lower = body.to_ascii_lowercase();
        lower.contains("context")
            || lower.contains("token")
            || lower.contains("tokens")
            || lower.contains("exceed")
            || lower.contains("exceeds")
            || lower.contains("too large")
            || lower.contains("too long")
            || lower.contains("overflow")
    }

    fn looks_like_vision_not_supported_response(body: &str) -> bool {
        let lower = body.to_ascii_lowercase();
        lower.contains("image input is not supported")
            || lower.contains("vision input is not supported")
            || (lower.contains("mmproj") && lower.contains("image"))
            || (lower.contains("mmproj") && lower.contains("vision"))
    }

    /// Detects the llama.cpp/llama-server failure where it cannot build a grammar/parser for the
    /// supplied tool (function) JSON schema — e.g. "Unable to generate parser for this template",
    /// "Automatic parser generation failed", "Unrecognized schema". This is a REQUEST-SHAPE problem
    /// (the model/template can't do native tool-calling), NOT a server-health failure. We recover by
    /// retrying the same turn WITHOUT tools (text mode); the agent loop then parses tool calls from
    /// text. This must never trip the circuit breaker.
    fn looks_like_tool_schema_unsupported_response(body: &str) -> bool {
        let lower = body.to_ascii_lowercase();
        (lower.contains("parser") && lower.contains("template"))
            || lower.contains("automatic parser generation failed")
            || lower.contains("unrecognized schema")
            || lower.contains("json schema conversion failed")
    }

    fn looks_like_transport_connectivity_error(message: &str) -> bool {
        let lower = message.to_ascii_lowercase();
        lower.contains("error sending request")
            || lower.contains("connection refused")
            || lower.contains("tcp connect")
            || lower.contains("dns error")
            || lower.contains("timed out")
            || lower.contains("connection reset")
            || lower.contains("broken pipe")
    }

    fn estimate_prompt_tokens(messages: &[ChatMessage], tools: Option<&[ToolSchema]>) -> usize {
        let message_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
        let image_overhead: usize = messages
            .iter()
            .map(|m| m.images.as_ref().map(|imgs| imgs.len()).unwrap_or(0) * 512)
            .sum();
        let tool_chars: usize = tools
            .map(|defs| {
                defs.iter()
                    .map(|schema| {
                        schema.name.len()
                            + schema.description.len()
                            + schema.parameters.to_string().len()
                    })
                    .sum()
            })
            .unwrap_or(0);

        // Approximation only; leave headroom for wire-format overhead.
        ((message_chars + image_overhead + tool_chars) / 4).saturating_add(64)
    }

    fn clamp_max_tokens_for_context(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        requested_max_tokens: u32,
    ) -> u32 {
        let context_window = self.context_window.load(Ordering::Acquire).max(512) as u32;
        let estimated_prompt_tokens = Self::estimate_prompt_tokens(messages, tools) as u32;
        let reserve_tokens = 96u32;
        let available_completion =
            context_window.saturating_sub(estimated_prompt_tokens.saturating_add(reserve_tokens));

        if available_completion < 64 {
            return 0;
        }

        // Never allow completion to monopolize the full context.
        let context_half_cap = (context_window / 2).max(128);
        requested_max_tokens
            .min(available_completion)
            .min(context_half_cap)
            .max(64)
    }

    fn should_ignore_for_circuit(error: &anyhow::Error) -> bool {
        if error.downcast_ref::<ContextTooLargeError>().is_some() {
            return true;
        }

        let message = error.to_string();
        Self::looks_like_transport_connectivity_error(&message)
            || Self::looks_like_vision_not_supported_response(&message)
    }

    /// Returns whether this backend can currently process image inputs.
    pub fn runtime_supports_vision(&self) -> bool {
        if !self.capabilities.iter().any(|cap| cap == "vision") {
            return false;
        }
        if let Some(mgr) = self.server_manager() {
            if mgr.state() == STATE_READY {
                return mgr.current_vision_enabled();
            }
            return mgr.vision_configured();
        }
        true
    }

    /// Result of a single `/health` probe. Distinguishes "alive but not idle/ready"
    /// from "truly unreachable" — the crux of the Issue-1 fix.
    async fn probe_health(&self) -> LocalHealthProbe {
        let health_url = self.resolve_api_url().replace("/v1", "/health");
        let fut = self.health_client.get(&health_url).send();
        match tokio::time::timeout(Duration::from_secs(5), fut).await {
            Ok(Ok(resp)) => {
                if resp.status().is_success() {
                    LocalHealthProbe::Ready
                } else {
                    // Reachable but not ready. llama.cpp returns 503 while a model is
                    // loading or (older builds) when all slots are busy. The server is
                    // ALIVE — never report this as "unreachable".
                    let loading = resp
                        .text()
                        .await
                        .map(|b| b.to_lowercase().contains("loading"))
                        .unwrap_or(false);
                    if loading {
                        LocalHealthProbe::Loading
                    } else {
                        LocalHealthProbe::Busy
                    }
                }
            }
            // Transport error or timed out → genuinely unreachable for this probe.
            Ok(Err(_)) | Err(_) => LocalHealthProbe::Unreachable,
        }
    }

    /// Strict readiness check (HTTP 200). Used by internal readiness loops and
    /// circuit recovery where we specifically require the server to be serving.
    async fn health_check_once(&self) -> bool {
        matches!(self.probe_health().await, LocalHealthProbe::Ready)
    }

    async fn wait_for_backend_ready(&self, timeout_secs: u64) -> bool {
        if !self.wait_for_swap(timeout_secs).await {
            return false;
        }

        // Without an orchestrator-backed server manager we have no swap lifecycle;
        // rely on the request path to surface errors directly.
        if self.server_manager().is_none() {
            return true;
        }

        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs.max(1));
        while tokio::time::Instant::now() < deadline {
            if self.health_check_once().await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        false
    }

    async fn try_recover_open_circuit(&self, name: &str, attempt: usize) -> bool {
        tracing::warn!(
            circuit = %name,
            attempt,
            "local LLM circuit is open; probing health for fast recovery"
        );

        let healthy = tokio::time::timeout(Duration::from_secs(3), self.health_check_once())
            .await
            .unwrap_or(false);

        if healthy {
            self.circuit.reset().await;
            tracing::info!(circuit = %name, "local LLM circuit reset after successful health probe");
            return true;
        }

        false
    }

    async fn chat_inner(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        let max_tokens = self.clamp_max_tokens_for_context(messages, tools, max_tokens);
        if max_tokens == 0 {
            tracing::warn!(
                context_window = self.context_window.load(Ordering::Acquire),
                estimated_prompt_tokens = Self::estimate_prompt_tokens(messages, tools),
                message_count = messages.len(),
                "local LLM prompt leaves no completion budget; returning context overflow"
            );
            return Err(ContextTooLargeError.into());
        }

        // Convert messages to the OpenAI wire format, using multimodal content
        // for any messages that contain images (required for vision models).
        let wire_messages: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                if m.has_images() {
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
            .collect();

        let base_payload = serde_json::json!({
            "model": self.model_label,
            "messages": wire_messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "stream": false,
        });

        let tool_defs: Option<Vec<serde_json::Value>> = tools.and_then(|t| {
            if t.is_empty() {
                None
            } else {
                Some(
                    t.iter()
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
                        .collect(),
                )
            }
        });

        let build_payload = |with_tools: bool| -> serde_json::Value {
            let mut payload = base_payload.clone();
            if with_tools {
                if let Some(ref defs) = tool_defs {
                    payload["tools"] = serde_json::Value::Array(defs.clone());
                }
            }
            payload
        };

        let url = format!("{}/chat/completions", self.resolve_api_url());
        let had_tools = tool_defs.is_some();

        let send = |payload: serde_json::Value| {
            let client = self.client.clone();
            let url = url.clone();
            async move {
                client
                    .post(&url)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!("local LLM transport error to {url}: {e}"))
            }
        };

        let mut resp = send(build_payload(had_tools)).await?;
        let mut status = resp.status();

        // Tool-schema recovery: if the server cannot build a grammar/parser for the tool schema,
        // retry the SAME turn without tools (text-mode tool parsing downstream). This is a
        // request-shape issue, not a server-health failure, so it must not trip the circuit.
        if had_tools && !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            if Self::looks_like_tool_schema_unsupported_response(&body_text) {
                tracing::warn!(
                    status = %status,
                    "local LLM rejected native tool schema (parser generation failed); \
                     retrying without tools — tool calls will be parsed from text"
                );
                resp = send(build_payload(false)).await?;
                status = resp.status();
            } else if Self::looks_like_context_overflow_response(status, &body_text) {
                return Err(ContextTooLargeError.into());
            } else if Self::looks_like_vision_not_supported_response(&body_text) {
                anyhow::bail!("local LLM vision unavailable: {body_text}");
            } else {
                tracing::error!(
                    status = %status,
                    response_body = %body_text,
                    "local LLM request failed with non-overflow error"
                );
                anyhow::bail!("local LLM API error (status {status}): {body_text}");
            }
        }

        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            if Self::looks_like_context_overflow_response(status, &body_text) {
                return Err(ContextTooLargeError.into());
            }
            if Self::looks_like_vision_not_supported_response(&body_text) {
                anyhow::bail!("local LLM vision unavailable: {body_text}");
            }

            tracing::error!(
                status = %status,
                response_body = %body_text,
                "local LLM request failed with non-overflow error"
            );
            anyhow::bail!("local LLM API error (status {status}): {body_text}");
        }

        let body: serde_json::Value = resp.json().await?;

        let choice = &body["choices"][0];
        let message = &choice["message"];
        let content = extract_openai_message_text(message);
        let tool_calls = extract_openai_tool_calls(message);

        let usage = body["usage"].as_object().map(|u| crate::llm::TokenUsage {
            prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: u["total_tokens"].as_u64().unwrap_or(0) as u32,
        });

        Ok(LlmResponse {
            content,
            model: self.model_label.clone(),
            usage,
            tool_calls,
        })
    }
}

#[async_trait]
impl LlmBackend for LocalBackend {
    fn model_label(&self) -> &str {
        &self.model_label
    }

    fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    fn is_configured(&self) -> bool {
        true
    }

    /// The local llama.cpp backend genuinely posts a `json_schema`
    /// `response_format` in [`chat_with_grammar`] to activate constrained
    /// decoding (llguidance), so its structured-output mode is
    /// [`StructuredOutputMode::Grammar`] (Task 2.2, Requirement 1.2/1.5/0.3).
    /// `supports_grammar()` derives `true` from this mode for back-compat. If the
    /// running server lacks `json_schema` support it logs a warning and degrades
    /// to an unconstrained chat at call time; the capability signal reflects the
    /// configured/expected path for this backend.
    fn structured_output_mode(&self) -> crate::llm::StructuredOutputMode {
        crate::llm::StructuredOutputMode::Grammar
    }

    fn tokenizer_base_url(&self) -> String {
        let url = self.resolve_api_url();
        url.strip_suffix("/v1").unwrap_or(&url).to_string()
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        // Mark this foreground turn active so the watchdog defers non-emergency swaps until it
        // finishes (HRA Task 12 / A4: never interrupt an active answer).
        let _stream_guard = self.server_manager().map(StreamActivityGuard::new);

        // ─── Pre-flight memory check ───────────────────────────────────────
        // Estimate the approximate context size and refuse if available RAM is
        // insufficient. Prevents silent OOM in llama-server.
        let estimated_context_chars: usize = messages.iter().map(|m| m.content.len()).sum();
        let estimated_context_tokens = estimated_context_chars / 4; // Rough heuristic
        if let Some(memory_warning) =
            check_memory_budget(estimated_context_tokens, max_tokens as usize)
        {
            tracing::warn!(
                target: "llm_local",
                warning = %memory_warning,
                "Pre-flight memory check warns about potential OOM"
            );
            // We log but don't refuse — the user might be willing to risk it.
            // A future improvement: refuse with HITL if estimated_context > available_ram * 0.7
        }

        let mut current_messages = messages.to_vec();
        const MAX_CHAT_ATTEMPTS: usize = 5;

        for attempt in 0..MAX_CHAT_ATTEMPTS {
            if !self.wait_for_backend_ready(120).await {
                anyhow::bail!("local LLM: backend readiness timeout exceeded (120s)");
            }

            match self
                .circuit
                .call(
                    self.chat_inner(&current_messages, tools, temperature, max_tokens),
                    |e: &anyhow::Error| Self::should_ignore_for_circuit(e),
                )
                .await
            {
                Ok(resp) => return Ok(resp),
                Err(CircuitBreakerError::Open(name)) => {
                    if attempt < 2 && self.try_recover_open_circuit(&name, attempt).await {
                        continue;
                    }

                    anyhow::bail!(
                        "local LLM unavailable (circuit open: {name}). Health probe failed; retry in 20-30s or restart the local model runtime"
                    );
                }
                Err(CircuitBreakerError::Inner(e)) => {
                    if e.downcast_ref::<ContextTooLargeError>().is_some() {
                        let total_chars: usize = current_messages
                            .iter()
                            .map(|m| m.content.chars().count())
                            .sum();
                        tracing::warn!(
                            attempt,
                            message_count = current_messages.len(),
                            total_chars,
                            "context too large, trimming"
                        );

                        if current_messages.len() <= 2 {
                            tracing::error!(
                                attempt,
                                message_count = current_messages.len(),
                                total_chars,
                                "context overflow persisted with minimal prompt window; aborting without further retries"
                            );
                            return Err(ContextTooLargeError.into());
                        }

                        let trimmed = trim_messages_for_context(&current_messages, attempt);
                        let trimmed_total_chars: usize =
                            trimmed.iter().map(|m| m.content.chars().count()).sum();
                        if trimmed.len() >= current_messages.len()
                            && trimmed_total_chars >= total_chars
                        {
                            tracing::error!(
                                attempt,
                                message_count = current_messages.len(),
                                total_chars,
                                trimmed_message_count = trimmed.len(),
                                trimmed_total_chars,
                                "context trimmer made no progress; aborting"
                            );
                            return Err(ContextTooLargeError.into());
                        }

                        current_messages = trimmed;
                        continue;
                    }
                    if Self::looks_like_transport_connectivity_error(&e.to_string()) {
                        tracing::warn!(
                            attempt,
                            "local LLM transport failed; waiting for swap/health before retry"
                        );
                        let _ = self.wait_for_backend_ready(30).await;
                        if attempt + 1 < MAX_CHAT_ATTEMPTS {
                            tokio::time::sleep(Duration::from_millis(250 * (attempt as u64 + 1)))
                                .await;
                            continue;
                        }
                    }
                    if attempt + 1 >= MAX_CHAT_ATTEMPTS {
                        return Err(e);
                    }
                }
            }
        }

        anyhow::bail!(
            "local LLM context overflow after {MAX_CHAT_ATTEMPTS} attempts; start a new session or increase model context"
        )
    }

    async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = String> + Send>>> {
        let max_tokens = self.clamp_max_tokens_for_context(messages, tools, max_tokens);
        if max_tokens == 0 {
            tracing::warn!(
                context_window = self.context_window.load(Ordering::Acquire),
                estimated_prompt_tokens = Self::estimate_prompt_tokens(messages, tools),
                message_count = messages.len(),
                "stream prompt leaves no completion budget; returning context overflow"
            );
            return Err(ContextTooLargeError.into());
        }

        // V10: Wait for swap to complete
        if !self.wait_for_swap(120).await {
            anyhow::bail!("local LLM: swap timeout exceeded (120s)");
        }

        if matches!(self.circuit.state().await, CircuitState::Open)
            && !self.try_recover_open_circuit("local-llm", 0).await
            && matches!(self.circuit.state().await, CircuitState::Open)
        {
            anyhow::bail!("local LLM stream unavailable (circuit open)");
        }

        let mut payload = serde_json::json!({
            "model": self.model_label,
            "messages": messages,
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

        let url = format!("{}/chat/completions", self.resolve_api_url());
        let resp = match tokio::time::timeout(
            Duration::from_secs(45),
            self.client.post(&url).json(&payload).send(),
        )
        .await
        {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                // Do not open the circuit for failures caused by a controlled
                // transition (swap OR intended restart): the server is mid-reload,
                // not broken. Counting these would leave the breaker OPEN after a
                // swap/restart ("sometimes it doesn't recover").
                if !self.is_warming() {
                    self.circuit.on_failure().await;
                }
                return Err(e.into());
            }
            Err(_) => {
                if !self.is_warming() {
                    self.circuit.on_failure().await;
                }
                anyhow::bail!("local LLM stream request timed out");
            }
        };

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            if Self::looks_like_context_overflow_response(status, &body_text) {
                return Err(ContextTooLargeError.into());
            }

            if !self.is_warming() {
                self.circuit.on_failure().await;
            }
            tracing::error!(
                status = %status,
                response_body = %body_text,
                "local LLM stream request failed with non-overflow error"
            );
            anyhow::bail!("local LLM stream API error (status {status}): {body_text}");
        }

        self.circuit.on_success().await;

        // V13: Build cancellable stream using select! on CancellationToken
        let cancel = self.cancel_token();
        // Keep the foreground-active marker alive for the whole stream lifetime (A4).
        let stream_guard = self.server_manager().map(StreamActivityGuard::new);

        let stream = futures::stream::unfold(
            (resp, cancel, stream_guard),
            |(mut resp, cancel, stream_guard)| async move {
                // If we have a cancel token, use select! to abort on cancellation
                let chunk_result = if let Some(ref token) = cancel {
                    tokio::select! {
                        biased;
                        _ = token.cancelled() => {
                            tracing::info!("local LLM stream: cancelled by orchestrator swap");
                            return None;
                        }
                        result = resp.chunk() => result,
                    }
                } else {
                    resp.chunk().await
                };

                match chunk_result {
                    Ok(Some(chunk)) => {
                        let text = String::from_utf8_lossy(&chunk).to_string();
                        // Parse SSE: lines starting with "data: "
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
                        Some((tokens, (resp, cancel, stream_guard)))
                    }
                    _ => None,
                }
            },
        );

        Ok(Box::pin(stream))
    }

    async fn chat_with_grammar(
        &self,
        messages: &[ChatMessage],
        json_schema: serde_json::Value,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        let max_tokens = self.clamp_max_tokens_for_context(messages, None, max_tokens);
        if max_tokens == 0 {
            tracing::warn!(
                context_window = self.context_window.load(Ordering::Acquire),
                estimated_prompt_tokens = Self::estimate_prompt_tokens(messages, None),
                message_count = messages.len(),
                "grammar prompt leaves no completion budget; returning context overflow"
            );
            return Err(ContextTooLargeError.into());
        }

        if !self.wait_for_swap(120).await {
            anyhow::bail!("local LLM: swap timeout exceeded (120s) waiting for grammar chat");
        }

        let wire_messages: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                if m.has_images() {
                    serde_json::json!({ "role": m.role, "content": m.to_multimodal_content() })
                } else {
                    let mut msg = serde_json::json!({ "role": m.role, "content": m.content });
                    if let Some(ref name) = m.name {
                        msg["name"] = serde_json::json!(name);
                    }
                    msg
                }
            })
            .collect();

        let payload = serde_json::json!({
            "model": self.model_label,
            "messages": wire_messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "stream": false,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "tool_call",
                    "strict": true,
                    "schema": json_schema,
                }
            }
        });

        let url = format!("{}/chat/completions", self.resolve_api_url());
        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("grammar chat transport error to {url}: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            if matches!(status.as_u16(), 400 | 422)
                && body_text.to_ascii_lowercase().contains("json_schema")
            {
                tracing::warn!(
                    "[LocalBackend] llama.cpp does not support json_schema response_format; \
                     falling back to unconstrained chat. Upgrade llama.cpp for llguidance support."
                );
                return self.chat(messages, None, temperature, max_tokens).await;
            }
            if Self::looks_like_context_overflow_response(status, &body_text) {
                return Err(ContextTooLargeError.into());
            }
            tracing::error!(
                status = %status,
                response_body = %body_text,
                "grammar chat request failed with non-overflow error"
            );
            anyhow::bail!("local LLM grammar API error (status {status}): {body_text}");
        }

        let body: serde_json::Value = resp.json().await?;
        let choice = &body["choices"][0];
        let message = &choice["message"];
        let content = extract_openai_message_text(message);

        let tool_calls =
            if content.trim_start().starts_with('{') || content.trim_start().starts_with('[') {
                extract_tool_calls_from_json_content(&content)
                    .or_else(|| extract_openai_tool_calls(message))
            } else {
                extract_openai_tool_calls(message)
            };

        let usage = body["usage"].as_object().map(|u| crate::llm::TokenUsage {
            prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: u["total_tokens"].as_u64().unwrap_or(0) as u32,
        });

        Ok(LlmResponse {
            content,
            model: self.model_label.clone(),
            usage,
            tool_calls,
        })
    }

    async fn health_check(&self) -> bool {
        // A controlled orchestrator transition — a swap (model reload at a turn
        // boundary) OR an intended, bounded restart (crash/idle recovery) — briefly
        // takes `/health` offline while the SAME model is coming back. Never report
        // that as "not reachable"; it is a warming state, not an outage. This removes
        // the residual banner flicker around legitimate swaps and restarts.
        if self.is_warming() {
            self.consecutive_health_failures.store(0, Ordering::Relaxed);
            return true;
        }
        // Reachability semantics (Issue-1 fix): a server that is alive but busy or
        // loading is NOT "unreachable". Only sustained transport failures count as
        // down, and even then we require several consecutive misses so a single
        // blip during an in-flight generation never flips the user-facing banner.
        match self.probe_health().await {
            LocalHealthProbe::Ready | LocalHealthProbe::Busy | LocalHealthProbe::Loading => {
                self.consecutive_health_failures.store(0, Ordering::Relaxed);
                true
            }
            LocalHealthProbe::Unreachable => {
                let failures = self
                    .consecutive_health_failures
                    .fetch_add(1, Ordering::Relaxed)
                    + 1;
                // Report reachable until the failure streak crosses the threshold.
                failures < HEALTH_FAILURE_THRESHOLD
            }
        }
    }
}

/// Parse tool calls from a JSON content string emitted under json_schema mode.
/// Handles both single `{"tool": "...", "arguments": {...}}` and
/// array `[{"tool": "...", "arguments": {...}}, ...]` forms.
/// Returns the same `Vec<serde_json::Value>` shape as `extract_openai_tool_calls`.
fn extract_tool_calls_from_json_content(content: &str) -> Option<Vec<serde_json::Value>> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(content) else {
        return None;
    };

    let items: Vec<serde_json::Value> = if val.is_array() {
        val.as_array().cloned().unwrap_or_default()
    } else {
        vec![val]
    };

    let calls: Vec<serde_json::Value> = items
        .into_iter()
        .filter_map(|item| {
            let name = item
                .get("tool")
                .or_else(|| item.get("name"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())?;
            let arguments = item
                .get("arguments")
                .or_else(|| item.get("args"))
                .cloned()
                .unwrap_or(serde_json::Value::Object(Default::default()));
            // Emit in the OpenAI tool_calls format so the rest of the pipeline is unchanged.
            Some(serde_json::json!({
                "id": format!("grammar_{}", uuid::Uuid::new_v4()),
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": serde_json::to_string(&arguments).unwrap_or_default(),
                }
            }))
        })
        .collect();

    if calls.is_empty() {
        None
    } else {
        Some(calls)
    }
}
