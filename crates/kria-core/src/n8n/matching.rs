use super::types::N8nWorkflowConfig;
use crate::safety::RiskLevel;
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::BTreeSet;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct N8nWorkflowMatchCandidate {
    pub workflow_id: String,
    pub display_name: String,
    pub status: String,
    pub matched_on: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum N8nWorkflowReferenceMatch<'a> {
    Unique {
        workflow: &'a N8nWorkflowConfig,
        matched_on: Vec<String>,
    },
    Ambiguous {
        matches: Vec<N8nWorkflowMatchCandidate>,
    },
    NoMatch {
        available: Vec<N8nWorkflowMatchCandidate>,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct WorkflowCandidate {
    pub workflow_id: String,
    pub workflow_version: String,
    pub display_name: String,
    pub category: String,
    pub risk_tier: String,
    pub status: String,
    pub hitl_policy: String,
    pub score: f32,
    pub confidence: f32,
    pub confidence_label: String,
    pub matched_on: Vec<String>,
    pub requires_confirmation: bool,
    pub suggested_input_payload: Value,
    pub missing_inputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next_actions: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct WorkflowSuggestionResponse {
    pub schema_version: String,
    pub prompt: String,
    pub reference: String,
    pub status: String,
    pub candidates: Vec<WorkflowCandidate>,
    pub requires_confirmation: bool,
    pub can_auto_run: bool,
    pub ambiguous: bool,
    pub hard_prompt: bool,
    pub message: String,
    pub confirmation_hint: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct N8nAuthoringWorkflowName {
    pub display_name: String,
    pub start: usize,
    pub end: usize,
    pub source: String,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nChatRouteStatus {
    ListWorkflows,
    ReadyToRun,
    ConfirmRequired,
    SuggestWorkflow,
    AskClarification,
    Blocked,
    OfferArchive,
    DangerDeleteRequested,
    CreateWorkflow,
    UpdateWorkflow,
    CreateFromTemplate,
    TestAuthoringDraft,
    ApproveAuthoringDraft,
    CleanupAuthoringDraft,
    UseOtherTool,
    NoMatch,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct N8nChatRouteRequest {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_user_prompt: Option<String>,
    #[serde(default)]
    pub manual_n8n_mode: bool,
    #[serde(default)]
    pub safe_auto_run_enabled: bool,
    #[serde(default)]
    pub workflows: Vec<N8nWorkflowConfig>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct N8nChatRouteDecision {
    pub schema_version: String,
    pub prompt: String,
    pub reference: String,
    pub status: N8nChatRouteStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_workflow: Option<WorkflowCandidate>,
    #[serde(default)]
    pub candidates: Vec<WorkflowCandidate>,
    #[serde(default)]
    pub inventory: Vec<N8nWorkflowMatchCandidate>,
    pub input_payload_preview: Value,
    #[serde(default)]
    pub missing_inputs: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub next_actions: Vec<String>,
    pub confidence: f32,
    pub reason: String,
    pub message: String,
    #[serde(default)]
    pub can_auto_run: bool,
    #[serde(default)]
    pub requires_confirmation: bool,
    #[serde(default)]
    pub ambiguous: bool,
    #[serde(default)]
    pub hard_prompt: bool,
    #[serde(default)]
    pub trace: Vec<String>,
}

impl N8nChatRouteDecision {
    pub fn to_workflow_suggestion_response(&self) -> WorkflowSuggestionResponse {
        let status = match self.status {
            N8nChatRouteStatus::ListWorkflows => "list_workflows",
            N8nChatRouteStatus::ReadyToRun => "ready_to_run",
            N8nChatRouteStatus::ConfirmRequired => "needs_confirmation",
            N8nChatRouteStatus::SuggestWorkflow => "suggest_workflow",
            N8nChatRouteStatus::AskClarification => "needs_clarification",
            N8nChatRouteStatus::Blocked => "blocked",
            N8nChatRouteStatus::OfferArchive => "offer_archive",
            N8nChatRouteStatus::DangerDeleteRequested => "danger_delete_requested",
            N8nChatRouteStatus::CreateWorkflow => "create_workflow",
            N8nChatRouteStatus::UpdateWorkflow => "update_workflow",
            N8nChatRouteStatus::CreateFromTemplate => "create_from_template",
            N8nChatRouteStatus::TestAuthoringDraft => "test_authoring_draft",
            N8nChatRouteStatus::ApproveAuthoringDraft => "approve_authoring_draft",
            N8nChatRouteStatus::CleanupAuthoringDraft => "cleanup_authoring_draft",
            N8nChatRouteStatus::UseOtherTool => "use_other_tool",
            N8nChatRouteStatus::NoMatch => "not_found",
        }
        .to_string();
        let confirmation_hint = self
            .selected_workflow
            .as_ref()
            .map(WorkflowConfirmationFlow::candidate_confirmation_text);

        WorkflowSuggestionResponse {
            schema_version: "kria.n8n.workflow_suggestion.v2".into(),
            prompt: self.prompt.clone(),
            reference: self.reference.clone(),
            status,
            candidates: self.candidates.clone(),
            requires_confirmation: self.requires_confirmation,
            can_auto_run: self.can_auto_run,
            ambiguous: self.ambiguous,
            hard_prompt: self.hard_prompt,
            message: self.message.clone(),
            confirmation_hint,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkflowRankingEngine {
    workflows: Vec<N8nWorkflowConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetadataField {
    WorkflowId,
    DisplayName,
    Alias,
    ExamplePrompt,
    Tag,
    Category,
    Description,
}

impl MetadataField {
    fn label(self) -> &'static str {
        match self {
            Self::WorkflowId => "workflow_id",
            Self::DisplayName => "display_name",
            Self::Alias => "alias",
            Self::ExamplePrompt => "example_prompt",
            Self::Tag => "tag",
            Self::Category => "category",
            Self::Description => "description",
        }
    }

    fn exact_score(self) -> f32 {
        match self {
            Self::WorkflowId => 100.0,
            Self::DisplayName => 96.0,
            Self::Alias => 92.0,
            Self::ExamplePrompt => 88.0,
            Self::Tag => 74.0,
            Self::Category => 62.0,
            Self::Description => 54.0,
        }
    }

    fn contains_score(self) -> f32 {
        match self {
            Self::WorkflowId => 86.0,
            Self::DisplayName => 82.0,
            Self::Alias => 78.0,
            Self::ExamplePrompt => 76.0,
            Self::Tag => 58.0,
            Self::Category => 48.0,
            Self::Description => 44.0,
        }
    }
}

#[derive(Debug, Clone)]
struct CandidateScore {
    workflow: N8nWorkflowConfig,
    score: f32,
    matched_on: BTreeSet<String>,
    reason: String,
}

/// Whole-word containment check: true if `needle` appears in `haystack` as a
/// complete word (or word sequence) at a word boundary, not as an arbitrary
/// substring inside a longer unrelated word. Both inputs are expected to
/// already be normalized (lowercase, single-spaced) via `normalize_reference`.
///
/// This is the root-cause fix for BUG #1 (n8n misrouting): plain `str::contains`
/// let a short tag like "test" match inside "...hash of 'test'" purely because
/// the substring existed, regardless of word boundaries.
fn contains_whole_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let hay_words: Vec<&str> = haystack.split_whitespace().collect();
    let needle_words: Vec<&str> = needle.split_whitespace().collect();
    if needle_words.is_empty() || needle_words.len() > hay_words.len() {
        return false;
    }
    hay_words
        .windows(needle_words.len())
        .any(|window| window == needle_words.as_slice())
}

fn normalize_reference(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';'))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn normalize_for_tokens(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push(' ');
        }
    }
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn token_set(value: &str) -> BTreeSet<String> {
    const STOPWORDS: &[&str] = &[
        "a", "an", "and", "are", "at", "by", "for", "from", "i", "in", "is", "it", "me", "my",
        "of", "on", "or", "please", "the", "this", "to", "with",
    ];
    normalize_for_tokens(value)
        .split_whitespace()
        .filter(|token| token.len() > 1 && !STOPWORDS.contains(token))
        .map(str::to_string)
        .collect()
}

fn broad_prompt(value: &str) -> bool {
    let normalized = normalize_reference(value);
    let hard_phrases = [
        "brief me",
        "check if automation is healthy",
        "clean up",
        "discuss the report",
        "find out what everyone sent",
        "get the report from mail",
        "handle my email",
        "handle the client follow-up",
        "handle this payment thing",
        "organize bug reports",
        "process this document",
        "publish the update",
        "reply to everyone",
        "send the report to the team",
        "share the report with everyone",
        "summarize everything",
        "test everything",
        "track this bug",
    ];

    hard_phrases
        .iter()
        .any(|phrase| normalized == *phrase || normalized.contains(phrase))
}

fn candidate(workflow: &N8nWorkflowConfig, matched_on: Vec<String>) -> N8nWorkflowMatchCandidate {
    N8nWorkflowMatchCandidate {
        workflow_id: workflow.workflow_id.clone(),
        display_name: workflow_display_or_id(workflow),
        status: format!("{:?}", workflow.status).to_ascii_lowercase(),
        matched_on,
    }
}

fn workflow_display_or_id(workflow: &N8nWorkflowConfig) -> String {
    if workflow.display_name.trim().is_empty() {
        workflow.workflow_id.clone()
    } else {
        workflow.display_name.clone()
    }
}

fn workflow_match_keys(workflow: &N8nWorkflowConfig) -> Vec<(String, String)> {
    let mut keys = vec![
        ("workflow_id".to_string(), workflow.workflow_id.clone()),
        ("display_name".to_string(), workflow.display_name.clone()),
        ("category".to_string(), workflow.category.clone()),
    ];

    keys.extend(
        workflow
            .aliases
            .iter()
            .cloned()
            .map(|alias| ("alias".to_string(), alias)),
    );
    keys.extend(
        workflow
            .tags
            .iter()
            .cloned()
            .map(|tag| ("tag".to_string(), tag)),
    );

    keys
}

fn ranking_keys(workflow: &N8nWorkflowConfig) -> Vec<(MetadataField, String)> {
    let mut keys = vec![
        (MetadataField::WorkflowId, workflow.workflow_id.clone()),
        (MetadataField::DisplayName, workflow.display_name.clone()),
        (MetadataField::Category, workflow.category.clone()),
    ];
    keys.extend(
        workflow
            .aliases
            .iter()
            .cloned()
            .map(|value| (MetadataField::Alias, value)),
    );
    keys.extend(
        workflow
            .tags
            .iter()
            .cloned()
            .map(|value| (MetadataField::Tag, value)),
    );
    keys.extend(
        workflow
            .example_prompts
            .iter()
            .cloned()
            .map(|value| (MetadataField::ExamplePrompt, value)),
    );
    if !workflow.description.trim().is_empty() {
        keys.push((MetadataField::Description, workflow.description.clone()));
    }
    keys
}

fn confidence_label(confidence: f32) -> String {
    if confidence >= 0.90 {
        "high".into()
    } else if confidence >= 0.70 {
        "medium".into()
    } else {
        "low".into()
    }
}

pub fn is_n8n_workflow_inventory_query(user_text: &str) -> bool {
    let lower = user_text.to_ascii_lowercase();
    let lower = lower.trim();
    if lower.starts_with("run ") || lower.contains("confirm workflow") {
        return false;
    }

    let mentions_workflows =
        lower.contains("workflow") || lower.contains("workflows") || lower.contains("automat");
    let asks_for_list = lower.contains("list")
        || lower.contains("show")
        || lower.contains("available")
        || lower.contains("what")
        || lower.contains("which")
        || lower.contains("have")
        || lower.contains("registered");
    let owned_or_n8n_context = lower.contains("n8n")
        || lower.contains("my")
        || lower.contains("i have")
        || lower.contains("all")
        || lower.contains("registered")
        || lower.contains("available");

    lower == "n8n discover"
        || lower.contains("what can i automate")
        || (mentions_workflows && asks_for_list && owned_or_n8n_context)
}

pub fn prompt_has_explicit_n8n_intent(user_text: &str) -> bool {
    let lower = user_text.to_ascii_lowercase();
    // BUG #1 FIX (root cause #3, category D: Dispatcher issue): a bare
    // "run "/"retry "/"rerun " prefix was treated as EXPLICIT n8n intent
    // unconditionally, which short-circuited `prompt_looks_like_non_n8n_tool_intent`
    // to `false` before its exclusion list (hash/skill vocabulary) ever ran.
    // "Run the skill oc_fake_skill_that_does_not_exist" starts with "run " but
    // is clearly an OpenClaw skill invocation, not an n8n workflow reference.
    // Keep the prefix heuristic (needed for legitimate "run <workflow_id>"
    // prompts) but do not let it override an explicit skill/OpenClaw mention.
    let mentions_skill_or_openclaw =
        lower.contains("skill") || lower.contains("openclaw") || lower.contains("oc_");
    let starts_with_run_prefix = !mentions_skill_or_openclaw
        && (lower.starts_with("run ")
            || lower.starts_with("retry ")
            || lower.starts_with("rerun ")
            || lower.starts_with("re-run "));
    lower.contains("n8n")
        || lower.contains("workflow")
        || lower.contains("workflows")
        || starts_with_run_prefix
        || WorkflowConfirmationFlow::parse_confirmation_reference(user_text).is_some()
}

pub fn prompt_looks_like_non_n8n_tool_intent(user_text: &str) -> bool {
    if prompt_has_explicit_n8n_intent(user_text) {
        return false;
    }

    let lower = user_text.to_ascii_lowercase();
    let search_or_browser = [
        "search the web",
        "search web",
        "web search",
        "search wen", // common typo for "web" observed in production usage
        "browser",
        "google",
        "youtube",
        "bing",
        "duckduckgo",
        "open url",
        "open http",
        "fetch article",
        "latest news",
        "breaking news",
        "weather",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase));
    let file_or_code = [
        "read file",
        "write file",
        "create file",
        "delete file",
        "list files",
        "create folder",
        "create directory",
        "run command",
        "terminal",
        "bash",
        "python script",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase));
    let mcp_or_external_tool = [
        "github",
        "git ",
        "mcp",
        "google drive",
        "google docs",
        "google sheets",
        "openai",
        "browser tool",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase));
    // BUG #1 FIX (n8n misrouting root cause, category D: Dispatcher issue):
    // prompts about hashing/cryptography or invoking a named skill were never
    // excluded here, so they fell through to WorkflowRankingEngine::suggest_for_reference
    // where a single generic word (e.g. "test") could token-match an approved
    // workflow's tag list and get incorrectly routed/blocked as an n8n workflow.
    let hash_or_crypto = [
        "hash", "sha1", "sha256", "sha512", "md5", "blake3", "checksum", "digest",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase));
    let skill_invocation = ["skill", "openclaw", "oc_"]
        .iter()
        .any(|phrase| lower.contains(phrase));

    search_or_browser || file_or_code || mcp_or_external_tool || hash_or_crypto || skill_invocation
}

pub fn extract_n8n_authoring_workflow_name(prompt: &str) -> Option<N8nAuthoringWorkflowName> {
    if prompt.trim().is_empty() {
        return None;
    }
    let lower = prompt.to_ascii_lowercase();
    let markers = [
        (" named ", "named"),
        (" called ", "called"),
        (" titled ", "titled"),
        (" named: ", "named"),
        (" called: ", "called"),
        (" titled: ", "titled"),
        (" with name ", "with_name"),
    ];
    for (marker, source) in markers {
        let Some(marker_start) = lower.find(marker) else {
            continue;
        };
        let mut start = marker_start + marker.len();
        while let Some(ch) = prompt[start..].chars().next() {
            if ch.is_whitespace() {
                start += ch.len_utf8();
            } else {
                break;
            }
        }
        if start >= prompt.len() {
            continue;
        }

        let first = prompt[start..].chars().next()?;
        let (name_start, end) = if matches!(first, '"' | '\'' | '`') {
            let name_start = start + first.len_utf8();
            let relative_end = prompt[name_start..].find(first)?;
            (name_start, name_start + relative_end)
        } else {
            (start, authoring_name_end(prompt, start))
        };
        let display_name = prompt[name_start..end]
            .trim_matches(|ch: char| {
                ch.is_whitespace() || matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';')
            })
            .trim_start_matches("the ")
            .trim()
            .to_string();
        if valid_authoring_workflow_name(&display_name) {
            return Some(N8nAuthoringWorkflowName {
                display_name,
                start: name_start,
                end,
                source: source.into(),
            });
        }
    }
    None
}

fn authoring_name_end(prompt: &str, start: usize) -> usize {
    let lower = prompt.to_ascii_lowercase();
    let stop_phrases = [
        " that ",
        " which ",
        " to receive ",
        " to fetch ",
        " to send ",
        " to post ",
        " using ",
        " with trigger ",
        " with a trigger ",
        " and fetches ",
        " and returns ",
        " and sends ",
        " and posts ",
        " then ",
        ",",
    ];
    let mut end = prompt.len();
    for phrase in stop_phrases {
        if let Some(relative) = lower[start..].find(phrase) {
            end = end.min(start + relative);
        }
    }
    if end == prompt.len() {
        let mut word_count = 0;
        let mut in_word = false;
        for (offset, ch) in prompt[start..].char_indices() {
            if ch.is_whitespace() {
                if in_word {
                    word_count += 1;
                    in_word = false;
                }
                if word_count >= 12 {
                    return start + offset;
                }
            } else {
                in_word = true;
            }
        }
    }
    end
}

fn valid_authoring_workflow_name(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.len() < 3 || trimmed.len() > 120 {
        return false;
    }
    trimmed
        .chars()
        .all(|ch| !ch.is_control() && !matches!(ch, '/' | '\\'))
}

fn prompt_has_authoring_create_verb(user_text: &str) -> bool {
    let lower = user_text.to_ascii_lowercase();
    [
        "create ",
        "build ",
        "make ",
        "generate ",
        "draft ",
        "set up ",
        "setup ",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

fn prompt_is_workflowish(user_text: &str) -> bool {
    let lower = user_text.to_ascii_lowercase();
    lower.contains("workflow") || lower.contains("automation") || lower.contains("n8n")
}

fn prompt_without_protected_authoring_name(user_text: &str) -> String {
    if !prompt_has_authoring_create_verb(user_text) || !prompt_is_workflowish(user_text) {
        return user_text.to_string();
    }
    let Some(name) = extract_n8n_authoring_workflow_name(user_text) else {
        return user_text.to_string();
    };
    let mut cleaned = String::with_capacity(user_text.len());
    cleaned.push_str(&user_text[..name.start]);
    cleaned.push(' ');
    cleaned.push_str(&user_text[name.end..]);
    cleaned
}

fn prompt_looks_like_n8n_delete_intent(user_text: &str) -> bool {
    let intent_text = prompt_without_protected_authoring_name(user_text);
    let lower = intent_text.to_ascii_lowercase();
    let delete_command = lower.starts_with("delete ")
        || lower.starts_with("remove ")
        || lower.starts_with("permanently delete ")
        || lower.contains(" delete workflow")
        || lower.contains(" remove workflow")
        || lower.contains(" delete n8n")
        || lower.contains(" remove n8n")
        || lower.contains(" permanently delete ")
        || lower.contains("permanently delete workflow")
        || lower.contains("permanently delete n8n");
    delete_command && (lower.contains("workflow") || lower.contains("n8n"))
}

fn prompt_looks_like_n8n_permanent_delete_intent(user_text: &str) -> bool {
    let intent_text = prompt_without_protected_authoring_name(user_text);
    let lower = intent_text.to_ascii_lowercase();
    (lower.contains("permanent") || lower.contains("permanently"))
        && prompt_looks_like_n8n_delete_intent(&intent_text)
}

fn prompt_looks_like_authoring_create_intent(user_text: &str) -> bool {
    prompt_is_workflowish(user_text)
        && prompt_has_authoring_create_verb(user_text)
        && !prompt_looks_like_n8n_delete_intent(user_text)
}

fn prompt_looks_like_destructive_authoring_request(user_text: &str) -> bool {
    if !prompt_looks_like_authoring_create_intent(user_text) {
        return false;
    }
    let intent_text = prompt_without_protected_authoring_name(user_text);
    let lower = intent_text.to_ascii_lowercase();
    let destructive_verb = [
        "delete",
        "deletes",
        "drop",
        "drops",
        "truncate",
        "truncates",
        "wipe",
        "wipes",
        "purge",
        "purges",
        "erase",
        "erases",
        "destroy",
        "destroys",
    ]
    .iter()
    .any(|term| {
        lower
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .any(|word| word == *term)
    }) || lower.contains("remove all");
    if !destructive_verb {
        return false;
    }
    let high_risk_target = [
        "production",
        "customer",
        "customers",
        "record",
        "records",
        "database",
        "table",
        "rows",
        "all rows",
        "credential",
        "credentials",
        "payment",
        "payments",
    ]
    .iter()
    .any(|term| lower.contains(term));
    high_risk_target || lower.contains("permanent") || lower.contains("permanently")
}

fn prompt_looks_like_authoring_update_intent(user_text: &str) -> bool {
    let lower = user_text.to_ascii_lowercase();
    let workflowish = lower.contains("workflow") || lower.contains("n8n");
    workflowish
        && prompt_has_authoring_update_verb(user_text)
        && !prompt_looks_like_n8n_delete_intent(user_text)
}

fn prompt_has_authoring_update_verb(user_text: &str) -> bool {
    let lower = user_text.to_ascii_lowercase();
    [
        "update ",
        "change ",
        "modify ",
        "edit ",
        "add ",
        "replace ",
        "make it ",
        "make this ",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

fn prompt_looks_like_authoring_test_intent(user_text: &str) -> bool {
    let lower = user_text.to_ascii_lowercase();
    let explicit_test_command = lower.starts_with("test ")
        || lower.starts_with("test n8n ")
        || lower.starts_with("test workflow ")
        || lower.starts_with("test draft ")
        || lower.contains(" test draft")
        || lower.contains(" test n8n draft")
        || lower.contains(" test workflow draft");
    explicit_test_command
        && (lower.contains("draft") || lower.contains("authored") || lower.contains("workflow"))
        && (lower.contains("n8n") || lower.contains("workflow"))
}

fn prompt_looks_like_authoring_approval_intent(user_text: &str) -> bool {
    let lower = user_text.to_ascii_lowercase();
    (lower.contains("approve") || lower.contains("register"))
        && (lower.contains("draft") || lower.contains("workflow") || lower.contains("n8n"))
}

fn prompt_looks_like_authoring_cleanup_intent(user_text: &str) -> bool {
    let lower = user_text.to_ascii_lowercase();
    (lower.contains("cleanup") || lower.contains("clean up") || lower.contains("reject"))
        && (lower.contains("draft") || lower.contains("authored") || lower.contains("workflow"))
}

fn workflow_lifecycle_blockers(workflow: &N8nWorkflowConfig) -> Vec<String> {
    let status = workflow.lifecycle_status.trim().to_ascii_lowercase();
    if status.is_empty()
        || matches!(
            status.as_str(),
            "current" | "safe_refresh_available" | "cleanup_available"
        )
    {
        return Vec::new();
    }

    let mut blockers = Vec::new();
    if status.contains("missing") {
        blockers.push("workflow is missing in n8n; refresh or cleanup is required".to_string());
    } else if status.contains("changed") || status.contains("drift") {
        blockers.push("workflow changed in n8n; refresh/review is required".to_string());
    } else if status.contains("retest") {
        blockers.push("workflow needs a new test before KRIA can run it".to_string());
    } else if status.contains("pending") {
        blockers.push("workflow setup has a pending recovery step".to_string());
    } else if status.contains("blocked") || status.contains("review") {
        blockers.push("workflow lifecycle state blocks execution until review".to_string());
    }

    blockers.extend(
        workflow
            .lifecycle_warnings
            .iter()
            .map(|warning| warning.trim())
            .filter(|warning| !warning.is_empty())
            .map(str::to_string),
    );
    blockers
}

fn workflow_adapter_blockers(workflow: &N8nWorkflowConfig) -> Vec<String> {
    let mut blockers = Vec::new();
    let trigger = workflow.trigger_strategy.trim();
    let result_mode = workflow.result_mode.trim();

    if trigger.eq_ignore_ascii_case("unsupported")
        || result_mode.eq_ignore_ascii_case("unsupported")
    {
        blockers.push("workflow trigger/result mode is unsupported".to_string());
    }

    if result_mode == "poll_execution"
        && trigger == "webhook"
        && workflow.webhook_method.trim().is_empty()
    {
        blockers.push("webhook method must be reviewed as GET or POST".to_string());
    }

    if result_mode == "monitor_only" {
        blockers
            .push("workflow is monitor-only; use View Executions or Run Now instead".to_string());
    }

    blockers
}

fn candidate_blockers(workflow: &N8nWorkflowConfig, missing_inputs: &[String]) -> Vec<String> {
    let mut blockers = Vec::new();
    if !workflow.is_approved_for_execution() {
        blockers.push(format!(
            "workflow is {:?}; approve it before running",
            workflow.status
        ));
    }
    blockers.extend(workflow_lifecycle_blockers(workflow));
    blockers.extend(workflow_adapter_blockers(workflow));
    if !missing_inputs.is_empty() {
        blockers.push(format!("missing input: {}", missing_inputs.join(", ")));
    }
    blockers
}

#[derive(Debug, Clone, PartialEq)]
enum UpdateTargetResolution {
    Exact(WorkflowCandidate),
    ArchivedOrDeleted {
        workflow_id: String,
        display_name: String,
    },
    NoExactMatch,
}

fn resolve_exact_update_workflow_target(
    prompt: &str,
    workflows: &[N8nWorkflowConfig],
) -> UpdateTargetResolution {
    let prompt_tokens = update_prompt_candidate_tokens(prompt);
    for workflow in workflows {
        let normalized_id = normalize_reference(&workflow.workflow_id);
        if normalized_id.is_empty() || !prompt_tokens.contains(&normalized_id) {
            continue;
        }

        let display_name = workflow_display_or_id(workflow);
        if workflow.is_archived_or_deleted() {
            return UpdateTargetResolution::ArchivedOrDeleted {
                workflow_id: workflow.workflow_id.clone(),
                display_name,
            };
        }

        return UpdateTargetResolution::Exact(WorkflowCandidate {
            workflow_id: workflow.workflow_id.clone(),
            workflow_version: workflow.workflow_version.clone(),
            display_name,
            category: workflow.category.clone(),
            risk_tier: format!("{:?}", workflow.risk_tier),
            status: format!("{:?}", workflow.status).to_ascii_lowercase(),
            hitl_policy: workflow.hitl_policy.clone(),
            score: 100.0,
            confidence: 1.0,
            confidence_label: "high".into(),
            matched_on: vec!["workflow_id".into()],
            requires_confirmation: true,
            suggested_input_payload: serde_json::json!({
                "prompt": prompt,
                "workflow_id": workflow.workflow_id.clone(),
                "update_mode": "create_updated_copy"
            }),
            missing_inputs: Vec::new(),
            blockers: Vec::new(),
            next_actions: vec![
                "Create updated draft copy".into(),
                "Review diff before testing".into(),
            ],
            reason: "Exact workflow_id match for update".into(),
        });
    }

    UpdateTargetResolution::NoExactMatch
}

fn update_prompt_candidate_tokens(prompt: &str) -> BTreeSet<String> {
    let mut normalized = String::with_capacity(prompt.len());
    for ch in prompt.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push(' ');
        }
    }
    normalized
        .split_whitespace()
        .filter(|token| token.len() > 1)
        .map(str::to_string)
        .collect()
}

fn next_actions_for_candidate(workflow: &N8nWorkflowConfig, blockers: &[String]) -> Vec<String> {
    if blockers.is_empty() {
        if WorkflowConfirmationFlow::workflow_requires_confirmation(workflow) {
            return vec![format!(
                "Review and confirm workflow {}",
                workflow.workflow_id
            )];
        }
        return vec![format!("Run workflow {}", workflow.workflow_id)];
    }

    let mut actions = Vec::new();
    for blocker in blockers {
        let lower = blocker.to_ascii_lowercase();
        if lower.contains("missing input") {
            actions.push("Provide the missing input fields".to_string());
        } else if lower.contains("changed") || lower.contains("lifecycle") {
            actions.push("Refresh workflow analysis in Dashboard -> n8n".to_string());
        } else if lower.contains("draft") || lower.contains("approve") {
            actions.push("Open Dashboard -> n8n -> Add Workflow and approve the draft".to_string());
        } else if lower.contains("monitor-only") {
            actions.push("Open Run History or View Executions for this workflow".to_string());
        } else if lower.contains("webhook method") {
            actions.push("Review webhook method in workflow setup".to_string());
        }
    }
    actions.sort();
    actions.dedup();
    if actions.is_empty() {
        actions.push("Review this workflow in Dashboard -> n8n".to_string());
    }
    actions
}

fn candidate_from_score(score: CandidateScore, prompt: &str) -> WorkflowCandidate {
    let workflow = score.workflow;
    let confidence = (score.score / 100.0).clamp(0.0, 1.0);
    let suggested_input_payload = build_n8n_suggested_input_payload(&workflow, prompt, false);
    let missing_inputs =
        super::schema::input_payload_validation_issues(&workflow, &suggested_input_payload);
    let blockers = candidate_blockers(&workflow, &missing_inputs);
    let next_actions = next_actions_for_candidate(&workflow, &blockers);
    WorkflowCandidate {
        workflow_id: workflow.workflow_id.clone(),
        workflow_version: workflow.workflow_version.clone(),
        display_name: if workflow.display_name.trim().is_empty() {
            workflow.workflow_id.clone()
        } else {
            workflow.display_name.clone()
        },
        category: workflow.category.clone(),
        risk_tier: format!("{:?}", workflow.risk_tier),
        status: format!("{:?}", workflow.status).to_ascii_lowercase(),
        hitl_policy: workflow.hitl_policy.clone(),
        score: (score.score * 10.0).round() / 10.0,
        confidence,
        confidence_label: confidence_label(confidence),
        matched_on: score.matched_on.into_iter().collect(),
        requires_confirmation: WorkflowConfirmationFlow::workflow_requires_confirmation(&workflow),
        suggested_input_payload,
        missing_inputs,
        blockers,
        next_actions,
        reason: score.reason,
    }
}

pub fn build_n8n_suggested_input_payload(
    workflow: &N8nWorkflowConfig,
    prompt: &str,
    confirmed: bool,
) -> Value {
    let mut payload = Map::new();
    insert_non_empty(&mut payload, "source_prompt", prompt.trim());

    match workflow.workflow_id.as_str() {
        "gmail_inbox_digest" => {
            if prompt.to_ascii_lowercase().contains("today") {
                insert_non_empty(&mut payload, "time_window", "today");
            }
            payload.insert("max_messages".into(), Value::from(10));
        }
        "gmail_search_messages" => {
            insert_non_empty(&mut payload, "query", prompt.trim());
            payload.insert("max_results".into(), Value::from(10));
        }
        "gmail_send_draft" => {
            if let Some(recipient) =
                extract_after_until(prompt, " to ", &[" about ", " saying ", " that ", ","])
            {
                insert_non_empty(&mut payload, "recipient", &recipient);
            }
            if let Some(subject) =
                extract_after_until(prompt, " about ", &[" saying ", " that ", "."])
            {
                insert_non_empty(&mut payload, "subject", &subject);
            }
            if let Some(body) = extract_quoted(prompt)
                .or_else(|| extract_after_until(prompt, " saying ", &["."]))
                .or_else(|| extract_after_until(prompt, " telling them ", &["."]))
                .or_else(|| extract_after_until(prompt, " that ", &["."]))
            {
                insert_non_empty(&mut payload, "body", &body);
            }
        }
        "calendar_create_meeting" => {
            insert_non_empty(&mut payload, "title", cleanup_sentence(prompt));
            if let Some(attendee) = extract_after_until(
                prompt,
                " with ",
                &[" tomorrow", " next week", " today", " for ", ",", "."],
            ) {
                let attendee = attendee.trim();
                if !attendee.is_empty() {
                    payload.insert(
                        "attendees".into(),
                        Value::Array(vec![Value::String(attendee.into())]),
                    );
                }
            }
            if let Some(start_time) = extract_calendar_time(prompt) {
                insert_non_empty(&mut payload, "start_time", &start_time);
            }
            if let Some(duration) = extract_duration_minutes(prompt) {
                payload.insert("duration_minutes".into(), Value::from(duration));
            }
        }
        "slack_post_update" => {
            if let Some(channel) = extract_slack_channel(prompt) {
                insert_non_empty(&mut payload, "channel", &channel);
            }
            if let Some(message) = extract_quoted(prompt)
                .or_else(|| {
                    extract_after_until(
                        prompt,
                        "let the team know ",
                        &[" to slack", " in slack", " to #", " in #", "."],
                    )
                })
                .or_else(|| {
                    extract_after_until(
                        prompt,
                        "announce ",
                        &[" to slack", " in slack", " to #", " in #", "."],
                    )
                })
                .or_else(|| {
                    extract_after_until(
                        prompt,
                        "publish ",
                        &[" to slack", " in slack", " to #", " in #", "."],
                    )
                })
                .or_else(|| {
                    extract_after_until(
                        prompt,
                        "share ",
                        &[" to slack", " in slack", " to #", " in #", "."],
                    )
                })
            {
                insert_non_empty(&mut payload, "message", &message);
            }
        }
        _ => {}
    }

    if WorkflowConfirmationFlow::workflow_requires_confirmation(workflow) {
        payload.insert("confirmed_by_user".into(), Value::Bool(confirmed));
    }

    Value::Object(payload)
}

pub fn mark_n8n_input_payload_confirmed(
    workflow: &N8nWorkflowConfig,
    input_payload: Value,
) -> Value {
    if !WorkflowConfirmationFlow::workflow_requires_confirmation(workflow) {
        return input_payload;
    }

    match input_payload {
        Value::Object(mut map) => {
            map.insert("confirmed_by_user".into(), Value::Bool(true));
            Value::Object(map)
        }
        other => serde_json::json!({
            "source_payload": other,
            "confirmed_by_user": true,
        }),
    }
}

fn insert_non_empty(payload: &mut Map<String, Value>, key: &str, value: &str) {
    let value = value.trim();
    if !value.is_empty() {
        payload.insert(key.into(), Value::String(value.to_string()));
    }
}

fn cleanup_sentence(value: &str) -> &str {
    value
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';'))
}

fn extract_quoted(value: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let Some(start) = value.find(quote) else {
            continue;
        };
        let rest = &value[start + quote.len_utf8()..];
        let Some(end) = rest.find(quote) else {
            continue;
        };
        let extracted = rest[..end].trim();
        if !extracted.is_empty() {
            return Some(extracted.to_string());
        }
    }
    None
}

fn extract_after_until(value: &str, phrase: &str, stops: &[&str]) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let start = lower.find(phrase)? + phrase.len();
    let mut end = value.len();
    let lower_tail = &lower[start..];
    for stop in stops {
        if let Some(offset) = lower_tail.find(stop) {
            end = end.min(start + offset);
        }
    }
    let extracted = cleanup_sentence(&value[start..end]);
    if extracted.is_empty() {
        None
    } else {
        Some(extracted.to_string())
    }
}

fn extract_slack_channel(prompt: &str) -> Option<String> {
    for token in prompt.split_whitespace() {
        let cleaned =
            token.trim_matches(|ch: char| matches!(ch, ',' | '.' | ':' | ';' | '"' | '\'' | '`'));
        if cleaned.starts_with('#') && cleaned.len() > 1 {
            return Some(cleaned.to_string());
        }
    }
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("team channel") {
        return Some("team".into());
    }
    if lower.contains("project channel") {
        return Some("project".into());
    }
    None
}

fn extract_calendar_time(prompt: &str) -> Option<String> {
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("tomorrow") {
        Some("tomorrow".into())
    } else if lower.contains("next week") {
        Some("next week".into())
    } else if lower.contains("today") {
        Some("today".into())
    } else {
        None
    }
}

fn extract_duration_minutes(prompt: &str) -> Option<i64> {
    let tokens = prompt
        .split_whitespace()
        .map(|token| token.trim_matches(|ch: char| matches!(ch, ',' | '.' | ':' | ';')))
        .collect::<Vec<_>>();
    for window in tokens.windows(2) {
        if window[1].to_ascii_lowercase().starts_with("minute") {
            if let Ok(value) = window[0].parse::<i64>() {
                return Some(value);
            }
        }
    }
    None
}

pub fn parse_n8n_workflow_run_reference(message: &str) -> Option<String> {
    let trimmed = message.trim();
    let lower = trimmed.to_ascii_lowercase();
    let prefixes = [
        "invoke n8n workflow ",
        "trigger n8n workflow ",
        "start n8n workflow ",
        "execute n8n workflow ",
        "run n8n workflow ",
        "invoke workflow ",
        "trigger workflow ",
        "start workflow ",
        "execute workflow ",
        "run workflow ",
        "retry n8n workflow ",
        "retry workflow ",
        "rerun n8n workflow ",
        "rerun workflow ",
        "re-run n8n workflow ",
        "re-run workflow ",
        "run ",
        "retry ",
        "rerun ",
        "re-run ",
    ];

    let prefix = prefixes.iter().find(|prefix| lower.starts_with(**prefix))?;
    let mut reference = trimmed[prefix.len()..].trim();
    if reference.to_ascii_lowercase().starts_with("the ") {
        reference = reference[4..].trim();
    }

    let mut cleaned = reference
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';'))
        .trim()
        .to_string();

    for suffix in [", please", " please", ", now", " now", ", again", " again"] {
        let lower_cleaned = cleaned.to_ascii_lowercase();
        if lower_cleaned.ends_with(suffix) && lower_cleaned.len() > suffix.len() {
            let new_len = cleaned.len().saturating_sub(suffix.len());
            cleaned.truncate(new_len);
            cleaned = cleaned
                .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';'))
                .trim()
                .to_string();
        }
    }

    if cleaned.is_empty() {
        return None;
    }
    if cleaned
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '/' | '\\'))
    {
        return None;
    }

    Some(cleaned)
}

impl WorkflowRankingEngine {
    pub fn new(workflows: Vec<N8nWorkflowConfig>) -> Self {
        Self { workflows }
    }

    pub fn route_chat(&self, request: N8nChatRouteRequest) -> N8nChatRouteDecision {
        let prompt = request.prompt.trim().to_string();
        let reference = parse_n8n_workflow_run_reference(&prompt).unwrap_or_else(|| prompt.clone());
        let mut trace = Vec::new();
        trace.push("router=deterministic_n8n_chat_v1".to_string());

        if prompt.is_empty() {
            return N8nChatRouteDecision {
                schema_version: "kria.n8n.chat_route.v1".into(),
                prompt,
                reference,
                status: N8nChatRouteStatus::NoMatch,
                selected_workflow: None,
                candidates: Vec::new(),
                inventory: Vec::new(),
                input_payload_preview: Value::Object(Map::new()),
                missing_inputs: Vec::new(),
                blockers: vec!["prompt is empty".into()],
                next_actions: vec!["Enter a workflow request".into()],
                confidence: 0.0,
                reason: "Empty prompt".into(),
                message: "Enter a workflow request before routing.".into(),
                can_auto_run: false,
                requires_confirmation: false,
                ambiguous: false,
                hard_prompt: false,
                trace,
            };
        }

        if prompt_looks_like_n8n_delete_intent(&prompt) {
            let permanent = prompt_looks_like_n8n_permanent_delete_intent(&prompt);
            let response = self.suggest_for_reference(&prompt, &reference);
            let mut candidates = response.candidates;
            candidates.truncate(3);
            let selected = candidates.first().cloned();
            trace.push(if permanent {
                "decision=danger_delete_requested".to_string()
            } else {
                "decision=offer_archive".to_string()
            });
            let confidence = if prompt_has_explicit_n8n_intent(&prompt) {
                0.95
            } else {
                0.75
            };
            return N8nChatRouteDecision {
                schema_version: "kria.n8n.chat_route.v1".into(),
                prompt,
                reference,
                status: if permanent {
                    N8nChatRouteStatus::DangerDeleteRequested
                } else {
                    N8nChatRouteStatus::OfferArchive
                },
                selected_workflow: selected,
                candidates,
                inventory: Vec::new(),
                input_payload_preview: Value::Object(Map::new()),
                missing_inputs: Vec::new(),
                blockers: if permanent {
                    vec!["Permanent deletion must be completed in the n8n Danger Zone.".into()]
                } else {
                    Vec::new()
                },
                next_actions: if permanent {
                    vec![
                        "Open Advanced delete options".into(),
                        "Archive instead".into(),
                    ]
                } else {
                    vec![
                        "Archive workflow".into(),
                        "Open Advanced delete options".into(),
                    ]
                },
                confidence,
                reason: if permanent {
                    "Explicit permanent delete intent requires Danger Zone confirmation".into()
                } else {
                    "Delete intent is converted to safe archive offer".into()
                },
                message: if permanent {
                    "KRIA will not permanently delete a workflow directly from chat. Open the Danger Zone to back it up and confirm deletion.".into()
                } else {
                    "KRIA does not permanently delete n8n workflows by default. Archive hides it from KRIA and keeps it in n8n.".into()
                },
                can_auto_run: false,
                requires_confirmation: true,
                ambiguous: false,
                hard_prompt: false,
                trace,
            };
        }

        if prompt_looks_like_destructive_authoring_request(&prompt) {
            trace.push("decision=blocked_destructive_authoring".to_string());
            return N8nChatRouteDecision {
                schema_version: "kria.n8n.chat_route.v1".into(),
                prompt,
                reference,
                status: N8nChatRouteStatus::Blocked,
                selected_workflow: None,
                candidates: Vec::new(),
                inventory: Vec::new(),
                input_payload_preview: serde_json::json!({
                    "authoring_prompt": request.prompt,
                    "mode": "blocked_destructive_authoring"
                }),
                missing_inputs: Vec::new(),
                blockers: vec![
                    "KRIA cannot safely create destructive n8n workflows from chat.".into(),
                    "Production data, customer records, database table deletion, and credential/payment destructive actions require manual review.".into(),
                ],
                next_actions: vec![
                    "Use manual review for destructive automation".into(),
                    "Create a non-destructive draft instead".into(),
                ],
                confidence: 0.95,
                reason: "Prompt asks to create a high-risk destructive n8n workflow".into(),
                message: "Blocked: KRIA cannot safely create destructive n8n workflows from chat. Use manual review for production data, customer records, database table deletion, credential, or payment actions.".into(),
                can_auto_run: false,
                requires_confirmation: true,
                ambiguous: false,
                hard_prompt: true,
                trace,
            };
        }

        if prompt_looks_like_authoring_create_intent(&prompt) {
            let requested_name = extract_n8n_authoring_workflow_name(&prompt);
            let create_from_template = ["template", "gmail", "slack", "sheet", "sheets"]
                .iter()
                .any(|phrase| prompt.to_ascii_lowercase().contains(phrase));
            trace.push(if create_from_template {
                "decision=create_from_template".to_string()
            } else {
                "decision=create_workflow".to_string()
            });
            if requested_name.is_some() {
                trace.push("create_target=explicit_workflow_name".to_string());
            }
            return N8nChatRouteDecision {
                schema_version: "kria.n8n.chat_route.v1".into(),
                prompt,
                reference,
                status: if create_from_template {
                    N8nChatRouteStatus::CreateFromTemplate
                } else {
                    N8nChatRouteStatus::CreateWorkflow
                },
                selected_workflow: None,
                candidates: Vec::new(),
                inventory: Vec::new(),
                input_payload_preview: serde_json::json!({
                    "authoring_prompt": request.prompt,
                    "mode": if create_from_template { "create_from_template" } else { "create_workflow" },
                    "requested_workflow_name": requested_name.as_ref().map(|name| name.display_name.clone()),
                    "requested_workflow_name_source": requested_name.as_ref().map(|name| name.source.clone())
                }),
                missing_inputs: Vec::new(),
                blockers: Vec::new(),
                next_actions: vec![
                    "Review generated draft plan".into(),
                    "Create inactive n8n draft".into(),
                ],
                confidence: 0.88,
                reason: "Prompt asks KRIA to create a new n8n workflow draft".into(),
                message: "KRIA can prepare an inactive n8n workflow draft for review.".into(),
                can_auto_run: false,
                requires_confirmation: true,
                ambiguous: false,
                hard_prompt: false,
                trace,
            };
        }

        let exact_update_target = if prompt_has_authoring_update_verb(&prompt)
            && !prompt_looks_like_n8n_delete_intent(&prompt)
        {
            resolve_exact_update_workflow_target(&prompt, &self.workflows)
        } else {
            UpdateTargetResolution::NoExactMatch
        };
        if prompt_looks_like_authoring_update_intent(&prompt)
            || !matches!(exact_update_target, UpdateTargetResolution::NoExactMatch)
        {
            match exact_update_target {
                UpdateTargetResolution::Exact(candidate) => {
                    trace.push("decision=update_workflow".to_string());
                    trace.push("update_target=exact_workflow_id".to_string());
                    let reference = candidate.workflow_id.clone();
                    return N8nChatRouteDecision {
                        schema_version: "kria.n8n.chat_route.v1".into(),
                        prompt,
                        reference,
                        status: N8nChatRouteStatus::UpdateWorkflow,
                        selected_workflow: Some(candidate.clone()),
                        candidates: vec![candidate],
                        inventory: Vec::new(),
                        input_payload_preview: serde_json::json!({
                            "authoring_prompt": request.prompt.clone(),
                            "mode": "update_workflow"
                        }),
                        missing_inputs: Vec::new(),
                        blockers: Vec::new(),
                        next_actions: vec![
                            "Create updated draft copy".into(),
                            "Review diff before testing".into(),
                        ],
                        confidence: 1.0,
                        reason: "Exact workflow_id match for update".into(),
                        message: "KRIA can create an updated draft copy for the exact workflow ID. The original workflow stays unchanged by default.".into(),
                        can_auto_run: false,
                        requires_confirmation: true,
                        ambiguous: false,
                        hard_prompt: false,
                        trace,
                    };
                }
                UpdateTargetResolution::ArchivedOrDeleted {
                    workflow_id,
                    display_name,
                } => {
                    trace.push("decision=update_workflow".to_string());
                    trace.push("update_target=archived_or_deleted_exact_workflow_id".to_string());
                    return N8nChatRouteDecision {
                        schema_version: "kria.n8n.chat_route.v1".into(),
                        prompt,
                        reference: workflow_id,
                        status: N8nChatRouteStatus::UpdateWorkflow,
                        selected_workflow: None,
                        candidates: Vec::new(),
                        inventory: Vec::new(),
                        input_payload_preview: serde_json::json!({
                            "authoring_prompt": request.prompt.clone(),
                            "mode": "update_workflow"
                        }),
                        missing_inputs: Vec::new(),
                        blockers: vec![
                            "Workflow is archived or removed. Restore it before updating.".into(),
                        ],
                        next_actions: vec!["Restore workflow".into()],
                        confidence: 1.0,
                        reason: "Exact update workflow_id is archived or removed".into(),
                        message: format!(
                            "Restore \"{display_name}\" before KRIA creates an updated draft copy."
                        ),
                        can_auto_run: false,
                        requires_confirmation: true,
                        ambiguous: false,
                        hard_prompt: false,
                        trace,
                    };
                }
                UpdateTargetResolution::NoExactMatch => {}
            }

            let response = self.suggest_for_reference(&prompt, &reference);
            let mut candidates = response.candidates;
            candidates.truncate(3);
            let selected = candidates.first().cloned();
            trace.push("decision=update_workflow".to_string());
            return N8nChatRouteDecision {
                schema_version: "kria.n8n.chat_route.v1".into(),
                prompt,
                reference: response.reference,
                status: N8nChatRouteStatus::UpdateWorkflow,
                selected_workflow: selected,
                candidates,
                inventory: Vec::new(),
                input_payload_preview: serde_json::json!({
                    "authoring_prompt": request.prompt.clone(),
                    "mode": "update_workflow"
                }),
                missing_inputs: Vec::new(),
                blockers: Vec::new(),
                next_actions: vec![
                    "Create updated draft copy".into(),
                    "Review diff before testing".into(),
                ],
                confidence: 0.84,
                reason: "Prompt asks KRIA to update an existing n8n workflow".into(),
                message: "KRIA can create an updated draft copy. The original workflow stays unchanged by default.".into(),
                can_auto_run: false,
                requires_confirmation: true,
                ambiguous: false,
                hard_prompt: false,
                trace,
            };
        }

        if prompt_looks_like_authoring_test_intent(&prompt) {
            trace.push("decision=test_authoring_draft".to_string());
            return N8nChatRouteDecision {
                schema_version: "kria.n8n.chat_route.v1".into(),
                prompt,
                reference,
                status: N8nChatRouteStatus::TestAuthoringDraft,
                selected_workflow: None,
                candidates: Vec::new(),
                inventory: Vec::new(),
                input_payload_preview: serde_json::json!({ "authoring_prompt": request.prompt }),
                missing_inputs: Vec::new(),
                blockers: Vec::new(),
                next_actions: vec!["Choose a KRIA-authored draft to test".into()],
                confidence: 0.78,
                reason: "Prompt asks to test an authored n8n draft".into(),
                message: "Choose the draft workflow and test input before KRIA runs it.".into(),
                can_auto_run: false,
                requires_confirmation: true,
                ambiguous: false,
                hard_prompt: false,
                trace,
            };
        }

        if prompt_looks_like_authoring_approval_intent(&prompt) {
            trace.push("decision=approve_authoring_draft".to_string());
            return N8nChatRouteDecision {
                schema_version: "kria.n8n.chat_route.v1".into(),
                prompt,
                reference,
                status: N8nChatRouteStatus::ApproveAuthoringDraft,
                selected_workflow: None,
                candidates: Vec::new(),
                inventory: Vec::new(),
                input_payload_preview: Value::Object(Map::new()),
                missing_inputs: Vec::new(),
                blockers: Vec::new(),
                next_actions: vec!["Review validation and last test result".into()],
                confidence: 0.78,
                reason: "Prompt asks to approve/register an authored n8n draft".into(),
                message:
                    "KRIA can approve an authored draft only after validation and test review."
                        .into(),
                can_auto_run: false,
                requires_confirmation: true,
                ambiguous: false,
                hard_prompt: false,
                trace,
            };
        }

        if prompt_looks_like_authoring_cleanup_intent(&prompt) {
            trace.push("decision=cleanup_authoring_draft".to_string());
            return N8nChatRouteDecision {
                schema_version: "kria.n8n.chat_route.v1".into(),
                prompt,
                reference,
                status: N8nChatRouteStatus::CleanupAuthoringDraft,
                selected_workflow: None,
                candidates: Vec::new(),
                inventory: Vec::new(),
                input_payload_preview: Value::Object(Map::new()),
                missing_inputs: Vec::new(),
                blockers: Vec::new(),
                next_actions: vec!["Choose a KRIA-generated draft to clean up".into()],
                confidence: 0.78,
                reason: "Prompt asks to reject or clean up an authored n8n draft".into(),
                message: "KRIA can clean up only verified KRIA-generated n8n drafts.".into(),
                can_auto_run: false,
                requires_confirmation: true,
                ambiguous: false,
                hard_prompt: false,
                trace,
            };
        }

        if is_n8n_workflow_inventory_query(&prompt) {
            trace.push("decision=list_workflows".to_string());
            let inventory = self
                .workflows
                .iter()
                .map(|workflow| candidate(workflow, Vec::new()))
                .collect::<Vec<_>>();
            let total = inventory.len();
            let executable = self
                .workflows
                .iter()
                .filter(|workflow| workflow.is_approved_for_execution())
                .count();
            return N8nChatRouteDecision {
                schema_version: "kria.n8n.chat_route.v1".into(),
                prompt,
                reference,
                status: N8nChatRouteStatus::ListWorkflows,
                selected_workflow: None,
                candidates: Vec::new(),
                inventory,
                input_payload_preview: Value::Object(Map::new()),
                missing_inputs: Vec::new(),
                blockers: Vec::new(),
                next_actions: if total == 0 {
                    vec!["Sync n8n profiles from Dashboard -> n8n".into()]
                } else {
                    vec!["Say Run <workflow_id> to review one workflow".into()]
                },
                confidence: 1.0,
                reason: "Inventory query".into(),
                message: if total == 0 {
                    "No n8n workflows are registered in KRIA.".into()
                } else {
                    format!("Available n8n workflows: {executable}/{total} executable.")
                },
                can_auto_run: false,
                requires_confirmation: false,
                ambiguous: false,
                hard_prompt: false,
                trace,
            };
        }

        if prompt_looks_like_non_n8n_tool_intent(&prompt) {
            trace.push("decision=use_other_tool".to_string());
            return N8nChatRouteDecision {
                schema_version: "kria.n8n.chat_route.v1".into(),
                prompt,
                reference,
                status: N8nChatRouteStatus::UseOtherTool,
                selected_workflow: None,
                candidates: Vec::new(),
                inventory: Vec::new(),
                input_payload_preview: Value::Object(Map::new()),
                missing_inputs: Vec::new(),
                blockers: Vec::new(),
                next_actions: vec!["Use normal chat/tool routing".into()],
                confidence: 1.0,
                reason: "Prompt looks like a non-n8n tool request".into(),
                message: "This prompt looks better handled by another KRIA tool, not n8n.".into(),
                can_auto_run: false,
                requires_confirmation: false,
                ambiguous: false,
                hard_prompt: false,
                trace,
            };
        }

        let response = self.suggest_for_reference(&prompt, &reference);
        let mut candidates = response.candidates;
        let hard_prompt = response.hard_prompt;
        let ambiguous = candidates.len() > 1
            && candidates
                .first()
                .map(|top| {
                    candidates
                        .iter()
                        .skip(1)
                        .any(|candidate| top.score - candidate.score <= 18.0)
                })
                .unwrap_or(false);

        if candidates.is_empty() {
            trace.push("decision=no_match".to_string());
            return N8nChatRouteDecision {
                schema_version: "kria.n8n.chat_route.v1".into(),
                prompt,
                reference: response.reference,
                status: if hard_prompt {
                    N8nChatRouteStatus::AskClarification
                } else {
                    N8nChatRouteStatus::NoMatch
                },
                selected_workflow: None,
                candidates: Vec::new(),
                inventory: Vec::new(),
                input_payload_preview: Value::Object(Map::new()),
                missing_inputs: Vec::new(),
                blockers: Vec::new(),
                next_actions: vec!["Try an exact workflow ID or open Dashboard -> n8n".into()],
                confidence: 0.0,
                reason: "No approved workflow matched".into(),
                message: response.message,
                can_auto_run: false,
                requires_confirmation: false,
                ambiguous: false,
                hard_prompt,
                trace,
            };
        }

        let selected = candidates.first().cloned();
        let selected_confidence = selected.as_ref().map(|c| c.confidence).unwrap_or_default();
        let selected_blockers = selected
            .as_ref()
            .map(|candidate| candidate.blockers.clone())
            .unwrap_or_default();
        let selected_missing = selected
            .as_ref()
            .map(|candidate| candidate.missing_inputs.clone())
            .unwrap_or_default();
        let input_payload_preview = selected
            .as_ref()
            .map(|candidate| candidate.suggested_input_payload.clone())
            .unwrap_or_else(|| Value::Object(Map::new()));

        let needs_confirmation = selected
            .as_ref()
            .map(|candidate| candidate.requires_confirmation)
            .unwrap_or(true);
        let exact_or_explicit = parse_n8n_workflow_run_reference(&prompt).is_some()
            || selected.as_ref().is_some_and(|candidate| {
                candidate
                    .matched_on
                    .iter()
                    .any(|m| m == "workflow_id" || m == "display_name" || m == "alias")
            });

        // Weak-match release: a prompt that only faintly touches a workflow (low
        // confidence, matched on a broad field like a single tag/category token
        // rather than the workflow id/name/alias or a curated example prompt) must
        // NOT be hijacked by n8n. Hijacking here was the root cause of general
        // requests ("install a web-fetch tool", "compress this folder") being
        // routed to an unrelated workflow instead of the agent's normal tool
        // routing (which includes the Capability Provider Platform / marketplace).
        // Defer to normal routing unless the match is explicit, strong, blocked,
        // hard (the user clearly asked for an n8n workflow), or ambiguous.
        let strong_field = selected.as_ref().is_some_and(|candidate| {
            candidate.matched_on.iter().any(|m| {
                matches!(
                    m.as_str(),
                    "workflow_id" | "display_name" | "alias" | "example_prompt"
                )
            })
        });
        let weak_match_release = !exact_or_explicit
            && !strong_field
            && !hard_prompt
            && !ambiguous
            && selected_blockers.is_empty()
            && selected_confidence < 0.60;
        if weak_match_release {
            trace.push("decision=use_other_tool(weak_match_release)".to_string());
            return N8nChatRouteDecision {
                schema_version: "kria.n8n.chat_route.v1".into(),
                prompt,
                reference: response.reference,
                status: N8nChatRouteStatus::UseOtherTool,
                selected_workflow: None,
                candidates: Vec::new(),
                inventory: Vec::new(),
                input_payload_preview: Value::Object(Map::new()),
                missing_inputs: Vec::new(),
                blockers: Vec::new(),
                next_actions: vec!["Use normal chat/tool routing".into()],
                confidence: selected_confidence,
                reason: "Only a weak/low-confidence n8n match — defer to normal tool routing"
                    .into(),
                message: "This prompt looks better handled by another KRIA tool, not n8n.".into(),
                can_auto_run: false,
                requires_confirmation: false,
                ambiguous: false,
                hard_prompt: false,
                trace,
            };
        }

        let safe_auto_run = selected.as_ref().is_some_and(|candidate| {
            candidate.risk_tier.eq_ignore_ascii_case("green")
                && candidate.hitl_policy.trim().eq_ignore_ascii_case("none")
                && selected_blockers.is_empty()
                && selected_missing.is_empty()
                && selected_confidence >= 0.90
                && exact_or_explicit
                && (request.manual_n8n_mode || request.safe_auto_run_enabled)
        });

        let status = if !selected_blockers.is_empty() {
            N8nChatRouteStatus::Blocked
        } else if hard_prompt || ambiguous {
            N8nChatRouteStatus::AskClarification
        } else if safe_auto_run {
            N8nChatRouteStatus::ReadyToRun
        } else if needs_confirmation {
            N8nChatRouteStatus::ConfirmRequired
        } else {
            N8nChatRouteStatus::SuggestWorkflow
        };

        trace.push(format!("decision={status:?}"));
        trace.push(format!("candidate_count={}", candidates.len()));

        let next_actions = match status {
            N8nChatRouteStatus::ReadyToRun => vec!["Run now".into()],
            N8nChatRouteStatus::ConfirmRequired => selected
                .as_ref()
                .map(|candidate| {
                    vec![WorkflowConfirmationFlow::candidate_confirmation_text(
                        candidate,
                    )]
                })
                .unwrap_or_default(),
            N8nChatRouteStatus::AskClarification => candidates
                .iter()
                .take(3)
                .map(|candidate| format!("Choose {}", candidate.workflow_id))
                .collect(),
            N8nChatRouteStatus::Blocked => selected
                .as_ref()
                .map(|candidate| candidate.next_actions.clone())
                .unwrap_or_default(),
            _ => vec!["Review workflow before running".into()],
        };

        let message = match status {
            N8nChatRouteStatus::ReadyToRun => selected
                .as_ref()
                .map(|candidate| format!("{} is ready to run.", candidate.display_name))
                .unwrap_or_else(|| "Workflow is ready to run.".into()),
            N8nChatRouteStatus::Blocked => selected
                .as_ref()
                .map(|candidate| {
                    format!(
                        "{} cannot run yet: {}",
                        candidate.display_name,
                        candidate.blockers.join("; ")
                    )
                })
                .unwrap_or_else(|| "Workflow cannot run yet.".into()),
            N8nChatRouteStatus::AskClarification => {
                format!(
                    "I found {} possible n8n workflow(s). Choose one before I run anything.",
                    candidates.len()
                )
            }
            _ => response.message,
        };

        // Keep only the top three for UI readability and deterministic eval parity.
        candidates.truncate(3);

        N8nChatRouteDecision {
            schema_version: "kria.n8n.chat_route.v1".into(),
            prompt,
            reference: response.reference,
            status,
            selected_workflow: selected,
            candidates,
            inventory: Vec::new(),
            input_payload_preview,
            missing_inputs: selected_missing,
            blockers: selected_blockers,
            next_actions,
            confidence: selected_confidence,
            reason: "Ranked approved n8n workflow metadata with readiness gates".into(),
            message,
            can_auto_run: safe_auto_run,
            requires_confirmation: needs_confirmation,
            ambiguous,
            hard_prompt,
            trace,
        }
    }

    pub fn suggest(&self, prompt: &str) -> WorkflowSuggestionResponse {
        let reference =
            parse_n8n_workflow_run_reference(prompt).unwrap_or_else(|| prompt.trim().to_string());
        self.suggest_for_reference(prompt, &reference)
    }

    pub fn suggest_for_reference(
        &self,
        prompt: &str,
        reference: &str,
    ) -> WorkflowSuggestionResponse {
        let normalized_reference = normalize_reference(reference);
        let reference_tokens = token_set(reference);
        let mut scored = Vec::new();

        for workflow in self
            .workflows
            .iter()
            .filter(|workflow| workflow.is_approved_for_execution())
        {
            let mut best_score = 0.0_f32;
            let mut matched_on = BTreeSet::new();
            let mut best_reason = String::new();

            for (field, value) in ranking_keys(workflow) {
                let key = normalize_reference(&value);
                if key.is_empty() || normalized_reference.is_empty() {
                    continue;
                }

                if key == normalized_reference {
                    let score = field.exact_score();
                    if score > best_score {
                        best_score = score;
                        best_reason = format!("Exact {} match", field.label());
                    }
                    matched_on.insert(field.label().to_string());
                    continue;
                }

                // BUG #1 FIX (n8n misrouting, category A: Semantic Router issue):
                // raw substring containment let a short, generic single-word tag
                // (e.g. "test") match ANY prompt that happened to contain that
                // sequence of characters, including inside unrelated words/phrases
                // (e.g. "sha512 hash of 'test'" contains the substring "test").
                // Require the shorter side to appear as a whole word (word-boundary
                // match) rather than an arbitrary substring, so single common words
                // can no longer masquerade as a strong metadata match.
                if key.len() >= 4
                    && normalized_reference.len() >= 4
                    && (contains_whole_word(&key, &normalized_reference)
                        || contains_whole_word(&normalized_reference, &key))
                {
                    let score = field.contains_score();
                    if score > best_score {
                        best_score = score;
                        best_reason = format!("Phrase overlap with {}", field.label());
                    }
                    matched_on.insert(field.label().to_string());
                    continue;
                }

                let key_tokens = token_set(&key);
                if key_tokens.is_empty() || reference_tokens.is_empty() {
                    continue;
                }
                let overlap = key_tokens.intersection(&reference_tokens).count();
                let denominator = key_tokens.len().max(reference_tokens.len());
                let ratio = overlap as f32 / denominator as f32;
                if overlap >= 2 && ratio >= 0.45 {
                    let score = (field.contains_score() * ratio).max(44.0);
                    if score > best_score {
                        best_score = score;
                        best_reason = format!("Token overlap with {}", field.label());
                    }
                    matched_on.insert(field.label().to_string());
                }
            }

            if best_score >= 44.0 {
                scored.push(CandidateScore {
                    workflow: workflow.clone(),
                    score: best_score,
                    matched_on,
                    reason: best_reason,
                });
            }
        }

        scored.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.workflow.workflow_id.cmp(&right.workflow.workflow_id))
        });

        let hard_prompt = broad_prompt(reference);
        let ambiguous = scored.len() > 1
            && scored
                .first()
                .map(|top| {
                    scored
                        .iter()
                        .skip(1)
                        .any(|candidate| top.score - candidate.score <= 18.0)
                })
                .unwrap_or(false);
        let top_candidates = scored
            .into_iter()
            .take(3)
            .map(|score| candidate_from_score(score, prompt))
            .collect::<Vec<_>>();

        let (status, message, confirmation_hint) = if top_candidates.is_empty() && hard_prompt {
            (
                "needs_clarification".to_string(),
                format!("I need more detail before choosing an n8n workflow for \"{reference}\"."),
                None,
            )
        } else if top_candidates.is_empty() {
            (
                "not_found".to_string(),
                format!("Workflow \"{reference}\" was not found in approved n8n workflows."),
                None,
            )
        } else if hard_prompt || ambiguous {
            (
                "needs_clarification".to_string(),
                format!(
                    "I found {} possible n8n workflow(s). Choose one before I run anything.",
                    top_candidates.len()
                ),
                Some(format!(
                    "Confirm with: Confirm workflow {}",
                    top_candidates[0].workflow_id
                )),
            )
        } else {
            (
                "needs_confirmation".to_string(),
                format!(
                    "I found \"{}\". Confirm before I run it.",
                    top_candidates[0].display_name
                ),
                Some(format!(
                    "Confirm with: Confirm workflow {}",
                    top_candidates[0].workflow_id
                )),
            )
        };

        WorkflowSuggestionResponse {
            schema_version: "kria.n8n.workflow_suggestion.v1".into(),
            prompt: prompt.to_string(),
            reference: reference.to_string(),
            status,
            candidates: top_candidates,
            requires_confirmation: true,
            can_auto_run: false,
            ambiguous,
            hard_prompt,
            message,
            confirmation_hint,
        }
    }
}

pub struct WorkflowConfirmationFlow;

impl WorkflowConfirmationFlow {
    pub fn parse_confirmation_reference(message: &str) -> Option<String> {
        let trimmed = message.trim();
        let lower = trimmed.to_ascii_lowercase();
        let prefixes = [
            "confirm n8n workflow ",
            "confirm workflow ",
            "confirm and run n8n workflow ",
            "confirm and run workflow ",
            "run confirmed n8n workflow ",
            "run confirmed workflow ",
            "execute confirmed n8n workflow ",
            "execute confirmed workflow ",
            "yes run n8n workflow ",
            "yes run workflow ",
            "yes run ",
        ];
        let prefix = prefixes.iter().find(|prefix| lower.starts_with(**prefix))?;
        let mut reference = trimmed[prefix.len()..]
            .trim()
            .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';'))
            .to_string();
        for suffix in [", please", " please", ", now", " now"] {
            let lower_reference = reference.to_ascii_lowercase();
            if lower_reference.ends_with(suffix) && lower_reference.len() > suffix.len() {
                let new_len = reference.len().saturating_sub(suffix.len());
                reference.truncate(new_len);
                reference = reference
                    .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';'))
                    .trim()
                    .to_string();
            }
        }
        if reference.is_empty()
            || reference
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, '/' | '\\'))
        {
            return None;
        }
        Some(reference)
    }

    pub fn workflow_requires_confirmation(workflow: &N8nWorkflowConfig) -> bool {
        !matches!(workflow.risk_tier, RiskLevel::Green)
            || !workflow.hitl_policy.trim().eq_ignore_ascii_case("none")
    }

    pub fn candidate_confirmation_text(candidate: &WorkflowCandidate) -> String {
        format!("Confirm workflow {}", candidate.workflow_id)
    }
}

pub fn resolve_n8n_workflow_reference<'a>(
    workflows: &'a [N8nWorkflowConfig],
    reference: &str,
) -> N8nWorkflowReferenceMatch<'a> {
    let normalized = normalize_reference(reference);
    if normalized.is_empty() {
        return N8nWorkflowReferenceMatch::NoMatch {
            available: workflows
                .iter()
                .map(|workflow| candidate(workflow, Vec::new()))
                .collect(),
        };
    }

    let mut matches: Vec<(&N8nWorkflowConfig, Vec<String>)> = Vec::new();
    for workflow in workflows {
        let mut matched_on = Vec::new();
        for (source, key) in workflow_match_keys(workflow) {
            let key = normalize_reference(&key);
            if !key.is_empty() && key == normalized {
                matched_on.push(source);
            }
        }
        if !matched_on.is_empty() {
            matches.push((workflow, matched_on));
        }
    }

    if matches.len() == 1 {
        let (workflow, matched_on) = matches.remove(0);
        return N8nWorkflowReferenceMatch::Unique {
            workflow,
            matched_on,
        };
    }

    if matches.len() > 1 {
        return N8nWorkflowReferenceMatch::Ambiguous {
            matches: matches
                .into_iter()
                .map(|(workflow, matched_on)| candidate(workflow, matched_on))
                .collect(),
        };
    }

    N8nWorkflowReferenceMatch::NoMatch {
        available: workflows
            .iter()
            .map(|workflow| candidate(workflow, Vec::new()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::n8n::{N8nWorkflowConfig, N8nWorkflowStatus};

    fn workflow(id: &str, display_name: &str) -> N8nWorkflowConfig {
        N8nWorkflowConfig {
            workflow_id: id.into(),
            workflow_version: "v1".into(),
            display_name: display_name.into(),
            endpoint_path: format!("/webhook/{id}"),
            status: N8nWorkflowStatus::Approved,
            category: "diagnostic".into(),
            input_schema_ref: "schemas/n8n/test_workflow.input.json".into(),
            output_schema_ref: "schemas/n8n/test_workflow.output.json".into(),
            aliases: vec![format!("{display_name} alias")],
            tags: vec!["diagnostic".into()],
            ..Default::default()
        }
    }

    fn route_fixture_workflows() -> Vec<N8nWorkflowConfig> {
        let mut fetch = workflow("fetch_movies", "Fetch Movies");
        fetch.category = "data_retrieval".into();
        fetch.description = "Fetch movie details from a read-only movie API.".into();
        fetch.aliases = vec!["movie lookup".into(), "find movie details".into()];
        fetch.tags = vec!["movie".into(), "movies".into(), "lookup".into()];
        fetch.example_prompts = vec!["Find movie Inception".into(), "Run fetch_movies".into()];
        fetch.risk_tier = RiskLevel::Green;
        fetch.hitl_policy = "none".into();
        fetch.trigger_strategy = "webhook".into();
        fetch.result_mode = "poll_execution".into();
        fetch.webhook_method = "POST".into();
        fetch.lifecycle_status = "current".into();

        let mut slack = workflow("slack_post_update", "Slack Update Poster");
        slack.category = "messaging".into();
        slack.description = "Post a message to Slack.".into();
        slack.aliases = vec!["post slack update".into()];
        slack.tags = vec!["slack".into(), "message".into()];
        slack.example_prompts = vec!["Post update to Slack".into()];
        slack.risk_tier = RiskLevel::Yellow;
        slack.hitl_policy = "required_review".into();
        slack.trigger_strategy = "webhook".into();
        slack.result_mode = "poll_execution".into();
        slack.webhook_method = "POST".into();
        slack.lifecycle_status = "current".into();

        let mut inbox = workflow("gmail_inbox_digest", "Inbox Digest");
        inbox.category = "email".into();
        inbox.description = "Summarize inbox messages.".into();
        inbox.aliases = vec!["summarize my inbox".into()];
        inbox.tags = vec!["email".into(), "inbox".into()];
        inbox.example_prompts = vec!["What did I miss in email this morning".into()];
        inbox.risk_tier = RiskLevel::Green;
        inbox.hitl_policy = "none".into();
        inbox.trigger_strategy = "manual_api_execute".into();
        inbox.result_mode = "poll_execution".into();
        inbox.lifecycle_status = "current".into();

        let mut search = workflow("gmail_search_messages", "Gmail Message Search");
        search.category = "email".into();
        search.description = "Search Gmail messages.".into();
        search.aliases = vec!["search gmail messages".into()];
        search.tags = vec!["email".into(), "search".into()];
        search.example_prompts = vec!["Search Gmail for invoices".into()];
        search.risk_tier = RiskLevel::Green;
        search.hitl_policy = "none".into();
        search.trigger_strategy = "manual_api_execute".into();
        search.result_mode = "poll_execution".into();
        search.lifecycle_status = "current".into();

        vec![fetch, slack, inbox, search]
    }

    /// BUG #1 regression fixture: a "Mail Schedule Test" workflow tagged with
    /// the generic word "test" among its tags — reproduces the exact real
    /// production workflow (`workflow_id: mail_schedule_test`) that caused the
    /// misrouting. `monitor_only` mirrors the real config: it cannot be
    /// auto-run from chat even if matched.
    fn mail_schedule_test_fixture() -> N8nWorkflowConfig {
        let mut wf = workflow("mail_schedule_test", "Mail Schedule Test");
        wf.category = "email".into();
        wf.description = "Runs the Mail Schedule Test n8n workflow.".into();
        wf.tags = vec![
            "email".into(),
            "mail_schedule_test".into(),
            "scheduledmonitor".into(),
            "monitoronly".into(),
            "mail".into(),
            "schedule".into(),
            "test".into(),
        ];
        wf.aliases = vec!["mail_schedule_test".into(), "mail schedule test".into()];
        wf.trigger_strategy = "scheduled_monitor".into();
        wf.result_mode = "monitor_only".into();
        wf.risk_tier = RiskLevel::Yellow;
        wf
    }

    /// BUG #1 regression (category A: Semantic Router + category D: Dispatcher).
    /// Root cause: (1) `prompt_looks_like_non_n8n_tool_intent` had no exclusion
    /// for hashing/crypto or skill-invocation vocabulary, and (2) the fuzzy
    /// tag-overlap scorer used plain substring containment, so the word "test"
    /// inside "sha512 hash of 'test'" matched the workflow's "test" tag by pure
    /// character-sequence coincidence, with zero relation to the tag's actual
    /// meaning. Fixed by (a) extending the exclusion list and (b) requiring
    /// whole-word matches in the phrase-overlap scorer.
    #[test]
    fn regr_bug1_hash_requests_never_match_mail_schedule_test_workflow() {
        let workflows = vec![mail_schedule_test_fixture()];
        let prompts = [
            "Give me sha512 hash of test",
            "Hash test using sha256",
            "What's the sha1 hash of 'production'?",
            "Give me the sha512 hash of 'test'",
        ];
        for prompt in prompts {
            assert!(
                prompt_looks_like_non_n8n_tool_intent(prompt),
                "prompt should be excluded from n8n routing: {prompt}"
            );
            let route =
                WorkflowRankingEngine::new(workflows.clone()).route_chat(N8nChatRouteRequest {
                    prompt: prompt.to_string(),
                    previous_user_prompt: None,
                    manual_n8n_mode: false,
                    safe_auto_run_enabled: false,
                    workflows: Vec::new(),
                });
            assert_ne!(
                route.status,
                N8nChatRouteStatus::Blocked,
                "hash prompt must not be blocked as an n8n workflow: {prompt}"
            );
        }
    }

    /// BUG #1 regression (category D: Dispatcher issue).
    /// Root cause: "Run oc_fake_skill_that_does_not_exist" starts with the bare
    /// "run " prefix that `parse_n8n_workflow_run_reference` matches
    /// unconditionally, with no exclusion check on that code path at all
    /// (unlike the sibling n8n dispatch block later in the same function).
    #[test]
    fn regr_bug1_run_skill_prompt_is_excluded_from_n8n_reference_parsing() {
        let prompt = "Run the skill oc_fake_skill_that_does_not_exist with no arguments";
        assert!(
            prompt_looks_like_non_n8n_tool_intent(prompt),
            "skill-invocation prompt must be excluded from n8n routing: {prompt}"
        );
        // Confirm the reference parser itself still recognizes the "run "
        // prefix (that part of the parser is not being changed) — the fix is
        // that the CALLER must consult the exclusion check before acting on it.
        assert!(parse_n8n_workflow_run_reference(prompt).is_some());
    }

    /// BUG #1 regression: the search-web typo variant seen in production
    /// ("wen" instead of "web") must also be excluded.
    #[test]
    fn regr_bug1_search_web_typo_excluded_from_n8n_routing() {
        assert!(prompt_looks_like_non_n8n_tool_intent(
            "Using openclaw search wen for todays latest breaking news in India"
        ));
        assert!(prompt_looks_like_non_n8n_tool_intent(
            "Using openclaw search web for todays latest breaking news in India"
        ));
    }

    /// BUG #1 regression (category A: Semantic Router issue) — direct unit test
    /// of the whole-word containment fix, independent of the exclusion list,
    /// so this stays protected even if the exclusion list is ever refactored.
    #[test]
    fn regr_bug1_whole_word_match_rejects_substring_inside_unrelated_word() {
        // "test" is NOT a whole word inside "production-testing-suite" or "latest"
        assert!(!contains_whole_word("production testing suite", "test"));
        assert!(!contains_whole_word("latest", "test"));
        // "test" IS a whole word inside "mail schedule test"
        assert!(contains_whole_word("mail schedule test", "test"));
        // exact equality still matches
        assert!(contains_whole_word("test", "test"));
    }

    #[test]
    fn parses_full_workflow_references_without_first_token_truncation() {
        assert_eq!(
            parse_n8n_workflow_run_reference("Run Test Workflow"),
            Some("Test Workflow".into())
        );
        assert_eq!(
            parse_n8n_workflow_run_reference("run workflow `demo_flow`, please"),
            Some("demo_flow".into())
        );
        assert_eq!(
            parse_n8n_workflow_run_reference("trigger n8n workflow invoice.sync-v1 now"),
            Some("invoice.sync-v1".into())
        );
        assert_eq!(parse_n8n_workflow_run_reference("run ../../secret"), None);
    }

    #[test]
    fn resolves_exact_id_display_name_alias_and_tag() {
        let workflows = vec![workflow("test_workflow", "Test Workflow")];

        assert!(matches!(
            resolve_n8n_workflow_reference(&workflows, "test_workflow"),
            N8nWorkflowReferenceMatch::Unique { .. }
        ));
        assert!(matches!(
            resolve_n8n_workflow_reference(&workflows, "Test Workflow"),
            N8nWorkflowReferenceMatch::Unique { .. }
        ));
        assert!(matches!(
            resolve_n8n_workflow_reference(&workflows, "Test Workflow Alias"),
            N8nWorkflowReferenceMatch::Unique { .. }
        ));
        assert!(matches!(
            resolve_n8n_workflow_reference(&workflows, "diagnostic"),
            N8nWorkflowReferenceMatch::Unique { .. }
        ));
    }

    #[test]
    fn returns_ambiguity_for_exact_multi_workflow_matches() {
        let workflows = vec![
            workflow("test_workflow", "Test Workflow"),
            workflow("other_workflow", "Other Workflow"),
        ];

        let result = resolve_n8n_workflow_reference(&workflows, "diagnostic");

        match result {
            N8nWorkflowReferenceMatch::Ambiguous { matches } => assert_eq!(matches.len(), 2),
            other => panic!("expected ambiguity, got {other:?}"),
        }
    }

    #[test]
    fn returns_available_workflows_for_no_match() {
        let workflows = vec![workflow("test_workflow", "Test Workflow")];

        let result = resolve_n8n_workflow_reference(&workflows, "missing");

        match result {
            N8nWorkflowReferenceMatch::NoMatch { available } => {
                assert_eq!(available[0].workflow_id, "test_workflow")
            }
            other => panic!("expected no match, got {other:?}"),
        }
    }

    #[test]
    fn ranks_candidates_from_metadata_without_auto_run() {
        let mut inbox = workflow("gmail_inbox_digest", "Inbox Digest");
        inbox.category = "email".into();
        inbox.aliases = vec!["summarize my inbox".into()];
        inbox.example_prompts = vec!["what did i miss in email this morning".into()];
        inbox.tags = vec!["email".into(), "inbox".into(), "digest".into()];

        let mut search = workflow("gmail_search_messages", "Gmail Message Search");
        search.category = "email".into();
        search.aliases = vec!["search gmail messages".into()];
        search.tags = vec!["email".into(), "search".into()];

        let engine = WorkflowRankingEngine::new(vec![inbox, search]);
        let response = engine.suggest("What did I miss in email this morning");

        assert_eq!(response.status, "needs_confirmation");
        assert!(!response.can_auto_run);
        assert_eq!(response.candidates[0].workflow_id, "gmail_inbox_digest");
        assert!(response.candidates[0]
            .matched_on
            .contains(&"example_prompt".to_string()));
    }

    #[test]
    fn hard_or_ambiguous_prompts_return_top_candidates_without_auto_run() {
        let mut draft = workflow("gmail_send_draft", "Gmail Draft Creator");
        draft.category = "email".into();
        draft.aliases = vec!["share the report with everyone".into()];
        draft.risk_tier = RiskLevel::Yellow;
        draft.hitl_policy = "required_review".into();

        let mut slack = workflow("slack_post_update", "Slack Update Poster");
        slack.category = "messaging".into();
        slack.aliases = vec!["share the report with everyone".into()];
        slack.risk_tier = RiskLevel::Yellow;
        slack.hitl_policy = "required_review".into();

        let engine = WorkflowRankingEngine::new(vec![draft, slack]);
        let response = engine.suggest("Share the report with everyone");

        assert_eq!(response.status, "needs_clarification");
        assert!(response.hard_prompt);
        assert!(!response.can_auto_run);
        assert_eq!(response.candidates.len(), 2);
    }

    #[test]
    fn chat_router_lists_workflows_without_hallucinating_lack_of_access() {
        let route =
            WorkflowRankingEngine::new(route_fixture_workflows()).route_chat(N8nChatRouteRequest {
                prompt: "List of n8n workflows i have".into(),
                previous_user_prompt: None,
                manual_n8n_mode: false,
                safe_auto_run_enabled: false,
                workflows: Vec::new(),
            });

        assert_eq!(route.status, N8nChatRouteStatus::ListWorkflows);
        assert!(route.message.contains("Available n8n workflows"));
        assert!(route
            .inventory
            .iter()
            .any(|item| item.workflow_id == "fetch_movies"));
    }

    #[test]
    fn chat_router_turns_delete_request_into_archive_offer() {
        let route =
            WorkflowRankingEngine::new(route_fixture_workflows()).route_chat(N8nChatRouteRequest {
                prompt: "delete fetch_movies workflow".into(),
                previous_user_prompt: None,
                manual_n8n_mode: false,
                safe_auto_run_enabled: false,
                workflows: Vec::new(),
            });

        assert_eq!(route.status, N8nChatRouteStatus::OfferArchive);
        assert!(!route.can_auto_run);
        assert!(route.next_actions.contains(&"Archive workflow".to_string()));
    }

    #[test]
    fn chat_router_never_runs_permanent_delete_from_chat() {
        let route =
            WorkflowRankingEngine::new(route_fixture_workflows()).route_chat(N8nChatRouteRequest {
                prompt: "permanently delete fetch_movies workflow from n8n".into(),
                previous_user_prompt: None,
                manual_n8n_mode: true,
                safe_auto_run_enabled: true,
                workflows: Vec::new(),
            });

        assert_eq!(route.status, N8nChatRouteStatus::DangerDeleteRequested);
        assert!(!route.can_auto_run);
        assert!(route
            .blockers
            .iter()
            .any(|blocker| blocker.contains("Danger Zone")));
    }

    #[test]
    fn chat_router_does_not_hijack_web_search_or_file_prompts() {
        for prompt in [
            "Search the web for Inception reviews",
            "Create file at /tmp/example.txt with contents 'hello'",
            "Open GitHub and search issues",
        ] {
            let route = WorkflowRankingEngine::new(route_fixture_workflows()).route_chat(
                N8nChatRouteRequest {
                    prompt: prompt.into(),
                    previous_user_prompt: None,
                    manual_n8n_mode: false,
                    safe_auto_run_enabled: false,
                    workflows: Vec::new(),
                },
            );
            assert_eq!(route.status, N8nChatRouteStatus::UseOtherTool, "{prompt}");
            assert!(route.candidates.is_empty());
            assert!(!route.can_auto_run);
        }
    }

    #[test]
    fn chat_router_routes_workflow_creation_to_authoring() {
        let route =
            WorkflowRankingEngine::new(route_fixture_workflows()).route_chat(N8nChatRouteRequest {
                prompt: "Create an n8n workflow that receives a movie title and returns details"
                    .into(),
                previous_user_prompt: None,
                manual_n8n_mode: false,
                safe_auto_run_enabled: false,
                workflows: Vec::new(),
            });

        assert_eq!(route.status, N8nChatRouteStatus::CreateWorkflow);
        assert!(route.requires_confirmation);
        assert!(!route.can_auto_run);
        assert!(route
            .next_actions
            .iter()
            .any(|action| action.contains("inactive n8n draft")));
    }

    #[test]
    fn chat_router_blocks_destructive_authoring_requests() {
        for prompt in [
            "Create an n8n workflow that deletes all rows from a production database table",
            "Create an n8n workflow that permanently deletes customer records and drops production database tables",
        ] {
            let route = WorkflowRankingEngine::new(route_fixture_workflows()).route_chat(
                N8nChatRouteRequest {
                    prompt: prompt.into(),
                    previous_user_prompt: None,
                    manual_n8n_mode: false,
                    safe_auto_run_enabled: false,
                    workflows: Vec::new(),
                },
            );

            assert_eq!(route.status, N8nChatRouteStatus::Blocked, "{prompt}");
            assert!(!route.can_auto_run);
            assert!(route.hard_prompt);
            assert!(route.message.contains("cannot safely"));
        }
    }

    #[test]
    fn chat_router_extracts_explicit_authoring_name_without_delete_hijack() {
        let prompt = "Create an n8n workflow named KRIA Desktop Command E2E Safe Delete Guard that receives a movie title";
        let name = extract_n8n_authoring_workflow_name(prompt).expect("expected name");
        assert_eq!(
            name.display_name,
            "KRIA Desktop Command E2E Safe Delete Guard"
        );

        let route =
            WorkflowRankingEngine::new(route_fixture_workflows()).route_chat(N8nChatRouteRequest {
                prompt: prompt.into(),
                previous_user_prompt: None,
                manual_n8n_mode: false,
                safe_auto_run_enabled: false,
                workflows: Vec::new(),
            });

        assert_eq!(route.status, N8nChatRouteStatus::CreateWorkflow);
        assert_eq!(
            route
                .input_payload_preview
                .get("requested_workflow_name")
                .and_then(serde_json::Value::as_str),
            Some("KRIA Desktop Command E2E Safe Delete Guard")
        );
    }

    #[test]
    fn chat_router_keeps_permanent_delete_inside_create_name_safe() {
        let route = WorkflowRankingEngine::new(route_fixture_workflows())
            .route_chat(N8nChatRouteRequest {
            prompt:
                "Create an n8n workflow named Permanent Delete Guard that receives a movie title"
                    .into(),
            previous_user_prompt: None,
            manual_n8n_mode: false,
            safe_auto_run_enabled: false,
            workflows: Vec::new(),
        });

        assert_eq!(route.status, N8nChatRouteStatus::CreateWorkflow);
    }

    #[test]
    fn chat_router_routes_workflow_update_to_authoring_copy() {
        let route =
            WorkflowRankingEngine::new(route_fixture_workflows()).route_chat(N8nChatRouteRequest {
                prompt: "Update fetch_movies workflow so it accepts movie title from prompt".into(),
                previous_user_prompt: None,
                manual_n8n_mode: false,
                safe_auto_run_enabled: false,
                workflows: Vec::new(),
            });

        assert_eq!(route.status, N8nChatRouteStatus::UpdateWorkflow);
        assert_eq!(
            route
                .selected_workflow
                .as_ref()
                .map(|candidate| candidate.workflow_id.as_str()),
            Some("fetch_movies")
        );
        assert!(!route.can_auto_run);
    }

    #[test]
    fn n8n_desktop_chat_prompt_contract_routes_crud_archive_intents() {
        let engine = WorkflowRankingEngine::new(route_fixture_workflows());
        let cases = [
            (
                "Create an n8n workflow that receives a movie title and fetches movie details using HTTP",
                N8nChatRouteStatus::CreateWorkflow,
                None,
            ),
            (
                "Update fetch_movies workflow so it accepts title from prompt",
                N8nChatRouteStatus::UpdateWorkflow,
                Some("fetch_movies"),
            ),
            (
                "Delete workflow fetch_movies",
                N8nChatRouteStatus::OfferArchive,
                Some("fetch_movies"),
            ),
            (
                "Permanently delete workflow fetch_movies from n8n",
                N8nChatRouteStatus::DangerDeleteRequested,
                Some("fetch_movies"),
            ),
        ];

        for (prompt, expected_status, expected_workflow) in cases {
            let route = engine.route_chat(N8nChatRouteRequest {
                prompt: prompt.into(),
                previous_user_prompt: None,
                manual_n8n_mode: false,
                safe_auto_run_enabled: false,
                workflows: Vec::new(),
            });

            assert_eq!(route.status, expected_status, "{prompt}");
            assert!(!route.can_auto_run, "{prompt}");
            assert_ne!(route.status, N8nChatRouteStatus::UseOtherTool, "{prompt}");
            if let Some(expected_workflow) = expected_workflow {
                assert_eq!(
                    route
                        .selected_workflow
                        .as_ref()
                        .map(|candidate| candidate.workflow_id.as_str()),
                    Some(expected_workflow),
                    "{prompt}"
                );
            }
        }
    }

    #[test]
    fn chat_router_update_exact_draft_id_beats_fuzzy_approved_ranking() {
        let mut draft = workflow("draft_movie_source", "KRIA E2E Test Update Source");
        draft.status = N8nWorkflowStatus::Draft;
        draft.category = "data_retrieval".into();
        draft.risk_tier = RiskLevel::Green;
        draft.hitl_policy = "none".into();
        draft.trigger_strategy = "webhook".into();
        draft.result_mode = "poll_execution".into();
        draft.webhook_method = "POST".into();

        let mut approved = workflow("mail_schedule_test", "Mail Schedule Test");
        approved.category = "email".into();
        approved.description = "Update mail schedules and accept title from prompt.".into();
        approved.aliases = vec!["accepts title from prompt".into()];
        approved.tags = vec!["update".into(), "title".into(), "prompt".into()];

        let route =
            WorkflowRankingEngine::new(vec![approved, draft]).route_chat(N8nChatRouteRequest {
                prompt: "Update draft_movie_source so it accepts title from prompt".into(),
                previous_user_prompt: None,
                manual_n8n_mode: false,
                safe_auto_run_enabled: false,
                workflows: Vec::new(),
            });

        assert_eq!(route.status, N8nChatRouteStatus::UpdateWorkflow);
        assert_eq!(
            route
                .selected_workflow
                .as_ref()
                .map(|candidate| candidate.workflow_id.as_str()),
            Some("draft_movie_source")
        );
        assert_eq!(route.candidates.len(), 1);
        assert_eq!(
            route.candidates[0].reason,
            "Exact workflow_id match for update"
        );
        assert!(route
            .trace
            .iter()
            .any(|entry| entry == "update_target=exact_workflow_id"));
        assert!(!route.can_auto_run);
    }

    #[test]
    fn chat_router_blocks_exact_archived_update_target() {
        let mut archived = workflow("archived_movie_workflow", "Archived Movie Workflow");
        archived.status = N8nWorkflowStatus::Draft;
        archived.archived = true;

        let route = WorkflowRankingEngine::new(vec![archived]).route_chat(N8nChatRouteRequest {
            prompt: "Update archived_movie_workflow so it accepts title from prompt".into(),
            previous_user_prompt: None,
            manual_n8n_mode: false,
            safe_auto_run_enabled: false,
            workflows: Vec::new(),
        });

        assert_eq!(route.status, N8nChatRouteStatus::UpdateWorkflow);
        assert!(route.selected_workflow.is_none());
        assert!(route
            .blockers
            .iter()
            .any(|blocker| blocker.contains("Restore it before updating")));
        assert_eq!(route.next_actions, vec!["Restore workflow".to_string()]);
        assert!(!route.can_auto_run);
    }

    #[test]
    fn chat_router_draft_exact_id_does_not_become_runnable() {
        let mut draft = workflow("draft_movie_source", "Draft Movie Workflow");
        draft.status = N8nWorkflowStatus::Draft;
        draft.trigger_strategy = "webhook".into();
        draft.result_mode = "poll_execution".into();
        draft.webhook_method = "POST".into();

        let route = WorkflowRankingEngine::new(vec![draft]).route_chat(N8nChatRouteRequest {
            prompt: "Run draft_movie_source".into(),
            previous_user_prompt: None,
            manual_n8n_mode: true,
            safe_auto_run_enabled: true,
            workflows: Vec::new(),
        });

        assert_ne!(route.status, N8nChatRouteStatus::ReadyToRun);
        assert!(route.selected_workflow.is_none());
        assert!(!route.can_auto_run);
    }

    #[test]
    fn chat_router_run_id_containing_test_is_not_authoring_test_intent() {
        let mut workflow = workflow("kria_e2e_test_runnable", "KRIA E2E Test Runnable");
        workflow.status = N8nWorkflowStatus::Approved;
        workflow.trigger_strategy = "webhook".into();
        workflow.result_mode = "poll_execution".into();
        workflow.webhook_method = "POST".into();

        let route = WorkflowRankingEngine::new(vec![workflow]).route_chat(N8nChatRouteRequest {
            prompt: "Run kria_e2e_test_runnable with title Inception".into(),
            previous_user_prompt: None,
            manual_n8n_mode: true,
            safe_auto_run_enabled: false,
            workflows: Vec::new(),
        });

        assert_ne!(route.status, N8nChatRouteStatus::TestAuthoringDraft);
        assert!(matches!(
            route.status,
            N8nChatRouteStatus::ConfirmRequired | N8nChatRouteStatus::ReadyToRun
        ));
    }

    #[test]
    fn chat_router_blocks_lifecycle_drift_before_run() {
        let mut workflows = route_fixture_workflows();
        let fetch = workflows
            .iter_mut()
            .find(|workflow| workflow.workflow_id == "fetch_movies")
            .unwrap();
        fetch.lifecycle_status = "copy_changed".into();
        fetch.lifecycle_warnings = vec!["copy was edited after approval".into()];

        let route = WorkflowRankingEngine::new(workflows).route_chat(N8nChatRouteRequest {
            prompt: "Run fetch_movies".into(),
            previous_user_prompt: None,
            manual_n8n_mode: true,
            safe_auto_run_enabled: true,
            workflows: Vec::new(),
        });

        assert_eq!(route.status, N8nChatRouteStatus::Blocked);
        assert!(route
            .blockers
            .iter()
            .any(|blocker| blocker.contains("changed")));
        assert!(!route.can_auto_run);
    }

    #[test]
    fn chat_router_only_auto_runs_safe_exact_manual_n8n_prompts() {
        let engine = WorkflowRankingEngine::new(route_fixture_workflows());
        let normal = engine.route_chat(N8nChatRouteRequest {
            prompt: "Run fetch_movies".into(),
            previous_user_prompt: None,
            manual_n8n_mode: false,
            safe_auto_run_enabled: false,
            workflows: Vec::new(),
        });
        assert!(!normal.can_auto_run);

        let manual = engine.route_chat(N8nChatRouteRequest {
            prompt: "Run fetch_movies".into(),
            previous_user_prompt: None,
            manual_n8n_mode: true,
            safe_auto_run_enabled: false,
            workflows: Vec::new(),
        });
        assert_eq!(manual.status, N8nChatRouteStatus::ReadyToRun);
        assert!(manual.can_auto_run);

        let slack = engine.route_chat(N8nChatRouteRequest {
            prompt: "Run slack_post_update".into(),
            previous_user_prompt: None,
            manual_n8n_mode: true,
            safe_auto_run_enabled: true,
            workflows: Vec::new(),
        });
        assert_eq!(slack.status, N8nChatRouteStatus::ConfirmRequired);
        assert!(!slack.can_auto_run);
    }

    #[derive(Debug, serde::Deserialize)]
    struct ChatRoutingEvalCase {
        id: String,
        prompt: String,
        expected_status: String,
        #[serde(default)]
        expected_top: Option<String>,
        #[serde(default)]
        expected_can_auto_run: Option<bool>,
        #[serde(default)]
        manual_n8n_mode: bool,
        #[serde(default)]
        safe_auto_run_enabled: bool,
    }

    #[test]
    #[ignore]
    fn n8n_chat_routing_eval_dataset() {
        let dataset_path = std::env::var("N8N_CHAT_ROUTING_EVAL_DATASET")
            .unwrap_or_else(|_| "planning_docs/n8n_chat_routing_eval_dataset.jsonl".into());
        let content = std::fs::read_to_string(&dataset_path)
            .unwrap_or_else(|error| panic!("failed to read {dataset_path}: {error}"));
        let cases = content
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
            .map(|line| {
                serde_json::from_str::<ChatRoutingEvalCase>(line)
                    .unwrap_or_else(|error| panic!("invalid eval row: {line}\n{error}"))
            })
            .collect::<Vec<_>>();
        assert!(!cases.is_empty(), "chat routing eval dataset is empty");

        let engine = WorkflowRankingEngine::new(route_fixture_workflows());
        let mut failures = Vec::new();
        let mut false_auto_run = 0usize;
        for case in &cases {
            let route = engine.route_chat(N8nChatRouteRequest {
                prompt: case.prompt.clone(),
                previous_user_prompt: None,
                manual_n8n_mode: case.manual_n8n_mode,
                safe_auto_run_enabled: case.safe_auto_run_enabled,
                workflows: Vec::new(),
            });
            let actual_status = serde_json::to_value(route.status)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default();
            if actual_status != case.expected_status {
                failures.push(format!(
                    "{} expected status={} got={} prompt={}",
                    case.id, case.expected_status, actual_status, case.prompt
                ));
            }
            if let Some(expected_top) = &case.expected_top {
                let actual_top = route
                    .selected_workflow
                    .as_ref()
                    .map(|candidate| candidate.workflow_id.as_str())
                    .unwrap_or("-");
                if actual_top != expected_top {
                    failures.push(format!(
                        "{} expected top={} got={} prompt={}",
                        case.id, expected_top, actual_top, case.prompt
                    ));
                }
            }
            if let Some(expected) = case.expected_can_auto_run {
                if route.can_auto_run != expected {
                    failures.push(format!(
                        "{} expected can_auto_run={} got={} prompt={}",
                        case.id, expected, route.can_auto_run, case.prompt
                    ));
                }
            }
            if route.can_auto_run && !case.manual_n8n_mode && !case.safe_auto_run_enabled {
                false_auto_run += 1;
            }
        }

        assert_eq!(false_auto_run, 0, "false auto-run count must stay zero");
        assert!(
            failures.is_empty(),
            "n8n chat routing eval failures:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn parses_confirmation_references() {
        assert_eq!(
            WorkflowConfirmationFlow::parse_confirmation_reference(
                "Confirm workflow gmail_inbox_digest"
            ),
            Some("gmail_inbox_digest".into())
        );
        assert_eq!(
            WorkflowConfirmationFlow::parse_confirmation_reference(
                "yes run workflow `slack_post_update`, please"
            ),
            Some("slack_post_update".into())
        );
        assert_eq!(
            WorkflowConfirmationFlow::parse_confirmation_reference("confirm workflow ../../secret"),
            None
        );
    }
}
