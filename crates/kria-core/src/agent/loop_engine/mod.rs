use chrono::{Datelike, Duration, Local, SecondsFormat, TimeZone, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::agent::response_parser::{
    extract_text_response, parse_tool_calls_with_known, ParsedToolCall,
};
use crate::agent::turn_context::{TurnAdmission, TurnAdmissionDecision, TurnAdmissionError};
use crate::agent::turn_gate::{Operation, ResourcePlan, TurnGate};
use crate::infra::isolation::run_isolated;
use crate::infra::pipeline_trace::{
    log_pipeline_step, sanitize_json_for_logs, sanitize_text_for_logs,
};
use crate::llm::orchestrator::vision_strategy::VisionMode;
use crate::llm::orchestrator::vram_budget::{calculate_safe_visual_tokens, estimate_visual_tokens};
use crate::llm::tokenize::count_tokens;
use crate::llm::{
    ChatMessage, ImageAttachment, LlmResponse, ModelRouter, ToolSchema,
    LLM_TOOL_RESULT_TOKEN_BUDGET, LLM_TURN_TOOL_BUDGET, TOOL_RESULT_MAX_CHARS,
};
use crate::mcp::payload_shaper::shape_for_llm;
use crate::safety::audit::{DecidedBy, Decision};
use crate::safety::hitl::{ApprovalResponse, HitlGateway};
use crate::safety::{AuditLogger, PolicyEngine, RiskLevel, RollbackManager};
use crate::tools::mount_manager::{google_meet_fallback_metadata, ToolMountManager};
use crate::tools::registry::{ToolDef, ToolRegistry};

mod helpers;
mod intent_fallback;
mod intent_extractors;
mod response_helpers;

use helpers::*;
use intent_fallback::*;
use intent_extractors::*;
use response_helpers::*;
fn build_message_preview(messages: &[ChatMessage], max_messages: usize) -> serde_json::Value {
    let start = messages.len().saturating_sub(max_messages);
    let preview: Vec<serde_json::Value> = messages
        .iter()
        .skip(start)
        .map(|m| {
            let content_chars = m.content.chars().count();
            let content_preview = if m.role.eq_ignore_ascii_case("system") {
                format!("[system prompt omitted; {content_chars} chars]")
            } else {
                sanitize_text_for_logs(&m.content, 160)
            };

            serde_json::json!({
                "role": m.role,
                "name": m.name,
                "has_images": m.has_images(),
                "content": content_preview,
                "content_chars": content_chars,
            })
        })
        .collect();

    serde_json::Value::Array(preview)
}

const MAX_ROUTED_TOOL_SCHEMAS_PER_TURN: usize = 8;
const CONTEXT_HISTORY_ITEM_CHAR_CAP: usize = 900;
const CONTEXT_TOTAL_CHAR_BUDGET: usize = 12_000;

fn extract_user_context_block(system_prompt: &str) -> Option<String> {
    const USER_CONTEXT_HEADER: &str = "## User Context";
    const RESPONSE_MARKER: &str = "Respond naturally.";

    let start = system_prompt.find(USER_CONTEXT_HEADER)?;
    let after_header = &system_prompt[start + USER_CONTEXT_HEADER.len()..];
    let end = after_header
        .find(RESPONSE_MARKER)
        .unwrap_or(after_header.len());
    let block = after_header[..end].trim();
    if block.is_empty() {
        None
    } else {
        Some(block.to_string())
    }
}

fn build_filtered_tool_schema_catalog(tool_schemas: &[ToolSchema]) -> String {
    if tool_schemas.is_empty() {
        return "No tools are enabled for this turn. Reply conversationally unless a tool-enabled follow-up is required.".to_string();
    }

    let mut lines = Vec::with_capacity(tool_schemas.len() + 2);
    lines.push(format!(
        "Only the following {} routed tool(s) are enabled for this turn.",
        tool_schemas.len()
    ));
    lines.push(
        "Use exact tool names. Function schemas are provided separately by the runtime."
            .to_string(),
    );

    for schema in tool_schemas {
        lines.push(format!(
            "- {}: {}",
            schema.name,
            sanitize_text_for_logs(&schema.description, 120)
        ));
    }

    lines.join("\n")
}

fn rewrite_system_prompt_tools_block(system_prompt: &str, tool_schemas: &[ToolSchema]) -> String {
    let user_context = extract_user_context_block(system_prompt);
    let mut rebuilt = String::with_capacity(2800);
    rebuilt.push_str(
        "You are K.R.I.A., a desktop AI assistant.\n\n\
## Core Rules\n\
1. Use tools when the user asks for actions or live data; otherwise answer conversationally.\n\
2. Never invent tool outputs. If a tool fails, report the failure and retry with a sensible alternative.\n\
3. Do not ask for confirmation when intent is clear. Execute the best matching tool.\n\
4. Keep responses concise and grounded in available evidence.\n\
5. Match the user's language.\n\
6. For web/info lookup use dedicated web/news tools, not browser-opening tools unless user explicitly asks to open a browser.\n\n\
## Enabled Tools\n",
    );
    rebuilt.push_str(&build_filtered_tool_schema_catalog(tool_schemas));

    if let Some(context) = user_context {
        rebuilt.push_str("\n\n## User Context\n");
        rebuilt.push_str(&sanitize_text_for_logs(&context, 1200));
    }

    rebuilt.push_str(
        "\n\nWhen tools are needed, emit:\n\
<tool_call>\n\
{\"name\":\"tool_name\",\"arguments\":{\"param\":\"value\"}}\n\
</tool_call>\n\
Then continue with grounded results.",
    );
    rebuilt
}

fn truncate_text_for_context(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    if max_chars <= 40 {
        return text.chars().take(max_chars).collect();
    }

    let head_budget = (max_chars * 3) / 4;
    let tail_budget = max_chars.saturating_sub(head_budget).saturating_sub(24);
    let head: String = text.chars().take(head_budget).collect();
    let tail: String = if tail_budget > 0 {
        text.chars()
            .rev()
            .take(tail_budget)
            .collect::<String>()
            .chars()
            .rev()
            .collect()
    } else {
        String::new()
    };
    let omitted = char_count.saturating_sub(head_budget + tail_budget);
    if tail.is_empty() {
        format!("{head}\n...[truncated {omitted} chars]")
    } else {
        format!("{head}\n...[truncated {omitted} chars]\n{tail}")
    }
}

fn compact_messages_for_chat(messages: &mut Vec<ChatMessage>) {
    if messages.is_empty() {
        return;
    }

    let mut latest_user_idx = messages.iter().rposition(|m| m.role == "user");

    for (idx, msg) in messages.iter_mut().enumerate() {
        if msg.role.eq_ignore_ascii_case("system") {
            let max_chars = if idx == 0 { 3_500 } else { 1_000 };
            msg.content = truncate_text_for_context(&msg.content, max_chars);
            continue;
        }

        if Some(idx) == latest_user_idx {
            msg.content = truncate_text_for_context(&msg.content, 2_000);
            continue;
        }

        msg.content = truncate_text_for_context(&msg.content, CONTEXT_HISTORY_ITEM_CHAR_CAP);
    }

    let mut total_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
    while total_chars > CONTEXT_TOTAL_CHAR_BUDGET && messages.len() > 2 {
        let removable_idx = messages.iter().enumerate().skip(1).find_map(|(idx, msg)| {
            if msg.role.eq_ignore_ascii_case("system") || Some(idx) == latest_user_idx {
                None
            } else {
                Some(idx)
            }
        });

        let Some(idx) = removable_idx else {
            break;
        };

        total_chars = total_chars.saturating_sub(messages[idx].content.chars().count());
        messages.remove(idx);

        if let Some(user_idx) = latest_user_idx {
            if idx < user_idx {
                latest_user_idx = Some(user_idx - 1);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct VisualTokenCapDecision {
    hard_cap: u32,
    safe_cap: u32,
    free_vram_mb: u64,
    safety_margin_mb: u64,
    vision_mode: VisionMode,
}

fn add_tool_if_available(
    allowed_tool_names: &HashSet<String>,
    selected: &mut HashSet<String>,
    name: &str,
) {
    if allowed_tool_names.contains(name) {
        selected.insert(name.to_string());
    }
}

fn fallback_routed_tool_candidates(
    user_text: &str,
    intent_hint: Option<&str>,
    allowed_tool_names: &HashSet<String>,
) -> HashSet<String> {
    let mut selected = HashSet::new();
    let lower = user_text.to_ascii_lowercase();

    if let Some(hint) = intent_hint.map(str::trim).filter(|s| !s.is_empty()) {
        add_tool_if_available(allowed_tool_names, &mut selected, hint);
    }

    if lower.contains("install")
        || lower.contains("uninstall")
        || lower.contains("package")
        || lower.contains("installed app")
    {
        for tool in [
            "search_package",
            "check_package_installed",
            "install_package",
            "uninstall_package",
        ] {
            add_tool_if_available(allowed_tool_names, &mut selected, tool);
        }
    }

    if lower.contains("news") || lower.contains("headline") {
        add_tool_if_available(allowed_tool_names, &mut selected, "search_news");
    }

    if lower.contains("search")
        || lower.contains("look up")
        || lower.contains("find information")
        || lower.contains("web")
    {
        add_tool_if_available(allowed_tool_names, &mut selected, "web_search");
        add_tool_if_available(allowed_tool_names, &mut selected, "searxng_search");
    }

    if lower.contains("file") || lower.contains("folder") || lower.contains("directory") {
        for tool in [
            "mcp_fs_search_files",
            "search_files",
            "find_files_by_pattern",
        ] {
            add_tool_if_available(allowed_tool_names, &mut selected, tool);
        }
    }

    if lower.contains("image")
        || lower.contains("draw")
        || lower.contains("generate")
        || lower.contains("art")
    {
        add_tool_if_available(allowed_tool_names, &mut selected, "generate_image");
    }

    if looks_like_google_workspace_request(&lower) {
        for tool in [
            "gw_gmail_inbox",
            "gw_gmail_search",
            "gw_gmail_read",
            "gw_gmail_send",
            "gw_calendar_search",
            "gw_calendar_create",
            "gw_drive_search",
            "gw_drive_read",
            "gw_docs_read",
            "gw_docs_edit",
        ] {
            add_tool_if_available(allowed_tool_names, &mut selected, tool);
        }
    }

    selected
}

fn score_tool_relevance(query_text: &str, schema: &ToolSchema) -> i32 {
    let query = query_text.to_ascii_lowercase();
    let name = schema.name.to_ascii_lowercase();
    let description = schema.description.to_ascii_lowercase();
    let mut score = 0;

    for token in query
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 3)
    {
        if name.contains(token) {
            score += 6;
        }
        if description.contains(token) {
            score += 2;
        }
    }

    if (query.contains("install") || query.contains("uninstall") || query.contains("package"))
        && schema.name.contains("package")
    {
        score += 8;
    }
    if query.contains("news") && schema.name == "search_news" {
        score += 10;
    }
    if (query.contains("search") || query.contains("web"))
        && (schema.name == "web_search" || schema.name == "searxng_search")
    {
        score += 8;
    }
    if query.contains("image") && schema.name == "generate_image" {
        score += 10;
    }

    score
}

#[allow(clippy::too_many_arguments)]
fn select_routed_tool_schemas(
    all_tool_schemas: &[ToolSchema],
    query_text: &str,
    direct_tool_hint: Option<&str>,
    selected_tool_names: &HashSet<String>,
    fallback_tool_names: &HashSet<String>,
    forced_tool_name: Option<&str>,
    tool_lock_name: Option<&str>,
    _conversation_only: bool,
) -> Vec<ToolSchema> {
    let mut include_names: HashSet<String> = if direct_tool_hint.is_some() {
        HashSet::new()
    } else {
        selected_tool_names.clone()
    };
    let mut pinned_names: HashSet<String> = HashSet::new();
    if let Some(tool) = direct_tool_hint.map(str::trim).filter(|s| !s.is_empty()) {
        include_names.insert(tool.to_string());
        pinned_names.insert(tool.to_string());
    }
    if let Some(tool) = forced_tool_name.map(str::trim).filter(|s| !s.is_empty()) {
        include_names.insert(tool.to_string());
        pinned_names.insert(tool.to_string());
    }
    if let Some(tool) = tool_lock_name.map(str::trim).filter(|s| !s.is_empty()) {
        include_names.insert(tool.to_string());
        pinned_names.insert(tool.to_string());
    }
    if include_names.is_empty() {
        include_names.extend(fallback_tool_names.iter().cloned());
        pinned_names.extend(fallback_tool_names.iter().cloned());
    }

    let filtered: Vec<ToolSchema> = if include_names.is_empty() {
        Vec::new()
    } else {
        all_tool_schemas
            .iter()
            .filter(|schema| include_names.contains(&schema.name))
            .cloned()
            .collect()
    };

    let mut ranked: Vec<(bool, i32, ToolSchema)> = filtered
        .into_iter()
        .map(|schema| {
            let pinned = pinned_names.contains(&schema.name);
            let score = score_tool_relevance(query_text, &schema);
            (pinned, score, schema)
        })
        .collect();

    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.name.cmp(&b.2.name))
    });

    if ranked.len() > MAX_ROUTED_TOOL_SCHEMAS_PER_TURN {
        ranked.truncate(MAX_ROUTED_TOOL_SCHEMAS_PER_TURN);
    }

    ranked.into_iter().map(|(_, _, schema)| schema).collect()
}

fn build_tool_calls_preview(tool_calls: &[ParsedToolCall]) -> serde_json::Value {
    let preview: Vec<serde_json::Value> = tool_calls
        .iter()
        .map(|call| {
            serde_json::json!({
                "name": call.name,
                "arguments": sanitize_json_for_logs(&call.arguments, 220, 8),
            })
        })
        .collect();

    serde_json::Value::Array(preview)
}

fn build_tool_call_history_content(tool_calls: &[ParsedToolCall]) -> String {
    tool_calls
        .iter()
        .map(|call| {
            format!(
                "<tool_call>\n{{\"name\":\"{}\",\"arguments\":{}}}\n</tool_call>",
                call.name, call.arguments
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}



fn tool_choice_label(name: &str) -> String {
    match name {
        "search_news" => "News Search".into(),
        "web_search" | "searxng_search" => "Web Search".into(),
        "search_files" | "find_files_by_pattern" | "mcp_fs_search_files" => "File Search".into(),
        "open_application" => "Open App".into(),
        "open_url" => "Open URL".into(),
        "browser_search" => "Browser Search".into(),
        "send_message" => "Send Message".into(),
        "close_application" | "kill_process" => "Close App".into(),
        "gw_gmail_inbox" | "gw_gmail_search" | "gw_gmail_read" | "gw_gmail_send"
        | "gw_gmail_delete" => "Gmail".into(),
        "gw_calendar_today" | "gw_calendar_search" | "gw_calendar_create"
        | "gw_calendar_delete" => "Google Calendar".into(),
        "gw_drive_search" | "gw_drive_list" | "gw_drive_read" | "gw_drive_delete" => {
            "Google Drive".into()
        }
        "gw_docs_create" | "gw_docs_read" | "gw_docs_edit" => "Google Docs".into(),
        "gw_sheets_create" | "gw_sheets_read" | "gw_sheets_edit" => "Google Sheets".into(),
        "gw_slides_create" | "gw_slides_read" => "Google Slides".into(),
        "gw_forms_list" | "gw_forms_create" => "Google Forms".into(),
        other if other.starts_with("mcp_") && other.contains("colab") => "Google Colab".into(),
        other => other.to_string(),
    }
}

fn push_tool_choice_candidate(
    candidates: &mut Vec<ToolChoiceCandidate>,
    allowed_tool_names: &HashSet<String>,
    name: &str,
    reason: &str,
    confidence: f32,
) {
    if !allowed_tool_names.contains(name) {
        return;
    }

    if candidates.iter().any(|c| c.name == name) {
        return;
    }

    candidates.push(ToolChoiceCandidate {
        name: name.to_string(),
        label: tool_choice_label(name),
        reason: reason.to_string(),
        confidence,
    });
}

fn build_tool_choice_candidates(
    user_text: &str,
    allowed_tool_names: &HashSet<String>,
    primary_hint: Option<&str>,
    confidence: f32,
) -> Vec<ToolChoiceCandidate> {
    let mut candidates: Vec<ToolChoiceCandidate> = Vec::new();
    let lower = user_text.to_lowercase();

    if let Some(primary) = primary_hint {
        push_tool_choice_candidate(
            &mut candidates,
            allowed_tool_names,
            primary,
            "Primary match from intent classifier",
            confidence,
        );
    }

    if lower.contains("news") || lower.contains("headline") {
        push_tool_choice_candidate(
            &mut candidates,
            allowed_tool_names,
            "search_news",
            "Best for current events and corroborated headlines",
            0.62,
        );
    }

    if lower.contains("search") || lower.contains("online") || lower.contains("web") {
        push_tool_choice_candidate(
            &mut candidates,
            allowed_tool_names,
            "web_search",
            "Best for broad web lookups",
            0.60,
        );
        push_tool_choice_candidate(
            &mut candidates,
            allowed_tool_names,
            "searxng_search",
            "Best for self-hosted/privacy web lookups",
            0.58,
        );
    }

    if lower.contains("file") || lower.contains("folder") || lower.contains("directory") {
        push_tool_choice_candidate(
            &mut candidates,
            allowed_tool_names,
            "mcp_fs_search_files",
            "Best for workspace/filesystem search",
            0.61,
        );
        push_tool_choice_candidate(
            &mut candidates,
            allowed_tool_names,
            "find_files_by_pattern",
            "Best for local file pattern lookup",
            0.57,
        );
    }

    if looks_like_google_workspace_request(&lower) {
        for tool in [
            "gw_gmail_inbox",
            "gw_gmail_search",
            "gw_gmail_send",
            "gw_calendar_search",
            "gw_calendar_create",
            "gw_drive_list",
            "gw_drive_search",
            "gw_docs_read",
            "gw_sheets_read",
            "gw_slides_read",
            "gw_forms_list",
        ] {
            push_tool_choice_candidate(
                &mut candidates,
                allowed_tool_names,
                tool,
                "Google Workspace request detected",
                0.56,
            );
        }
    }

    if looks_like_colab_request(&lower) {
        for tool in allowed_tool_names
            .iter()
            .filter(|name| name.starts_with("mcp_") && name.contains("colab"))
            .take(6)
        {
            push_tool_choice_candidate(
                &mut candidates,
                allowed_tool_names,
                tool,
                "Google Colab request detected",
                0.56,
            );
        }
    }

    candidates.truncate(6);
    candidates
}

fn build_grounding_count_note(tool_name: &str, tool_result: &serde_json::Value) -> Option<String> {
    if !tool_name.starts_with("gw_") {
        return None;
    }

    let payload = tool_result.get("data").unwrap_or(tool_result);
    let requested = payload.get("requested_count").and_then(|v| v.as_u64())?;
    let returned = payload
        .get("returned_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(requested);

    if let Some(visible) = payload
        .get("llm_visible_message_count")
        .and_then(|v| v.as_u64())
    {
        if visible < returned {
            return Some(format!(
                "GROUNDING_NOTE: requested {requested} item(s), returned {returned} grounded item(s), but only {visible} row(s) are visible in this context. Do NOT invent or duplicate hidden rows; enumerate at most {visible} visible row(s) and mention that additional rows were omitted."
            ));
        }
    }

    Some(format!(
        "GROUNDING_NOTE: requested {requested} item(s), returned {returned} grounded item(s). Never claim or enumerate more than {returned}."
    ))
}

const LLM_GMAIL_MESSAGES_CHAR_BUDGET: usize = 3500;
const LLM_GMAIL_PREVIEW_MAX_CHARS: usize = 220;
const LLM_GMAIL_FIELD_MAX_CHARS: usize = 160;
const LLM_GMAIL_WARNING_MAX_CHARS: usize = 180;
const LLM_GMAIL_WARNING_LIMIT: usize = 3;

fn compact_text_for_llm(raw: &str, max_chars: usize) -> String {
    let filtered: String = raw
        .chars()
        .filter(|ch| {
            !matches!(
                *ch,
                '\u{034F}' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}'
            )
        })
        .collect();
    let collapsed = filtered.split_whitespace().collect::<Vec<_>>().join(" ");

    let trimmed = collapsed.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let mut truncated: String = trimmed.chars().take(max_chars).collect();
    truncated.push_str("...");
    truncated
}

fn first_non_empty_string_field(
    message: &serde_json::Value,
    keys: &[&str],
    max_chars: usize,
) -> Option<String> {
    keys.iter().find_map(|key| {
        message
            .get(*key)
            .and_then(|v| v.as_str())
            .map(|v| compact_text_for_llm(v, max_chars))
            .filter(|v| !v.is_empty())
    })
}

fn compact_gmail_message_for_llm(message: &serde_json::Value) -> serde_json::Value {
    if !message.is_object() {
        return message.clone();
    }

    let mut compacted = serde_json::Map::new();

    if let Some(subject) = first_non_empty_string_field(
        message,
        &["subject", "title", "summary"],
        LLM_GMAIL_FIELD_MAX_CHARS,
    ) {
        compacted.insert("subject".into(), serde_json::Value::String(subject));
    }

    if let Some(from) = first_non_empty_string_field(
        message,
        &["from", "sender", "organizer"],
        LLM_GMAIL_FIELD_MAX_CHARS,
    ) {
        compacted.insert("from".into(), serde_json::Value::String(from));
    }

    if let Some(date) = first_non_empty_string_field(
        message,
        &["date", "updated", "created"],
        LLM_GMAIL_FIELD_MAX_CHARS,
    ) {
        compacted.insert("date".into(), serde_json::Value::String(date));
    }

    if let Some(id) = first_non_empty_string_field(
        message,
        &["id", "messageId", "message_id", "threadId", "thread_id"],
        LLM_GMAIL_FIELD_MAX_CHARS,
    ) {
        compacted.insert("id".into(), serde_json::Value::String(id));
    }

    if let Some(preview) = first_non_empty_string_field(
        message,
        &[
            "preview",
            "snippet",
            "description",
            "text",
            "content",
            "body",
        ],
        LLM_GMAIL_PREVIEW_MAX_CHARS,
    ) {
        compacted.insert("preview".into(), serde_json::Value::String(preview));
    }

    if let Some(url) = first_non_empty_string_field(
        message,
        &["url", "htmlLink", "webViewLink", "alternateLink"],
        LLM_GMAIL_FIELD_MAX_CHARS,
    ) {
        compacted.insert("url".into(), serde_json::Value::String(url));
    }

    serde_json::Value::Object(compacted)
}

fn compact_gmail_messages_for_llm(
    messages: &[serde_json::Value],
) -> (Vec<serde_json::Value>, usize) {
    let mut visible = Vec::new();
    let mut used_chars = 0usize;
    let mut omitted = 0usize;

    for (index, message) in messages.iter().enumerate() {
        let compacted = compact_gmail_message_for_llm(message);
        let chunk_len = compacted.to_string().len();

        if index == 0 || used_chars + chunk_len <= LLM_GMAIL_MESSAGES_CHAR_BUDGET {
            used_chars += chunk_len;
            visible.push(compacted);
        } else {
            omitted += 1;
        }
    }

    (visible, omitted)
}

fn compact_gmail_payload_for_llm(payload: &serde_json::Value) -> serde_json::Value {
    let Some(payload_obj) = payload.as_object() else {
        return payload.clone();
    };

    let mut compacted = payload_obj.clone();

    if let Some(query) = compacted.get("query").and_then(|v| v.as_str()) {
        compacted.insert(
            "query".into(),
            serde_json::Value::String(compact_text_for_llm(query, LLM_GMAIL_FIELD_MAX_CHARS)),
        );
    }

    if let Some(warnings) = compacted.get("warnings").and_then(|v| v.as_array()) {
        let compacted_warnings: Vec<serde_json::Value> = warnings
            .iter()
            .take(LLM_GMAIL_WARNING_LIMIT)
            .filter_map(|warning| warning.as_str())
            .map(|warning| {
                serde_json::Value::String(compact_text_for_llm(
                    warning,
                    LLM_GMAIL_WARNING_MAX_CHARS,
                ))
            })
            .collect();
        compacted.insert(
            "warnings".into(),
            serde_json::Value::Array(compacted_warnings),
        );
    }

    let messages = compacted
        .get("messages")
        .or_else(|| compacted.get("results"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if !messages.is_empty() {
        let total = messages.len();
        let (visible_messages, omitted_messages) = compact_gmail_messages_for_llm(&messages);
        compacted.insert(
            "messages".into(),
            serde_json::Value::Array(visible_messages.clone()),
        );
        compacted.insert(
            "llm_visible_message_count".into(),
            serde_json::json!(visible_messages.len()),
        );
        if omitted_messages > 0 {
            compacted.insert(
                "llm_omitted_message_count".into(),
                serde_json::json!(omitted_messages),
            );
            compacted.insert(
                "warnings".into(),
                match compacted.get("warnings").and_then(|v| v.as_array()) {
                    Some(existing) => {
                        let mut merged = existing.clone();
                        merged.push(serde_json::Value::String(format!(
                            "{} Gmail message(s) omitted from LLM context to stay within context budget.",
                            omitted_messages
                        )));
                        serde_json::Value::Array(merged)
                    }
                    None => serde_json::Value::Array(vec![serde_json::Value::String(format!(
                        "{} Gmail message(s) omitted from LLM context to stay within context budget.",
                        omitted_messages
                    ))]),
                },
            );
        } else {
            compacted.remove("llm_omitted_message_count");
        }
        compacted.insert("count".into(), serde_json::json!(total));
    }

    serde_json::Value::Object(compacted)
}

fn compact_tool_result_for_llm(
    tool_name: &str,
    tool_result: &serde_json::Value,
) -> serde_json::Value {
    let is_gmail_tool = matches!(tool_name, "gw_gmail_inbox" | "gw_gmail_search");
    if !is_gmail_tool {
        return tool_result.clone();
    }

    if tool_result
        .get("provider")
        .and_then(|v| v.as_str())
        .map(|provider| provider.eq_ignore_ascii_case("google_workspace"))
        .unwrap_or(false)
    {
        let mut envelope = tool_result.clone();
        if let Some(env_obj) = envelope.as_object_mut() {
            env_obj.remove("raw_text");
            if let Some(payload) = env_obj.get_mut("data") {
                *payload = compact_gmail_payload_for_llm(payload);
            }
        }
        return envelope;
    }

    compact_gmail_payload_for_llm(tool_result)
}

fn extract_preprocessed_image_attachments(
    tool_data: &serde_json::Value,
    default_mime_type: &str,
) -> Option<Vec<ImageAttachment>> {
    let analysis = tool_data.get("analysis").unwrap_or(tool_data);

    let thumbnail_attachment = analysis
        .get("thumbnail_base64")
        .or_else(|| tool_data.get("thumbnail_base64"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|thumb_b64| ImageAttachment {
            data: thumb_b64.to_string(),
            mime_type: analysis
                .get("thumbnail_mime_type")
                .or_else(|| tool_data.get("thumbnail_mime_type"))
                .and_then(|v| v.as_str())
                .filter(|m| !m.trim().is_empty())
                .unwrap_or(default_mime_type)
                .to_string(),
        });

    if let Some(items) = analysis.get("selected_images").and_then(|v| v.as_array()) {
        let mut attachments = Vec::new();
        let mut has_global_frame = false;
        for item in items {
            let data = item
                .get("data_base64")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if data.is_empty() {
                continue;
            }

            let mime_type = item
                .get("mime_type")
                .and_then(|v| v.as_str())
                .filter(|m| !m.trim().is_empty())
                .unwrap_or(default_mime_type)
                .to_string();

            if item
                .get("kind")
                .and_then(|v| v.as_str())
                .map(|kind| kind.eq_ignore_ascii_case("global"))
                .unwrap_or(false)
            {
                has_global_frame = true;
            }

            attachments.push(ImageAttachment {
                data: data.to_string(),
                mime_type,
            });
        }

        if !has_global_frame {
            if let Some(thumb) = thumbnail_attachment.clone() {
                attachments.push(thumb);
            }
        }

        if !attachments.is_empty() {
            return Some(attachments);
        }
    }

    if let Some(thumb) = thumbnail_attachment {
        return Some(vec![thumb]);
    }

    None
}

// ─── Colab workflow state machine ────────────────────────────────────────────

/// What the user ultimately wants to do in Google Colab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColabIntent {
    /// Create a new .ipynb notebook (via Google Drive, then open in Colab).
    CreateNotebook,
    /// Open an existing notebook URL in Colab.
    OpenNotebook,
    /// Execute code in the currently active Colab notebook.
    ExecuteCode,
    /// General Colab request that needs the browser bridge but nothing specific.
    Generic,
}

/// Multi-step state machine that orchestrates the Colab workflow:
///   1. For CreateNotebook: drive_create → open_colab_browser_connection
///   2. For OpenNotebook / ExecuteCode / Generic: open_colab_browser_connection → (execute_cell)
#[derive(Debug, Clone)]
struct ColabFlowState {
    intent: ColabIntent,
    /// Notebook title supplied by the user (for CreateNotebook).
    notebook_title: Option<String>,
    /// Code supplied by the user (for ExecuteCode).
    code_snippet: Option<String>,
    /// Whether Drive file creation was attempted (CreateNotebook only).
    drive_create_attempted: bool,
    /// Whether Drive file creation succeeded and what the file ID is.
    drive_file_id: Option<String>,
    /// Whether open_colab_browser_connection has been called.
    browser_open_attempted: bool,
    /// Whether the browser session is confirmed connected.
    browser_connected: bool,
    /// Whether a code execute call has been dispatched.
    execute_attempted: bool,
}

impl ColabFlowState {
    fn from_user_text(text: &str) -> Option<Self> {
        let (intent, title, code) = detect_colab_intent(text)?;
        Some(Self {
            intent,
            notebook_title: title,
            code_snippet: code,
            drive_create_attempted: false,
            drive_file_id: None,
            browser_open_attempted: false,
            browser_connected: false,
            execute_attempted: false,
        })
    }

    /// Drive-create tool call for CreateNotebook flow.
    fn drive_create_call(&self) -> ParsedToolCall {
        let title = self
            .notebook_title
            .as_deref()
            .unwrap_or("Untitled Notebook");
        // gworkspace MCP creates a Google Doc; we use the same pattern but
        // flag it as an ipynb by appending the extension in the title.
        let full_title = if title.ends_with(".ipynb") {
            title.to_string()
        } else {
            format!("{}.ipynb", title)
        };
        ParsedToolCall {
            name: "gw_drive_create".into(),
            arguments: serde_json::json!({
                "title": full_title,
                "mime_type": "application/vnd.google.colab",
            }),
        }
    }

    /// Browser-connection bootstrap call.
    fn browser_open_call() -> ParsedToolCall {
        ParsedToolCall {
            name: "mcp_colab-mcp_open_colab_browser_connection".into(),
            arguments: serde_json::json!({}),
        }
    }

    /// Execute-cell call (only for ExecuteCode intent).
    fn execute_call(&self) -> Option<ParsedToolCall> {
        let code = self.code_snippet.as_deref()?;
        Some(ParsedToolCall {
            name: "mcp_colab-mcp_execute_cell".into(),
            arguments: serde_json::json!({ "code": code }),
        })
    }

    /// Returns the next forced calls for this workflow, if any.
    fn next_required_calls(
        &self,
        allowed_tool_names: &std::collections::HashSet<String>,
    ) -> Vec<ParsedToolCall> {
        // Step 1 (CreateNotebook only): create the Drive file first.
        if self.intent == ColabIntent::CreateNotebook && !self.drive_create_attempted {
            let call = self.drive_create_call();
            if allowed_tool_names.contains(&call.name) {
                return vec![call];
            }
            // Drive tool not available — fall through to browser open.
        }

        // Step 2: open the browser connection (once Drive file exists or not needed).
        if !self.browser_open_attempted {
            let call = Self::browser_open_call();
            if allowed_tool_names.contains(&call.name) {
                return vec![call];
            }
        }

        // Step 3 (ExecuteCode only): execute after browser is confirmed connected.
        if self.intent == ColabIntent::ExecuteCode
            && self.browser_connected
            && !self.execute_attempted
        {
            if let Some(call) = self.execute_call() {
                if allowed_tool_names.contains(&call.name) {
                    return vec![call];
                }
            }
        }

        vec![]
    }

    fn observe_tool_result(
        &mut self,
        call: &ParsedToolCall,
        success: bool,
        data: &serde_json::Value,
    ) {
        match call.name.as_str() {
            "gw_drive_create" => {
                self.drive_create_attempted = true;
                if success {
                    self.drive_file_id = data
                        .get("id")
                        .or_else(|| data.get("file_id"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
            }
            n if n.contains("open_colab_browser_connection") => {
                self.browser_open_attempted = true;
                // The tool returns {result: true/false}.
                let connected = data
                    .get("result")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(success);
                self.browser_connected = connected;
            }
            n if n.contains("execute_cell") => {
                self.execute_attempted = true;
            }
            _ => {}
        }
    }

    fn status_summary(&self) -> String {
        match self.intent {
            ColabIntent::CreateNotebook => {
                if self.browser_connected {
                    format!(
                        "Notebook '{}' created on Drive and opened in Colab.",
                        self.notebook_title.as_deref().unwrap_or("Untitled")
                    )
                } else if self.drive_create_attempted {
                    format!(
                        "Notebook '{}' created on Drive. Opening Colab browser...",
                        self.notebook_title.as_deref().unwrap_or("Untitled")
                    )
                } else {
                    "Creating notebook on Google Drive...".into()
                }
            }
            ColabIntent::OpenNotebook => {
                if self.browser_connected {
                    "Colab notebook opened in browser.".into()
                } else {
                    "Opening Colab browser connection...".into()
                }
            }
            ColabIntent::ExecuteCode => {
                if self.execute_attempted {
                    "Code dispatched to Colab.".into()
                } else if self.browser_connected {
                    "Browser connected. Executing code...".into()
                } else {
                    "Connecting to Colab browser...".into()
                }
            }
            ColabIntent::Generic => {
                if self.browser_connected {
                    "Colab browser connection established.".into()
                } else {
                    "Connecting to Colab browser...".into()
                }
            }
        }
    }
}

/// Detect whether the user text is a Colab-related request and classify its intent.
/// Returns `(ColabIntent, optional_title, optional_code)` or `None` if not Colab.
fn detect_colab_intent(text: &str) -> Option<(ColabIntent, Option<String>, Option<String>)> {
    let lower = text.to_ascii_lowercase();

    let is_colab = lower.contains("colab")
        || lower.contains("google colab")
        || (lower.contains("notebook")
            && (lower.contains("python") || lower.contains("jupyter") || lower.contains("ipynb")));

    if !is_colab {
        return None;
    }

    // Create intent
    let is_create = [
        "create",
        "new",
        "make",
        "start a",
        "open a new",
        "banao",
        "bana",
    ]
    .iter()
    .any(|kw| lower.contains(kw));

    if is_create {
        // Extract notebook title if present
        let title = infer_title(text, "")
            .pipe_nonempty()
            .or_else(|| extract_notebook_title_from_text(text));
        return Some((ColabIntent::CreateNotebook, title, None));
    }

    // Execute intent
    let is_execute = [
        "run", "execute", "chalao", "chala", "print(", "import ", "code:",
    ]
    .iter()
    .any(|kw| lower.contains(kw));

    if is_execute {
        let code = extract_code_from_text(text);
        return Some((ColabIntent::ExecuteCode, None, code));
    }

    // Open intent
    let is_open = [
        "open",
        "kholo",
        "kho do",
        "launch",
        "set as active",
        "active",
    ]
    .iter()
    .any(|kw| lower.contains(kw));

    if is_open {
        return Some((ColabIntent::OpenNotebook, None, None));
    }

    // Generic Colab request
    Some((ColabIntent::Generic, None, None))
}

/// Attempt to extract a notebook title from text like "named X" or "called X".
fn extract_notebook_title_from_text(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for marker in ["named ", "called ", "name ", "title "] {
        if let Some(idx) = lower.find(marker) {
            let rest = text[idx + marker.len()..].trim();
            let title = rest
                .split(|c: char| {
                    matches!(c, ' ') && !rest[..rest.find(c).unwrap_or(0)].ends_with('.')
                })
                .next()
                .unwrap_or(rest)
                .trim_matches(|c: char| c == '"' || c == '\'' || c == '`' || c == '.')
                .trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }

    // Try quoted text
    if let Some(caps) = QUOTED_TEXT_RE.captures(text) {
        if let Some(m) = caps.get(1).or_else(|| caps.get(2)) {
            let t = m.as_str().trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }

    None
}

/// Extract inline code from a user request (for execute intent).
fn extract_code_from_text(text: &str) -> Option<String> {
    // Fenced code block
    if let Some(caps) = FENCED_CODE_BLOCK_RE.captures(text) {
        if let Some(m) = caps.get(1) {
            let code = m.as_str().trim();
            if !code.is_empty() {
                return Some(code.to_string());
            }
        }
    }

    // Backtick inline
    if let Some(caps) = QUOTED_TEXT_RE.captures(text) {
        if let Some(m) = caps.get(1).or_else(|| caps.get(2)) {
            let code = m.as_str().trim();
            if code.contains('\n') || code.contains('(') {
                return Some(code.to_string());
            }
        }
    }

    // "run: ..." or "execute: ..."
    let lower = text.to_ascii_lowercase();
    for marker in ["run:", "execute:", "code:"] {
        if let Some(idx) = lower.find(marker) {
            let rest = text[idx + marker.len()..].trim();
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }

    None
}

/// Helper: turn a `String` into `Option<String>`, returning `None` if empty.
trait PipeNonEmpty {
    fn pipe_nonempty(self) -> Option<String>;
}
impl PipeNonEmpty for String {
    fn pipe_nonempty(self) -> Option<String> {
        if self.is_empty() {
            None
        } else {
            Some(self)
        }
    }
}

#[derive(Debug, Clone)]
struct PackageFlowState {
    intent: PackageIntent,
    query: String,
    package_name: String,
    search_done: bool,
    search_found: Option<bool>,
    search_preferred_source: Option<String>,
    precheck_done: bool,
    precheck_installed: Option<bool>,
    precheck_source: Option<String>,
    action_attempted: bool,
    action_success: Option<bool>,
    postcheck_done: bool,
    postcheck_installed: Option<bool>,
}

impl PackageFlowState {
    fn from_user_text(user_text: &str) -> Option<Self> {
        let intent = detect_package_intent(user_text)?;
        let query = extract_package_query(user_text, intent)?;
        let package_name = query.split_whitespace().next()?.to_string();
        Some(Self {
            intent,
            query,
            package_name,
            search_done: false,
            search_found: None,
            search_preferred_source: None,
            precheck_done: false,
            precheck_installed: None,
            precheck_source: None,
            action_attempted: false,
            action_success: None,
            postcheck_done: false,
            postcheck_installed: None,
        })
    }

    fn action_tool_name(&self) -> &'static str {
        match self.intent {
            PackageIntent::Install => "install_package",
            PackageIntent::Uninstall => "uninstall_package",
        }
    }

    fn check_call(&self) -> ParsedToolCall {
        ParsedToolCall {
            name: "check_package_installed".into(),
            arguments: serde_json::json!({ "name": self.package_name }),
        }
    }

    fn action_call(&self) -> ParsedToolCall {
        let mut arguments = serde_json::json!({ "name": self.package_name });
        if let Some(source) = self.source_for_action() {
            arguments["source"] = serde_json::Value::String(source);
        }
        ParsedToolCall {
            name: self.action_tool_name().into(),
            arguments,
        }
    }

    fn search_call(&self) -> ParsedToolCall {
        ParsedToolCall {
            name: "search_package".into(),
            arguments: serde_json::json!({ "query": self.query }),
        }
    }

    fn should_take_action(&self) -> Option<bool> {
        match self.intent {
            PackageIntent::Install => self.precheck_installed.map(|installed| !installed),
            PackageIntent::Uninstall => self.precheck_installed,
        }
    }

    fn source_for_action(&self) -> Option<String> {
        match self.intent {
            PackageIntent::Install => self
                .search_preferred_source
                .clone()
                .or_else(|| self.precheck_source.clone()),
            PackageIntent::Uninstall => self.precheck_source.clone(),
        }
    }

    fn next_required_calls(&self) -> Vec<ParsedToolCall> {
        if matches!(self.intent, PackageIntent::Install) {
            if !self.search_done {
                return vec![self.search_call()];
            }
            // If the package was not found during search, stop forcing actions.
            if matches!(self.search_found, Some(false)) {
                return vec![];
            }
            // If search failed and we have no reliable result, avoid loops.
            if self.search_found.is_none() {
                return vec![];
            }
        }

        if !self.precheck_done {
            return vec![self.check_call()];
        }
        // If precheck failed and we have no reliable installed flag, avoid loops.
        if self.precheck_installed.is_none() {
            return vec![];
        }

        match self.intent {
            PackageIntent::Install => {
                if matches!(self.should_take_action(), Some(true)) {
                    if !self.action_attempted {
                        return vec![self.action_call()];
                    }
                    // Always re-check after an install attempt.
                    if !self.postcheck_done {
                        return vec![self.check_call()];
                    }
                }
            }
            PackageIntent::Uninstall => {
                if matches!(self.precheck_installed, Some(false)) {
                    return vec![];
                }
                if !self.action_attempted {
                    return vec![self.action_call()];
                }
                // Always re-check after each uninstall attempt.
                if !self.postcheck_done {
                    return vec![self.check_call()];
                }
                // If still installed, try uninstalling again using the latest observed source.
                if matches!(self.postcheck_installed, Some(true)) {
                    return vec![self.action_call()];
                }
            }
        }

        vec![]
    }

    fn observe_tool_result(
        &mut self,
        call: &ParsedToolCall,
        success: bool,
        data: &serde_json::Value,
    ) {
        match call.name.as_str() {
            "search_package" => {
                self.search_done = true;
                self.search_found = data
                    .get("count")
                    .and_then(|v| v.as_u64())
                    .map(|count| count > 0);
                self.search_preferred_source = data
                    .get("results")
                    .and_then(|v| v.as_array())
                    .and_then(|results| {
                        let target = self.package_name.to_lowercase();
                        results
                            .iter()
                            .find(|row| {
                                row.get("name")
                                    .and_then(|v| v.as_str())
                                    .map(|name| {
                                        let n = name.to_lowercase();
                                        n == target
                                            || n.starts_with(&(target.clone() + "-"))
                                            || n.contains(&target)
                                    })
                                    .unwrap_or(false)
                            })
                            .or_else(|| results.first())
                    })
                    .and_then(|row| row.get("source"))
                    .and_then(|v| v.as_str())
                    .and_then(normalize_package_source_for_action);
            }
            "check_package_installed" => {
                let installed = data.get("installed").and_then(|v| v.as_bool());
                let source = data
                    .get("source")
                    .and_then(|v| v.as_str())
                    .and_then(normalize_package_source_for_action);
                if !self.precheck_done {
                    self.precheck_done = true;
                    self.precheck_installed = installed;
                    self.precheck_source = source;
                } else if self.action_attempted {
                    self.postcheck_done = true;
                    self.postcheck_installed = installed;
                    self.precheck_source = source.or_else(|| self.precheck_source.clone());
                } else {
                    // A repeated pre-check still refreshes observed state.
                    self.precheck_installed = installed;
                    self.precheck_source = source.or_else(|| self.precheck_source.clone());
                }
            }
            "install_package" if matches!(self.intent, PackageIntent::Install) => {
                self.action_attempted = true;
                self.action_success = Some(success);
                self.postcheck_done = false;
                self.postcheck_installed = None;
            }
            "uninstall_package" if matches!(self.intent, PackageIntent::Uninstall) => {
                self.action_attempted = true;
                self.action_success = Some(success);
                self.postcheck_done = false;
                self.postcheck_installed = None;
            }
            _ => {}
        }
    }

    fn verified_summary(&self) -> Option<String> {
        match self.intent {
            PackageIntent::Install => {
                if matches!(self.precheck_installed, Some(true)) {
                    return Some(format!(
                        "Verified: '{}' is already installed.",
                        self.package_name
                    ));
                }
                if !self.action_attempted || !self.postcheck_done {
                    return None;
                }
                match self.postcheck_installed {
                    Some(true) => Some(format!(
                        "Verified: '{}' is installed after the install attempt.",
                        self.package_name
                    )),
                    Some(false) => Some(format!(
                        "Verification result: '{}' is still not installed after the install attempt.",
                        self.package_name
                    )),
                    None => Some(format!(
                        "Install attempt completed for '{}', but final verification could not determine installed state.",
                        self.package_name
                    )),
                }
            }
            PackageIntent::Uninstall => {
                if matches!(self.precheck_installed, Some(false)) {
                    return Some(format!(
                        "Verified: '{}' is not installed.",
                        self.package_name
                    ));
                }
                if !self.action_attempted || !self.postcheck_done {
                    return None;
                }
                match self.postcheck_installed {
                    Some(false) => Some(format!(
                        "Verified: '{}' is not installed after the uninstall attempt.",
                        self.package_name
                    )),
                    Some(true) => Some(format!(
                        "Verification result: '{}' is still installed after the uninstall attempt.",
                        self.package_name
                    )),
                    None => Some(format!(
                        "Uninstall attempt completed for '{}', but final verification could not determine installed state.",
                        self.package_name
                    )),
                }
            }
        }
    }
}

/// Events emitted during agent loop execution.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Marks the admitted turn identity for this stream.
    TurnAccepted { session_id: String, turn_id: String },
    /// Text token from the LLM.
    Token(String),
    /// Tool is being called.
    ToolStart {
        name: String,
        params: serde_json::Value,
    },
    /// Tool completed.
    ToolEnd {
        name: String,
        result: serde_json::Value,
        success: bool,
    },
    /// Mid-execution heartbeat / progress update from a long-running tool.
    /// `call_id` matches the `name` field of the surrounding `ToolStart`/`ToolEnd`.
    /// `percent` is `None` when progress is indeterminate.
    ToolProgress {
        call_id: String,
        message: String,
        percent: Option<u8>,
    },
    /// A chunk of the **full** MCP payload streamed directly to the UI.
    /// The LLM only ever sees the compact summary; the UI can render full data
    /// by reassembling these chunks.
    ToolPayloadChunk {
        call_id: String,
        seq: u32,
        is_final: bool,
        data: serde_json::Value,
    },
    /// Waiting for HITL approval.
    ApprovalRequired {
        request_id: String,
        action: String,
        risk_level: String,
        parameters: serde_json::Value,
    },
    /// Approval result.
    ApprovalResult { action: String, approved: bool },
    /// Tool choice confirmation required for low-confidence routing.
    ToolChoiceRequired {
        query: String,
        confidence: f32,
        min_confidence: f32,
        candidates: Vec<ToolChoiceCandidate>,
    },
    /// Planning step.
    Plan(String),
    /// Error.
    Error(String),
    /// Final response text.
    Done(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnExecutionMode {
    Assistant,
    PromptLab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptLabToolSelectionStrategy {
    DirectLockedTool,
    RoutedWithinLock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnExecutionProfile {
    pub mode: TurnExecutionMode,
    pub app_lock: Option<String>,
    pub tool_lock: Option<String>,
    pub prompt_lab_strategy: PromptLabToolSelectionStrategy,
}

impl TurnExecutionProfile {
    pub fn assistant() -> Self {
        Self::default()
    }

    pub fn prompt_lab(
        app_lock: Option<String>,
        tool_lock: Option<String>,
        prompt_lab_strategy: PromptLabToolSelectionStrategy,
    ) -> Self {
        Self {
            mode: TurnExecutionMode::PromptLab,
            app_lock: app_lock
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty()),
            tool_lock: tool_lock
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            prompt_lab_strategy,
        }
    }

    fn is_prompt_lab(&self) -> bool {
        matches!(self.mode, TurnExecutionMode::PromptLab)
    }

    fn uses_direct_strategy(&self) -> bool {
        self.is_prompt_lab()
            && matches!(
                self.prompt_lab_strategy,
                PromptLabToolSelectionStrategy::DirectLockedTool
            )
    }
}

impl Default for TurnExecutionProfile {
    fn default() -> Self {
        Self {
            mode: TurnExecutionMode::Assistant,
            app_lock: None,
            tool_lock: None,
            prompt_lab_strategy: PromptLabToolSelectionStrategy::RoutedWithinLock,
        }
    }
}

fn tool_matches_lab_app_lock(tool_name: &str, app_lock: &str) -> bool {
    let lower = app_lock.to_ascii_lowercase();
    let tool_name_lower = tool_name.to_ascii_lowercase();

    match lower.as_str() {
        "gmail" => tool_name_lower.starts_with("gw_gmail_"),
        "drive" => tool_name_lower.starts_with("gw_drive_"),
        "docs" => tool_name_lower.starts_with("gw_docs_"),
        "sheets" => tool_name_lower.starts_with("gw_sheets_"),
        "calendar" => tool_name_lower.starts_with("gw_calendar_"),
        "slides" => tool_name_lower.starts_with("gw_slides_"),
        "forms" => tool_name_lower.starts_with("gw_forms_"),
        "google" | "gworkspace" | "google_workspace" => tool_name_lower.starts_with("gw_"),
        "colab" | "google_colab" | "notebook" => {
            tool_name_lower.starts_with("mcp_") && tool_name_lower.contains("colab")
        }
        _ => {
            if let Some(prefix) = lower.strip_prefix("mcp_") {
                tool_name_lower.starts_with(&format!("mcp_{}", prefix))
            } else {
                false
            }
        }
    }
}

fn tool_allowed_by_execution_profile(profile: &TurnExecutionProfile, tool_name: &str) -> bool {
    if !profile.is_prompt_lab() {
        return true;
    }

    if let Some(tool_lock) = profile.tool_lock.as_deref() {
        return tool_name == tool_lock;
    }

    if let Some(app_lock) = profile.app_lock.as_deref() {
        return tool_matches_lab_app_lock(tool_name, app_lock);
    }

    true
}

/// The core ReAct agent loop.
pub struct AgentLoop {
    model_router: Arc<ModelRouter>,
    tool_registry: Arc<ToolRegistry>,
    mount_manager: Arc<tokio::sync::RwLock<ToolMountManager>>,
    policy_engine: Arc<PolicyEngine>,
    hitl_gateway: Arc<HitlGateway>,
    audit_logger: Arc<AuditLogger>,
    #[allow(dead_code)]
    rollback_mgr: Arc<RollbackManager>,
    /// Semantic router — None until initialised (falls back to regex router).
    semantic_router: Option<Arc<crate::routing::Router>>,
    /// Tool-level semantic index for direct execution fast path.
    tool_index: Option<Arc<crate::routing::tool_index::SharedToolIndex>>,
    /// Feedback collector for online learning.
    feedback_collector: Option<Arc<tokio::sync::Mutex<crate::routing::feedback::FeedbackCollector>>>,
    max_tool_rounds: usize,
    hardware_tier: String,
    min_confidence_to_act: f32,
    clarify_threshold: f32,
    /// Per-session admission gate with supersession-aware cancellation.
    turn_admission: Arc<TurnAdmission>,
    /// Top-level planning boundary (Phase 3 scaffold).
    turn_gate: Arc<TurnGate>,
}

impl AgentLoop {
    pub fn new(
        model_router: Arc<ModelRouter>,
        tool_registry: Arc<ToolRegistry>,
        mount_manager: Arc<tokio::sync::RwLock<ToolMountManager>>,
        policy_engine: Arc<PolicyEngine>,
        hitl_gateway: Arc<HitlGateway>,
        audit_logger: Arc<AuditLogger>,
        rollback_mgr: Arc<RollbackManager>,
    ) -> Self {
        Self {
            model_router,
            tool_registry,
            mount_manager,
            policy_engine,
            hitl_gateway,
            audit_logger,
            rollback_mgr,
            semantic_router: None,
            tool_index: None,
            feedback_collector: None,
            max_tool_rounds: 10,
            hardware_tier: "standard".into(),
            min_confidence_to_act: 0.55,
            clarify_threshold: 0.40,
            turn_admission: Arc::new(TurnAdmission::new()),
            turn_gate: Arc::new(TurnGate::new()),
        }
    }

    /// Attach an initialised semantic Router.
    pub fn with_semantic_router(mut self, router: Arc<crate::routing::Router>) -> Self {
        self.semantic_router = Some(router);
        self
    }

    /// Attach a tool-level semantic index for direct execution.
    pub fn with_tool_index(mut self, index: Arc<crate::routing::tool_index::SharedToolIndex>) -> Self {
        self.tool_index = Some(index);
        self
    }

    /// Attach a feedback collector for online learning.
    pub fn with_feedback_collector(
        mut self,
        collector: Arc<tokio::sync::Mutex<crate::routing::feedback::FeedbackCollector>>,
    ) -> Self {
        self.feedback_collector = Some(collector);
        self
    }

    /// Try direct tool execution via semantic tool index (Phase 3 fast path).
    /// Returns Some(tool_schema) if a high-confidence direct match is found.
    async fn try_direct_tool_match(&self, query_text: &str) -> Option<ToolSchema> {
        let tool_index = self.tool_index.as_ref()?;
        if !crate::config::RoutingConfig::default().tool_index_enabled {
            return None;
        }
        let tier = &self.hardware_tier;
        let match_result = tool_index.match_by_text(query_text, tier).await?;
        if !match_result.direct_execution {
            return None;
        }
        // Find the matching ToolSchema
        let schema = self.tool_registry
            .list_defs()
            .iter()
            .find(|def| def.name == match_result.name)
            .map(|def| ToolSchema {
                name: def.name.clone(),
                description: def.description.clone(),
                parameters: def.to_function_schema(),
            });
        schema
    }

    /// Override the maximum tool rounds for a single user turn.
    pub fn with_max_tool_rounds(mut self, max_tool_rounds: usize) -> Self {
        if max_tool_rounds > 0 {
            self.max_tool_rounds = max_tool_rounds;
        }
        self
    }

    /// Set the hardware tier used for tool visibility and execution gating.
    pub fn with_hardware_tier(mut self, hardware_tier: impl Into<String>) -> Self {
        let tier = hardware_tier.into();
        if !tier.trim().is_empty() {
            self.hardware_tier = tier;
        }
        self
    }

    /// Configure confidence thresholds for autonomous intent fallback.
    pub fn with_confidence_thresholds(
        mut self,
        min_confidence_to_act: f32,
        clarify_threshold: f32,
    ) -> Self {
        if (0.0..=1.0).contains(&min_confidence_to_act) {
            self.min_confidence_to_act = min_confidence_to_act;
        }
        if (0.0..=1.0).contains(&clarify_threshold) {
            self.clarify_threshold = clarify_threshold;
        }
        self
    }

    /// Cancel all in-flight work for `session_id`.
    ///
    /// Safe to call from any thread/task.  If no turn is active for the session
    /// this is a no-op.
    pub fn cancel_session(&self, session_id: &str) {
        self.turn_admission.cancel_session(session_id);
    }

    /// Return the local LLM backend used for semantic memory parsing.
    pub fn memory_parser_backend(&self) -> Option<Arc<dyn crate::llm::LlmBackend>> {
        self.model_router.get_local()
    }

    /// Fast stale-turn invalidation check for async callbacks.
    pub fn is_turn_active(&self, session_id: &str, turn_id: &str) -> bool {
        self.turn_admission.is_active(session_id, turn_id)
    }

    /// Returns a clone of the HITL gateway so that remote transports (e.g.
    /// Telegram) can resolve pending approval requests without direct access
    /// to `AgentLoop` internals.
    pub fn hitl_gateway(&self) -> Arc<HitlGateway> {
        Arc::clone(&self.hitl_gateway)
    }

    /// Best-effort pre-flight cap for visual tokens before `analyze_image`.
    async fn compute_visual_token_cap(&self) -> VisualTokenCapDecision {
        let mut safety_margin_mb = 512u64;
        let mut profile = crate::config::ModelProfile::default();
        let mut current_ngl = 0u32;
        let mut vision_enabled = self.model_router.has_vision();

        if let Some(mgr) = self.model_router.orchestrator_server_manager() {
            safety_margin_mb = mgr.safety_margin_mb();
            profile = mgr.model_profile();
            let (ngl, _ctx) = mgr.current_params();
            current_ngl = ngl;
            vision_enabled = mgr.current_vision_enabled();
        }

        let vision_mode = match (vision_enabled, current_ngl) {
            (false, _) => VisionMode::Disabled,
            (true, 0) => VisionMode::CpuVision,
            (true, ngl) if ngl < profile.vision_min_ngl => VisionMode::ReducedGpu,
            (true, _) => VisionMode::FullGpu,
        };

        if !vision_mode.has_vision() {
            return VisualTokenCapDecision {
                hard_cap: 0,
                safe_cap: 0,
                free_vram_mb: 0,
                safety_margin_mb,
                vision_mode,
            };
        }

        let free_vram_mb = crate::platform::vram::build_profiler()
            .snapshot()
            .await
            .free_mb;
        let safe_cap = calculate_safe_visual_tokens(
            free_vram_mb,
            safety_margin_mb,
            &profile,
            0, // Conservative fallback until live KV occupancy is exposed.
        );

        let mode_cap = match vision_mode.max_image_dimension() {
            0 => u32::MAX, // full-resolution mode
            dim => estimate_visual_tokens(dim, dim, 14),
        };

        let cap = if safe_cap == 0 {
            // If telemetry is unavailable, fall back to the mode cap.
            if mode_cap == u32::MAX {
                4096
            } else {
                mode_cap
            }
        } else if mode_cap == u32::MAX {
            safe_cap
        } else {
            safe_cap.min(mode_cap)
        };

        VisualTokenCapDecision {
            hard_cap: cap.max(64),
            safe_cap,
            free_vram_mb,
            safety_margin_mb,
            vision_mode,
        }
    }

    /// Run the agent loop for a single user turn.
    /// Returns a channel of StreamEvents.
    pub async fn run(
        &self,
        session_id: &str,
        messages: &mut Vec<ChatMessage>,
        event_tx: mpsc::UnboundedSender<StreamEvent>,
    ) {
        self.run_with_profile(session_id, messages, event_tx, None)
            .await;
    }

    /// Run the agent loop for a single user turn with an optional execution profile.
    pub async fn run_with_profile(
        &self,
        session_id: &str,
        messages: &mut Vec<ChatMessage>,
        event_tx: mpsc::UnboundedSender<StreamEvent>,
        execution_profile: Option<TurnExecutionProfile>,
    ) {
        let execution_profile = execution_profile.unwrap_or_default();

        let last_user_text = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.clone())
            .unwrap_or_default();
        let explicit_queue_requested = user_requested_explicit_queue(&last_user_text);

        // ── Per-turn admission + cancellation tree ─────────────────────────────
        let turn_id = Uuid::new_v4().to_string();
        let turn_tree = match self.turn_admission.admit_or_enqueue_turn(
            session_id.to_string(),
            turn_id.clone(),
            explicit_queue_requested,
        ) {
            Ok(TurnAdmissionDecision::Admitted(cancellation)) => cancellation,
            Ok(TurnAdmissionDecision::Queued { depth }) => {
                let _ = event_tx.send(StreamEvent::Plan(format!(
                    "Current turn is busy. Queued this request at position {depth}."
                )));

                match self
                    .turn_admission
                    .wait_for_turn_activation(session_id, &turn_id)
                    .await
                {
                    Some(cancellation) => {
                        let _ = event_tx
                            .send(StreamEvent::Plan("Starting queued turn now.".to_string()));
                        cancellation
                    }
                    None => {
                        let canceled_msg =
                            "Queued request was canceled before execution.".to_string();
                        let _ = event_tx.send(StreamEvent::Error(canceled_msg.clone()));
                        let _ = event_tx.send(StreamEvent::Done(canceled_msg));
                        return;
                    }
                }
            }
            Err(TurnAdmissionError::QueueFull { limit, .. }) => {
                let queue_full_msg = format!(
                    "A turn is already running and the queue is full (limit {limit}). Please wait and try again."
                );
                let _ = event_tx.send(StreamEvent::Error(queue_full_msg.clone()));
                let _ = event_tx.send(StreamEvent::Done(queue_full_msg));
                return;
            }
        };
        let turn_tools_cancel = turn_tree.tools.clone();
        let turn_sidecar_cancel = turn_tree.sidecar.clone();
        let turn_mcp_cancel = turn_tree.mcp.clone();
        let turn_image_cancel = turn_tree.image.clone();
        let turn_id_for_checks = turn_id.clone();
        let turn_admission_for_async = Arc::clone(&self.turn_admission);
        let session_id_for_async = session_id.to_string();
        let turn_id_for_async = turn_id_for_checks.clone();

        // Guard: clear this turn only if it is still active on function exit.
        struct TurnGuard {
            admission: Arc<TurnAdmission>,
            session_id: String,
            turn_id: String,
        }
        impl Drop for TurnGuard {
            fn drop(&mut self) {
                self.admission
                    .complete_turn(&self.session_id, &self.turn_id);
            }
        }
        let _turn_guard = TurnGuard {
            admission: Arc::clone(&self.turn_admission),
            session_id: session_id.to_string(),
            turn_id,
        };

        let is_turn_active = || {
            self.turn_admission
                .is_active(session_id, &turn_id_for_checks)
        };
        let return_if_stale = || {
            if is_turn_active() {
                false
            } else {
                log_pipeline_step(
                    session_id,
                    "stale_turn_dropped",
                    "Turn became stale; dropping in-flight result",
                    Some(serde_json::json!({
                        "turn_id": turn_id_for_checks,
                    })),
                );
                true
            }
        };

        let _ = event_tx.send(StreamEvent::TurnAccepted {
            session_id: session_id.to_string(),
            turn_id: turn_id_for_checks.clone(),
        });

        // ── Per-turn error-loop guards ─────────────────────────────────────────
        // Maps call_dedup_hash(tool, args) -> (failure_count, last_error_msg).
        let mut failed_calls: HashMap<u64, (u8, String)> = HashMap::new();
        // Count of *consecutive* tool failures this turn (reset on any success).
        let mut consecutive_failures: u8 = 0;
        const MAX_CONSECUTIVE_FAILURES: u8 = 3;

        // ── Per-turn token budget tracker ─────────────────────────────────────
        // Approximate cumulative tokens consumed by all tool outputs this turn.
        let mut turn_tool_tokens: usize = 0;

        // Check if the user message contains images and route accordingly
        let has_images = messages.last().is_some_and(|m| m.has_images());
        let mut routing_focus_text = routing_focus_text_from_user_content(&last_user_text);
        if execution_profile.uses_direct_strategy() {
            if let Some(tool_lock) = execution_profile.tool_lock.as_deref() {
                if extract_forced_tool_directive(&routing_focus_text).is_none() {
                    routing_focus_text = format!("#tool:{} {}", tool_lock, routing_focus_text);
                }
            }
        }
        let routing_focus_lower = routing_focus_text.to_lowercase();
        let mut turn_gate_plan = self.turn_gate.plan_turn(&last_user_text, has_images);
        let pure_image_analysis_turn =
            has_images && matches!(turn_gate_plan.intent.operation, Operation::AnalyzeImage);
        let wants_vision_backend =
            has_images && matches!(turn_gate_plan.resource_plan, ResourcePlan::L1Vision { .. });
        let reflex_cancel_turn = matches!(turn_gate_plan.intent.operation, Operation::Cancel)
            && matches!(turn_gate_plan.resource_plan, ResourcePlan::ReflexRust);
        let mut inline_images_allowed_for_turn = true;
        let mut inline_image_vision_mode = VisionMode::FullGpu;
        if has_images {
            let cap_probe = self.compute_visual_token_cap().await;
            inline_image_vision_mode = cap_probe.vision_mode;
            inline_images_allowed_for_turn = cap_probe.vision_mode.has_vision();
        }

        log_pipeline_step(
            session_id,
            "prompt_entered",
            "Agent loop received prompt",
            Some(serde_json::json!({
                "has_images": has_images,
                "pure_image_analysis_turn": pure_image_analysis_turn,
                "wants_vision_backend": wants_vision_backend,
                "reflex_cancel_turn": reflex_cancel_turn,
                "prompt_lab_mode": execution_profile.is_prompt_lab(),
                "prompt_lab_strategy": format!("{:?}", execution_profile.prompt_lab_strategy),
                "app_lock": execution_profile.app_lock.clone(),
                "tool_lock": execution_profile.tool_lock.clone(),
                "turn_gate": {
                    "modality": format!("{:?}", turn_gate_plan.intent.modality),
                    "operation": format!("{:?}", turn_gate_plan.intent.operation),
                    "hazard_hint": format!("{:?}", turn_gate_plan.intent.hazard_hint),
                    "compute": format!("{:?}", turn_gate_plan.intent.compute),
                    "source": format!("{:?}", turn_gate_plan.intent.source),
                    "confidence": turn_gate_plan.intent.confidence,
                    "resource_plan": format!("{:?}", turn_gate_plan.resource_plan),
                },
                "message_count": messages.len(),
                "prompt_preview": sanitize_text_for_logs(&routing_focus_text, 260),
            })),
        );

        if reflex_cancel_turn {
            let final_text = "Stopped current operation.";
            log_pipeline_step(
                session_id,
                "turn_gate_reflex_short_circuit",
                "TurnGate resolved a reflex cancel turn; skipping backend and tool routing",
                Some(serde_json::json!({
                    "turn_gate": {
                        "operation": format!("{:?}", turn_gate_plan.intent.operation),
                        "compute": format!("{:?}", turn_gate_plan.intent.compute),
                    },
                    "final_text": final_text,
                })),
            );
            let _ = event_tx.send(StreamEvent::Plan(
                "Stopping current operation immediately.".into(),
            ));
            let _ = event_tx.send(StreamEvent::Done(final_text.into()));
            return;
        }

        let backend = if wants_vision_backend {
            if inline_images_allowed_for_turn {
                match self.model_router.route_vision().await {
                    Some(b) => b,
                    None => {
                        log_pipeline_step(
                            session_id,
                            "backend_unavailable",
                            "No vision backend available despite enabled VisionMode; falling back to chat backend",
                            Some(serde_json::json!({
                                "requested": "vision",
                                "fallback": "chat_backend_inline_images_preserved",
                                "vision_mode": inline_image_vision_mode.as_str(),
                            })),
                        );
                        match self.model_router.route("chat").await {
                            Some(b) => b,
                            None => {
                                let _ = event_tx
                                    .send(StreamEvent::Error("no LLM backend available".into()));
                                return;
                            }
                        }
                    }
                }
            } else {
                log_pipeline_step(
                    session_id,
                    "vision_mode_disabled",
                    "VisionMode is disabled for this runtime; stripping inline images for LLM rounds",
                    Some(serde_json::json!({
                        "vision_mode": inline_image_vision_mode.as_str(),
                    })),
                );
                match self.model_router.route("chat").await {
                    Some(b) => b,
                    None => {
                        let _ =
                            event_tx.send(StreamEvent::Error("no LLM backend available".into()));
                        return;
                    }
                }
            }
        } else {
            match self.model_router.route("chat").await {
                Some(b) => b,
                None => {
                    log_pipeline_step(
                        session_id,
                        "backend_unavailable",
                        "No chat backend available",
                        Some(serde_json::json!({ "requested": "chat" })),
                    );
                    let _ = event_tx.send(StreamEvent::Error("no LLM backend available".into()));
                    return;
                }
            }
        };

        log_pipeline_step(
            session_id,
            "backend_selected",
            "Model backend selected",
            Some(serde_json::json!({
                "model_label": backend.model_label(),
                "capabilities": backend.capabilities(),
            })),
        );

        // Auto-mount tool groups based on user message keywords
        let mut meet_fallback_metadata: Option<serde_json::Value> = None;
        if pure_image_analysis_turn {
            log_pipeline_step(
                session_id,
                "preprocessing_skipped",
                "Skipped keyword auto-mount for pure image analysis turn",
                None,
            );
        } else if let Some(last_msg) = messages.last() {
            if last_msg.role == "user" {
                let mount_probe_text = routing_focus_text_from_user_content(&last_msg.content);
                meet_fallback_metadata = google_meet_fallback_metadata(&mount_probe_text);
                let mut mm = self.mount_manager.write().await;
                let newly = mm.auto_mount_from_message(&mount_probe_text);
                if !newly.is_empty() {
                    tracing::info!(groups = ?newly, "auto-mounted tool groups from user message");
                    log_pipeline_step(
                        session_id,
                        "preprocessing_applied",
                        "Tool auto-mount preprocessing applied",
                        Some(serde_json::json!({ "mounted_groups": newly })),
                    );
                } else {
                    log_pipeline_step(
                        session_id,
                        "preprocessing_skipped",
                        "No tool auto-mount preprocessing needed",
                        None,
                    );
                }
            }
        }

        if let Some(metadata) = meet_fallback_metadata {
            let metadata_json =
                serde_json::to_string_pretty(&metadata).unwrap_or_else(|_| metadata.to_string());
            messages.push(ChatMessage {
                role: "system".into(),
                content: format!(
                    "Google Meet fallback metadata:\n{}\nTool selection rule: when the user requests Google Meet/video-call scheduling, use Calendar conference-link mode with gw_calendar_create (and gw_calendar_search for availability checks).",
                    metadata_json
                ),
                name: None,
                images: None,
            });

            let _ = event_tx.send(StreamEvent::Plan(
                "Applying Google Meet fallback via Calendar conference-link mode metadata".into(),
            ));

            log_pipeline_step(
                session_id,
                "preprocessing_applied",
                "Google Meet fallback metadata injected",
                Some(serde_json::json!({
                    "metadata": sanitize_json_for_logs(&metadata, 220, 8),
                })),
            );
        }
        let google_workspace_intent =
            !pure_image_analysis_turn && looks_like_google_workspace_request(&routing_focus_lower);

        // ── Colab workflow: inject tool-routing guidance into context ──────────
        // This tells the LLM exactly which tools map to each Colab sub-task so
        // it never hallucinates a "colab create" verb.
        if !pure_image_analysis_turn && looks_like_colab_request(&routing_focus_lower) {
            let colab_guidance = concat!(
                "TOOL ROUTING RULES for Google Colab requests:\n",
                "1. CREATE a new Colab notebook → call `gw_drive_create` with mime_type=\"application/vnd.google.colab\", then call `mcp_colab-mcp_open_colab_browser_connection`.\n",
                "2. OPEN an existing Colab notebook / set active → call `mcp_colab-mcp_open_colab_browser_connection` (this opens the Colab tab in the browser).\n",
                "3. RUN / EXECUTE code in Colab → first ensure browser is connected via `mcp_colab-mcp_open_colab_browser_connection`, then call `mcp_colab-mcp_execute_cell` with the code.\n",
                "NEVER output plain text like 'colab create ...' — always emit a structured tool call JSON.",
            );
            messages.push(ChatMessage {
                role: "system".into(),
                content: colab_guidance.to_string(),
                name: None,
                images: None,
            });
            log_pipeline_step(
                session_id,
                "preprocessing_applied",
                "Colab tool-routing guidance injected",
                None,
            );
        }

        // Build tool schemas for the LLM (filtered by mount manager)
        let mount_mgr = self.mount_manager.read().await;
        let tool_defs = self.tool_registry.list_for_tier(&self.hardware_tier);
        let mut tool_schemas: Vec<ToolSchema> = tool_defs
            .iter()
            .filter(|d| mount_mgr.is_mounted(&d.name))
            .filter(|d| {
                if pure_image_analysis_turn {
                    is_tool_allowed_for_image_focus(d)
                } else {
                    true
                }
            })
            .filter(|d| {
                if d.name.starts_with("mcp_gworkspace_") {
                    google_workspace_intent
                } else {
                    true
                }
            })
            .filter(|d| tool_allowed_by_execution_profile(&execution_profile, &d.name))
            .map(|d| ToolSchema {
                name: d.name.clone(),
                description: d.description.clone(),
                parameters: d.to_function_schema()["function"]["parameters"].clone(),
            })
            .collect();
        tool_schemas.sort_by(|a, b| a.name.cmp(&b.name));
        let allowed_tool_names: HashSet<String> =
            tool_schemas.iter().map(|s| s.name.clone()).collect();
        drop(mount_mgr);

        let prompt_lab_direct_mode = execution_profile.uses_direct_strategy();

        log_pipeline_step(
            session_id,
            "tool_schemas_built",
            "Prepared mounted tool schemas for LLM",
            Some(serde_json::json!({
                "google_workspace_intent": google_workspace_intent,
                "pure_image_analysis_turn": pure_image_analysis_turn,
                "prompt_lab_mode": execution_profile.is_prompt_lab(),
                "prompt_lab_direct_mode": prompt_lab_direct_mode,
                "tool_count": tool_schemas.len(),
                "tool_names": tool_schemas
                    .iter()
                    .map(|schema| schema.name.clone())
                    .collect::<Vec<_>>(),
            })),
        );

        // Track tools already approved in this user-turn to avoid re-asking.
        // Key: "tool_name|args_json"
        let mut approved_this_turn: HashSet<String> = HashSet::new();
        let mut package_flow = PackageFlowState::from_user_text(&routing_focus_text);
        let mut colab_flow = ColabFlowState::from_user_text(&routing_focus_text);
        let mut intent_fallback_used = false;
        let mut had_successful_gmail_tool = false;
        let mut had_failed_gmail_tool = false;
        let mut last_successful_gmail_result: Option<serde_json::Value> = None;
        let mut last_successful_image_result: Option<serde_json::Value> = None;
        let forced_tool_directive = extract_forced_tool_directive(&routing_focus_text);
        let forced_tool_requested = forced_tool_directive.is_some();
        let forced_tool_name = forced_tool_directive.as_ref().map(|(name, _)| name.clone());
        let initial_turn_gate_tool_hint = self
            .turn_gate
            .direct_tool_hint(&turn_gate_plan, &allowed_tool_names);
        let mut turn_modality = if let Some(router) = &self.semantic_router {
            let ctx = self.turn_gate.context();
            let (_, modality, _) = router.route_with_context(&routing_focus_text, ctx).await;
            modality
        } else {
            crate::routing::verbs::classify_modality(&routing_focus_text)
        };
        let base_system_prompt_template = messages
            .first()
            .filter(|m| m.role.eq_ignore_ascii_case("system"))
            .map(|m| m.content.clone());

        log_pipeline_step(
            session_id,
            "intent_classified",
            "Intent classification complete",
            Some(serde_json::json!({
                "turn_gate_operation": format!("{:?}", turn_gate_plan.intent.operation),
                "turn_gate_source": format!("{:?}", turn_gate_plan.intent.source),
                "turn_gate_tool_hint": initial_turn_gate_tool_hint,
                "confidence": turn_gate_plan.intent.confidence,
                "forced_tool_requested": forced_tool_requested,
                "package_flow_detected": package_flow.is_some(),
                "colab_flow_detected": colab_flow.is_some(),
            })),
        );

        for round in 0..self.max_tool_rounds {
            if return_if_stale() {
                return;
            }

            let round_user_text = messages
                .iter()
                .rev()
                .find(|m| m.role == "user")
                .map(|m| m.content.as_str())
                .unwrap_or(routing_focus_text.as_str());
            let round_focus_text = routing_focus_text_from_user_content(round_user_text);
            let mut routed_tool_names: HashSet<String> = HashSet::new();
            let mut conversation_only_route = false;

            if let Some(router) = &self.semantic_router {
                let ctx = self.turn_gate.context();
                let (decision, modality, trace) = router.route_with_context(&round_focus_text, ctx).await;
                turn_modality = modality;
                conversation_only_route =
                    matches!(decision, crate::routing::RouteDecision::Conversation);
                routed_tool_names.extend(trace.selected_tools.into_iter());
            }
            let round_direct_tool_hint = self
                .turn_gate
                .direct_tool_hint(&turn_gate_plan, &allowed_tool_names);
            let fallback_tool_names = fallback_routed_tool_candidates(
                &round_focus_text,
                round_direct_tool_hint.as_deref(),
                &allowed_tool_names,
            );

            let round_tool_schemas = if pure_image_analysis_turn {
                Vec::new()
            } else {
                // Phase 3: Try direct tool match first (skip LLM)
                if let Some(direct_schema) = self.try_direct_tool_match(&round_focus_text).await {
                    tracing::info!(
                        tool = %direct_schema.name,
                        "Direct tool match via semantic index — skipping LLM"
                    );
                    vec![direct_schema]
                } else {
                    select_routed_tool_schemas(
                        &tool_schemas,
                        &round_focus_text,
                        round_direct_tool_hint.as_deref(),
                        &routed_tool_names,
                        &fallback_tool_names,
                        forced_tool_name.as_deref(),
                        execution_profile.tool_lock.as_deref(),
                        conversation_only_route,
                    )
                }
            };

            if let Some(template) = base_system_prompt_template.as_ref() {
                if let Some(system_msg) = messages
                    .first_mut()
                    .filter(|m| m.role.eq_ignore_ascii_case("system"))
                {
                    system_msg.content =
                        rewrite_system_prompt_tools_block(template, &round_tool_schemas);
                } else {
                    messages.insert(
                        0,
                        ChatMessage {
                            role: "system".into(),
                            content: rewrite_system_prompt_tools_block(
                                template,
                                &round_tool_schemas,
                            ),
                            name: None,
                            images: None,
                        },
                    );
                }
            }

            let total_chars_before_compaction: usize =
                messages.iter().map(|m| m.content.chars().count()).sum();
            compact_messages_for_chat(messages);
            let total_chars_after_compaction: usize =
                messages.iter().map(|m| m.content.chars().count()).sum();
            if total_chars_after_compaction < total_chars_before_compaction {
                log_pipeline_step(
                    session_id,
                    "llm_context_compacted",
                    "Compacted message history to fit context budget",
                    Some(serde_json::json!({
                        "round": round,
                        "before_chars": total_chars_before_compaction,
                        "after_chars": total_chars_after_compaction,
                        "message_count": messages.len(),
                    })),
                );
            }

            let llm_tool_schemas: Option<&[ToolSchema]> = if pure_image_analysis_turn {
                None
            } else {
                Some(round_tool_schemas.as_slice())
            };

            let mut llm_messages = messages.clone();
            let should_strip_images_for_round = has_images && !inline_images_allowed_for_turn;
            if should_strip_images_for_round {
                for message in &mut llm_messages {
                    if message.has_images() {
                        message.images = None;
                    }
                }
            }
            let round_has_images = llm_messages.iter().any(|message| message.has_images());

            log_pipeline_step(
                session_id,
                "llm_input_prepared",
                "Prepared LLM request payload",
                Some(serde_json::json!({
                    "round": round,
                    "tool_schema_count": llm_tool_schemas.map(|schemas| schemas.len()).unwrap_or(0),
                    "routed_tool_count": routed_tool_names.len(),
                    "fallback_tool_count": fallback_tool_names.len(),
                    "direct_hint_tool": round_direct_tool_hint,
                    "images_stripped_for_round": should_strip_images_for_round,
                    "history_message_count": messages.len(),
                    "messages_preview": build_message_preview(&llm_messages, 6),
                })),
            );

            // Call LLM
            let response = match backend
                .chat(&llm_messages, llm_tool_schemas, 0.7, 4096)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let error_text = e.to_string();
                    if round_has_images && looks_like_vision_unavailable_error(&error_text) {
                        log_pipeline_step(
                            session_id,
                            "vision_runtime_fallback",
                            "Vision runtime unavailable; keeping VisionMode-driven inline-image policy (no blind stripping)",
                            Some(serde_json::json!({
                                "round": round,
                                "vision_mode": inline_image_vision_mode.as_str(),
                                "error": sanitize_text_for_logs(&error_text, 260),
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Plan(
                            "Vision runtime is temporarily unavailable; using OCR/image tools fallback."
                                .into(),
                        ));
                        LlmResponse {
                            content: String::new(),
                            model: backend.model_label().to_string(),
                            usage: None,
                            tool_calls: None,
                        }
                    } else {
                        log_pipeline_step(
                            session_id,
                            "llm_error",
                            "LLM call failed",
                            Some(serde_json::json!({
                                "round": round,
                                "error": sanitize_text_for_logs(&error_text, 260),
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Error(format!("LLM error: {e}")));
                        return;
                    }
                }
            };

            if return_if_stale() {
                return;
            }

            log_pipeline_step(
                session_id,
                "llm_response_received",
                "LLM response received",
                Some(serde_json::json!({
                    "round": round,
                    "model": response.model.clone(),
                    "usage": response.usage.as_ref().map(|u| serde_json::json!({
                        "prompt_tokens": u.prompt_tokens,
                        "completion_tokens": u.completion_tokens,
                        "total_tokens": u.total_tokens,
                    })),
                    "native_tool_calls": response
                        .tool_calls
                        .as_ref()
                        .map(|v| v.len())
                        .unwrap_or(0),
                    "content_preview": sanitize_text_for_logs(&response.content, 320),
                })),
            );

            // Parse tool calls from response — prefer native function-calling format
            // (returned by llama.cpp / OpenAI), fall back to text-embedded format.
            // Pattern 7 (Python-style fallback) fires last, only for single-required-param tools.
            let parse_mode = if response.tool_calls.is_some() {
                "native_function_call"
            } else {
                "text_pattern_fallback"
            };

            let mut tool_calls: Vec<ParsedToolCall> = if let Some(native) = &response.tool_calls {
                native
                    .iter()
                    .filter_map(|tc| {
                        let name = tc["function"]["name"].as_str()?.to_string();
                        let arguments: serde_json::Value = tc["function"]["arguments"]
                            .as_str()
                            .and_then(|s| serde_json::from_str(s).ok())
                            .unwrap_or_else(|| tc["function"]["arguments"].clone());
                        Some(ParsedToolCall { name, arguments })
                    })
                    .collect()
            } else {
                // Build the single-required-param lookup for Pattern 7
                let single_param_tools: Vec<(String, String)> = self
                    .tool_registry
                    .list_defs()
                    .into_iter()
                    .filter_map(|d| {
                        let required: Vec<_> = d.parameters.iter().filter(|p| p.required).collect();
                        if required.len() == 1 {
                            Some((d.name.clone(), required[0].name.clone()))
                        } else {
                            None
                        }
                    })
                    .collect();
                let known: Vec<(&str, &str)> = single_param_tools
                    .iter()
                    .map(|(n, p)| (n.as_str(), p.as_str()))
                    .collect();
                parse_tool_calls_with_known(&response.content, &known)
            };
            let text_response_raw = extract_text_response(&response.content);
            let text_response = sanitize_assistant_text_response(&text_response_raw);

            log_pipeline_step(
                session_id,
                "tool_calls_parsed",
                "Parsed tool calls from LLM response",
                Some(serde_json::json!({
                    "round": round,
                    "parse_mode": parse_mode,
                    "tool_call_count": tool_calls.len(),
                    "tool_calls": build_tool_calls_preview(&tool_calls),
                    "text_response_preview": sanitize_text_for_logs(&text_response, 320),
                })),
            );

            let mut synthetic_package_calls = false;
            let mut synthetic_colab_calls = false;
            let mut synthetic_intent_calls = false;
            if tool_calls.is_empty() {
                if let Some(flow) = package_flow.as_ref() {
                    let fallback_calls = flow.next_required_calls();
                    if !fallback_calls.is_empty() {
                        synthetic_package_calls = true;
                        tool_calls = fallback_calls;
                        log_pipeline_step(
                            session_id,
                            "synthetic_package_calls",
                            "Injected package workflow tool calls",
                            Some(serde_json::json!({
                                "round": round,
                                "tool_calls": build_tool_calls_preview(&tool_calls),
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Plan(
                            "Enforcing package workflow with pre/post verification".into(),
                        ));
                    }
                }
            }

            // Colab workflow: inject next required Colab step if LLM produced no calls.
            if tool_calls.is_empty() {
                if let Some(flow) = colab_flow.as_ref() {
                    let colab_calls = flow.next_required_calls(&allowed_tool_names);
                    if !colab_calls.is_empty() {
                        synthetic_colab_calls = true;
                        let status = flow.status_summary();
                        tool_calls = colab_calls;
                        log_pipeline_step(
                            session_id,
                            "synthetic_colab_calls",
                            "Injected Colab workflow tool calls",
                            Some(serde_json::json!({
                                "round": round,
                                "tool_calls": build_tool_calls_preview(&tool_calls),
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Plan(status));
                    }
                }
            }

            if tool_calls.is_empty() && !intent_fallback_used {
                let intent_fallback_query =
                    resolve_intent_fallback_query(&routing_focus_text, messages);
                let fallback_plan = self.turn_gate.plan_turn(&intent_fallback_query, has_images);
                let fallback_confidence = fallback_plan
                    .intent
                    .confidence
                    .max(turn_gate_plan.intent.confidence);
                let fallback_calls: Vec<ParsedToolCall> = self
                    .turn_gate
                    .fallback_tool_hints(&fallback_plan, &allowed_tool_names)
                    .into_iter()
                    .filter_map(|hint| {
                        build_fallback_call_for_hint(
                            &hint,
                            &intent_fallback_query,
                            &allowed_tool_names,
                        )
                    })
                    .collect();

                if !fallback_calls.is_empty() {
                    if forced_tool_requested || fallback_confidence >= self.min_confidence_to_act {
                        intent_fallback_used = true;
                        synthetic_intent_calls = true;
                        turn_gate_plan = fallback_plan;
                        let names: Vec<&str> =
                            fallback_calls.iter().map(|c| c.name.as_str()).collect();
                        let plan_message = if intent_fallback_query == routing_focus_text {
                            format!(
                                "No tool call returned; applying turn_gate fallback via {}",
                                names.join(", "),
                            )
                        } else {
                            format!(
                                "No tool call returned; applying context-aware turn_gate fallback via {}",
                                names.join(", "),
                            )
                        };
                        let _ = event_tx.send(StreamEvent::Plan(plan_message));
                        tool_calls = fallback_calls;
                        log_pipeline_step(
                            session_id,
                            "synthetic_intent_call",
                            "Injected intent fallback tool call",
                            Some(serde_json::json!({
                                "round": round,
                                "fallback_query": sanitize_text_for_logs(&intent_fallback_query, 220),
                                "source": "turn_gate",
                                "confidence": fallback_confidence,
                                "tool_calls": build_tool_calls_preview(&tool_calls),
                            })),
                        );
                    } else if fallback_confidence >= self.clarify_threshold {
                        let fallback_primary_hint = self
                            .turn_gate
                            .direct_tool_hint(&fallback_plan, &allowed_tool_names);
                        let candidates = build_tool_choice_candidates(
                            &intent_fallback_query,
                            &allowed_tool_names,
                            fallback_primary_hint.as_deref(),
                            fallback_confidence,
                        );

                        if !candidates.is_empty() {
                            log_pipeline_step(
                                session_id,
                                "tool_choice_required",
                                "Low-confidence route needs user tool choice",
                                Some(serde_json::json!({
                                    "round": round,
                                    "fallback_query": sanitize_text_for_logs(&intent_fallback_query, 220),
                                    "confidence": fallback_confidence,
                                    "candidate_count": candidates.len(),
                                })),
                            );
                            let _ = event_tx.send(StreamEvent::ToolChoiceRequired {
                                query: intent_fallback_query.clone(),
                                confidence: fallback_confidence,
                                min_confidence: self.min_confidence_to_act,
                                candidates,
                            });
                            let _ = event_tx.send(StreamEvent::Done(
                                "Please choose a tool so I can continue this request.".into(),
                            ));
                            return;
                        }
                    }
                }
            }

            // If no tool calls, we're done
            if tool_calls.is_empty() {
                if return_if_stale() {
                    return;
                }

                log_pipeline_step(
                    session_id,
                    "no_tool_calls",
                    "No tool calls returned for this round",
                    Some(serde_json::json!({
                        "round": round,
                        "synthetic_package_calls": synthetic_package_calls,
                        "synthetic_colab_calls": synthetic_colab_calls,
                        "synthetic_intent_calls": synthetic_intent_calls,
                    })),
                );

                if let Some(flow) = package_flow.as_ref() {
                    if let Some(summary) = flow.verified_summary() {
                        log_pipeline_step(
                            session_id,
                            "final_output_ready",
                            "Using package-flow verification summary",
                            Some(serde_json::json!({
                                "round": round,
                                "final_preview": sanitize_text_for_logs(&summary, 260),
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Token(summary.clone()));
                        let _ = event_tx.send(StreamEvent::Done(summary));
                        return;
                    }
                }
                let mut final_text = if had_successful_gmail_tool && !had_failed_gmail_tool {
                    strip_spurious_gmail_error_lines(&text_response)
                } else {
                    text_response.clone()
                };

                log_pipeline_step(
                    session_id,
                    "final_formatting_started",
                    "Preparing final assistant output",
                    Some(serde_json::json!({
                        "round": round,
                        "had_successful_gmail_tool": had_successful_gmail_tool,
                        "had_failed_gmail_tool": had_failed_gmail_tool,
                        "text_preview": sanitize_text_for_logs(&final_text, 280),
                    })),
                );

                if had_successful_gmail_tool && !had_failed_gmail_tool && !final_text.is_empty() {
                    let has_placeholder_scaffold = contains_gmail_placeholder_scaffold(&final_text);
                    let has_raw_payload = looks_like_raw_gmail_payload_json(final_text.trim());
                    let has_duplicate_rows = contains_duplicate_gmail_rows(&final_text);
                    let should_force_grounded =
                        has_placeholder_scaffold || has_raw_payload || has_duplicate_rows;

                    if should_force_grounded {
                        if let Some(grounded_summary) = last_successful_gmail_result
                            .as_ref()
                            .and_then(build_grounded_gmail_message_list_summary)
                        {
                            tracing::warn!(
                                has_images,
                                round,
                                has_placeholder_scaffold,
                                has_raw_payload,
                                has_duplicate_rows,
                                "LLM returned non-grounded Gmail response; replacing with grounded summary"
                            );
                            log_pipeline_step(
                                session_id,
                                "final_formatting_adjusted",
                                "Replaced non-grounded Gmail output with grounded summary",
                                Some(serde_json::json!({
                                    "round": round,
                                    "has_placeholder_scaffold": has_placeholder_scaffold,
                                    "has_raw_payload": has_raw_payload,
                                    "has_duplicate_rows": has_duplicate_rows,
                                })),
                            );
                            final_text = grounded_summary;
                        }
                    }
                }

                if !final_text.is_empty() {
                    log_pipeline_step(
                        session_id,
                        "final_output_ready",
                        "Final assistant response ready",
                        Some(serde_json::json!({
                            "round": round,
                            "final_preview": sanitize_text_for_logs(&final_text, 320),
                            "final_chars": final_text.chars().count(),
                        })),
                    );
                    let _ = event_tx.send(StreamEvent::Token(final_text.clone()));
                    let _ = event_tx.send(StreamEvent::Done(final_text));
                } else if had_successful_gmail_tool && !had_failed_gmail_tool {
                    if let Some(summary) = last_successful_gmail_result
                        .as_ref()
                        .and_then(build_grounded_gmail_count_summary)
                    {
                        tracing::info!(
                            has_images,
                            round,
                            "LLM returned empty response with no tool calls; using grounded Gmail count summary"
                        );
                        log_pipeline_step(
                            session_id,
                            "final_output_ready",
                            "Using grounded Gmail count summary fallback",
                            Some(serde_json::json!({
                                "round": round,
                                "final_preview": sanitize_text_for_logs(&summary, 260),
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Token(summary.clone()));
                        let _ = event_tx.send(StreamEvent::Done(summary));
                    } else {
                        let fallback =
                            "I could not generate a response for this request. Please try again."
                                .to_string();
                        tracing::warn!(
                            has_images,
                            round,
                            "LLM returned empty response with no tool calls and no grounded Gmail summary"
                        );
                        log_pipeline_step(
                            session_id,
                            "final_output_fallback",
                            "Generated generic fallback due empty grounded response",
                            Some(serde_json::json!({
                                "round": round,
                                "final_preview": sanitize_text_for_logs(&fallback, 200),
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Token(fallback.clone()));
                        let _ = event_tx.send(StreamEvent::Done(fallback));
                    }
                } else {
                    let fallback =
                        "I could not generate a response for this request. Please try again."
                            .to_string();
                    tracing::warn!(
                        has_images,
                        round,
                        "LLM returned empty response with no tool calls"
                    );
                    log_pipeline_step(
                        session_id,
                        "final_output_fallback",
                        "Generated generic fallback due empty response",
                        Some(serde_json::json!({
                            "round": round,
                            "final_preview": sanitize_text_for_logs(&fallback, 200),
                        })),
                    );
                    let _ = event_tx.send(StreamEvent::Token(fallback.clone()));
                    let _ = event_tx.send(StreamEvent::Done(fallback));
                }
                return;
            }

            // Add assistant message to history
            if !synthetic_package_calls && !synthetic_intent_calls {
                messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: build_tool_call_history_content(&tool_calls),
                    name: None,
                    images: None,
                });

                log_pipeline_step(
                    session_id,
                    "assistant_tool_history_added",
                    "Added assistant tool-call turn to history",
                    Some(serde_json::json!({
                        "round": round,
                        "tool_calls": build_tool_calls_preview(&tool_calls),
                    })),
                );
            }

            // Execute each tool call
            for call in &tool_calls {
                if return_if_stale() {
                    return;
                }

                let mut execution_args = call.arguments.clone();
                if call.name == "analyze_image" {
                    let cap_decision = self.compute_visual_token_cap().await;
                    let mut payload_obj = match &execution_args {
                        serde_json::Value::Object(obj) => obj.clone(),
                        serde_json::Value::String(raw) => {
                            serde_json::from_str::<serde_json::Value>(raw)
                                .ok()
                                .and_then(|v| v.as_object().cloned())
                                .unwrap_or_default()
                        }
                        _ => serde_json::Map::new(),
                    };
                    payload_obj.insert(
                        "hard_visual_token_cap".to_string(),
                        serde_json::json!(cap_decision.hard_cap),
                    );
                    execution_args = serde_json::Value::Object(payload_obj);

                    tracing::info!(
                        hard_visual_token_cap = cap_decision.hard_cap,
                        safe_visual_token_cap = cap_decision.safe_cap,
                        free_vram_mb = cap_decision.free_vram_mb,
                        safety_margin_mb = cap_decision.safety_margin_mb,
                        vision_mode = %cap_decision.vision_mode,
                        "agent_loop: injected hard_visual_token_cap for analyze_image pre-flight"
                    );
                }

                log_pipeline_step(
                    session_id,
                    "tool_call_started",
                    "Beginning tool execution",
                    Some(serde_json::json!({
                        "round": round,
                        "tool": call.name.clone(),
                        "arguments": sanitize_json_for_logs(&execution_args, 220, 8),
                    })),
                );

                // Never execute tools outside the current mounted+tier visible set.
                if !allowed_tool_names.contains(&call.name) {
                    let unavailable_msg = format!(
                        "tool '{}' is not available for current hardware tier '{}' or mounted tool groups",
                        call.name, self.hardware_tier
                    );

                    log_pipeline_step(
                        session_id,
                        "tool_call_rejected",
                        "Tool blocked by tier/mount gating",
                        Some(serde_json::json!({
                            "round": round,
                            "tool": call.name.clone(),
                            "reason": sanitize_text_for_logs(&unavailable_msg, 220),
                        })),
                    );

                    let _ = event_tx.send(StreamEvent::ToolEnd {
                        name: call.name.clone(),
                        result: serde_json::json!({ "error": unavailable_msg }),
                        success: false,
                    });
                    if let Some(flow) = package_flow.as_mut() {
                        flow.observe_tool_result(call, false, &serde_json::Value::Null);
                    }
                    messages.push(ChatMessage {
                        role: "tool".into(),
                        content: format!(
                            "TOOL_ERROR: '{}' is not available in this context (tier/mount gating).",
                            call.name
                        ),
                        name: Some(call.name.clone()),
                        images: None,
                    });
                    continue;
                }

                let _ = event_tx.send(StreamEvent::ToolStart {
                    name: call.name.clone(),
                    params: execution_args.clone(),
                });

                // ── Colab browser-connection gate ────────────────────────────
                // If the LLM emits an execute_cell call but the browser connection
                // has not been established yet, transparently prepend the bootstrap
                // call so code never fires into a disconnected session.
                if call.name.contains("execute_cell") && call.name.contains("colab") {
                    let already_connected = colab_flow
                        .as_ref()
                        .map(|f| f.browser_connected)
                        .unwrap_or(false);
                    if !already_connected
                        && allowed_tool_names
                            .contains("mcp_colab-mcp_open_colab_browser_connection")
                    {
                        let _ = event_tx.send(StreamEvent::Plan(
                            "Colab browser not connected — establishing connection first.".into(),
                        ));
                        let bootstrap = ColabFlowState::browser_open_call();
                        // Inject bootstrap ahead of execute — push current call back.
                        // We handle this by bumping execute_cell to the next round
                        // after the browser is confirmed via observe_tool_result.
                        // Replace current call slice with [bootstrap_call, original_call].
                        // The simplest way: execute bootstrap now via recursive inject.
                        // We'll just replace the current `call` reference by mutating
                        // the iteration — instead, mark as gate-injected and continue.
                        let _ = event_tx.send(StreamEvent::ToolStart {
                            name: bootstrap.name.clone(),
                            params: bootstrap.arguments.clone(),
                        });
                        let gate_result = if let Some(gate_handler) =
                            self.tool_registry.get_handler(&bootstrap.name)
                        {
                            let gate_handler = gate_handler.clone();
                            let gate_args = bootstrap.arguments.clone();
                            let gate_cancel = turn_mcp_cancel.clone();
                            let gate_context =
                                self.tool_registry.make_tool_context(gate_cancel.clone());
                            run_isolated(
                                "tool:mcp_colab-mcp_open_colab_browser_connection",
                                std::time::Duration::from_secs(60),
                                gate_cancel,
                                None,
                                move || async move {
                                    gate_handler
                                        .execute_with_context(gate_args, gate_context)
                                        .await
                                },
                            )
                            .await
                        } else {
                            crate::infra::isolation::ToolResult::err(
                                "open_colab_browser_connection handler not found".to_string(),
                            )
                        };
                        if let Some(flow) = colab_flow.as_mut() {
                            flow.observe_tool_result(
                                &bootstrap,
                                gate_result.success,
                                &gate_result.data,
                            );
                        }
                        let _ = event_tx.send(StreamEvent::ToolEnd {
                            name: bootstrap.name.clone(),
                            result: gate_result.data.clone(),
                            success: gate_result.success,
                        });
                        messages.push(ChatMessage {
                            role: "tool".into(),
                            content: serde_json::to_string(&gate_result.data).unwrap_or_default(),
                            name: Some(bootstrap.name.clone()),
                            images: None,
                        });
                        if !gate_result.success {
                            messages.push(ChatMessage {
                                role: "system".into(),
                                content: "Colab browser connection failed. Cannot execute cell."
                                    .into(),
                                name: None,
                                images: None,
                            });
                            continue;
                        }
                    }
                }

                // Policy check — pass destructive hint from semantic router modality
                let decision = self.policy_engine.evaluate_with_modality_hint(
                    &call.name,
                    &execution_args,
                    turn_modality.destructive,
                );

                log_pipeline_step(
                    session_id,
                    "policy_evaluated",
                    "Policy evaluation completed for tool call",
                    Some(serde_json::json!({
                        "round": round,
                        "tool": call.name.clone(),
                        "risk_level": decision.risk_level.as_str(),
                        "requires_approval": decision.requires_approval,
                        "blocked": decision.blocked,
                        "reason": sanitize_text_for_logs(&decision.reason, 220),
                    })),
                );

                if decision.blocked {
                    // BLACK tier — always denied
                    self.audit_logger.log(
                        session_id,
                        &call.name,
                        &execution_args,
                        RiskLevel::Black,
                        Decision::Blocked,
                        DecidedBy::Hardcoded,
                    );
                    let _ = event_tx.send(StreamEvent::ToolEnd {
                        name: call.name.clone(),
                        result: serde_json::json!({ "error": "blocked by safety policy" }),
                        success: false,
                    });
                    messages.push(ChatMessage {
                        role: "tool".into(),
                        content: format!(
                            "Tool '{}' blocked by safety policy: {}",
                            call.name, decision.reason
                        ),
                        name: Some(call.name.clone()),
                        images: None,
                    });

                    log_pipeline_step(
                        session_id,
                        "tool_call_blocked",
                        "Tool call blocked by safety policy",
                        Some(serde_json::json!({
                            "round": round,
                            "tool": call.name.clone(),
                            "reason": sanitize_text_for_logs(&decision.reason, 220),
                        })),
                    );

                    continue;
                }

                if decision.requires_approval {
                    // RED tier — needs HITL approval (but skip if same tool+args already approved this turn)
                    let dedup_key = format!("{}|{}", call.name, execution_args);
                    let already_approved = approved_this_turn.contains(&dedup_key);

                    if already_approved {
                        // Already approved earlier in this turn — auto-proceed, log it
                        self.audit_logger.log(
                            session_id,
                            &call.name,
                            &execution_args,
                            decision.risk_level,
                            Decision::Approved,
                            DecidedBy::Policy,
                        );

                        log_pipeline_step(
                            session_id,
                            "approval_reused",
                            "Reused earlier approval for identical tool call",
                            Some(serde_json::json!({
                                "round": round,
                                "tool": call.name.clone(),
                            })),
                        );
                    } else {
                        // Generate the request ID up front so the frontend receives the
                        // same ID that the HITL gateway stores in its pending map.
                        let request_id = HitlGateway::generate_request_id();

                        let _ = event_tx.send(StreamEvent::ApprovalRequired {
                            request_id: request_id.clone(),
                            action: call.name.clone(),
                            risk_level: decision.risk_level.as_str().into(),
                            parameters: execution_args.clone(),
                        });

                        log_pipeline_step(
                            session_id,
                            "approval_requested",
                            "Approval requested for RED-tier tool call",
                            Some(serde_json::json!({
                                "round": round,
                                "tool": call.name.clone(),
                                "request_id": request_id.clone(),
                                "risk_level": decision.risk_level.as_str(),
                            })),
                        );

                        let approval = self
                            .hitl_gateway
                            .request_approval_with_id(
                                &request_id,
                                &call.name,
                                execution_args.clone(),
                                decision.risk_level,
                                &format!("Execute {} with params: {}", call.name, execution_args),
                                true,
                            )
                            .await;

                        let (audit_decision, decided_by, approved, denial_reason) = match approval {
                            ApprovalResponse::Approved => {
                                (Decision::Approved, DecidedBy::UserGui, true, "")
                            }
                            ApprovalResponse::Denied => (
                                Decision::Denied,
                                DecidedBy::UserGui,
                                false,
                                "denied by user",
                            ),
                            ApprovalResponse::Timeout => (
                                Decision::Timeout,
                                DecidedBy::Timeout,
                                false,
                                "approval timed out — user did not respond",
                            ),
                        };

                        self.audit_logger.log(
                            session_id,
                            &call.name,
                            &execution_args,
                            decision.risk_level,
                            audit_decision,
                            decided_by,
                        );

                        let _ = event_tx.send(StreamEvent::ApprovalResult {
                            action: call.name.clone(),
                            approved,
                        });

                        log_pipeline_step(
                            session_id,
                            "approval_result",
                            "Approval decision received",
                            Some(serde_json::json!({
                                "round": round,
                                "tool": call.name.clone(),
                                "approved": approved,
                            })),
                        );

                        if !approved {
                            // Emit ToolEnd so the UI shows the tool as failed (not just pending).
                            let _ = event_tx.send(StreamEvent::ToolEnd {
                                name: call.name.clone(),
                                result: serde_json::json!({ "error": denial_reason }),
                                success: false,
                            });
                            messages.push(ChatMessage {
                                role: "tool".into(),
                                content: format!(
                                    "TOOL_ERROR: '{}' was NOT executed — {}. \
                                     The operation did not happen. \
                                     You MUST tell the user the action failed and why.",
                                    call.name, denial_reason
                                ),
                                name: Some(call.name.clone()),
                                images: None,
                            });

                            log_pipeline_step(
                                session_id,
                                "tool_call_denied",
                                "Tool call not executed due denied/timeout approval",
                                Some(serde_json::json!({
                                    "round": round,
                                    "tool": call.name.clone(),
                                    "reason": denial_reason,
                                })),
                            );

                            continue;
                        }

                        // Remember this approval for the rest of this turn
                        approved_this_turn.insert(dedup_key);

                        // Create rollback snapshot for RED actions
                        // (actual file backup happens inside specific tool handlers)
                    }
                }

                // ── Dedup guard: abort on repeated identical failure ───────────
                let call_hash = call_dedup_hash(&call.name, &execution_args);
                if let Some((fail_count, cached_err)) = failed_calls.get(&call_hash) {
                    if *fail_count >= 1 {
                        let abort_msg = format!(
                            "repeated_identical_failure: '{}' with the same arguments already \
                             failed in this turn: {}. Aborting to prevent an infinite loop.",
                            call.name, cached_err
                        );
                        tracing::warn!(
                            session = session_id,
                            tool = %call.name,
                            "dedup guard: aborting duplicate failed call"
                        );
                        log_pipeline_step(
                            session_id,
                            "tool_retry_blocked",
                            "Blocked duplicate failed tool call",
                            Some(serde_json::json!({
                                "round": round,
                                "tool": call.name.clone(),
                                "fail_count": fail_count,
                                "cached_error": cached_err,
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Error(abort_msg.clone()));
                        return;
                    }
                }

                // ── Turn budget guard: skip tool if cumulative tokens exhausted ─
                if turn_tool_tokens >= LLM_TURN_TOOL_BUDGET {
                    let budget_msg = format!(
                        "TOOL_BUDGET_EXHAUSTED: turn tool-output token budget ({LLM_TURN_TOOL_BUDGET}) \
                         reached; skipping '{}'. Summarise what you have and answer the user.",
                        call.name
                    );
                    tracing::warn!(
                        session = session_id,
                        turn_tool_tokens,
                        tool = %call.name,
                        "turn tool-output budget exhausted; skipping tool"
                    );
                    messages.push(ChatMessage {
                        role: "tool".into(),
                        content: budget_msg,
                        name: Some(call.name.clone()),
                        images: None,
                    });
                    continue;
                }

                // ── Heartbeat: emit ToolProgress every 2 s while tool runs ─────
                let hb_cancel = CancellationToken::new();
                let hb_cancel_clone = hb_cancel.clone();
                let hb_tx = event_tx.clone();
                let hb_tool = call.name.clone();
                let hb_admission = Arc::clone(&turn_admission_for_async);
                let hb_session_id = session_id_for_async.clone();
                let hb_turn_id = turn_id_for_async.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
                    interval.tick().await; // skip the immediate first tick
                    loop {
                        tokio::select! {
                            biased;
                            _ = hb_cancel_clone.cancelled() => break,
                            _ = interval.tick() => {
                                if !hb_admission.is_active(&hb_session_id, &hb_turn_id) {
                                    break;
                                }
                                let _ = hb_tx.send(StreamEvent::ToolProgress {
                                    call_id: hb_tool.clone(),
                                    message: format!("⏳ {} is still running…", hb_tool),
                                    percent: None,
                                });
                            }
                        }
                    }
                });

                // Execute the tool
                let tool_result = if let Some(handler) = self.tool_registry.get_handler(&call.name)
                {
                    let handler = handler.clone();
                    let args = execution_args.clone();
                    // Long-running tools get extended timeouts
                    let timeout_secs = match call.name.as_str() {
                        "install_application"
                        | "uninstall_application"
                        | "update_all_packages"
                        | "install_package"
                        | "uninstall_package"
                        | "execute_fleet_command" => 300,
                        "generate_image" => 300,
                        "search_news" | "fetch_article" => 60,
                        "execute_bash" | "execute_python" | "execute_powershell" => 120,
                        "download_file" => 120,
                        _ => 30,
                    };
                    let execution_cancel = if call.name == "generate_image" {
                        turn_image_cancel.clone()
                    } else if call.name.starts_with("mcp_") {
                        turn_mcp_cancel.clone()
                    } else if is_sidecar_backed_tool_name(&call.name) {
                        turn_sidecar_cancel.clone()
                    } else {
                        turn_tools_cancel.clone()
                    };
                    let isolation_name = format!("tool:{}", call.name);
                    let tool_context = self
                        .tool_registry
                        .make_tool_context(execution_cancel.clone());
                    run_isolated(
                        &isolation_name,
                        std::time::Duration::from_secs(timeout_secs),
                        execution_cancel,
                        None,
                        move || async move { handler.execute_with_context(args, tool_context).await },
                    ).await
                } else {
                    crate::infra::isolation::ToolResult::err(format!("unknown tool: {}", call.name))
                };

                if return_if_stale() {
                    return;
                }

                // Stop the heartbeat task.
                hb_cancel.cancel();

                // Phase 5: Record routing feedback for online learning
                if let Some(ref feedback_collector) = self.feedback_collector {
                    let outcome = crate::routing::feedback::detect_outcome(
                        crate::routing::domain::Domain::Conversation, // Will be resolved by context
                        Some(&call.name),
                        None, // next_text unknown at this point
                        tool_result.success,
                        tool_result.error.as_deref(),
                    );
                    let mut collector = feedback_collector.lock().await;
                    collector.record(crate::routing::feedback::RoutingFeedback {
                        input_text_hash: {
                            use std::hash::{Hash, Hasher};
                            let mut hasher = std::collections::hash_map::DefaultHasher::new();
                            routing_focus_text.hash(&mut hasher);
                            hasher.finish()
                        },
                        domain_selected: crate::routing::domain::Domain::Conversation,
                        tool_selected: Some(call.name.clone()),
                        intent_source: format!("{:?}", turn_gate_plan.intent.source),
                        confidence: turn_gate_plan.intent.confidence,
                        outcome,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        session_id: session_id.to_string(),
                        embedding: Vec::new(),
                    });
                }

                // ── Update error-loop counters ─────────────────────────────────
                if tool_result.success {
                    consecutive_failures = 0;
                } else {
                    let err_text = tool_result
                        .error
                        .clone()
                        .unwrap_or_else(|| "unknown error".to_string());
                    let entry = failed_calls
                        .entry(call_hash)
                        .or_insert((0, err_text.clone()));
                    entry.0 += 1;
                    entry.1 = err_text;
                    consecutive_failures += 1;

                    let replanned = self.turn_gate.replan_after_error(
                        &turn_gate_plan,
                        &round_focus_text,
                        has_images,
                        &call.name,
                        &entry.1,
                    );
                    turn_gate_plan = replanned;

                    log_pipeline_step(
                        session_id,
                        "executor_replan_requested",
                        "Tool failure triggered TurnGate replanning",
                        Some(serde_json::json!({
                            "round": round,
                            "failed_tool": call.name.clone(),
                            "error": sanitize_text_for_logs(&entry.1, 220),
                            "replanned_operation": format!("{:?}", turn_gate_plan.intent.operation),
                            "replanned_compute": format!("{:?}", turn_gate_plan.intent.compute),
                            "replanned_confidence": turn_gate_plan.intent.confidence,
                        })),
                    );
                    let _ = event_tx.send(StreamEvent::Plan(format!(
                        "Replanning via TurnGate after '{}' failed.",
                        call.name
                    )));

                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        tracing::warn!(
                            session = session_id,
                            consecutive_failures,
                            "3 consecutive tool failures — injecting corrective prompt"
                        );
                        log_pipeline_step(
                            session_id,
                            "consecutive_failures_threshold",
                            "3 consecutive tool failures; injecting corrective system message",
                            Some(serde_json::json!({ "round": round })),
                        );
                        // Inject a corrective system message so the LLM knows to
                        // stop using tools and answer with what it has.
                        messages.push(ChatMessage {
                            role: "system".into(),
                            content: "SYSTEM: 3 consecutive tool executions have failed. \
                                      Stop issuing tool calls. Respond to the user using \
                                      whatever information you have, or ask the user for \
                                      guidance to resolve the problem."
                                .to_string(),
                            name: None,
                            images: None,
                        });
                        // Reset so we don't inject repeatedly.
                        consecutive_failures = 0;
                    }
                }

                if let Some(flow) = package_flow.as_mut() {
                    flow.observe_tool_result(call, tool_result.success, &tool_result.data);
                }

                if let Some(flow) = colab_flow.as_mut() {
                    flow.observe_tool_result(call, tool_result.success, &tool_result.data);
                }

                if is_gmail_tool_name(&call.name) {
                    if tool_result.success {
                        had_successful_gmail_tool = true;
                        last_successful_gmail_result = Some(tool_result.data.clone());
                    } else {
                        had_failed_gmail_tool = true;
                    }
                }

                if call.name == "generate_image" && tool_result.success {
                    last_successful_image_result = Some(tool_result.data.clone());
                }

                // For generate_image failures: emit a structured user-visible message
                // and skip the LLM round so the user gets clear feedback immediately.
                if call.name == "generate_image" && !tool_result.success {
                    let failure_msg = build_image_failure_response(&tool_result.data);
                    tracing::warn!(
                        session = session_id,
                        "generate_image failed; returning structured failure to user"
                    );
                    let _ = event_tx.send(StreamEvent::Token(failure_msg.clone()));
                    let _ = event_tx.send(StreamEvent::Done(failure_msg));
                    return;
                }

                // Log GREEN/YELLOW auto-executed
                if !decision.requires_approval {
                    let eval_synthetic_approval = std::env::var("KRIA_EVAL_MODE").is_ok()
                        && decision.reason.contains("EvalHarness auto-approved");
                    let (audit_decision, decided_by) = if eval_synthetic_approval {
                        (Decision::Approved, DecidedBy::Hardcoded)
                    } else {
                        (Decision::AutoExecuted, DecidedBy::Policy)
                    };

                    self.audit_logger.log(
                        session_id,
                        &call.name,
                        &call.arguments,
                        decision.risk_level,
                        audit_decision,
                        decided_by,
                    );
                }

                // Build the string the LLM will see.
                // IMPORTANT: if the tool failed, send the error — not "null" —
                // so the LLM knows to report the failure instead of hallucinating.
                //
                // For successful results we apply a two-stage budget strategy:
                //   1. Shape the raw payload (drop bodies/base64, truncate strings)
                //      using the domain-aware shaper.  Gmail payloads first go
                //      through the existing compact_tool_result_for_llm() path.
                //   2. Count tokens via llama.cpp /tokenize; if still over the
                //      per-tool budget, re-shape with a tighter char budget.
                //   3. Hard char-cap as a final safety net.
                let mut extracted_tool_images =
                    if tool_result.success && call.name == "analyze_image" {
                        extract_preprocessed_image_attachments(&tool_result.data, "image/jpeg")
                    } else {
                        None
                    };
                let extracted_tool_image_count = extracted_tool_images
                    .as_ref()
                    .map(|imgs| imgs.len())
                    .unwrap_or(0);
                if !inline_images_allowed_for_turn {
                    extracted_tool_images = None;
                }

                let llm_tool_result = compact_tool_result_for_llm(&call.name, &tool_result.data);
                let result_str = if !tool_result.success {
                    let err_msg = tool_result
                        .error
                        .as_deref()
                        .unwrap_or("tool execution failed with no details");
                    format!("TOOL_ERROR: {err_msg}")
                } else {
                    // ── Context Bomb mitigation ────────────────────────────
                    // Per-tool char budget derived from token budget.
                    let char_budget = LLM_TOOL_RESULT_TOKEN_BUDGET * 4; // ~4 chars/token heuristic

                    // Stream the full payload to the UI via ToolPayloadChunk so
                    // the user always sees complete data while the LLM only gets
                    // the compact summary.
                    let full_payload_str = llm_tool_result.to_string();
                    if full_payload_str.len() > char_budget {
                        // Emit a single final chunk with full data for UI rendering.
                        let _ = event_tx.send(StreamEvent::ToolPayloadChunk {
                            call_id: call.name.clone(),
                            seq: 0,
                            is_final: true,
                            data: llm_tool_result.clone(),
                        });
                    }

                    // Stage 1: structural shaping.
                    let shaped = shape_for_llm(&call.name, &llm_tool_result, char_budget);
                    let mut shaped_str = shaped.value.to_string();

                    // Stage 2: token counting — tighten budget if needed.
                    let tokenizer_url = backend.tokenizer_base_url();
                    let token_count = count_tokens(&shaped_str, &tokenizer_url).await;
                    if token_count > LLM_TOOL_RESULT_TOKEN_BUDGET {
                        // Re-shape with a char budget proportional to how much
                        // we need to shrink.
                        let tighter =
                            (char_budget * LLM_TOOL_RESULT_TOKEN_BUDGET / token_count).max(512);
                        let reshaped = shape_for_llm(&call.name, &llm_tool_result, tighter);
                        shaped_str = reshaped.value.to_string();
                    }

                    // Stage 3: hard char cap as final safety net.
                    if shaped_str.len() > TOOL_RESULT_MAX_CHARS {
                        format!("{}...<truncated>", &shaped_str[..TOOL_RESULT_MAX_CHARS])
                    } else {
                        shaped_str
                    }
                };

                // Update the cumulative turn token counter.
                let result_tokens = count_tokens(&result_str, &backend.tokenizer_base_url()).await;
                turn_tool_tokens = turn_tool_tokens.saturating_add(result_tokens);

                // Auto-route: if tool result contains a file path, check if a
                // precognitive tool should process it automatically
                let auto_enrichment = self
                    .auto_route_file_result(&call.name, &tool_result.data)
                    .await;

                log_pipeline_step(
                    session_id,
                    "tool_result_ready",
                    "Tool execution completed",
                    Some(serde_json::json!({
                        "round": round,
                        "tool": call.name.clone(),
                        "success": tool_result.success,
                        "error": tool_result
                            .error
                            .as_ref()
                            .map(|e| sanitize_text_for_logs(e, 220)),
                        "migrated_tool_images": extracted_tool_image_count,
                        "result_preview": sanitize_json_for_logs(&tool_result.data, 220, 8),
                        "result_tokens": result_tokens,
                        "turn_tool_tokens_total": turn_tool_tokens,
                        "auto_enriched": auto_enrichment.is_some(),
                    })),
                );

                let _ = event_tx.send(StreamEvent::ToolEnd {
                    name: call.name.clone(),
                    result: tool_result.data.clone(),
                    success: tool_result.success,
                });

                let tool_msg = if let Some(enrichment) = auto_enrichment {
                    format!(
                        "{}\n\n[Auto-enriched via sidecar]\n{}",
                        result_str, enrichment
                    )
                } else {
                    result_str
                };

                let tool_msg =
                    if let Some(note) = build_grounding_count_note(&call.name, &llm_tool_result) {
                        format!("{tool_msg}\n\n{note}")
                    } else {
                        tool_msg
                    };

                messages.push(ChatMessage {
                    role: "tool".into(),
                    content: tool_msg,
                    name: Some(call.name.clone()),
                    images: extracted_tool_images,
                });
            }

            // ── Image-generation early exit ────────────────────────────────────────
            // When generate_image succeeded this round, skip the round-N LLM summary
            // call entirely — that call would crash the GPU with ctx=2048 + 167 schemas.
            // Instead, emit a pre-built confirmation response and return immediately.
            if let Some(ref img_data) = last_successful_image_result {
                if return_if_stale() {
                    return;
                }

                let summary = build_image_success_response(img_data);
                log_pipeline_step(
                    session_id,
                    "final_output_ready",
                    "Image generation succeeded; skipping LLM summary call",
                    Some(serde_json::json!({
                        "round": round,
                        "final_preview": sanitize_text_for_logs(&summary, 280),
                    })),
                );
                let _ = event_tx.send(StreamEvent::Token(summary.clone()));
                let _ = event_tx.send(StreamEvent::Done(summary));
                return;
            }

            log_pipeline_step(
                session_id,
                "round_completed",
                "Round completed with tool outputs appended; continuing loop",
                Some(serde_json::json!({
                    "round": round,
                    "history_message_count": messages.len(),
                })),
            );
        }

        log_pipeline_step(
            session_id,
            "max_rounds_reached",
            "Agent loop reached max tool rounds",
            Some(serde_json::json!({
                "max_tool_rounds": self.max_tool_rounds,
            })),
        );

        if !is_turn_active() {
            return;
        }

        let _ = event_tx.send(StreamEvent::Error(format!(
            "max tool rounds ({}) reached",
            self.max_tool_rounds
        )));
    }

    /// Check if a tool result contains a file path that should be auto-routed
    /// to a precognitive processor for enrichment.
    async fn auto_route_file_result(
        &self,
        tool_name: &str,
        result: &serde_json::Value,
    ) -> Option<String> {
        // Only auto-route results from file-related tools, not from precognitive tools themselves
        if tool_name.starts_with("image_")
            || tool_name.starts_with("document_")
            || tool_name.starts_with("code_")
            || tool_name.starts_with("audio_")
            || tool_name.starts_with("web_")
            || tool_name.starts_with("embeddings_")
        {
            return None;
        }

        // Look for a file path in the result
        let path = result
            .get("path")
            .or_else(|| result.get("file_path"))
            .or_else(|| result.get("output_path"))
            .and_then(|v| v.as_str())?;

        // Determine the target precognitive tool based on extension
        let ext = path.rsplit('.').next()?.to_lowercase();
        let target_tool = match ext.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" | "svg" => "image_analyze",
            "pdf" | "docx" | "doc" | "csv" | "tsv" | "xlsx" => "document_extract",
            "py" | "rs" | "js" | "ts" | "jsx" | "tsx" | "go" | "java" | "c" | "cpp" | "h"
            | "rb" | "cs" => "code_analyze_ast",
            "wav" | "mp3" | "ogg" | "flac" | "m4a" => "audio_preprocess",
            _ => return None,
        };

        // Execute the precognitive tool
        if let Some(handler) = self.tool_registry.get_handler(target_tool) {
            let params = serde_json::json!({"file_path": path});
            let handler = handler.clone();
            let tool_context = self
                .tool_registry
                .make_tool_context(CancellationToken::new());
            match tokio::time::timeout(
                std::time::Duration::from_secs(30),
                handler.execute_with_context(params, tool_context),
            )
            .await
            {
                Ok(result) if result.success => {
                    // Return summary only to save tokens
                    result
                        .data
                        .get("summary")
                        .and_then(|s| s.as_str())
                        .map(|summary| format!("[{}] {}", target_tool, summary))
                }
                _ => None,
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests;
