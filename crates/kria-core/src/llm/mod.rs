pub mod budget;
pub mod cloud;
pub mod failover;
pub mod local;
pub mod model_manager;
pub mod model_router;
pub mod orchestrator;
pub mod provider;
pub mod server_binary;
pub mod tokenize;

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

pub use model_manager::ModelManager;
pub use model_router::ModelRouter;

/// A chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional image attachments (base64-encoded) for vision models.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageAttachment>>,
}

/// An image attachment for multimodal messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageAttachment {
    /// Base64-encoded image data.
    pub data: String,
    /// MIME type (e.g. "image/png", "image/jpeg").
    pub mime_type: String,
}

impl ChatMessage {
    /// Check if this message contains images.
    pub fn has_images(&self) -> bool {
        self.images.as_ref().is_some_and(|imgs| !imgs.is_empty())
    }

    /// Convert to OpenAI multimodal content format for vision APIs.
    pub fn to_multimodal_content(&self) -> serde_json::Value {
        if !self.has_images() {
            return serde_json::json!(self.content);
        }
        let mut parts = Vec::new();
        // Add text first
        if !self.content.is_empty() {
            parts.push(serde_json::json!({
                "type": "text",
                "text": self.content,
            }));
        }
        // Add images
        if let Some(ref images) = self.images {
            for img in images {
                parts.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:{};base64,{}", img.mime_type, img.data),
                    },
                }));
            }
        }
        serde_json::json!(parts)
    }
}

/// Response from an LLM backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub model: String,
    pub usage: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Tool schema for LLM function calling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// The structured-output method a backend genuinely honors for schema-valid
/// JSON decoding (Requirement 0.1). Ordered strongest → weakest:
///
/// * `Grammar` — grammar / guided-JSON constrained decoding (local llama.cpp
///   `json_schema` `response_format` driving llguidance). Token stream is
///   physically restricted to valid JSON.
/// * `JsonSchema` — OpenAI-compatible `response_format:{type:"json_schema"}`
///   honored by the endpoint (strict schema enforced server-side).
/// * `JsonObject` — OpenAI-compatible `response_format:{type:"json_object"}`
///   (the model is asked for *a* JSON object; the schema + a few-shot are
///   injected into the prompt because the wire constraint alone is loose).
/// * `ToolCalling` — function/tool-calling, where the typed plan is delivered
///   as `tool_calls[0].function.arguments` and normalized to a JSON object.
/// * `None` — the backend honors no structured method; the universal
///   validate-and-re-ask safety net is the only guard.
///
/// This is the capability signal the GUI Cognition planner uses to decide
/// whether the LLM planner can be relied upon for schema-valid plans. It is
/// additive and truthful — a backend reports a non-`None` mode ONLY for a
/// method it actually posts/honors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutputMode {
    Grammar,
    JsonSchema,
    JsonObject,
    ToolCalling,
    None,
}

impl StructuredOutputMode {
    /// Stable lowercase identifier used in events / capability reports.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Grammar => "grammar",
            Self::JsonSchema => "json_schema",
            Self::JsonObject => "json_object",
            Self::ToolCalling => "tool_calling",
            Self::None => "none",
        }
    }

    /// Whether this mode genuinely constrains/guides structured output (anything
    /// other than [`StructuredOutputMode::None`]).
    pub fn is_structured(&self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Extract human-readable text from OpenAI-compatible `content` values.
/// Handles string, object, and array-part formats returned by different providers.
pub fn extract_openai_content_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(parts) => {
            let mut chunks: Vec<String> = Vec::new();
            for part in parts {
                let piece = extract_openai_content_text(part);
                if !piece.trim().is_empty() {
                    chunks.push(piece);
                }
            }
            chunks.join("\n")
        }
        serde_json::Value::Object(map) => {
            if let Some(v) = map.get("text") {
                return extract_openai_content_text(v);
            }
            if let Some(v) = map.get("content") {
                return extract_openai_content_text(v);
            }
            if let Some(v) = map.get("value") {
                return extract_openai_content_text(v);
            }
            if let Some(v) = map.get("output_text") {
                return extract_openai_content_text(v);
            }
            if let Some(v) = map.get("input_text") {
                return extract_openai_content_text(v);
            }
            String::new()
        }
        _ => String::new(),
    }
}

/// Extract text from `choice.message` object across provider variants.
pub fn extract_openai_message_text(message: &serde_json::Value) -> String {
    extract_openai_content_text(&message["content"])
}

/// Extract the "reasoning channel" text a thinking/reasoning model may return
/// INSTEAD of `content`. Different OpenAI-compatible providers/proxies use
/// different wire keys for this channel:
///   * `message.reasoning`                    — OpenRouter / opencode(zen) style
///   * `message.reasoning_details[].text`     — opencode(zen) structured variant
///   * `message.reasoning_content`            — DeepSeek style
///
/// This is provider-neutral (keyed off the wire shape, never a model name or a
/// user prompt) and is only meant as a fallback when `content` is empty and the
/// turn carries no tool calls. The value is never logged by callers.
pub fn extract_openai_reasoning_text(message: &serde_json::Value) -> String {
    // Plain string reasoning channels first.
    if let Some(s) = message.get("reasoning").and_then(|v| v.as_str()) {
        if !s.trim().is_empty() {
            return s.to_string();
        }
    }
    if let Some(s) = message.get("reasoning_content").and_then(|v| v.as_str()) {
        if !s.trim().is_empty() {
            return s.to_string();
        }
    }
    // Structured `reasoning_details: [{ type, text, ... }]` variant.
    if let Some(parts) = message.get("reasoning_details").and_then(|v| v.as_array()) {
        let mut chunks: Vec<String> = Vec::new();
        for part in parts {
            if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                if !t.trim().is_empty() {
                    chunks.push(t.to_string());
                }
            }
        }
        if !chunks.is_empty() {
            return chunks.join("\n");
        }
    }
    String::new()
}

/// Extract assistant text from `choice.message`, falling back to the reasoning
/// channel when `content` is empty. Use this on user-facing (non-tool) turns so
/// that a reasoning model that placed its answer in `reasoning`/`reasoning_content`
/// (with `content: null`) still yields visible text instead of a blank reply.
pub fn extract_openai_message_text_with_reasoning(message: &serde_json::Value) -> String {
    let content = extract_openai_content_text(&message["content"]);
    if !content.trim().is_empty() {
        return content;
    }
    extract_openai_reasoning_text(message)
}

/// Extract tool calls from `choice.message` across provider variants.
/// Supports modern `tool_calls` and legacy `function_call` fields.
pub fn extract_openai_tool_calls(message: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
    if let Some(arr) = message.get("tool_calls").and_then(|v| v.as_array()) {
        if !arr.is_empty() {
            return Some(arr.clone());
        }
    }

    if let Some(fc) = message.get("function_call") {
        let name = fc.get("name").and_then(|v| v.as_str())?;
        let args = fc
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| serde_json::json!("{}"));
        return Some(vec![serde_json::json!({
            "type": "function",
            "function": {
                "name": name,
                "arguments": args,
            }
        })]);
    }

    None
}

/// Conservative cleanup of structured model output into a single JSON object
/// (Requirement 0.4) for the planner path. Handles the wrappers a well-behaved
/// "thinking" model / proxy adds around an otherwise-clean object:
///   * a leading `<think>...</think>` reasoning block (deepseek-style),
///   * a full ```json / ``` Markdown code-fence wrapper,
///   * leading/trailing whitespace.
/// After stripping those, the result is accepted ONLY if it is itself a single
/// balanced object that starts with `{`, ends with `}`, and parses as a JSON
/// object. It deliberately does NOT scrape an object out of surrounding prose
/// (e.g. `"Here is the plan: {...}"` is rejected) — that preserves the invariant
/// that arbitrary prose is never lenient-scraped into a plan. Returns `None`
/// when the cleaned content is not a clean JSON object.
pub fn sanitize_json_object_content(content: &str) -> Option<String> {
    let mut s = content.trim();

    // Strip a single leading <think>...</think> reasoning block if present.
    if let Some(rest) = s.strip_prefix("<think>") {
        if let Some(end) = rest.find("</think>") {
            s = rest[end + "</think>".len()..].trim();
        }
    }

    // Strip a full Markdown code-fence wrapper (```lang ... ```), if present.
    let fenced = strip_code_fences(s);
    let cleaned = fenced.trim();

    if cleaned.starts_with('{')
        && cleaned.ends_with('}')
        && serde_json::from_str::<serde_json::Value>(cleaned)
            .map(|v| v.is_object())
            .unwrap_or(false)
    {
        Some(cleaned.to_string())
    } else {
        None
    }
}

/// Tolerant extraction of the first balanced, top-level JSON object from model
/// output (Requirement 0.4). "Thinking" models (e.g. deepseek-v4-flash) and
/// proxies often wrap the object in ```json code fences, leading prose /
/// chain-of-thought preamble, or trailing commentary. This finds the FIRST
/// balanced `{...}` object — respecting string literals and escapes so braces
/// inside strings don't break balancing — and returns it as an owned string
/// **only if it parses as a JSON object**.
///
/// This does NOT lenient-scrape arbitrary prose: it extracts a single
/// syntactically-complete object and leaves strict schema validation to the
/// caller. Returns `None` when no balanced object that parses as an object is
/// found.
pub fn extract_first_json_object(content: &str) -> Option<String> {
    // Fast path: already a clean object once fences/whitespace are stripped.
    let stripped = strip_code_fences(content);
    let trimmed = stripped.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        if serde_json::from_str::<serde_json::Value>(trimmed)
            .map(|v| v.is_object())
            .unwrap_or(false)
        {
            return Some(trimmed.to_string());
        }
    }

    // Scan for the first balanced top-level `{...}` object.
    let bytes = stripped.as_bytes();
    let mut depth: i32 = 0;
    let mut start: Option<usize> = None;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, &b) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = Some(idx);
                }
                depth += 1;
            }
            b'}' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(s) = start {
                            let candidate = &stripped[s..=idx];
                            if serde_json::from_str::<serde_json::Value>(candidate)
                                .map(|v| v.is_object())
                                .unwrap_or(false)
                            {
                                return Some(candidate.to_string());
                            }
                            // Not a valid object; keep scanning for the next one.
                            start = None;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Strip a single leading/trailing Markdown code-fence wrapper (```json / ```)
/// if the trimmed content is fully wrapped in one. Inner content is returned
/// untrimmed of interior whitespace (the caller trims). When no full fence
/// wrapper is present the original string is returned unchanged.
fn strip_code_fences(content: &str) -> String {
    let trimmed = content.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        // Drop an optional language tag on the first fence line.
        let after_lang = match rest.find('\n') {
            Some(nl) => &rest[nl + 1..],
            None => rest,
        };
        if let Some(body) = after_lang
            .strip_suffix("```")
            .or_else(|| after_lang.trim_end().strip_suffix("```"))
        {
            return body.to_string();
        }
        return after_lang.to_string();
    }
    content.to_string()
}

/// Trait for all LLM backends (local and cloud).
#[async_trait]
pub trait LlmBackend: Send + Sync {
    fn model_label(&self) -> &str;
    fn capabilities(&self) -> &[String];
    fn is_configured(&self) -> bool;

    /// Whether this backend can perform grammar-constrained (JSON-schema)
    /// decoding through [`chat_with_grammar`].
    ///
    /// Backends that rely on the DEFAULT [`chat_with_grammar`] implementation
    /// (which silently falls back to an unconstrained `chat` call and therefore
    /// cannot guarantee schema-valid JSON) MUST leave this `false`. Only
    /// backends that genuinely post a grammar / `json_schema` constraint to the
    /// inference server should override it to `true`.
    ///
    /// This is the capability signal used by the GUI Cognition planner
    /// (Requirement 1.2/1.5) to decide whether the LLM planner can be relied
    /// upon for schema-valid plans, or whether the deterministic fallback is the
    /// expected path for this model. It is additive and always-on (truthful
    /// reporting), independent of any feature flag.
    ///
    /// Back-compat (Requirement 0.1): the default now derives from
    /// [`structured_output_mode`](LlmBackend::structured_output_mode) — it is
    /// `true` exactly when the backend's structured mode is
    /// [`StructuredOutputMode::Grammar`]. A backend MAY still override this
    /// directly (existing overrides keep their meaning).
    fn supports_grammar(&self) -> bool {
        matches!(self.structured_output_mode(), StructuredOutputMode::Grammar)
    }

    /// The structured-output method this backend genuinely honors
    /// (Requirement 0.1). Default is [`StructuredOutputMode::None`] — a backend
    /// that posts no constraint and relies on the DEFAULT
    /// [`chat_with_grammar`](LlmBackend::chat_with_grammar)/
    /// [`chat_structured`](LlmBackend::chat_structured) (which fall back to an
    /// unconstrained `chat`) MUST leave this `None`.
    ///
    /// This is the synchronous, cached/configured view. For OpenAI-compatible
    /// proxies whose real capability is unknown until probed (e.g. opencode/zen
    /// may strip `response_format`), the runtime probe
    /// [`detect_structured_output_mode`](LlmBackend::detect_structured_output_mode)
    /// refines and caches this value; until a probe runs this returns the
    /// configured/default expectation.
    fn structured_output_mode(&self) -> StructuredOutputMode {
        StructuredOutputMode::None
    }

    /// Cheap, cached per-(provider, model) runtime capability probe
    /// (Requirement 0.2). Detects what the endpoint ACTUALLY honors rather than
    /// assuming a proxy passes `response_format` through, caching the result so
    /// repeated planner turns do not re-probe.
    ///
    /// The DEFAULT performs NO network I/O and simply returns the static
    /// [`structured_output_mode`](LlmBackend::structured_output_mode), so test
    /// backends and non-cloud backends never hit the network. Only backends that
    /// override it (the OpenAI-compatible cloud client) perform the real probe,
    /// and even there the probe is gated so CI/tests inject capability instead of
    /// reaching the network.
    async fn detect_structured_output_mode(&self) -> StructuredOutputMode {
        self.structured_output_mode()
    }

    /// Returns the base HTTP URL of the backend's inference server, if any.
    /// Used by the tokenizer helper (`llm::tokenize::count_tokens`) to obtain
    /// exact token counts without adding a new crate dependency.
    /// Backends that do not expose a local HTTP server should return `""`.
    fn tokenizer_base_url(&self) -> String {
        String::new()
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse>;

    async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = String> + Send>>>;

    async fn health_check(&self) -> bool;

    /// Grammar-constrained chat call.
    ///
    /// Posts a `json_schema` field to activate constrained decoding.
    /// Default implementation falls back to unconstrained `chat`.
    async fn chat_with_grammar(
        &self,
        messages: &[ChatMessage],
        json_schema: serde_json::Value,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        let _ = json_schema;
        self.chat(messages, None, temperature, max_tokens).await
    }

    /// Shared multi-backend structured-output entry point (Requirement 0.2).
    ///
    /// Returns a response whose `content` is a single JSON object that satisfies
    /// `json_schema`, produced via the STRONGEST method the backend genuinely
    /// honors ([`structured_output_mode`](LlmBackend::structured_output_mode)):
    /// grammar/guided-JSON (local) → `response_format` `json_schema` → else
    /// `json_object` (with the word "json" + a compact schema + one few-shot
    /// injected into the prompt) → else function/tool-calling (normalizing
    /// `tool_calls[0].function.arguments` back into `content`). The structured
    /// request is always NON-streaming.
    ///
    /// `schema_name` is a short identifier for the schema (used by the
    /// `json_schema` `response_format` envelope). The DEFAULT implementation
    /// delegates to [`chat_with_grammar`](LlmBackend::chat_with_grammar) so a
    /// backend that does not override this keeps its EXACT prior behavior
    /// (local posts the grammar; everything else falls back to unconstrained
    /// `chat`). Only the OpenAI-compatible cloud client overrides this to post
    /// the structured request.
    async fn chat_structured(
        &self,
        messages: &[ChatMessage],
        json_schema: serde_json::Value,
        schema_name: &str,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        let _ = schema_name;
        self.chat_with_grammar(messages, json_schema, temperature, max_tokens)
            .await
    }
}

/// Context overflow error — exempted from circuit breaker failure counts.
#[derive(Debug, thiserror::Error)]
#[error("context too large for model")]
pub struct ContextTooLargeError;

/// Max chars for tool results in context.
pub const TOOL_RESULT_MAX_CHARS: usize = 3000;

/// Per-tool token budget for shaped LLM injection (≈ 1 024 tokens).
pub const LLM_TOOL_RESULT_TOKEN_BUDGET: usize = 1024;

/// Per-turn aggregate token budget for all tool outputs combined (≈ 4 096 tokens).
/// When the turn total exceeds this, subsequent tools are short-circuited.
pub const LLM_TURN_TOOL_BUDGET: usize = 4096;

/// Trim messages to fit context window.
///
/// Attempt 0: compress tool-result and very large messages.
/// Attempt 1: keep only the latest 8 non-system messages and shorten the system prompt.
/// Attempt 2+: keep only the latest 3 non-system messages and a minimal system prompt.
pub fn trim_messages_for_context(messages: &[ChatMessage], attempt: usize) -> Vec<ChatMessage> {
    if messages.is_empty() {
        return Vec::new();
    }

    match attempt {
        0 => {
            // Stage 1: compress large tool results and oversized non-system messages while
            // preserving the system prompt and full turn history shape.
            messages
                .iter()
                .map(|m| {
                    let mut msg = m.clone();
                    if msg.role == "system" {
                        // Never truncate the system prompt in stage 0 — it contains
                        // the tool-calling schema and critical rules.
                    } else if msg.role == "tool" {
                        msg.content =
                            truncate_with_suffix(&msg.content, 500, "...<tool-truncated>");
                    } else {
                        msg.content = truncate_with_suffix(&msg.content, 1800, "...<truncated>");
                    }
                    msg
                })
                .collect()
        }
        1 => {
            // Stage 2: keep the latest conversation turns and compact the system prompt.
            let mut systems: Vec<ChatMessage> = messages
                .iter()
                .filter(|m| m.role == "system")
                .cloned()
                .collect();
            if let Some(first) = systems.first_mut() {
                first.content = minimal_system_prompt();
            }

            let mut non_system: Vec<ChatMessage> = messages
                .iter()
                .filter(|m| m.role != "system")
                .cloned()
                .collect();
            if non_system.len() > 8 {
                non_system = non_system.split_off(non_system.len() - 8);
            }
            for msg in &mut non_system {
                let max_chars = if msg.role == "tool" { 350 } else { 900 };
                let suffix = if msg.role == "tool" {
                    "...<tool-truncated>"
                } else {
                    "...<truncated>"
                };
                msg.content = truncate_with_suffix(&msg.content, max_chars, suffix);
            }

            systems.into_iter().chain(non_system).collect()
        }
        _ => {
            // Stage 3: emergency context fit — keep minimal instruction and only
            // the newest few turns.
            let mut out = Vec::new();
            out.push(ChatMessage {
                role: "system".into(),
                content: minimal_system_prompt(),
                name: None,
                images: None,
            });

            let mut non_system: Vec<ChatMessage> = messages
                .iter()
                .filter(|m| m.role != "system")
                .cloned()
                .collect();
            if non_system.len() > 3 {
                non_system = non_system.split_off(non_system.len() - 3);
            }
            for msg in &mut non_system {
                let max_chars = if msg.role == "tool" { 240 } else { 700 };
                let suffix = if msg.role == "tool" {
                    "...<tool-truncated>"
                } else {
                    "...<truncated>"
                };
                msg.content = truncate_with_suffix(&msg.content, max_chars, suffix);
            }
            out.extend(non_system);
            out
        }
    }
}

fn truncate_with_suffix(text: &str, max_chars: usize, suffix: &str) -> String {
    let len = text.chars().count();
    if len <= max_chars {
        return text.to_string();
    }

    let suffix_chars = suffix.chars().count();
    let keep = max_chars.saturating_sub(suffix_chars).max(1);
    let mut s: String = text.chars().take(keep).collect();
    s.push_str(suffix);
    s
}

fn minimal_system_prompt() -> String {
    "You are KRIA, an AI assistant. Be concise, accurate, and safe. \
 CRITICAL: When the user asks you to perform an action (generate an image, search the web, \
 send email, run code, etc.), you MUST call the appropriate tool — never refuse or say you cannot. \
 Always respond with a tool call JSON when a tool is available for the request. \
 Use available tools for live/current information instead of claiming no real-time access. \
 Avoid repeating unchanged context."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        extract_openai_content_text, extract_openai_tool_calls, trim_messages_for_context,
    };
    use crate::llm::ChatMessage;

    #[test]
    fn extract_content_text_handles_string_and_parts() {
        let plain = serde_json::json!("hello world");
        assert_eq!(extract_openai_content_text(&plain), "hello world");

        let parts = serde_json::json!([
            {"type": "text", "text": "first"},
            {"type": "text", "text": "second"}
        ]);
        assert_eq!(extract_openai_content_text(&parts), "first\nsecond");
    }

    #[test]
    fn extract_tool_calls_supports_legacy_function_call() {
        let msg = serde_json::json!({
            "function_call": {
                "name": "analyze_image",
                "arguments": "{\"path\":\"/tmp/a.png\"}"
            }
        });

        let calls = extract_openai_tool_calls(&msg).expect("tool calls expected");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "analyze_image");
    }

    #[test]
    fn trim_attempt_two_keeps_minimal_context() {
        let mut msgs = Vec::new();
        msgs.push(ChatMessage {
            role: "system".into(),
            content: "very long system prompt".repeat(40),
            name: None,
            images: None,
        });
        for i in 0..8 {
            msgs.push(ChatMessage {
                role: if i % 2 == 0 {
                    "user".into()
                } else {
                    "assistant".into()
                },
                content: format!("message {i} {}", "x".repeat(1200)),
                name: None,
                images: None,
            });
        }

        let trimmed = trim_messages_for_context(&msgs, 2);
        assert_eq!(trimmed[0].role, "system");
        assert!(trimmed.len() <= 4, "should keep system + latest few turns");
        assert!(trimmed[0].content.contains("You are KRIA"));
    }
}
