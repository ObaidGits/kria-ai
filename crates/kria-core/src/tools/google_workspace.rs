//! Google Workspace tools — hybrid MCP + sidecar handlers.
//!
//! Architecture: tools are ALWAYS registered in the ToolRegistry so the LLM
//! can see them regardless of whether the MCP server is connected.  The actual
//! MCP connection is held in a lazy `GwClientRef` (Arc<RwLock<Option<…>>>).
//! Once the gworkspace MCP server starts successfully, `init_runtime` populates
//! that ref via `set_client()`.  Until then, every handler returns a clear
//! "not connected" message rather than panicking or silently failing.
//!
//! Mount groups:
//!   ambient: gw_gmail_inbox, gw_gmail_search, gw_gmail_read,
//!            gw_calendar_today, gw_calendar_search,
//!            gw_drive_search, gw_drive_list, gw_drive_read
//!   docs:    gw_docs_read, gw_docs_create, gw_docs_edit,
//!            gw_sheets_read, gw_sheets_create, gw_sheets_edit,
//!            gw_slides_read, gw_slides_create,
//!            gw_forms_list, gw_forms_create
//!   admin:   gw_gmail_send, gw_gmail_delete,
//!            gw_drive_delete, gw_calendar_create, gw_calendar_delete

use crate::infra::ToolResult;
use crate::mcp::McpClient;
use crate::safety::RiskLevel;
use crate::sidecar::SidecarBridge;
use crate::tools::availability;
use crate::tools::google_workspace_contract as gw_contract;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

/// Lazy reference to the gworkspace MCP client.
/// Starts as None; populated by `set_client()` once the MCP server connects.
pub type GwClientRef = Arc<tokio::sync::RwLock<Option<Arc<McpClient>>>>;

/// Create an empty lazy client reference (call `set_client()` later).
pub fn new_client_ref() -> GwClientRef {
    Arc::new(tokio::sync::RwLock::new(None))
}

/// Wire in the live MCP client after the server starts.
pub async fn set_client(gw_ref: &GwClientRef, client: Arc<McpClient>) {
    tracing::info!("[GW] wiring live McpClient into GwClientRef");
    *gw_ref.write().await = Some(client);
}

/// Lazy reference to the GitHub MCP client (mirrors [`GwClientRef`]).
pub type GhClientRef = GwClientRef;

/// Create an empty lazy GitHub client reference.
pub fn new_github_client_ref() -> GhClientRef {
    new_client_ref()
}

/// Wire in the live GitHub MCP client after the `github` server connects.
pub async fn set_github_client(gh_ref: &GhClientRef, client: Arc<McpClient>) {
    tracing::info!("[GH] wiring live McpClient into GhClientRef");
    *gh_ref.write().await = Some(client);
}

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

/// Shared handles for hybrid MCP + sidecar calls.
#[derive(Clone)]
struct GwBridge {
    /// Lazy client ref — None until the gworkspace MCP server connects.
    mcp: GwClientRef,
    sidecar: Arc<SidecarBridge>,
}

fn parse_input<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, ToolResult> {
    let normalized = if params.is_null() {
        serde_json::json!({})
    } else {
        params
    };

    serde_json::from_value(normalized)
        .map_err(|error| ToolResult::err(format!("invalid parameters: {error}")))
}

fn require_non_empty(value: &str, field: &str) -> Result<(), ToolResult> {
    if value.trim().is_empty() {
        return Err(ToolResult::err(format!("{field} is required")));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum ToolExecutionError {
    #[error("google_workspace mcp request timed out for tool '{tool}' after {timeout_secs}s")]
    McpTimeout { tool: String, timeout_secs: u64 },
    #[error("google_workspace mcp request failed for tool '{tool}': {reason}")]
    McpRequest { tool: String, reason: String },
    #[error(
        "google_workspace sidecar request timed out for method '{method}' after {timeout_secs}s"
    )]
    SidecarTimeout { method: String, timeout_secs: u64 },
    #[error("google_workspace sidecar request failed for method '{method}': {reason}")]
    SidecarRequest { method: String, reason: String },
}

fn active_google_account() -> String {
    std::env::var("KRIA_GW_ACCOUNT").unwrap_or_else(|_| "personal".into())
}

const GMAIL_MAX_RESULTS_CAP: u64 = 200;
const GMAIL_PAGE_SIZE_CAP: u64 = 50;
const GMAIL_MAX_PAGE_FETCHES: usize = 6;
const MCP_REQUEST_TIMEOUT_SECS: u64 = 30;
const SIDECAR_REQUEST_TIMEOUT_SECS: u64 = 20;

fn default_gmail_max_results() -> u64 {
    10
}

fn default_calendar_max_results() -> u64 {
    20
}

fn default_calendar_today_max_results() -> u64 {
    50
}

fn default_sheet_range() -> String {
    "A1".to_string()
}

fn default_untitled_title() -> String {
    "Untitled".to_string()
}

fn default_untitled_form_title() -> String {
    "Untitled Form".to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GmailInboxInput {
    #[serde(default)]
    query: Option<String>,
    #[serde(default = "default_gmail_max_results")]
    max_results: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GmailSearchInput {
    query: String,
    #[serde(default = "default_gmail_max_results")]
    max_results: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadEmailInput {
    message_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SendEmailInput {
    to: String,
    subject: String,
    body: String,
    #[serde(default)]
    cc: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteEmailInput {
    message_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReplyEmailInput {
    message_id: String,
    body: String,
    #[serde(default)]
    reply_all: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MarkEmailInput {
    message_id: String,
    #[serde(default)]
    read: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LabelEmailInput {
    message_id: String,
    label: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DriveCreateFileInput {
    name: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    folder_id: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DriveCreateFolderInput {
    name: String,
    #[serde(default)]
    parent_folder_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DriveMoveInput {
    file_id: String,
    target_folder_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DriveRenameInput {
    file_id: String,
    new_name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CalendarSearchInput {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    time_min: Option<String>,
    #[serde(default)]
    time_max: Option<String>,
    #[serde(default = "default_calendar_max_results")]
    max_results: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateCalendarEventInput {
    summary: String,
    start: String,
    end: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    attendees: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteCalendarEventInput {
    event_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DriveSearchInput {
    query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DriveListInput {
    #[serde(default)]
    folder_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DriveReadInput {
    file_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DriveDeleteInput {
    file_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DocsReadInput {
    document_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateDocumentInput {
    #[serde(default = "default_untitled_title")]
    title: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditDocumentInput {
    document_id: String,
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SheetsReadInput {
    spreadsheet_id: String,
    #[serde(default)]
    range: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateSpreadsheetInput {
    #[serde(default = "default_untitled_title")]
    title: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditSpreadsheetInput {
    spreadsheet_id: String,
    #[serde(default = "default_sheet_range")]
    range: String,
    values: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SlidesReadInput {
    presentation_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreatePresentationInput {
    #[serde(default = "default_untitled_title")]
    title: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FormsListInput {
    #[serde(default)]
    query: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateFormInput {
    #[serde(default = "default_untitled_form_title")]
    title: String,
}

fn new_correlation_id() -> String {
    gw_contract::new_correlation_id()
}

fn parse_json_or_text(text: &str) -> serde_json::Value {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return serde_json::Value::Null;
    }

    serde_json::from_str::<serde_json::Value>(trimmed)
        .unwrap_or_else(|_| serde_json::json!({ "text": trimmed }))
}

fn envelope_result_with_meta(
    tool: &str,
    data: serde_json::Value,
    raw_text: Option<&str>,
    correlation_id: Option<&str>,
    account: Option<&str>,
) -> serde_json::Value {
    gw_contract::envelope_for_tool(tool, data, raw_text, correlation_id, account)
}

fn envelope_result(
    tool: &str,
    data: serde_json::Value,
    raw_text: Option<&str>,
) -> serde_json::Value {
    envelope_result_with_meta(tool, data, raw_text, None, None)
}

fn looks_like_drive_listing_phrase(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let has_list_intent = [
        "list",
        "show",
        "browse",
        "contents",
        "what is in",
        "what's in",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    let has_search_intent = ["search", "find", "look for", "locate"]
        .iter()
        .any(|needle| lower.contains(needle));

    has_list_intent && !has_search_intent
}

fn looks_like_gmail_message_object(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .map(|obj| {
            [
                "id",
                "messageId",
                "message_id",
                "threadId",
                "subject",
                "from",
                "snippet",
            ]
            .iter()
            .any(|key| obj.contains_key(*key))
        })
        .unwrap_or(false)
}

fn parse_gmail_heading_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if !(trimmed.starts_with("**") && trimmed.ends_with("**") && trimmed.len() > 4) {
        return None;
    }

    let inner = &trimmed[2..trimmed.len() - 2];
    let (index, rest) = inner.split_once(". ")?;
    if !index.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    let subject = rest.trim();
    if subject.is_empty() {
        None
    } else {
        Some(subject.to_string())
    }
}

fn parse_gmail_labels(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(|label| label.to_string())
        .collect()
}

fn parse_gmail_messages_from_text(raw: &str) -> Vec<serde_json::Value> {
    let mut messages: Vec<serde_json::Value> = Vec::new();
    let mut current: Option<serde_json::Map<String, serde_json::Value>> = None;

    for line in raw.lines() {
        if let Some(subject) = parse_gmail_heading_line(line) {
            if let Some(msg) = current.take() {
                messages.push(serde_json::Value::Object(msg));
            }

            let mut msg = serde_json::Map::new();
            msg.insert("subject".into(), serde_json::Value::String(subject));
            current = Some(msg);
            continue;
        }

        let Some(msg) = current.as_mut() else {
            continue;
        };
        let trimmed = line.trim();

        if let Some(value) = trimmed.strip_prefix("From:") {
            msg.insert(
                "from".into(),
                serde_json::Value::String(value.trim().to_string()),
            );
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("Date:") {
            msg.insert(
                "date".into(),
                serde_json::Value::String(value.trim().to_string()),
            );
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("ID:") {
            msg.insert(
                "id".into(),
                serde_json::Value::String(value.trim().to_string()),
            );
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("Labels:") {
            let labels = parse_gmail_labels(value.trim());
            msg.insert(
                "labels".into(),
                serde_json::Value::Array(
                    labels.into_iter().map(serde_json::Value::String).collect(),
                ),
            );
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("Preview:") {
            msg.insert(
                "preview".into(),
                serde_json::Value::String(value.trim().to_string()),
            );
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("Link:") {
            msg.insert(
                "url".into(),
                serde_json::Value::String(value.trim().to_string()),
            );
            continue;
        }
    }

    if let Some(msg) = current.take() {
        messages.push(serde_json::Value::Object(msg));
    }

    messages
}

fn gmail_messages_from_payload(payload: &serde_json::Value) -> Vec<serde_json::Value> {
    if let Some(messages) = payload.get("messages").and_then(|v| v.as_array()) {
        return messages.clone();
    }

    if let Some(results) = payload.get("results").and_then(|v| v.as_array()) {
        return results.clone();
    }

    if let Some(rows) = payload.as_array() {
        return rows.clone();
    }

    if let Some(text) = payload.get("text").and_then(|v| v.as_str()) {
        let parsed = parse_gmail_messages_from_text(text);
        if !parsed.is_empty() {
            return parsed;
        }
    }

    if looks_like_gmail_message_object(payload) {
        return vec![payload.clone()];
    }

    Vec::new()
}

fn gmail_next_page_token(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("nextPageToken")
        .or_else(|| payload.get("next_page_token"))
        .or_else(|| payload.get("nextPage"))
        .or_else(|| {
            payload
                .get("pagination")
                .and_then(|v| v.get("nextPageToken"))
        })
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| token.to_string())
}

fn gmail_message_identifier(message: &serde_json::Value) -> Option<String> {
    ["id", "messageId", "message_id", "threadId", "thread_id"]
        .iter()
        .find_map(|key| {
            message
                .get(*key)
                .and_then(|v| v.as_str())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
}

fn should_ignore_gmail_page_token_error(error: Option<&str>) -> bool {
    let Some(raw) = error else {
        return false;
    };

    let lower = raw.to_ascii_lowercase();
    let mentions_page_token = lower.contains("pagetoken") || lower.contains("page token");
    let looks_like_schema_error = lower.contains("unexpected parameter")
        || lower.contains("additional properties")
        || lower.contains("unknown")
        || lower.contains("invalid argument");

    mentions_page_token && looks_like_schema_error
}

fn find_string_field_recursive(value: &serde_json::Value, key: &str) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(found) = map
                .get(key)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                return Some(found.to_string());
            }

            for child in map.values() {
                if let Some(found) = find_string_field_recursive(child, key) {
                    return Some(found);
                }
            }

            None
        }
        serde_json::Value::Array(items) => {
            for item in items {
                if let Some(found) = find_string_field_recursive(item, key) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn extract_first_string_recursive(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| find_string_field_recursive(value, key))
}

fn extract_gmail_draft_id(payload: &serde_json::Value) -> Option<String> {
    if let Some(draft_id) = extract_first_string_recursive(payload, &["draftId", "draft_id"]) {
        return Some(draft_id);
    }

    let raw_text = find_string_field_recursive(payload, "raw_text")?;
    let parsed = parse_json_or_text(&raw_text);
    extract_first_string_recursive(&parsed, &["draftId", "draft_id"])
}

fn extract_id_from_google_url(url: &str, marker: &str) -> Option<String> {
    let (_, rest) = url.split_once(marker)?;
    let id = rest
        .split('/')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .trim();
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

fn extract_google_resource_id(
    payload: &serde_json::Value,
    id_keys: &[&str],
    url_keys: &[&str],
    url_marker: &str,
) -> Option<String> {
    if let Some(id) = extract_first_string_recursive(payload, id_keys) {
        return Some(id);
    }

    for url_key in url_keys {
        if let Some(url) = find_string_field_recursive(payload, url_key) {
            if let Some(id) = extract_id_from_google_url(&url, url_marker) {
                return Some(id);
            }
        }
    }

    None
}

fn build_google_resource_url(resource_kind: &str, resource_id: &str) -> Option<String> {
    let id = resource_id.trim();
    if id.is_empty() {
        return None;
    }

    match resource_kind {
        "document" => Some(format!("https://docs.google.com/document/d/{id}/edit")),
        "spreadsheet" => Some(format!("https://docs.google.com/spreadsheets/d/{id}/edit")),
        "presentation" => Some(format!("https://docs.google.com/presentation/d/{id}/edit")),
        _ => None,
    }
}

fn calendar_create_args(
    input: &CreateCalendarEventInput,
    alternate_shape: bool,
) -> serde_json::Value {
    let summary = input.summary.clone();
    let start = input.start.clone();
    let end = input.end.clone();
    let description = input.description.clone().unwrap_or_default();
    let location = input.location.clone().unwrap_or_default();

    let mut args = if alternate_shape {
        serde_json::json!({
            "summary": summary,
            "startDateTime": start,
            "endDateTime": end,
            "description": description,
            "location": location,
        })
    } else {
        serde_json::json!({
            "summary": summary,
            "start": { "dateTime": start },
            "end": { "dateTime": end },
            "description": description,
            "location": location,
        })
    };

    if let Some(attendees) = input.attendees.as_ref().filter(|arr| !arr.is_empty()) {
        args["attendees"] = serde_json::Value::Array(attendees.clone());
    }

    args
}

fn should_retry_calendar_with_alternate_shape(error: Option<&str>) -> bool {
    let Some(raw) = error else {
        return true;
    };
    let lower = raw.to_ascii_lowercase();
    if lower.contains("not connected") || lower.contains("mcp call failed") {
        return false;
    }
    if lower.contains("rate limit") || lower.contains("quota") {
        return false;
    }
    true
}

impl GwBridge {
    /// Inject the `account` field into the params object (required by every tool in
    /// `google-workspace-mcp`), then call the MCP tool.
    async fn mcp_call_raw(&self, tool: &str, mut args: serde_json::Value) -> ToolResult {
        // Ensure args is an object
        if !args.is_object() {
            args = serde_json::json!({});
        }
        let account = active_google_account();
        let correlation_id = new_correlation_id();
        // Inject account — never overwrite if the caller already set it
        if let Some(obj) = args.as_object_mut() {
            obj.entry("account")
                .or_insert_with(|| serde_json::json!(account));
        }

        tracing::info!(
            "[GW] mcp_call: tool='{}' account='{}' correlation_id='{}'",
            tool,
            account,
            correlation_id
        );
        tracing::debug!("[GW] mcp_call args: {}", args);

        let guard = self.mcp.read().await;
        let client = match guard.as_ref() {
            Some(c) => c.clone(),
            None => {
                let msg = format!(
                    "Google Workspace credentials are not connected/authenticated. \
                     Run: npx google-workspace-mcp accounts add personal  \
                     Then restart KRIA. (tool={tool})"
                );
                let gw_error = GwErrorDescriptor {
                    code: "account_not_connected",
                    category: "configuration",
                    recovery_action: "refresh_auth",
                    retryable: false,
                    user_facing: msg.clone(),
                };
                tracing::warn!("[GW] {}", msg);
                return ToolResult {
                    success: false,
                    data: envelope_result_with_meta(
                        tool,
                        serde_json::json!({ "error": gw_error_payload(&gw_error, Some(&msg)) }),
                        None,
                        Some(&correlation_id),
                        Some(&account),
                    ),
                    error: Some(msg),
                };
            }
        };
        drop(guard);

        let timed_call = tokio::time::timeout(
            Duration::from_secs(MCP_REQUEST_TIMEOUT_SECS),
            client.call_tool(tool, Some(args)),
        )
        .await;

        match timed_call {
            Ok(Ok(result)) => {
                let text: String = result
                    .content
                    .iter()
                    .filter_map(|c| c.text.as_deref())
                    .collect::<Vec<_>>()
                    .join("\n");
                if result.is_error {
                    tracing::warn!(
                        "[GW] tool '{}' returned MCP error: {}",
                        tool,
                        &text[..text.len().min(300)]
                    );
                    let gw_error = parse_gw_error(&text);
                    let user_error = gw_error.user_facing.clone();
                    ToolResult {
                        success: false,
                        data: envelope_result_with_meta(
                            tool,
                            serde_json::json!({ "error": gw_error_payload(&gw_error, Some(&text)) }),
                            Some(&text),
                            Some(&correlation_id),
                            Some(&account),
                        ),
                        error: Some(user_error),
                    }
                } else {
                    tracing::info!("[GW] tool '{}' succeeded ({} chars)", tool, text.len());
                    ToolResult {
                        success: true,
                        data: serde_json::json!(text),
                        error: None,
                    }
                }
            }
            Ok(Err(error)) => {
                let execution_error = ToolExecutionError::McpRequest {
                    tool: tool.to_string(),
                    reason: error.to_string(),
                };
                tracing::error!("[GW] tool '{}' call error: {}", tool, execution_error);
                let user_error = execution_error.to_string();
                let gw_error = mcp_transport_error(&user_error);
                ToolResult {
                    success: false,
                    data: envelope_result_with_meta(
                        tool,
                        serde_json::json!({ "error": gw_error_payload(&gw_error, Some(&user_error)) }),
                        None,
                        Some(&correlation_id),
                        Some(&account),
                    ),
                    error: Some(user_error),
                }
            }
            Err(_) => {
                let execution_error = ToolExecutionError::McpTimeout {
                    tool: tool.to_string(),
                    timeout_secs: MCP_REQUEST_TIMEOUT_SECS,
                };
                tracing::error!("[GW] tool '{}' call timeout: {}", tool, execution_error);
                let user_error = execution_error.to_string();
                let gw_error = GwErrorDescriptor {
                    code: "mcp_timeout",
                    category: "transient",
                    recovery_action: "retry",
                    retryable: true,
                    user_facing: user_error.clone(),
                };
                ToolResult {
                    success: false,
                    data: envelope_result_with_meta(
                        tool,
                        serde_json::json!({ "error": gw_error_payload(&gw_error, Some(&user_error)) }),
                        None,
                        Some(&correlation_id),
                        Some(&account),
                    ),
                    error: Some(user_error),
                }
            }
        }
    }

    async fn mcp_call(&self, tool: &str, args: serde_json::Value) -> ToolResult {
        let correlation_id = new_correlation_id();
        let account = active_google_account();
        let raw = self.mcp_call_raw(tool, args).await;
        if !raw.success {
            return raw;
        }

        let raw_text = raw.data.as_str().unwrap_or("");
        ToolResult {
            success: true,
            data: envelope_result_with_meta(
                tool,
                parse_json_or_text(raw_text),
                Some(raw_text),
                Some(&correlation_id),
                Some(&account),
            ),
            error: None,
        }
    }

    /// Fetch-then-buffer: MCP call → raw data → sidecar digest.
    async fn fetch_and_buffer(
        &self,
        mcp_tool: &str,
        mcp_args: serde_json::Value,
        sidecar_method: &str,
    ) -> ToolResult {
        let correlation_id = new_correlation_id();
        let account = active_google_account();
        tracing::debug!(
            "[GW] fetch_and_buffer: mcp_tool={} sidecar={}",
            mcp_tool,
            sidecar_method
        );
        let raw_result = self.mcp_call_raw(mcp_tool, mcp_args).await;
        if !raw_result.success {
            return raw_result;
        }
        let raw_text = raw_result.data.as_str().unwrap_or("").to_string();

        let buffer_params = serde_json::json!({ "raw": raw_result.data });
        let timed_sidecar_call = tokio::time::timeout(
            Duration::from_secs(SIDECAR_REQUEST_TIMEOUT_SECS),
            self.sidecar.request(sidecar_method, buffer_params),
        )
        .await;

        match timed_sidecar_call {
            Ok(Ok(digest)) => {
                tracing::info!("[GW] sidecar '{}' digest produced", sidecar_method);
                ToolResult {
                    success: true,
                    data: envelope_result_with_meta(
                        mcp_tool,
                        digest,
                        Some(&raw_text),
                        Some(&correlation_id),
                        Some(&account),
                    ),
                    error: None,
                }
            }
            Ok(Err(error)) => {
                let execution_error = ToolExecutionError::SidecarRequest {
                    method: sidecar_method.to_string(),
                    reason: error.to_string(),
                };
                tracing::warn!(
                    "[GW] sidecar '{}' failed ({}), returning raw",
                    sidecar_method,
                    execution_error
                );

                let mut fallback_data = parse_json_or_text(&raw_text);
                if let Some(object) = fallback_data.as_object_mut() {
                    object.insert(
                        "sidecar_warning".into(),
                        serde_json::json!(execution_error.to_string()),
                    );
                } else {
                    fallback_data = serde_json::json!({
                        "data": fallback_data,
                        "sidecar_warning": execution_error.to_string(),
                    });
                }

                ToolResult {
                    success: true,
                    data: envelope_result_with_meta(
                        mcp_tool,
                        fallback_data,
                        Some(&raw_text),
                        Some(&correlation_id),
                        Some(&account),
                    ),
                    error: None,
                }
            }
            Err(_) => {
                let execution_error = ToolExecutionError::SidecarTimeout {
                    method: sidecar_method.to_string(),
                    timeout_secs: SIDECAR_REQUEST_TIMEOUT_SECS,
                };
                tracing::warn!(
                    "[GW] sidecar '{}' failed ({}), returning raw",
                    sidecar_method,
                    execution_error
                );

                let mut fallback_data = parse_json_or_text(&raw_text);
                if let Some(object) = fallback_data.as_object_mut() {
                    object.insert(
                        "sidecar_warning".into(),
                        serde_json::json!(execution_error.to_string()),
                    );
                } else {
                    fallback_data = serde_json::json!({
                        "data": fallback_data,
                        "sidecar_warning": execution_error.to_string(),
                    });
                }

                ToolResult {
                    success: true,
                    data: envelope_result_with_meta(
                        mcp_tool,
                        fallback_data,
                        Some(&raw_text),
                        Some(&correlation_id),
                        Some(&account),
                    ),
                    error: None,
                }
            }
        }
    }

    async fn grounded_gmail_search(&self, query: String, requested_max: u64) -> ToolResult {
        let correlation_id = new_correlation_id();
        let account = active_google_account();
        let requested_count = requested_max.clamp(1, GMAIL_MAX_RESULTS_CAP);
        let page_size = requested_count.clamp(1, GMAIL_PAGE_SIZE_CAP);

        let mut pages_fetched = 0usize;
        let mut collected: Vec<serde_json::Value> = Vec::new();
        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut raw_pages: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut partial_error: Option<String> = None;

        let mut page_token: Option<String> = None;
        let mut has_more_results = false;
        let mut page_cap_reached = false;

        while (collected.len() as u64) < requested_count {
            if pages_fetched >= GMAIL_MAX_PAGE_FETCHES {
                page_cap_reached = true;
                break;
            }

            let mut args = serde_json::json!({
                "query": query,
                "maxResults": page_size,
            });
            if let Some(token) = page_token.clone() {
                args["pageToken"] = serde_json::Value::String(token);
            }

            let page_result = self.mcp_call_raw("searchGmail", args).await;
            if !page_result.success {
                if pages_fetched > 0
                    && should_ignore_gmail_page_token_error(page_result.error.as_deref())
                {
                    warnings.push(
                        "Gmail pagination token replay was rejected by upstream schema; returning grounded results from fetched page(s).".into(),
                    );
                    break;
                }

                if collected.is_empty() {
                    return page_result;
                }

                partial_error = page_result.error.clone();
                break;
            }

            pages_fetched += 1;

            let raw_text = page_result.data.as_str().unwrap_or("").to_string();
            raw_pages.push(raw_text.clone());

            let parsed = parse_json_or_text(&raw_text);
            let page_messages = gmail_messages_from_payload(&parsed);
            for message in page_messages {
                if let Some(id) = gmail_message_identifier(&message) {
                    if seen_ids.insert(id) {
                        collected.push(message);
                    }
                } else {
                    collected.push(message);
                }

                if (collected.len() as u64) >= requested_count {
                    break;
                }
            }

            page_token = gmail_next_page_token(&parsed);
            has_more_results = page_token.is_some();

            if (collected.len() as u64) >= requested_count || !has_more_results {
                break;
            }
        }

        let returned_count = collected.len() as u64;
        if returned_count < requested_count {
            warnings.push(format!(
                "Requested {requested_count} message(s), but only {returned_count} grounded message(s) were returned by Gmail."
            ));
        }
        if page_cap_reached {
            warnings.push(
                "Stopped Gmail retrieval after reaching safety page cap before satisfying full requested count.".into(),
            );
        }

        let mut data = serde_json::json!({
            "query": query,
            "messages": collected,
            "count": returned_count,
            "requested_count": requested_count,
            "returned_count": returned_count,
            "fully_satisfied": returned_count >= requested_count,
            "pages_fetched": pages_fetched,
            "page_size": page_size,
            "has_more_results": has_more_results,
            "pagination_exhausted": !has_more_results,
        });

        if let Some(token) = page_token {
            data["next_page_token"] = serde_json::Value::String(token);
        }
        if let Some(err) = partial_error {
            data["partial_error"] = serde_json::Value::String(err);
        }
        if !warnings.is_empty() {
            data["warnings"] = serde_json::Value::Array(
                warnings
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            );
        }

        let raw_text = raw_pages.join("\n");
        ToolResult {
            success: true,
            data: envelope_result_with_meta(
                "searchGmail",
                data,
                Some(&raw_text),
                Some(&correlation_id),
                Some(&account),
            ),
            error: None,
        }
    }
}

fn gmail_max_results(max_results: u64, default: u64) -> u64 {
    let normalized = if max_results == 0 {
        default
    } else {
        max_results
    };

    normalized.clamp(1, GMAIL_MAX_RESULTS_CAP)
}

fn normalize_gmail_inbox_query(query: Option<&str>) -> String {
    let trimmed = query.unwrap_or("").trim();
    if trimmed.is_empty() {
        return "in:inbox".into();
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("in:") {
        trimmed.to_string()
    } else {
        format!("in:inbox {trimmed}")
    }
}

// ── Error helpers ──────────────────────────────────────────────────────────────

type GwErrorDescriptor = gw_contract::GwErrorDescriptor;

fn gw_error_payload(error: &GwErrorDescriptor, raw: Option<&str>) -> serde_json::Value {
    gw_contract::error_payload(error, raw)
}

/// Convert verbose Google API error messages into structured, actionable metadata.
///
/// Google errors for "API not enabled" are typically hundreds of characters long
/// and contain a URL to fix the issue. This function extracts the key info so
/// KRIA can preserve compatibility (`error` string) while also emitting a typed
/// error envelope for downstream clients.
fn parse_gw_error(raw: &str) -> GwErrorDescriptor {
    gw_contract::parse_error(raw)
}

fn mcp_transport_error(raw: &str) -> GwErrorDescriptor {
    gw_contract::mcp_transport_error(raw)
}

// ── Gmail tools ────────────────────────────────────────────────────────────────
// Real tool names from `google-workspace-mcp` package (v2.x):
//   listGmailMessages, searchGmail, readGmailMessage,
//   createGmailDraft, sendGmailDraft, deleteGmailMessage

struct GwGmailInbox(GwBridge);
#[async_trait]
impl ToolHandler for GwGmailInbox {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: GmailInboxInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };

        // searchGmail returns sender, subject, date, labels, preview, and IDs.
        // That is much more useful for "check my inbox" flows than listGmailMessages,
        // which only returns IDs and links.
        let query = normalize_gmail_inbox_query(input.query.as_deref());
        let requested = gmail_max_results(input.max_results, default_gmail_max_results());
        self.0.grounded_gmail_search(query, requested).await
    }
}

struct GwGmailSearch(GwBridge);
#[async_trait]
impl ToolHandler for GwGmailSearch {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: GmailSearchInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.query, "query") {
            return err;
        }

        // searchGmail: account, query, maxResults?
        let requested = gmail_max_results(input.max_results, default_gmail_max_results());
        self.0.grounded_gmail_search(input.query, requested).await
    }
}

struct GwGmailRead(GwBridge);
#[async_trait]
impl ToolHandler for GwGmailRead {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: ReadEmailInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.message_id, "message_id") {
            return err;
        }

        // readGmailMessage: account, messageId
        let args = serde_json::json!({ "messageId": input.message_id });
        self.0.mcp_call("readGmailMessage", args).await
    }
}

struct GwGmailSend(GwBridge);
#[async_trait]
impl ToolHandler for GwGmailSend {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: SendEmailInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.to, "to") {
            return err;
        }

        // Safe send workflow: create draft first, then send it
        // Step 1: createGmailDraft
        // TODO (ADR Pillar 4): Implement pre-flight idempotency check when Sidecar API supports read-before-write
        let mut draft_args = serde_json::json!({
            "to": input.to,
            "subject": input.subject,
            "body": input.body,
        });
        if let Some(cc) = input.cc.filter(|value| !value.trim().is_empty()) {
            draft_args["cc"] = serde_json::json!(cc);
        }

        let draft_result = self.0.mcp_call("createGmailDraft", draft_args).await;
        if !draft_result.success {
            return draft_result;
        }

        if let Some(draft_id) = extract_gmail_draft_id(&draft_result.data) {
            return self
                .0
                .mcp_call("sendGmailDraft", serde_json::json!({ "draftId": draft_id }))
                .await;
        }

        // Fallback: return the draft result and let user know to send manually
        tracing::warn!(
            "[GW] could not extract draftId from createGmailDraft response — draft created but not sent"
        );
        ToolResult {
            success: false,
            data: draft_result.data,
            error: Some(
                "Draft created but could not auto-send: draftId not found in response. Check Gmail drafts."
                    .into(),
            ),
        }
    }
}

struct GwGmailDelete(GwBridge);
#[async_trait]
impl ToolHandler for GwGmailDelete {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: DeleteEmailInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.message_id, "message_id") {
            return err;
        }

        // TODO (ADR Pillar 4): Implement pre-flight idempotency check when Sidecar API supports read-before-write
        self.0
            .mcp_call(
                "deleteGmailMessage",
                serde_json::json!({ "messageId": input.message_id }),
            )
            .await
    }
}

// ── New Gmail tools ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DraftCreateInput {
    to: String,
    subject: String,
    body: String,
    #[serde(default)]
    cc: Option<String>,
    #[serde(default)]
    account: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SendDraftInput {
    draft_id: String,
    #[serde(default)]
    account: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct BulkSendInput {
    recipients: Vec<String>,
    subject: String,
    body: String,
    #[serde(default)]
    cc: Option<String>,
    #[serde(default)]
    account: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AccountSwitchInput {
    account: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CalendarUpdateInput {
    event_id: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    end: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    account: Option<String>,
}

const BULK_SEND_CAP: usize = 50;

fn gmail_preview_markdown(to: &str, subject: &str, body: &str, cc: Option<&str>) -> String {
    let cc_line = cc
        .filter(|c| !c.trim().is_empty())
        .map(|c| format!("**Cc:** {c}\n"))
        .unwrap_or_default();
    format!("**To:** {to}\n{cc_line}**Subject:** {subject}\n\n{body}")
}


struct GwGmailDraftCreate(GwBridge);
#[async_trait]
impl ToolHandler for GwGmailDraftCreate {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: DraftCreateInput = match parse_input(params) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if let Err(e) = require_non_empty(&input.to, "to") {
            return e;
        }
        let mut args = serde_json::json!({
            "to": input.to,
            "subject": input.subject,
            "body": input.body,
        });
        if let Some(cc) = input.cc.as_deref().filter(|c| !c.trim().is_empty()) {
            args["cc"] = serde_json::json!(cc);
        }
        if let Some(acc) = input.account.as_deref().filter(|a| !a.trim().is_empty()) {
            args["account"] = serde_json::json!(acc);
        }
        let res = self.0.mcp_call("createGmailDraft", args).await;
        if !res.success {
            return res;
        }
        let draft_id = extract_gmail_draft_id(&res.data);
        let preview =
            gmail_preview_markdown(&input.to, &input.subject, &input.body, input.cc.as_deref());
        ToolResult::ok(serde_json::json!({
            "draft_id": draft_id,
            "to": input.to,
            "subject": input.subject,
            "body": input.body,
            "preview_markdown": preview,
            "sent": false,
            "hint": "Use gw_gmail_send_draft with this draft_id to send.",
        }))
    }
}

struct GwGmailSendDraft(GwBridge);
#[async_trait]
impl ToolHandler for GwGmailSendDraft {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: SendDraftInput = match parse_input(params) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if let Err(e) = require_non_empty(&input.draft_id, "draft_id") {
            return e;
        }
        let mut args = serde_json::json!({ "draftId": input.draft_id });
        if let Some(acc) = input.account.as_deref().filter(|a| !a.trim().is_empty()) {
            args["account"] = serde_json::json!(acc);
        }
        self.0.mcp_call("sendGmailDraft", args).await
    }
}

struct GwGmailSendBulk(GwBridge);
#[async_trait]
impl ToolHandler for GwGmailSendBulk {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: BulkSendInput = match parse_input(params) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let recipients: Vec<String> = input
            .recipients
            .into_iter()
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty())
            .collect();
        if recipients.is_empty() {
            return ToolResult::err("recipients must be a non-empty list");
        }
        if recipients.len() > BULK_SEND_CAP {
            return ToolResult::err(format!(
                "bulk send capped at {BULK_SEND_CAP} recipients (got {})",
                recipients.len()
            ));
        }

        let mut results = Vec::new();
        let mut sent_count = 0usize;
        for to in &recipients {
            let mut draft_args = serde_json::json!({
                "to": to,
                "subject": input.subject,
                "body": input.body,
            });
            if let Some(cc) = input.cc.as_deref().filter(|c| !c.trim().is_empty()) {
                draft_args["cc"] = serde_json::json!(cc);
            }
            if let Some(acc) = input.account.as_deref().filter(|a| !a.trim().is_empty()) {
                draft_args["account"] = serde_json::json!(acc);
            }
            let draft = self.0.mcp_call("createGmailDraft", draft_args).await;
            if !draft.success {
                results.push(serde_json::json!({ "to": to, "sent": false, "error": draft.error }));
                continue;
            }
            match extract_gmail_draft_id(&draft.data) {
                Some(id) => {
                    let mut send_args = serde_json::json!({ "draftId": id });
                    if let Some(acc) = input.account.as_deref().filter(|a| !a.trim().is_empty()) {
                        send_args["account"] = serde_json::json!(acc);
                    }
                    let sent = self.0.mcp_call("sendGmailDraft", send_args).await;
                    if sent.success {
                        sent_count += 1;
                        results.push(serde_json::json!({ "to": to, "sent": true }));
                    } else {
                        results.push(serde_json::json!({ "to": to, "sent": false, "error": sent.error }));
                    }
                }
                None => results.push(serde_json::json!({
                    "to": to, "sent": false, "error": "draftId not found"
                })),
            }
        }

        ToolResult::ok(serde_json::json!({
            "total": recipients.len(),
            "sent_count": sent_count,
            "failed_count": recipients.len() - sent_count,
            "subject": input.subject,
            "results": results,
        }))
    }
}

struct GwAccountSwitch;
#[async_trait]
impl ToolHandler for GwAccountSwitch {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: AccountSwitchInput = match parse_input(params) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if let Err(e) = require_non_empty(&input.account, "account") {
            return e;
        }
        std::env::set_var("KRIA_GW_ACCOUNT", input.account.trim());
        ToolResult::ok(serde_json::json!({
            "active_account": input.account.trim(),
            "note": "Default Google account switched for subsequent tool calls.",
        }))
    }
}

struct GwCalendarUpdate(GwBridge);
#[async_trait]
impl ToolHandler for GwCalendarUpdate {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: CalendarUpdateInput = match parse_input(params) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if let Err(e) = require_non_empty(&input.event_id, "event_id") {
            return e;
        }
        let mut args = serde_json::json!({ "eventId": input.event_id });
        if let Some(s) = input.summary.as_deref().filter(|v| !v.trim().is_empty()) {
            args["summary"] = serde_json::json!(s);
        }
        if let Some(s) = input.start.as_deref().filter(|v| !v.trim().is_empty()) {
            args["start"] = serde_json::json!({ "dateTime": s });
        }
        if let Some(s) = input.end.as_deref().filter(|v| !v.trim().is_empty()) {
            args["end"] = serde_json::json!({ "dateTime": s });
        }
        if let Some(s) = input.description.as_deref() {
            args["description"] = serde_json::json!(s);
        }
        if let Some(s) = input.location.as_deref() {
            args["location"] = serde_json::json!(s);
        }
        if let Some(acc) = input.account.as_deref().filter(|a| !a.trim().is_empty()) {
            args["account"] = serde_json::json!(acc);
        }
        self.0.mcp_call("updateCalendarEvent", args).await
    }
}

struct GwGmailReply(GwBridge);
#[async_trait]
impl ToolHandler for GwGmailReply {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: ReplyEmailInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.message_id, "message_id") {
            return err;
        }
        if let Err(err) = require_non_empty(&input.body, "body") {
            return err;
        }

        // Step 1: Read the original message to get thread/reply metadata
        let read_result = self
            .0
            .mcp_call_raw(
                "readGmailMessage",
                serde_json::json!({ "messageId": input.message_id }),
            )
            .await;

        if !read_result.success {
            return ToolResult {
                success: false,
                data: read_result.data,
                error: Some(format!(
                    "Could not read original message to reply: {}",
                    read_result.error.unwrap_or_default()
                )),
            };
        }

        let raw_text = read_result.data.as_str().unwrap_or("").to_string();
        let parsed = parse_json_or_text(&raw_text);

        // Extract thread ID and original sender for reply
        let thread_id = find_string_field_recursive(&parsed, "threadId")
            .or_else(|| find_string_field_recursive(&parsed, "thread_id"));
        let from = find_string_field_recursive(&parsed, "from")
            .or_else(|| find_string_field_recursive(&parsed, "sender"));
        let subject = find_string_field_recursive(&parsed, "subject")
            .unwrap_or_else(|| "Re: (no subject)".to_string());

        let reply_subject = if subject.to_ascii_lowercase().starts_with("re:") {
            subject
        } else {
            format!("Re: {}", subject)
        };

        // Build reply draft
        let mut draft_args = serde_json::json!({
            "subject": reply_subject,
            "body": input.body,
            "replyToMessageId": input.message_id,
        });

        if let Some(to) = from {
            draft_args["to"] = serde_json::json!(to);
        }
        if let Some(tid) = thread_id {
            draft_args["threadId"] = serde_json::json!(tid);
        }
        if input.reply_all {
            draft_args["replyAll"] = serde_json::json!(true);
        }

        // Step 2: Create draft reply
        let draft_result = self.0.mcp_call("createGmailDraft", draft_args).await;
        if !draft_result.success {
            return draft_result;
        }

        // Step 3: Send the draft
        if let Some(draft_id) = extract_gmail_draft_id(&draft_result.data) {
            return self
                .0
                .mcp_call("sendGmailDraft", serde_json::json!({ "draftId": draft_id }))
                .await;
        }

        ToolResult {
            success: false,
            data: draft_result.data,
            error: Some("Reply draft created but could not auto-send: draftId not found.".into()),
        }
    }
}

struct GwGmailMarkRead(GwBridge);
#[async_trait]
impl ToolHandler for GwGmailMarkRead {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: MarkEmailInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.message_id, "message_id") {
            return err;
        }

        // Use modifyGmailMessage to add/remove UNREAD label
        let (add_labels, remove_labels): (Vec<&str>, Vec<&str>) = if input.read {
            (vec![], vec!["UNREAD"])
        } else {
            (vec!["UNREAD"], vec![])
        };

        self.0
            .mcp_call(
                "modifyGmailMessage",
                serde_json::json!({
                    "messageId": input.message_id,
                    "addLabelIds": add_labels,
                    "removeLabelIds": remove_labels,
                }),
            )
            .await
    }
}

struct GwGmailLabel(GwBridge);
#[async_trait]
impl ToolHandler for GwGmailLabel {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: LabelEmailInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.message_id, "message_id") {
            return err;
        }
        if let Err(err) = require_non_empty(&input.label, "label") {
            return err;
        }

        self.0
            .mcp_call(
                "modifyGmailMessage",
                serde_json::json!({
                    "messageId": input.message_id,
                    "addLabelIds": [input.label.to_uppercase()],
                    "removeLabelIds": [],
                }),
            )
            .await
    }
}

// ── New Drive tools ────────────────────────────────────────────────────────────

struct GwDriveCreateFile(GwBridge);
#[async_trait]
impl ToolHandler for GwDriveCreateFile {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: DriveCreateFileInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.name, "name") {
            return err;
        }

        let mut args = serde_json::json!({ "name": input.name });
        if let Some(content) = input.content.filter(|c| !c.is_empty()) {
            args["content"] = serde_json::json!(content);
        }
        if let Some(folder_id) = input.folder_id.filter(|f| !f.is_empty()) {
            args["folderId"] = serde_json::json!(folder_id);
        }
        if let Some(mime_type) = input.mime_type.filter(|m| !m.is_empty()) {
            args["mimeType"] = serde_json::json!(mime_type);
        }

        self.0.mcp_call("createDriveFile", args).await
    }
}

struct GwDriveCreateFolder(GwBridge);
#[async_trait]
impl ToolHandler for GwDriveCreateFolder {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: DriveCreateFolderInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.name, "name") {
            return err;
        }

        let mut args = serde_json::json!({
            "name": input.name,
            "mimeType": "application/vnd.google-apps.folder",
        });
        if let Some(parent_id) = input.parent_folder_id.filter(|p| !p.is_empty()) {
            args["folderId"] = serde_json::json!(parent_id);
        }

        self.0.mcp_call("createDriveFile", args).await
    }
}

struct GwDriveMove(GwBridge);
#[async_trait]
impl ToolHandler for GwDriveMove {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: DriveMoveInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.file_id, "file_id") {
            return err;
        }
        if let Err(err) = require_non_empty(&input.target_folder_id, "target_folder_id") {
            return err;
        }

        self.0
            .mcp_call(
                "moveDriveFile",
                serde_json::json!({
                    "fileId": input.file_id,
                    "targetFolderId": input.target_folder_id,
                }),
            )
            .await
    }
}

struct GwDriveRename(GwBridge);
#[async_trait]
impl ToolHandler for GwDriveRename {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: DriveRenameInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.file_id, "file_id") {
            return err;
        }
        if let Err(err) = require_non_empty(&input.new_name, "new_name") {
            return err;
        }

        self.0
            .mcp_call(
                "renameDriveFile",
                serde_json::json!({
                    "fileId": input.file_id,
                    "newName": input.new_name,
                }),
            )
            .await
    }
}

struct GwCalendarToday(GwBridge);
#[async_trait]
impl ToolHandler for GwCalendarToday {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let _: EmptyInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };

        // listCalendarEvents with today's date range
        let now = chrono::Utc::now();
        let start = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .to_rfc3339();
        let end = now
            .date_naive()
            .and_hms_opt(23, 59, 59)
            .unwrap()
            .and_utc()
            .to_rfc3339();
        let args = serde_json::json!({
            "timeMin": start,
            "timeMax": end,
            "maxResults": default_calendar_today_max_results(),
        });
        self.0.mcp_call("listCalendarEvents", args).await
    }
}

struct GwCalendarSearch(GwBridge);
#[async_trait]
impl ToolHandler for GwCalendarSearch {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: CalendarSearchInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };

        let max_results = if input.max_results == 0 {
            default_calendar_max_results()
        } else {
            input.max_results
        };

        let mut args = serde_json::json!({ "maxResults": max_results });
        if let Some(q) = input.query.filter(|value| !value.trim().is_empty()) {
            args["q"] = serde_json::json!(q);
        }
        if let Some(t) = input.time_min.filter(|value| !value.trim().is_empty()) {
            args["timeMin"] = serde_json::json!(t);
        }
        if let Some(t) = input.time_max.filter(|value| !value.trim().is_empty()) {
            args["timeMax"] = serde_json::json!(t);
        }
        self.0.mcp_call("listCalendarEvents", args).await
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AvailabilityInput {
    #[serde(default)]
    time_min: Option<String>,
    #[serde(default)]
    time_max: Option<String>,
    #[serde(default)]
    min_slot_minutes: Option<i64>,
}

fn parse_rfc3339_utc(value: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

fn start_of_today_utc() -> chrono::DateTime<chrono::Utc> {
    let now = chrono::Utc::now();
    let midnight = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("valid midnight");
    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(midnight, chrono::Utc)
}

/// Compute free slots + scheduling conflicts for a time window.
struct GwCalendarAvailability(GwBridge);
#[async_trait]
impl ToolHandler for GwCalendarAvailability {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: AvailabilityInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };

        let now = chrono::Utc::now();
        let window_start = parse_rfc3339_utc(input.time_min.as_deref()).unwrap_or(now);
        let window_end = parse_rfc3339_utc(input.time_max.as_deref())
            .unwrap_or_else(|| window_start + chrono::Duration::hours(24));
        let min_slot = input.min_slot_minutes.unwrap_or(15).max(1);

        if window_end <= window_start {
            return ToolResult::err("time_max must be after time_min".to_string());
        }

        let args = serde_json::json!({
            "timeMin": window_start.to_rfc3339(),
            "timeMax": window_end.to_rfc3339(),
            "maxResults": 250,
        });
        let raw = self.0.mcp_call_raw("listCalendarEvents", args).await;
        if !raw.success {
            return raw;
        }
        let raw_text = raw.data.as_str().unwrap_or("").to_string();
        let payload = parse_json_or_text(&raw_text);
        let events = availability::parse_google_events(&payload);
        let free = availability::free_slots(window_start, window_end, &events, min_slot);
        let conflicts = availability::detect_conflicts(&events);

        let data = serde_json::json!({
            "window": {
                "start": window_start.to_rfc3339(),
                "end": window_end.to_rfc3339(),
            },
            "min_slot_minutes": min_slot,
            "busy_count": events.len(),
            "free_slots": free,
            "conflicts": conflicts,
        });
        ToolResult {
            success: true,
            data: envelope_result("gw_calendar_availability", data, Some(&raw_text)),
            error: None,
        }
    }
}

/// Best-effort GitHub section for the morning briefing. Calls the GitHub MCP
/// (if connected) via a configurable read tool (`KRIA_GH_BRIEFING_TOOL`,
/// default `list_notifications`). Always degrades gracefully — never fails the
/// briefing if GitHub is unavailable or the tool name doesn't match.
async fn github_briefing_section(
    github: &GhClientRef,
    tool_override: Option<&str>,
) -> serde_json::Value {
    let client = {
        let guard = github.read().await;
        match guard.as_ref() {
            Some(c) => c.clone(),
            None => return serde_json::json!({ "connected": false }),
        }
    };

    let tool = tool_override
        .map(|s| s.to_string())
        .or_else(|| std::env::var("KRIA_GH_BRIEFING_TOOL").ok())
        .unwrap_or_else(|| "list_notifications".to_string());

    let call = tokio::time::timeout(
        Duration::from_secs(MCP_REQUEST_TIMEOUT_SECS),
        client.call_tool(&tool, Some(serde_json::json!({}))),
    )
    .await;

    match call {
        Ok(Ok(result)) => {
            let text: String = result
                .content
                .iter()
                .filter_map(|c| c.text.as_deref())
                .collect::<Vec<_>>()
                .join("\n");
            if result.is_error {
                serde_json::json!({ "connected": true, "tool": tool, "error": text })
            } else {
                serde_json::json!({ "connected": true, "tool": tool, "data": parse_json_or_text(&text) })
            }
        }
        Ok(Err(error)) => {
            serde_json::json!({ "connected": true, "tool": tool, "error": error.to_string() })
        }
        Err(_) => serde_json::json!({ "connected": true, "tool": tool, "error": "github mcp timeout" }),
    }
}

async fn briefing_gmail_section(gw: &GwBridge, query: &str, max: u64) -> serde_json::Value {
    let res = gw.grounded_gmail_search(query.to_string(), max).await;
    serde_json::json!({ "ok": res.success, "query": query, "data": res.data })
}

async fn briefing_calendar_section(
    gw: &GwBridge,
    window: &str,
    include_conflicts: bool,
) -> serde_json::Value {
    let (win_start, win_end) = if window == "next24h" {
        let now = chrono::Utc::now();
        (now, now + chrono::Duration::hours(24))
    } else {
        let start = start_of_today_utc();
        (start, start + chrono::Duration::days(1))
    };
    let args = serde_json::json!({
        "timeMin": win_start.to_rfc3339(),
        "timeMax": win_end.to_rfc3339(),
        "maxResults": 50,
    });
    let raw = gw.mcp_call_raw("listCalendarEvents", args).await;
    let events = if raw.success {
        availability::parse_google_events(&parse_json_or_text(raw.data.as_str().unwrap_or("")))
    } else {
        Vec::new()
    };
    let conflicts = if include_conflicts {
        availability::detect_conflicts(&events)
    } else {
        Vec::new()
    };
    serde_json::json!({
        "window": window,
        "event_count": events.len(),
        "events": events,
        "conflicts": conflicts,
    })
}

fn briefing_tasks_section(filter: &str) -> serde_json::Value {
    let paths = crate::platform::paths::KriaPaths::resolve();
    let store = match crate::tasks::TaskStore::open(&paths.db_path) {
        Ok(s) => s,
        Err(e) => return serde_json::json!({ "error": e.to_string() }),
    };
    let tasks = match filter {
        "all" => store.list_tasks(&crate::tasks::TaskFilter::default()),
        _ => store.list_tasks(&crate::tasks::TaskFilter {
            active_only: true,
            ..Default::default()
        }),
    }
    .unwrap_or_default();

    let now = chrono::Utc::now();
    let selected: Vec<_> = if filter == "urgent_and_overdue" {
        tasks
            .into_iter()
            .filter(|t| {
                t.priority_bucket == "urgent"
                    || t.due_at.map(|d| d < now).unwrap_or(false)
            })
            .collect()
    } else {
        tasks
    };
    serde_json::json!({ "count": selected.len(), "tasks": selected })
}

/// Configurable daily briefing — reads the user's BriefingConfig and renders
/// each enabled section (gmail / calendar / github / tasks).
struct GwMorningBriefing {
    gw: GwBridge,
    github: GhClientRef,
}
#[async_trait]
impl ToolHandler for GwMorningBriefing {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let _: EmptyInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };

        let paths = crate::platform::paths::KriaPaths::resolve();
        let config = crate::briefing::BriefingStore::open(&paths.db_path)
            .map(|s| s.get())
            .unwrap_or_default();

        let mut sections_out = Vec::new();
        for section in config.sections.iter().filter(|s| s.enabled) {
            let block = match section.source.as_str() {
                "gmail" => {
                    briefing_gmail_section(
                        &self.gw,
                        section.query.as_deref().unwrap_or("is:unread"),
                        section.max.unwrap_or(10),
                    )
                    .await
                }
                "calendar" => {
                    briefing_calendar_section(
                        &self.gw,
                        section.window.as_deref().unwrap_or("today"),
                        section.include_conflicts.unwrap_or(true),
                    )
                    .await
                }
                "github" => github_briefing_section(&self.github, section.tool.as_deref()).await,
                "tasks" => {
                    briefing_tasks_section(section.filter.as_deref().unwrap_or("urgent_and_overdue"))
                }
                other => serde_json::json!({ "error": format!("unknown source: {other}") }),
            };
            sections_out.push(serde_json::json!({
                "source": section.source,
                "data": block,
            }));
        }

        let data = serde_json::json!({
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "section_count": sections_out.len(),
            "sections": sections_out,
        });
        ToolResult {
            success: true,
            data: envelope_result("gw_morning_briefing", data, None),
            error: None,
        }
    }
}
struct GwCalendarCreate(GwBridge);
#[async_trait]
impl ToolHandler for GwCalendarCreate {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: CreateCalendarEventInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.summary, "summary") {
            return err;
        }
        if let Err(err) = require_non_empty(&input.start, "start") {
            return err;
        }
        if let Err(err) = require_non_empty(&input.end, "end") {
            return err;
        }

        // TODO (ADR Pillar 4): Implement pre-flight idempotency check when Sidecar API supports read-before-write
        let primary_args = calendar_create_args(&input, false);
        let primary_result = self.0.mcp_call("createCalendarEvent", primary_args).await;
        if primary_result.success
            || !should_retry_calendar_with_alternate_shape(primary_result.error.as_deref())
        {
            return primary_result;
        }

        tracing::warn!(
            "[GW] calendar create primary argument shape failed; retrying with alternate datetime shape"
        );
        let alternate_result = self
            .0
            .mcp_call("createCalendarEvent", calendar_create_args(&input, true))
            .await;
        if alternate_result.success {
            return alternate_result;
        }

        let merged_error = match (
            primary_result.error.as_deref(),
            alternate_result.error.as_deref(),
        ) {
            (Some(primary), Some(alternate)) => {
                format!("{primary} (alternate argument retry failed: {alternate})")
            }
            (Some(primary), None) => primary.to_string(),
            (None, Some(alternate)) => alternate.to_string(),
            (None, None) => "Calendar create failed for both supported argument shapes".to_string(),
        };

        ToolResult {
            success: false,
            data: alternate_result.data,
            error: Some(merged_error),
        }
    }
}

struct GwCalendarDelete(GwBridge);
#[async_trait]
impl ToolHandler for GwCalendarDelete {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: DeleteCalendarEventInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.event_id, "event_id") {
            return err;
        }

        // TODO (ADR Pillar 4): Implement pre-flight idempotency check when Sidecar API supports read-before-write
        self.0
            .mcp_call(
                "deleteCalendarEvent",
                serde_json::json!({ "eventId": input.event_id }),
            )
            .await
    }
}

// ── Drive tools ────────────────────────────────────────────────────────────────
// Real tool names: searchGoogleDocs, listFolderContents, readGoogleDoc, deleteFile

struct GwDriveSearch(GwBridge);
#[async_trait]
impl ToolHandler for GwDriveSearch {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: DriveSearchInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };

        if input.query.trim().is_empty() || looks_like_drive_listing_phrase(&input.query) {
            return self
                .0
                .fetch_and_buffer(
                    "listFolderContents",
                    serde_json::json!({}),
                    "google.summarize_drive_folder",
                )
                .await;
        }

        let args = serde_json::json!({ "query": input.query });
        self.0
            .fetch_and_buffer("searchGoogleDocs", args, "google.summarize_drive_folder")
            .await
    }
}

struct GwDriveList(GwBridge);
#[async_trait]
impl ToolHandler for GwDriveList {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: DriveListInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };

        let args = if let Some(id) = input.folder_id.filter(|value| !value.trim().is_empty()) {
            serde_json::json!({ "folderId": id })
        } else {
            serde_json::json!({})
        };
        self.0
            .fetch_and_buffer("listFolderContents", args, "google.summarize_drive_folder")
            .await
    }
}

struct GwDriveRead(GwBridge);
#[async_trait]
impl ToolHandler for GwDriveRead {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: DriveReadInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.file_id, "file_id") {
            return err;
        }

        // Try as a Doc first; format=text is safe for all readable files
        let args = serde_json::json!({ "documentId": input.file_id, "format": "text" });
        self.0.mcp_call("readGoogleDoc", args).await
    }
}

struct GwDriveDelete(GwBridge);
#[async_trait]
impl ToolHandler for GwDriveDelete {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: DriveDeleteInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.file_id, "file_id") {
            return err;
        }

        // TODO (ADR Pillar 4): Implement pre-flight idempotency check when Sidecar API supports read-before-write
        self.0
            .mcp_call("deleteFile", serde_json::json!({ "fileId": input.file_id }))
            .await
    }
}

// ── Docs tools ─────────────────────────────────────────────────────────────────
// Real tool names: readGoogleDoc, createDocument, appendToGoogleDoc

struct GwDocsRead(GwBridge);
#[async_trait]
impl ToolHandler for GwDocsRead {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: DocsReadInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.document_id, "document_id") {
            return err;
        }

        let args = serde_json::json!({ "documentId": input.document_id, "format": "markdown" });
        self.0
            .fetch_and_buffer("readGoogleDoc", args, "google.extract_doc")
            .await
    }
}

struct GwDocsCreate(GwBridge);
#[async_trait]
impl ToolHandler for GwDocsCreate {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: CreateDocumentInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        let title = input.title;

        // TODO (ADR Pillar 4): Implement pre-flight idempotency check when Sidecar API supports read-before-write
        let create_result = self
            .0
            .mcp_call_raw(
                "createDocument",
                serde_json::json!({ "title": title.clone() }),
            )
            .await;
        if !create_result.success {
            return create_result;
        }

        let create_raw = create_result.data.as_str().unwrap_or("").to_string();
        let create_data = parse_json_or_text(&create_raw);

        let document_id = extract_google_resource_id(
            &create_data,
            &["documentId", "document_id", "id"],
            &["url", "documentUrl", "document_link", "link", "webViewLink"],
            "/document/d/",
        );

        let mut result_data = serde_json::json!({
            "resource": "document",
            "title": title.clone(),
            "status": "created_unverified",
            "verified": false,
            "create": create_data,
            "document_id": document_id,
            "url": document_id
                .as_deref()
                .and_then(|id| build_google_resource_url("document", id)),
        });

        if let Some(id) = document_id {
            let verify_result = self
                .0
                .mcp_call_raw(
                    "readGoogleDoc",
                    serde_json::json!({ "documentId": id, "format": "markdown" }),
                )
                .await;

            if verify_result.success {
                result_data["status"] = serde_json::json!("created_verified");
                result_data["verified"] = serde_json::json!(true);
                result_data["verify"] =
                    parse_json_or_text(verify_result.data.as_str().unwrap_or(""));
            } else {
                result_data["verification_error"] = serde_json::json!(verify_result
                    .error
                    .unwrap_or_else(|| "Document verification failed after create".into()));
            }
        } else {
            result_data["verification_error"] = serde_json::json!(
                "Could not extract document ID from create response for post-create verification"
            );
        }

        ToolResult {
            success: true,
            data: envelope_result("createDocument", result_data, Some(&create_raw)),
            error: None,
        }
    }
}

struct GwDocsEdit(GwBridge);
#[async_trait]
impl ToolHandler for GwDocsEdit {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: EditDocumentInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.document_id, "document_id") {
            return err;
        }

        // Default to append operation
        // TODO (ADR Pillar 4): Implement pre-flight idempotency check when Sidecar API supports read-before-write
        self.0
            .mcp_call(
                "appendToGoogleDoc",
                serde_json::json!({ "documentId": input.document_id, "text": input.text }),
            )
            .await
    }
}

// ── Sheets tools ───────────────────────────────────────────────────────────────
// Real tool names: readSpreadsheet, createSpreadsheet, writeSpreadsheet

struct GwSheetsRead(GwBridge);
#[async_trait]
impl ToolHandler for GwSheetsRead {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: SheetsReadInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.spreadsheet_id, "spreadsheet_id") {
            return err;
        }

        let mut args = serde_json::json!({ "spreadsheetId": input.spreadsheet_id });
        if let Some(r) = input.range.filter(|value| !value.trim().is_empty()) {
            args["range"] = serde_json::json!(r);
        }
        self.0
            .fetch_and_buffer("readSpreadsheet", args, "google.extract_sheet")
            .await
    }
}

struct GwSheetsCreate(GwBridge);
#[async_trait]
impl ToolHandler for GwSheetsCreate {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: CreateSpreadsheetInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        let title = input.title;

        // TODO (ADR Pillar 4): Implement pre-flight idempotency check when Sidecar API supports read-before-write
        let create_result = self
            .0
            .mcp_call_raw(
                "createSpreadsheet",
                serde_json::json!({ "title": title.clone() }),
            )
            .await;
        if !create_result.success {
            return create_result;
        }

        let create_raw = create_result.data.as_str().unwrap_or("").to_string();
        let create_data = parse_json_or_text(&create_raw);

        let spreadsheet_id = extract_google_resource_id(
            &create_data,
            &["spreadsheetId", "spreadsheet_id", "id"],
            &[
                "url",
                "spreadsheetUrl",
                "spreadsheet_link",
                "link",
                "webViewLink",
            ],
            "/spreadsheets/d/",
        );

        let mut result_data = serde_json::json!({
            "resource": "spreadsheet",
            "title": title.clone(),
            "status": "created_unverified",
            "verified": false,
            "create": create_data,
            "spreadsheet_id": spreadsheet_id,
            "url": spreadsheet_id
                .as_deref()
                .and_then(|id| build_google_resource_url("spreadsheet", id)),
        });

        if let Some(id) = spreadsheet_id {
            let verify_result = self
                .0
                .mcp_call_raw(
                    "readSpreadsheet",
                    serde_json::json!({ "spreadsheetId": id }),
                )
                .await;

            if verify_result.success {
                result_data["status"] = serde_json::json!("created_verified");
                result_data["verified"] = serde_json::json!(true);
                result_data["verify"] =
                    parse_json_or_text(verify_result.data.as_str().unwrap_or(""));
            } else {
                result_data["verification_error"] = serde_json::json!(verify_result
                    .error
                    .unwrap_or_else(|| "Spreadsheet verification failed after create".into()));
            }
        } else {
            result_data["verification_error"] = serde_json::json!(
                "Could not extract spreadsheet ID from create response for post-create verification"
            );
        }

        ToolResult {
            success: true,
            data: envelope_result("createSpreadsheet", result_data, Some(&create_raw)),
            error: None,
        }
    }
}

struct GwSheetsEdit(GwBridge);
#[async_trait]
impl ToolHandler for GwSheetsEdit {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: EditSpreadsheetInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.spreadsheet_id, "spreadsheet_id") {
            return err;
        }

        let values_str = input.values;
        let values: serde_json::Value =
            serde_json::from_str(&values_str).unwrap_or(serde_json::json!([]));

        // TODO (ADR Pillar 4): Implement pre-flight idempotency check when Sidecar API supports read-before-write
        self.0
            .mcp_call(
                "writeSpreadsheet",
                serde_json::json!({
                    "spreadsheetId": input.spreadsheet_id, "range": input.range, "values": values
                }),
            )
            .await
    }
}

// ── Slides tools ───────────────────────────────────────────────────────────────
// Real tool names: readPresentation, createPresentation

struct GwSlidesRead(GwBridge);
#[async_trait]
impl ToolHandler for GwSlidesRead {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: SlidesReadInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        if let Err(err) = require_non_empty(&input.presentation_id, "presentation_id") {
            return err;
        }

        self.0
            .fetch_and_buffer(
                "readPresentation",
                serde_json::json!({ "presentationId": input.presentation_id }),
                "google.extract_slides",
            )
            .await
    }
}

struct GwSlidesCreate(GwBridge);
#[async_trait]
impl ToolHandler for GwSlidesCreate {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: CreatePresentationInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };
        let title = input.title;

        // TODO (ADR Pillar 4): Implement pre-flight idempotency check when Sidecar API supports read-before-write
        let create_result = self
            .0
            .mcp_call_raw(
                "createPresentation",
                serde_json::json!({ "title": title.clone() }),
            )
            .await;
        if !create_result.success {
            return create_result;
        }

        let create_raw = create_result.data.as_str().unwrap_or("").to_string();
        let create_data = parse_json_or_text(&create_raw);

        let presentation_id = extract_google_resource_id(
            &create_data,
            &["presentationId", "presentation_id", "id"],
            &[
                "url",
                "presentationUrl",
                "presentation_link",
                "link",
                "webViewLink",
            ],
            "/presentation/d/",
        );

        let mut result_data = serde_json::json!({
            "resource": "presentation",
            "title": title.clone(),
            "status": "created_unverified",
            "verified": false,
            "create": create_data,
            "presentation_id": presentation_id,
            "url": presentation_id
                .as_deref()
                .and_then(|id| build_google_resource_url("presentation", id)),
        });

        if let Some(id) = presentation_id {
            let verify_result = self
                .0
                .mcp_call_raw(
                    "readPresentation",
                    serde_json::json!({ "presentationId": id }),
                )
                .await;

            if verify_result.success {
                result_data["status"] = serde_json::json!("created_verified");
                result_data["verified"] = serde_json::json!(true);
                result_data["verify"] =
                    parse_json_or_text(verify_result.data.as_str().unwrap_or(""));
            } else {
                result_data["verification_error"] = serde_json::json!(verify_result
                    .error
                    .unwrap_or_else(|| "Presentation verification failed after create".into()));
            }
        } else {
            result_data["verification_error"] = serde_json::json!(
                "Could not extract presentation ID from create response for post-create verification"
            );
        }

        ToolResult {
            success: true,
            data: envelope_result("createPresentation", result_data, Some(&create_raw)),
            error: None,
        }
    }
}

// ── Forms tools ───────────────────────────────────────────────────────────────
// Real tool names: listForms, createForm

struct GwFormsList(GwBridge);
#[async_trait]
impl ToolHandler for GwFormsList {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: FormsListInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };

        let mut args = serde_json::json!({});
        if let Some(query) = input.query.filter(|value| !value.trim().is_empty()) {
            args["query"] = serde_json::json!(query);
        }
        self.0.mcp_call("listForms", args).await
    }
}

struct GwFormsCreate(GwBridge);
#[async_trait]
impl ToolHandler for GwFormsCreate {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: CreateFormInput = match parse_input(params) {
            Ok(value) => value,
            Err(err) => return err,
        };

        // TODO (ADR Pillar 4): Implement pre-flight idempotency check when Sidecar API supports read-before-write
        self.0
            .mcp_call("createForm", serde_json::json!({ "title": input.title }))
            .await
    }
}

// ── Registration ───────────────────────────────────────────────────────────────

/// Register all Google Workspace tools.
///
/// Always registers all curated Google Workspace tools regardless of whether the MCP server is up.
/// Pass the `GwClientRef` returned by `new_client_ref()`; call `set_client()`
/// after the MCP server connects so handlers start forwarding requests.
pub fn register(reg: &ToolRegistry, mcp_ref: GwClientRef, github_ref: GhClientRef, sidecar: Arc<SidecarBridge>) {
    tracing::info!(
        "[GW] registering Google Workspace tools (account source=KRIA_GW_ACCOUNT, lazy MCP ref)"
    );

    let gw = GwBridge {
        mcp: mcp_ref,
        sidecar,
    };

    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        // ── Ambient (always mounted) ─────────────────────
        (
            ToolDef {
                name: "gw_gmail_inbox".into(),
                description: "List recent emails from Gmail inbox. USE THIS to check inbox, see recent mail, or list emails. Returns sender, subject, date, preview, labels, and message IDs.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("query", "string", "Optional Gmail search filter (e.g. 'is:unread')", false),
                    param("max_results", "integer", "Maximum messages to return (default 10, max 200)", false),
                ],
            },
            Arc::new(GwGmailInbox(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_gmail_search".into(),
                description: "Search Gmail with a query string (same syntax as Gmail search bar). Use for filtering by sender, subject, label, date, etc.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("query", "string", "Gmail search query (e.g. 'from:boss subject:report')", true),
                    param("max_results", "integer", "Maximum messages to return (default 10, max 200)", false),
                ],
            },
            Arc::new(GwGmailSearch(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_gmail_read".into(),
                description: "Read the FULL content of a single Gmail message. Requires the message_id obtained from gw_gmail_inbox or gw_gmail_search. Do NOT use this to list or check inbox.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("message_id", "string", "Gmail message ID", true),
                ],
            },
            Arc::new(GwGmailRead(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_calendar_today".into(),
                description: "Get today's calendar events from Google Calendar.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GwCalendarToday(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_calendar_search".into(),
                description: "Search Google Calendar events by keyword or date range.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("query", "string", "Search text for event titles/descriptions", false),
                    param("time_min", "string", "Start of time range (ISO 8601)", false),
                    param("time_max", "string", "End of time range (ISO 8601)", false),
                ],
            },
            Arc::new(GwCalendarSearch(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_calendar_availability".into(),
                description: "Find free time slots and scheduling conflicts in Google Calendar for a window (e.g. 'when am I free tomorrow', 'do I have any clashes today').".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("time_min", "string", "Window start (ISO 8601). Defaults to now.", false),
                    param("time_max", "string", "Window end (ISO 8601). Defaults to now + 24h.", false),
                    param("min_slot_minutes", "number", "Minimum free slot length in minutes (default 15).", false),
                ],
            },
            Arc::new(GwCalendarAvailability(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_morning_briefing".into(),
                description: "Daily briefing: unread Gmail messages plus today's Google Calendar events (and any conflicts) in one summary.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GwMorningBriefing {
                gw: gw.clone(),
                github: github_ref.clone(),
            }),
        ),
        (
            ToolDef {
                name: "gw_drive_search".into(),
                description: "Search Google Drive files by name or content.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("query", "string", "Search query (supports Drive search operators)", true),
                ],
            },
            Arc::new(GwDriveSearch(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_drive_list".into(),
                description: "List files in a Google Drive folder.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("folder_id", "string", "Drive folder ID (omit for root)", false),
                ],
            },
            Arc::new(GwDriveList(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_drive_read".into(),
                description: "Read content of a Google Drive file / Google Doc by ID.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("file_id", "string", "Google Drive file or Google Doc ID", true),
                ],
            },
            Arc::new(GwDriveRead(gw.clone())),
        ),

        // ── Docs group (on-demand mount) ─────────────────
        (
            ToolDef {
                name: "gw_docs_read".into(),
                description: "Read a Google Doc by ID (markdown format).".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("document_id", "string", "Google Docs document ID", true),
                ],
            },
            Arc::new(GwDocsRead(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_docs_create".into(),
                description: "Create a new Google Doc.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("title", "string", "Document title", true),
                ],
            },
            Arc::new(GwDocsCreate(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_docs_edit".into(),
                description: "Append text to an existing Google Doc.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("document_id", "string", "Google Docs document ID", true),
                    param("text", "string", "Text to append", true),
                ],
            },
            Arc::new(GwDocsEdit(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_sheets_read".into(),
                description: "Read a Google Spreadsheet by ID.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("spreadsheet_id", "string", "Google Sheets spreadsheet ID", true),
                    param("range", "string", "Cell range like 'Sheet1!A1:D10' (optional)", false),
                ],
            },
            Arc::new(GwSheetsRead(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_sheets_create".into(),
                description: "Create a new Google Spreadsheet.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("title", "string", "Spreadsheet title", true),
                ],
            },
            Arc::new(GwSheetsCreate(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_sheets_edit".into(),
                description: "Write data to a Google Sheet range.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("spreadsheet_id", "string", "Google Sheets spreadsheet ID", true),
                    param("range", "string", "Target cell range (e.g. 'Sheet1!A1:C3')", true),
                    param("values", "string", "JSON array of row arrays, e.g. [[\"a\",\"b\"],[\"c\",\"d\"]]", true),
                ],
            },
            Arc::new(GwSheetsEdit(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_slides_read".into(),
                description: "Read a Google Slides presentation by ID.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("presentation_id", "string", "Google Slides presentation ID", true),
                ],
            },
            Arc::new(GwSlidesRead(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_slides_create".into(),
                description: "Create a new Google Slides presentation.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("title", "string", "Presentation title", true),
                ],
            },
            Arc::new(GwSlidesCreate(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_forms_list".into(),
                description: "List Google Forms (optionally filtered by query).".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("query", "string", "Optional search query for forms", false),
                ],
            },
            Arc::new(GwFormsList(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_forms_create".into(),
                description: "Create a new Google Form.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("title", "string", "Google Form title", true),
                ],
            },
            Arc::new(GwFormsCreate(gw.clone())),
        ),

        // ── Admin group (on-demand mount) ────────────────
        (
            ToolDef {
                name: "gw_gmail_send".into(),
                description: "Send an email via Gmail (creates draft then sends). Requires HITL approval.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("to", "string", "Recipient email address", true),
                    param("subject", "string", "Email subject line", true),
                    param("body", "string", "Email body (plain text)", true),
                    param("cc", "string", "CC recipients (comma-separated)", false),
                ],
            },
            Arc::new(GwGmailSend(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_gmail_delete".into(),
                description: "Delete a Gmail message. Requires HITL approval.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("message_id", "string", "Gmail message ID to delete", true),
                ],
            },
            Arc::new(GwGmailDelete(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_drive_delete".into(),
                description: "Delete a file from Google Drive. Requires HITL approval.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("file_id", "string", "Google Drive file ID to delete", true),
                ],
            },
            Arc::new(GwDriveDelete(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_calendar_create".into(),
                description: "Create a new Google Calendar event. Requires HITL approval.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("summary", "string", "Event title", true),
                    param("start", "string", "Start time (ISO 8601)", true),
                    param("end", "string", "End time (ISO 8601)", true),
                    param("description", "string", "Event description", false),
                    param("location", "string", "Event location", false),
                ],
            },
            Arc::new(GwCalendarCreate(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_calendar_delete".into(),
                description: "Delete a Google Calendar event. Requires HITL approval.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("event_id", "string", "Calendar event ID to delete", true),
                ],
            },
            Arc::new(GwCalendarDelete(gw.clone())),
        ),
        // ── New Gmail tools ──────────────────────────────
        (
            ToolDef {
                name: "gw_gmail_reply".into(),
                description: "Reply to a Gmail message. Reads the original, creates a reply draft, and sends it. Requires HITL approval.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("message_id", "string", "Gmail message ID to reply to", true),
                    param("body", "string", "Reply body text", true),
                    param("reply_all", "boolean", "Reply to all recipients (default: false)", false),
                ],
            },
            Arc::new(GwGmailReply(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_gmail_draft_create".into(),
                description: "Create a Gmail DRAFT (does NOT send). Returns draft_id + a formatted preview (to/subject/body). Use gw_gmail_send_draft to send it later.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("to", "string", "Recipient email", true),
                    param("subject", "string", "Subject", true),
                    param("body", "string", "Body text", true),
                    param("cc", "string", "Optional Cc", false),
                    param("account", "string", "Optional Google account override", false),
                ],
            },
            Arc::new(GwGmailDraftCreate(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_gmail_send_draft".into(),
                description: "Send a previously created Gmail draft by its draft_id. Requires HITL approval.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("draft_id", "string", "Draft id from gw_gmail_draft_create", true),
                    param("account", "string", "Optional Google account override", false),
                ],
            },
            Arc::new(GwGmailSendDraft(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_gmail_send_bulk".into(),
                description: "Send the same email to multiple recipients (max 50). Each is drafted then sent. Requires HITL approval.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("recipients", "array", "List of recipient emails", true),
                    param("subject", "string", "Subject", true),
                    param("body", "string", "Body text", true),
                    param("cc", "string", "Optional Cc on all", false),
                    param("account", "string", "Optional Google account override", false),
                ],
            },
            Arc::new(GwGmailSendBulk(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_account_switch".into(),
                description: "Switch the active Google account used for subsequent Google Workspace tool calls.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param("account", "string", "Account name (e.g. personal, work)", true)],
            },
            Arc::new(GwAccountSwitch),
        ),
        (
            ToolDef {
                name: "gw_calendar_update".into(),
                description: "Update/reschedule an existing Google Calendar event (time, title, description, location). Requires HITL approval.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("event_id", "string", "Calendar event id", true),
                    param("summary", "string", "New title", false),
                    param("start", "string", "New start (ISO 8601)", false),
                    param("end", "string", "New end (ISO 8601)", false),
                    param("description", "string", "New description", false),
                    param("location", "string", "New location", false),
                    param("account", "string", "Optional Google account override", false),
                ],
            },
            Arc::new(GwCalendarUpdate(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_gmail_mark_read".into(),
                description: "Mark a Gmail message as read or unread.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("message_id", "string", "Gmail message ID", true),
                    param("read", "boolean", "true = mark as read, false = mark as unread", true),
                ],
            },
            Arc::new(GwGmailMarkRead(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_gmail_label".into(),
                description: "Add a label to a Gmail message (e.g. STARRED, IMPORTANT, or custom label ID).".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("message_id", "string", "Gmail message ID", true),
                    param("label", "string", "Label to add (e.g. STARRED, IMPORTANT)", true),
                ],
            },
            Arc::new(GwGmailLabel(gw.clone())),
        ),
        // ── New Drive tools ──────────────────────────────
        (
            ToolDef {
                name: "gw_drive_create_file".into(),
                description: "Create a new file in Google Drive with optional content and folder placement.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("name", "string", "File name", true),
                    param("content", "string", "File content (optional)", false),
                    param("folder_id", "string", "Parent folder ID (optional, defaults to root)", false),
                    param("mime_type", "string", "MIME type (optional, e.g. text/plain)", false),
                ],
            },
            Arc::new(GwDriveCreateFile(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_drive_create_folder".into(),
                description: "Create a new folder in Google Drive.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("name", "string", "Folder name", true),
                    param("parent_folder_id", "string", "Parent folder ID (optional, defaults to root)", false),
                ],
            },
            Arc::new(GwDriveCreateFolder(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_drive_move".into(),
                description: "Move a Google Drive file to a different folder.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("file_id", "string", "File ID to move", true),
                    param("target_folder_id", "string", "Target folder ID", true),
                ],
            },
            Arc::new(GwDriveMove(gw.clone())),
        ),
        (
            ToolDef {
                name: "gw_drive_rename".into(),
                description: "Rename a Google Drive file or folder.".into(),
                category: "google_workspace".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("file_id", "string", "File or folder ID to rename", true),
                    param("new_name", "string", "New name", true),
                ],
            },
            Arc::new(GwDriveRename(gw.clone())),
        ),
    ];

    let gw_tool_count = tools.len();
    for (def, handler) in tools {
        tracing::debug!("[GW] registering tool: {}", def.name);
        reg.register(def, handler);
    }

    tracing::info!(
        "[GW] {} Google Workspace tools registered (MCP connection pending)",
        gw_tool_count
    );
}

#[cfg(test)]
mod tests {
    use super::{
        build_google_resource_url, calendar_create_args, envelope_result, extract_gmail_draft_id,
        extract_google_resource_id, gmail_max_results, gmail_messages_from_payload,
        gmail_next_page_token, gmail_preview_markdown, looks_like_drive_listing_phrase,
        normalize_gmail_inbox_query, parse_gmail_messages_from_text, parse_gw_error,
        CreateCalendarEventInput,
    };

    #[test]
    fn gmail_preview_includes_cc_when_present() {
        let p = gmail_preview_markdown("a@x.com", "Hi", "Body", Some("c@x.com"));
        assert!(p.contains("**To:** a@x.com"));
        assert!(p.contains("**Cc:** c@x.com"));
        assert!(p.contains("**Subject:** Hi"));
        assert!(p.ends_with("Body"));
        let no_cc = gmail_preview_markdown("a@x.com", "Hi", "Body", None);
        assert!(!no_cc.contains("Cc:"));
    }

    #[test]
    fn gmail_inbox_query_defaults_to_inbox() {
        assert_eq!(normalize_gmail_inbox_query(None), "in:inbox");
        assert_eq!(
            normalize_gmail_inbox_query(Some("is:unread")),
            "in:inbox is:unread"
        );
        assert_eq!(normalize_gmail_inbox_query(Some("in:sent")), "in:sent");
    }

    #[test]
    fn gmail_max_results_uses_param_and_caps_values() {
        assert_eq!(gmail_max_results(0, 10), 10);
        assert_eq!(gmail_max_results(3, 10), 3);
        assert_eq!(gmail_max_results(500, 10), 200);
    }

    #[test]
    fn envelope_result_contains_contract_metadata() {
        let envelope = envelope_result(
            "searchGmail",
            serde_json::json!({ "messages": [] }),
            Some("{\"messages\":[]}"),
        );

        assert_eq!(envelope["provider"], "google_workspace");
        assert_eq!(envelope["kind"], "gmail");
        assert_eq!(envelope["_meta"]["schema_version"], "1.1");
        assert!(!envelope["_meta"]["correlation_id"]
            .as_str()
            .unwrap_or("")
            .is_empty());
        assert!(!envelope["_meta"]["timestamp"]
            .as_str()
            .unwrap_or("")
            .is_empty());
        assert!(!envelope["_meta"]["account"]
            .as_str()
            .unwrap_or("")
            .is_empty());
    }

    #[test]
    fn gw_error_parser_classifies_quota_errors() {
        let parsed = parse_gw_error("rateLimitExceeded: Too many requests");

        assert_eq!(parsed.code, "quota_exceeded");
        assert_eq!(parsed.category, "quota");
        assert_eq!(parsed.recovery_action, "wait_and_retry");
        assert!(parsed.retryable);
        assert!(parsed.user_facing.contains("rate limit") || parsed.user_facing.contains("quota"));
    }

    #[test]
    fn extract_gmail_draft_id_handles_wrapped_and_raw_shapes() {
        let wrapped = serde_json::json!({
            "provider": "google_workspace",
            "data": {
                "result": {
                    "draftId": "draft_wrapped_123"
                }
            }
        });
        assert_eq!(
            extract_gmail_draft_id(&wrapped).as_deref(),
            Some("draft_wrapped_123")
        );

        let raw_text_shape = serde_json::json!({
            "provider": "google_workspace",
            "data": {
                "status": "created"
            },
            "raw_text": "{\"draftId\":\"draft_raw_456\"}"
        });
        assert_eq!(
            extract_gmail_draft_id(&raw_text_shape).as_deref(),
            Some("draft_raw_456")
        );
    }

    #[test]
    fn gmail_helpers_extract_messages_and_next_page_token() {
        let payload = serde_json::json!({
            "messages": [
                {"id": "m1", "subject": "A"},
                {"id": "m2", "subject": "B"}
            ],
            "nextPageToken": "token-2"
        });

        let messages = gmail_messages_from_payload(&payload);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["id"], "m1");
        assert_eq!(gmail_next_page_token(&payload).as_deref(), Some("token-2"));
    }

    #[test]
    fn gmail_helpers_parse_text_search_results_into_messages() {
        let raw = r#"
**Search Results for:** "in:inbox is:unread"
Total estimate: 201 messages

**1. Invitation: Kria Presentation Pitching**
   From: obaidullah zeeshan <obaidzeeshan.official@gmail.com>
   Date: Sat, 18 Apr 2026 05:49:26 +0000
   ID: 19d9f230a2e500b1
   Labels: UNREAD, IMPORTANT, CATEGORY_PERSONAL, INBOX
   Preview: You have been invited
   Link: https://mail.google.com/mail/?authuser=personal#all/19d9f230a2e500b1

**2. Meet the new Make Grid**
   From: Make <info@make.com>
   Date: Fri, 10 Apr 2026 10:47:32 +0000
   ID: 19d770115374cefc
   Labels: CATEGORY_PROMOTIONS, UNREAD, INBOX
   Preview: You asked, we delivered
   Link: https://mail.google.com/mail/?authuser=personal#all/19d770115374cefc
"#;

        let parsed = parse_gmail_messages_from_text(raw);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["id"], "19d9f230a2e500b1");
        assert_eq!(parsed[1]["id"], "19d770115374cefc");
        assert!(parsed[0].get("category").is_none());
        assert!(parsed[1].get("category").is_none());
        assert_eq!(parsed[0]["labels"][0], "UNREAD");
        assert_eq!(parsed[1]["labels"][0], "CATEGORY_PROMOTIONS");

        let wrapped = serde_json::json!({ "text": raw });
        let wrapped_parsed = gmail_messages_from_payload(&wrapped);
        assert_eq!(wrapped_parsed.len(), 2);
    }

    #[test]
    fn drive_listing_phrase_detector_distinguishes_list_from_search() {
        assert!(looks_like_drive_listing_phrase(
            "list files in my google drive"
        ));
        assert!(looks_like_drive_listing_phrase("show drive contents"));
        assert!(!looks_like_drive_listing_phrase(
            "search drive for quarterly report"
        ));
    }

    #[test]
    fn calendar_create_args_supports_primary_and_alternate_shapes() {
        let params = CreateCalendarEventInput {
            summary: "Google Meet".to_string(),
            start: "2026-04-19T09:30:00Z".to_string(),
            end: "2026-04-19T10:00:00Z".to_string(),
            description: None,
            location: None,
            attendees: Some(vec![serde_json::json!({"email":"example@domain.com"})]),
        };

        let primary = calendar_create_args(&params, false);
        assert_eq!(primary["start"]["dateTime"], "2026-04-19T09:30:00Z");
        assert_eq!(primary["end"]["dateTime"], "2026-04-19T10:00:00Z");
        assert_eq!(primary["attendees"][0]["email"], "example@domain.com");

        let alternate = calendar_create_args(&params, true);
        assert_eq!(alternate["startDateTime"], "2026-04-19T09:30:00Z");
        assert_eq!(alternate["endDateTime"], "2026-04-19T10:00:00Z");
        assert_eq!(alternate["attendees"][0]["email"], "example@domain.com");
    }

    #[test]
    fn extract_google_resource_id_supports_direct_and_url_based_ids() {
        let direct_payload = serde_json::json!({
            "documentId": "doc_direct_id"
        });
        assert_eq!(
            extract_google_resource_id(
                &direct_payload,
                &["documentId", "id"],
                &["url", "link"],
                "/document/d/"
            )
            .as_deref(),
            Some("doc_direct_id")
        );

        let url_payload = serde_json::json!({
            "result": {
                "link": "https://docs.google.com/document/d/doc_from_url/edit?usp=sharing"
            }
        });
        assert_eq!(
            extract_google_resource_id(
                &url_payload,
                &["documentId", "id"],
                &["url", "link"],
                "/document/d/"
            )
            .as_deref(),
            Some("doc_from_url")
        );
    }

    #[test]
    fn build_google_resource_url_generates_edit_links() {
        assert_eq!(
            build_google_resource_url("document", "abc123").as_deref(),
            Some("https://docs.google.com/document/d/abc123/edit")
        );
        assert_eq!(
            build_google_resource_url("spreadsheet", "sheet456").as_deref(),
            Some("https://docs.google.com/spreadsheets/d/sheet456/edit")
        );
        assert_eq!(
            build_google_resource_url("presentation", "slide789").as_deref(),
            Some("https://docs.google.com/presentation/d/slide789/edit")
        );
    }
}
