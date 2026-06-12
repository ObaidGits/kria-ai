use super::context::GuiContext;
use super::perception::{sanitize_gui_text, stable_hash};

const MAX_GOAL_SUMMARY_CHARS: usize = 180;
const MAX_HINT_CHARS: usize = 80;
const MAX_FINAL_STATE_CHARS: usize = 160;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GuiActionType {
    Observe,
    AnalyzePlan,
    FocusInput,
    TypeText,
    ClickControl,
    BrowserSearch,
    BrowserNavigate,
    FillForm,
    OpenApp,
    SwitchWindow,
    Save,
    Download,
    CopyContent,
    PasteContent,
    Recovery,
    RiskApproval,
    SafeAction,
    Unknown,
}

impl GuiActionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::AnalyzePlan => "analyze_plan",
            Self::FocusInput => "focus_input",
            Self::TypeText => "type_text",
            Self::ClickControl => "click_control",
            Self::BrowserSearch => "browser_search",
            Self::BrowserNavigate => "browser_navigate",
            Self::FillForm => "fill_form",
            Self::OpenApp => "open_app",
            Self::SwitchWindow => "switch_window",
            Self::Save => "save",
            Self::Download => "download",
            Self::CopyContent => "copy_content",
            Self::PasteContent => "paste_content",
            Self::Recovery => "recovery",
            Self::RiskApproval => "risk_approval",
            Self::SafeAction => "safe_action",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GuiRiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl GuiRiskLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GuiGoalExtractionMode {
    Deterministic,
    LlmFallbackUnavailable,
}

impl GuiGoalExtractionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Deterministic => "deterministic",
            Self::LlmFallbackUnavailable => "llm_fallback_unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GuiGoalAmbiguity {
    pub kind: String,
    pub field: Option<String>,
    pub message: String,
}

impl GuiGoalAmbiguity {
    pub fn new(kind: impl Into<String>, field: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            field: field.map(str::to_string),
            message: sanitize_gui_text(&message.into(), 180).text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiGoalEvidence {
    pub source: String,
    pub field: String,
    pub summary: String,
    pub confidence: f64,
}

impl GuiGoalEvidence {
    pub fn new(
        source: impl Into<String>,
        field: impl Into<String>,
        summary: impl Into<String>,
        confidence: f64,
    ) -> Self {
        Self {
            source: sanitize_gui_text(&source.into(), 40).text,
            field: sanitize_gui_text(&field.into(), 60).text,
            summary: sanitize_gui_text(&summary.into(), 160).text,
            confidence: confidence.clamp(0.0, 1.0),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiGoalContract {
    pub contract_id: String,
    pub observation_id: String,
    pub context_id: String,
    pub prompt_hash: String,
    pub goal_summary: String,
    pub intent_kind: String,
    pub action_type: GuiActionType,
    pub target_app_kind: Option<String>,
    pub target_app_hint: Option<String>,
    pub target_window_hint: Option<String>,
    pub target_control_hint: Option<String>,
    pub query_summary: Option<String>,
    pub query_hash: Option<String>,
    pub text_payload_summary: Option<String>,
    pub text_payload_hash: Option<String>,
    pub desired_final_state: String,
    pub risk_level: GuiRiskLevel,
    pub requires_user_approval: bool,
    pub ambiguities: Vec<GuiGoalAmbiguity>,
    pub source_evidence: Vec<GuiGoalEvidence>,
    pub extraction_confidence: f64,
    pub extractor_mode: GuiGoalExtractionMode,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiGoalExtractionReport {
    pub contract: GuiGoalContract,
    pub warnings: Vec<String>,
}

impl GuiGoalContract {
    pub fn event_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "GoalContractCreated",
            "contract_id": self.contract_id,
            "observation_id": self.observation_id,
            "context_id": self.context_id,
            "goal_summary": self.goal_summary,
            "intent_kind": self.intent_kind,
            "action_type": self.action_type.as_str(),
            "prompt_hash": self.prompt_hash,
            "target_app_kind": self.target_app_kind,
            "target_app_hint": self.target_app_hint,
            "target_window_hint": self.target_window_hint,
            "target_control_hint": self.target_control_hint,
            "query_summary": self.query_summary,
            "query_hash": self.query_hash,
            "text_payload_summary": self.text_payload_summary,
            "text_payload_hash": self.text_payload_hash,
            "desired_final_state": self.desired_final_state,
            "risk_level": self.risk_level.as_str(),
            "requires_user_approval": self.requires_user_approval,
            "ambiguity_count": self.ambiguities.len(),
            "ambiguities": self.ambiguities,
            "source_evidence": self.source_evidence,
            "extraction_confidence": self.extraction_confidence,
            "extractor_mode": self.extractor_mode.as_str(),
        })
    }

    pub fn response_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "contract_id": self.contract_id,
            "observation_id": self.observation_id,
            "context_id": self.context_id,
            "prompt_hash": self.prompt_hash,
            "goal_summary": self.goal_summary,
            "intent_kind": self.intent_kind,
            "action_type": self.action_type.as_str(),
            "target_app_kind": self.target_app_kind,
            "target_app_hint": self.target_app_hint,
            "target_window_hint": self.target_window_hint,
            "target_control_hint": self.target_control_hint,
            "query_summary": self.query_summary,
            "query_hash": self.query_hash,
            "text_payload_summary": self.text_payload_summary,
            "text_payload_hash": self.text_payload_hash,
            "desired_final_state": self.desired_final_state,
            "risk_level": self.risk_level.as_str(),
            "requires_user_approval": self.requires_user_approval,
            "ambiguity_count": self.ambiguities.len(),
            "ambiguities": self.ambiguities,
            "source_evidence": self.source_evidence,
            "extraction_confidence": self.extraction_confidence,
            "extractor_mode": self.extractor_mode.as_str(),
        })
    }
}

pub fn extract_gui_goal_contract(
    prompt: &str,
    context: Option<&GuiContext>,
) -> GuiGoalExtractionReport {
    let safe_prompt = sanitize_gui_text(prompt, MAX_GOAL_SUMMARY_CHARS);
    let lower = normalize_prompt(prompt);
    let quoted = extract_first_quoted_segment(prompt);
    let typed_text = extract_text_payload(prompt, &lower, quoted.as_deref());
    let target_control_hint = extract_target_control_hint(prompt, &lower);
    let app_mentions = extract_app_mentions(&lower);
    let multiple_app_targets = app_mentions.len() > 1;
    let (target_app_kind, target_app_hint, app_source) = resolve_app_hint(&app_mentions, context);
    let target_window_hint = extract_window_hint(prompt, &lower, context);
    let query_summary = extract_query_summary(prompt, &lower, &app_mentions);
    let risk_reasons = risk_reasons_for(&lower);
    let requires_user_approval = !risk_reasons.is_empty();
    let explicit_risk_instruction = requires_user_approval
        && contains_any(
            &lower,
            &[
                "ask",
                "approval",
                "confirmation",
                "pause",
                "prepare",
                "before",
            ],
        );
    let action_type = action_type_for(
        &lower,
        typed_text.as_deref(),
        query_summary.as_deref(),
        target_control_hint.as_deref(),
        target_app_hint.as_deref(),
        explicit_risk_instruction,
    );
    let intent_kind = legacy_intent_kind_for(&action_type, &lower);
    let risk_level = risk_level_for(&risk_reasons, &lower);
    let mut ambiguities = ambiguities_for(
        &action_type,
        typed_text.as_deref(),
        query_summary.as_deref(),
        target_control_hint.as_deref(),
        target_app_hint.as_deref(),
        multiple_app_targets,
        requires_user_approval,
        explicit_risk_instruction,
    );
    if context.map(|ctx| ctx.ocr_has_injection()).unwrap_or(false) {
        ambiguities.push(GuiGoalAmbiguity::new(
            "untrusted_ocr_present",
            Some("ocr"),
            "OCR injection-like text is present and was ignored as an instruction source.",
        ));
    }
    let desired_final_state = desired_final_state_for(
        &action_type,
        typed_text.as_deref(),
        query_summary.as_deref(),
        target_control_hint.as_deref(),
        requires_user_approval,
    );
    let goal_summary = goal_summary_for(
        &action_type,
        target_app_hint.as_deref(),
        target_window_hint.as_deref(),
        target_control_hint.as_deref(),
        typed_text.as_deref(),
        query_summary.as_deref(),
        &safe_prompt.text,
    );
    let source_evidence = source_evidence_for(
        &action_type,
        target_app_kind.as_deref(),
        target_app_hint.as_deref(),
        app_source.as_deref(),
        target_control_hint.as_deref(),
        query_summary.as_deref(),
        typed_text.as_deref(),
        &risk_reasons,
    );
    let mut confidence = confidence_for(
        &action_type,
        &ambiguities,
        target_control_hint.as_deref(),
        query_summary.as_deref(),
        typed_text.as_deref(),
    );
    if context.is_none() {
        confidence = (confidence - 0.08_f64).max(0.35_f64);
    }

    let contract = GuiGoalContract {
        contract_id: stable_hash(&format!(
            "{}|{}|{}",
            prompt,
            context
                .map(|ctx| ctx.observation_id.as_str())
                .unwrap_or("no-observation"),
            context
                .map(|ctx| ctx.context_id.as_str())
                .unwrap_or("no-context")
        )),
        observation_id: context
            .map(|ctx| ctx.observation_id.clone())
            .unwrap_or_default(),
        context_id: context
            .map(|ctx| ctx.context_id.clone())
            .unwrap_or_default(),
        prompt_hash: stable_hash(prompt),
        goal_summary,
        intent_kind,
        action_type,
        target_app_kind,
        target_app_hint,
        target_window_hint,
        target_control_hint,
        query_hash: query_summary.as_ref().map(|value| stable_hash(value)),
        query_summary,
        text_payload_hash: typed_text.as_ref().map(|value| stable_hash(value)),
        text_payload_summary: typed_text,
        desired_final_state,
        risk_level,
        requires_user_approval,
        ambiguities,
        source_evidence,
        extraction_confidence: confidence,
        extractor_mode: GuiGoalExtractionMode::Deterministic,
    };
    let mut warnings = Vec::new();
    if safe_prompt.redaction_applied {
        warnings.push("Prompt summary was redacted before goal extraction.".into());
    }
    GuiGoalExtractionReport { contract, warnings }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppMention {
    kind: &'static str,
    label: &'static str,
    aliases: &'static [&'static str],
}

fn normalize_prompt(prompt: &str) -> String {
    prompt
        .to_lowercase()
        .replace(['\n', '\t'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_app_mentions(lower: &str) -> Vec<AppMention> {
    const APPS: &[AppMention] = &[
        AppMention {
            kind: "browser",
            label: "Chrome",
            aliases: &["chrome", "google chrome"],
        },
        AppMention {
            kind: "browser",
            label: "Chromium",
            aliases: &["chromium"],
        },
        AppMention {
            kind: "browser",
            label: "Firefox",
            aliases: &["firefox"],
        },
        AppMention {
            kind: "browser",
            label: "Brave",
            aliases: &["brave"],
        },
        AppMention {
            kind: "browser",
            label: "browser",
            aliases: &["browser", "web browser"],
        },
        AppMention {
            kind: "browser",
            label: "Google",
            aliases: &["google"],
        },
        AppMention {
            kind: "editor",
            label: "VS Code",
            aliases: &["vscode", "vs code", "visual studio code"],
        },
        AppMention {
            kind: "editor",
            label: "editor",
            aliases: &["editor", "ide"],
        },
        AppMention {
            kind: "terminal",
            label: "terminal",
            aliases: &["terminal", "console", "shell"],
        },
        AppMention {
            kind: "file_manager",
            label: "file manager",
            aliases: &["file manager", "files", "folder"],
        },
        AppMention {
            kind: "email",
            label: "Gmail",
            aliases: &["gmail"],
        },
        AppMention {
            kind: "email",
            label: "email",
            aliases: &["email", "mail"],
        },
        AppMention {
            kind: "meeting",
            label: "Zoom",
            aliases: &["zoom"],
        },
        AppMention {
            kind: "chat",
            label: "Slack",
            aliases: &["slack"],
        },
    ];

    let mut mentions = Vec::new();
    for app in APPS {
        if app.aliases.iter().any(|alias| phrase_present(lower, alias)) {
            if !mentions
                .iter()
                .any(|existing: &AppMention| existing.label == app.label)
            {
                mentions.push(app.clone());
            }
        }
    }
    mentions
}

fn phrase_present(lower: &str, phrase: &str) -> bool {
    if phrase.contains(' ') {
        lower.contains(phrase)
    } else if let Ok(re) = regex::Regex::new(&format!(r"\b{}\b", regex::escape(phrase))) {
        re.is_match(lower)
    } else {
        lower.contains(phrase)
    }
}

fn resolve_app_hint(
    app_mentions: &[AppMention],
    context: Option<&GuiContext>,
) -> (Option<String>, Option<String>, Option<String>) {
    if let Some(app) = app_mentions.first() {
        return (
            Some(app.kind.into()),
            Some(app.label.into()),
            Some("user_prompt".into()),
        );
    }
    if let Some(app_name) = context
        .and_then(|ctx| ctx.active_window.app_name.as_ref())
        .filter(|value| !value.trim().is_empty())
    {
        let safe = sanitize_gui_text(app_name, MAX_HINT_CHARS).text;
        return (
            Some(app_kind_for_label(&safe).into()),
            Some(safe),
            Some("context".into()),
        );
    }
    (None, None, None)
}

fn app_kind_for_label(label: &str) -> &'static str {
    let lower = label.to_lowercase();
    if contains_any(
        &lower,
        &[
            "chrome", "chromium", "firefox", "brave", "browser", "google",
        ],
    ) {
        "browser"
    } else if contains_any(&lower, &["vscode", "vs code", "editor", "ide"]) {
        "editor"
    } else if contains_any(&lower, &["terminal", "console", "shell"]) {
        "terminal"
    } else if contains_any(&lower, &["file", "folder"]) {
        "file_manager"
    } else if contains_any(&lower, &["mail", "gmail", "email"]) {
        "email"
    } else {
        "unknown"
    }
}

pub fn contains_any(lower: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| lower.contains(needle))
}

pub fn extract_first_quoted_segment(input: &str) -> Option<String> {
    let mut quote = None;
    let mut start = 0usize;
    for (idx, ch) in input.char_indices() {
        match quote {
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                start = idx + ch.len_utf8();
            }
            Some(expected) if ch == expected => {
                let value = input[start..idx].trim();
                if !value.is_empty() {
                    let sanitized = sanitize_gui_text(value, 10_000).text;
                    return Some(sanitized.chars().take(240).collect());
                }
                quote = None;
            }
            _ => {}
        }
    }
    None
}

pub fn extract_named_control(prompt: &str, lower: &str) -> Option<String> {
    if let Ok(re) = regex::Regex::new(
        r#"(?i)\b(?:button|control|field|input)?\s*(?:named|called|label(?:ed)?)\s+["']?([A-Za-z0-9][A-Za-z0-9 _./:-]{0,48})"#,
    ) {
        if let Some(cap) = re.captures(prompt) {
            if let Some(matched) = cap.get(1) {
                let mut value = sanitize_gui_text(matched.as_str(), MAX_HINT_CHARS)
                    .text
                    .trim_matches(['"', '\''])
                    .to_string();
                for stop in [" and ", " then ", " verify ", " but ", " before ", " if "] {
                    if let Some((head, _)) = value.split_once(stop) {
                        value = head.trim().to_string();
                    }
                }
                value = value
                    .trim()
                    .trim_end_matches(['.', ',', ';', ':'])
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
    }

    if lower.contains("search") && (lower.contains("button") || lower.contains("click")) {
        return Some("Search".into());
    }
    if lower.contains("submit") && lower.contains("button") {
        return Some("Submit".into());
    }
    None
}

fn extract_target_control_hint(prompt: &str, lower: &str) -> Option<String> {
    extract_named_control(prompt, lower).or_else(|| {
        if contains_any(
            lower,
            &["search/input", "search field", "input field", "text field"],
        ) {
            Some("visible text input".into())
        } else {
            None
        }
    })
}

fn extract_text_payload(prompt: &str, lower: &str, quoted: Option<&str>) -> Option<String> {
    if let Some(value) = quoted {
        return Some(sanitize_gui_text(value, MAX_HINT_CHARS).text);
    }
    let patterns = [
        r#"(?i)\b(?:type|write|enter)\s+(.+?)\s+(?:into|in|to)\s+(?:the\s+)?(?:visible\s+)?(?:text\s+)?(?:field|input|box|search field|search box)\b"#,
        r#"(?i)\b(?:type|write|enter)\s+(.+?)\s+(?:on|in)\s+(?:the\s+)?(?:current\s+)?(?:screen|page|window)\b"#,
    ];
    for pattern in patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(cap) = re.captures(prompt) {
                if let Some(value) = cap.get(1) {
                    let value = clean_slot_value(value.as_str(), &[]);
                    if useful_text_payload(&value) {
                        return Some(value);
                    }
                }
            }
        }
    }
    if contains_any(lower, &["type ", "write ", "enter "]) {
        None
    } else {
        quoted.map(|value| sanitize_gui_text(value, MAX_HINT_CHARS).text)
    }
}

fn useful_text_payload(value: &str) -> bool {
    let lower = value.trim().to_lowercase();
    !lower.is_empty()
        && !matches!(
            lower.as_str(),
            "the" | "a" | "an" | "in" | "into" | "to" | "field" | "input"
        )
}

fn extract_query_summary(prompt: &str, lower: &str, app_mentions: &[AppMention]) -> Option<String> {
    let patterns = [
        r#"(?i)\bsearch(?:\s+for)?\s+(.+)"#,
        r#"(?i)\blook\s+up\s+(.+)"#,
        r#"(?i)\bgoogle\s+(.+)"#,
        r#"(?i)\bfind\s+(.+)"#,
        r#"(?i)^(.+?)\s+search\s+karo\b"#,
        r#"(?i)^(.+?)\s+find\s+karo\b"#,
        r#"(?i)^(.+?)\s+dhoondo\b"#,
        r#"(?i)^(.+?)\s+dekho\b"#,
    ];
    for pattern in patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(cap) = re.captures(prompt) {
                if let Some(value) = cap.get(1) {
                    let value = clean_query_value(value.as_str(), app_mentions);
                    if !value.is_empty() {
                        return Some(value);
                    }
                }
            }
        }
    }
    if lower.contains("search") || lower.contains("google") || lower.contains("look up") {
        None
    } else {
        None
    }
}

fn clean_query_value(value: &str, app_mentions: &[AppMention]) -> String {
    let mut value = value.to_string();
    for stop in [
        " and summarize",
        " and report",
        " then ",
        " but ",
        " before ",
        " after ",
        " with approval",
        " ask approval",
    ] {
        if let Some((head, _)) = value.to_lowercase().split_once(stop) {
            value = value.chars().take(head.len()).collect();
        }
    }
    let aliases = app_mentions
        .iter()
        .flat_map(|app| app.aliases.iter().copied())
        .chain(["browser", "google", "chrome", "firefox", "brave"]);
    clean_slot_value(&value, &aliases.collect::<Vec<_>>())
}

fn clean_slot_value(value: &str, remove_phrases: &[&str]) -> String {
    let mut cleaned = sanitize_gui_text(value, MAX_HINT_CHARS).text;
    for phrase in remove_phrases {
        if phrase.trim().is_empty() {
            continue;
        }
        if let Ok(re) = regex::Regex::new(&format!(r"(?i)\b{}\b", regex::escape(phrase))) {
            cleaned = re.replace_all(&cleaned, " ").to_string();
        }
    }
    if let Ok(re) =
        regex::Regex::new(r"(?i)\b(in|on|with|using|me|mein|pe|for|and|karo|dekho|dhoondo)\b")
    {
        cleaned = re.replace_all(&cleaned, " ").to_string();
    }
    cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(['"', '\'', '.', ',', ';', ':'])
        .trim()
        .to_string()
}

fn extract_window_hint(prompt: &str, lower: &str, context: Option<&GuiContext>) -> Option<String> {
    if contains_any(
        lower,
        &["current screen", "current window", "active window"],
    ) {
        return context
            .map(|ctx| ctx.active_window.label.clone())
            .filter(|value| !value.trim().is_empty() && value != "unknown")
            .or_else(|| Some("active window".into()));
    }
    if let Ok(re) = regex::Regex::new(
        r#"(?i)\bwindow\s+(?:named|called|title(?:d)?)\s+["']?([A-Za-z0-9][A-Za-z0-9 _./:-]{0,64})"#,
    ) {
        if let Some(cap) = re.captures(prompt) {
            if let Some(value) = cap.get(1) {
                return Some(sanitize_gui_text(value.as_str(), MAX_HINT_CHARS).text);
            }
        }
    }
    context
        .map(|ctx| ctx.active_window.label.clone())
        .filter(|value| !value.trim().is_empty() && value != "unknown")
}

fn risk_reasons_for(lower: &str) -> Vec<String> {
    let mut risk_reasons = Vec::new();
    for (needle, reason) in [
        ("delete", "destructive delete/remove action"),
        ("remove", "destructive delete/remove action"),
        ("archive", "destructive or state-changing archive action"),
        ("install", "installation or system modification action"),
        ("system setting", "system setting change"),
        ("security setting", "security setting change"),
        ("submit", "external submit action"),
        ("send", "external communication action"),
        ("confirm order", "external commitment action"),
        ("pay", "financial/payment action"),
        ("payment", "financial/payment action"),
        ("purchase", "financial/purchase action"),
        ("book", "booking/commitment action"),
        ("password", "credential/authentication action"),
        ("credential", "credential/authentication action"),
        ("git push", "remote git write action"),
        ("git merge", "risky git history/state action"),
        ("git rebase", "risky git history/state action"),
        ("overwrite", "overwrite action"),
    ] {
        if contains_unnegated_risk_phrase(lower, needle)
            && !risk_reasons.iter().any(|item| item == reason)
        {
            risk_reasons.push(reason.to_string());
        }
    }
    risk_reasons
}

fn risk_level_for(risk_reasons: &[String], lower: &str) -> GuiRiskLevel {
    if ["pay", "payment", "purchase", "irreversible"]
        .iter()
        .any(|needle| contains_unnegated_risk_phrase(lower, needle))
    {
        GuiRiskLevel::Critical
    } else if !risk_reasons.is_empty() {
        GuiRiskLevel::High
    } else if contains_any(
        lower,
        &[
            "type ", "enter ", "write ", "fill", "save", "download", "copy", "paste",
        ],
    ) {
        GuiRiskLevel::Medium
    } else {
        GuiRiskLevel::Low
    }
}

fn contains_unnegated_risk_phrase(lower: &str, needle: &str) -> bool {
    lower.match_indices(needle).any(|(index, _)| {
        let start = index.saturating_sub(56);
        let prefix = &lower[start..index];
        ![
            "do not", "don't", "dont", "without", "never", "avoid", "not ", "no ",
        ]
        .iter()
        .any(|marker| prefix.contains(marker))
    })
}

fn action_type_for(
    lower: &str,
    typed_text: Option<&str>,
    query_summary: Option<&str>,
    target_control_hint: Option<&str>,
    target_app_hint: Option<&str>,
    explicit_risk_instruction: bool,
) -> GuiActionType {
    if explicit_risk_instruction {
        GuiActionType::RiskApproval
    } else if contains_any(
        lower,
        &[
            "focus is lost",
            "recover focus",
            "focus lost",
            "focus moves away",
            "lost focus",
        ],
    ) {
        GuiActionType::Recovery
    } else if is_browser_search_intent(lower, query_summary, target_app_hint) {
        GuiActionType::BrowserSearch
    } else if is_browser_navigation_intent(lower) {
        GuiActionType::BrowserNavigate
    } else if contains_any(lower, &["type ", "enter ", "write "]) || typed_text.is_some() {
        GuiActionType::TypeText
    } else if lower.contains("focus") && contains_any(lower, &["input", "field", "search", "text"])
    {
        GuiActionType::FocusInput
    } else if lower.contains("click") {
        GuiActionType::ClickControl
    } else if lower.contains("fill") && lower.contains("form") {
        GuiActionType::FillForm
    } else if contains_any(
        lower,
        &["open ", "open app", "launch", "start app", "start "],
    ) && target_app_hint.is_some()
    {
        GuiActionType::OpenApp
    } else if contains_any(lower, &["switch window", "switch to"]) {
        GuiActionType::SwitchWindow
    } else if phrase_present(lower, "save") {
        GuiActionType::Save
    } else if phrase_present(lower, "download") {
        GuiActionType::Download
    } else if phrase_present(lower, "copy") {
        GuiActionType::CopyContent
    } else if phrase_present(lower, "paste") {
        GuiActionType::PasteContent
    } else if lower.contains("target")
        && contains_any(lower, &["missing", "ambiguous", "not found"])
    {
        GuiActionType::AnalyzePlan
    } else if contains_any(lower, &["multiple", "ambiguous", "similar"]) {
        GuiActionType::AnalyzePlan
    } else if contains_any(lower, &["perform one safe", "one safe gui action"]) {
        GuiActionType::SafeAction
    } else if contains_any(lower, &["plan", "analyze", "validate"]) {
        GuiActionType::AnalyzePlan
    } else if target_control_hint.is_some() && lower.contains("search") {
        GuiActionType::BrowserSearch
    } else if is_observe_request(lower) {
        GuiActionType::Observe
    } else {
        GuiActionType::Unknown
    }
}

fn is_browser_search_intent(
    lower: &str,
    _query_summary: Option<&str>,
    target_app_hint: Option<&str>,
) -> bool {
    if lower.contains("click") && contains_any(lower, &["button", "control"]) {
        return false;
    }
    if lower.contains("focus") && contains_any(lower, &["input", "field", "search/input", "text"]) {
        return false;
    }
    let has_search_phrase = contains_any(
        lower,
        &[
            "search",
            "search for",
            "look up",
            "google",
            "search karo",
            "find karo",
            "dhoondo",
            "dekho",
        ],
    );
    let has_find_browser = lower.contains("find")
        && (target_app_hint
            .map(|hint| app_kind_for_label(hint) == "browser")
            .unwrap_or(false)
            || contains_any(lower, &["browser", "web", "google", "weather"]));
    has_search_phrase || has_find_browser
}

fn is_browser_navigation_intent(lower: &str) -> bool {
    contains_any(
        lower,
        &["navigate", "go to", "open website", "open url", "visit "],
    ) || regex::Regex::new(r#"(?i)\bhttps?://|\b[a-z0-9-]+\.(com|org|net|io|dev)\b"#)
        .map(|re| re.is_match(lower))
        .unwrap_or(false)
}

fn is_observe_request(lower: &str) -> bool {
    contains_any(
        lower,
        &[
            "observe",
            "what is on my screen",
            "what's on my screen",
            "what is focused on my screen",
            "what's focused on my screen",
            "focused on my screen",
            "what controls are visible",
            "what buttons or visual controls",
            "visible controls",
            "visual controls",
            "buttons or visual controls",
            "controls are visible",
            "report accessibility health",
            "report ocr status",
            "ocr trust status",
            "perception latency",
            "gui action backend status",
            "gui context quality",
            "freshness status",
            "which controls are executable",
            "report which controls are executable",
            "check the current desktop",
            "current desktop",
            "report active window",
            "report focus source",
            "editable target status",
            "current screen",
            "confirmation dialog",
            "permission popup",
            "visible popup",
            "which app/window is active",
            "whether ocr/accessibility are usable",
            "tell me the active window",
        ],
    )
}

fn legacy_intent_kind_for(action_type: &GuiActionType, lower: &str) -> String {
    if lower.contains("target") && contains_any(lower, &["missing", "ambiguous", "not found"]) {
        return "target_availability_check".into();
    }
    if contains_any(lower, &["multiple", "ambiguous", "similar"]) {
        return "ambiguity_check".into();
    }
    match action_type {
        GuiActionType::Observe => "observe",
        GuiActionType::AnalyzePlan => "analyze_plan",
        GuiActionType::FocusInput => "focus_input",
        GuiActionType::TypeText => "type_text",
        GuiActionType::ClickControl => "click_control",
        GuiActionType::BrowserSearch => "browser_search",
        GuiActionType::BrowserNavigate => "browser_navigate",
        GuiActionType::FillForm => "fill_form_plan",
        GuiActionType::OpenApp | GuiActionType::SwitchWindow => "analyze_plan",
        GuiActionType::Save => "save",
        GuiActionType::Download => "download",
        GuiActionType::CopyContent => "copy_content",
        GuiActionType::PasteContent => "paste_content",
        GuiActionType::Recovery => "focus_recovery",
        GuiActionType::RiskApproval => "risk_approval",
        GuiActionType::SafeAction => "safe_action",
        GuiActionType::Unknown => "unknown",
    }
    .into()
}

fn ambiguities_for(
    action_type: &GuiActionType,
    typed_text: Option<&str>,
    query_summary: Option<&str>,
    target_control_hint: Option<&str>,
    target_app_hint: Option<&str>,
    multiple_app_targets: bool,
    requires_user_approval: bool,
    explicit_risk_instruction: bool,
) -> Vec<GuiGoalAmbiguity> {
    let mut ambiguities = Vec::new();
    if matches!(action_type, GuiActionType::Unknown) {
        ambiguities.push(GuiGoalAmbiguity::new(
            "unsupported_goal",
            Some("action_type"),
            "The prompt does not describe a supported GUI cognition goal clearly enough.",
        ));
    }
    if matches!(action_type, GuiActionType::BrowserSearch) && query_summary.is_none() {
        ambiguities.push(GuiGoalAmbiguity::new(
            "missing_query",
            Some("query_summary"),
            "The prompt asks for a browser search but does not include a clear query.",
        ));
    }
    if matches!(action_type, GuiActionType::TypeText) && typed_text.is_none() {
        ambiguities.push(GuiGoalAmbiguity::new(
            "missing_text_payload",
            Some("typed_text"),
            "The prompt asks for text entry but does not include exact text in quotes.",
        ));
    }
    if matches!(action_type, GuiActionType::ClickControl) && target_control_hint.is_none() {
        ambiguities.push(GuiGoalAmbiguity::new(
            "missing_target_control",
            Some("target_control_hint"),
            "The prompt asks for a click but does not name an exact visible control.",
        ));
    }
    if multiple_app_targets {
        ambiguities.push(GuiGoalAmbiguity::new(
            "multiple_app_targets",
            Some("target_app_hint"),
            "The prompt mentions multiple possible app targets; clarification is required.",
        ));
    }
    if matches!(
        action_type,
        GuiActionType::OpenApp | GuiActionType::SwitchWindow
    ) && target_app_hint.is_none()
    {
        ambiguities.push(GuiGoalAmbiguity::new(
            "missing_app_target",
            Some("target_app_hint"),
            "The prompt asks for an app/window operation but the target app is unclear.",
        ));
    }
    if requires_user_approval && !explicit_risk_instruction {
        ambiguities.push(GuiGoalAmbiguity::new(
            "risky_without_explicit_approval_language",
            Some("approval"),
            "The prompt contains a risky action; explicit approval is required before execution.",
        ));
    }
    ambiguities
}

fn desired_final_state_for(
    action_type: &GuiActionType,
    typed_text: Option<&str>,
    query_summary: Option<&str>,
    target_control_hint: Option<&str>,
    requires_user_approval: bool,
) -> String {
    let value = match action_type {
        GuiActionType::Observe => "desktop state observed and summarized".to_string(),
        GuiActionType::AnalyzePlan => "safe GUI plan produced without execution".to_string(),
        GuiActionType::FocusInput => "target input field is focused and verified".to_string(),
        GuiActionType::TypeText => format!(
            "requested text is present in the resolved field{}",
            typed_text
                .map(|text| format!(": {text}"))
                .unwrap_or_default()
        ),
        GuiActionType::ClickControl => format!(
            "button/control clicked and screen change verified{}",
            target_control_hint
                .map(|target| format!(": {target}"))
                .unwrap_or_default()
        ),
        GuiActionType::BrowserSearch => query_summary
            .map(|query| format!("search results visible for {query}"))
            .unwrap_or_else(|| "search results visible after query is clarified".to_string()),
        GuiActionType::BrowserNavigate => {
            "browser navigation target is prepared safely".to_string()
        }
        GuiActionType::FillForm => "form field plan is validated before submit/send".to_string(),
        GuiActionType::OpenApp => {
            "target application is opened or blocker is explained".to_string()
        }
        GuiActionType::SwitchWindow => {
            "target window is focused or ambiguity is reported".to_string()
        }
        GuiActionType::Save => "requested content or state is saved and verified".to_string(),
        GuiActionType::Download => "requested download is prepared and verified".to_string(),
        GuiActionType::CopyContent => {
            "requested content is copied after safe target resolution".to_string()
        }
        GuiActionType::PasteContent => {
            "clipboard paste target is resolved before pasting".to_string()
        }
        GuiActionType::Recovery => "focus/state recovery is attempted only when safe".to_string(),
        GuiActionType::RiskApproval => {
            "bounded action is prepared and paused for approval".to_string()
        }
        GuiActionType::SafeAction => {
            "one uniquely resolvable low-risk action is executed and verified".to_string()
        }
        GuiActionType::Unknown => {
            "clarification is required before planning or execution".to_string()
        }
    };
    let value = if requires_user_approval {
        format!("{value}; approval required before risky execution")
    } else {
        value
    };
    sanitize_gui_text(&value, MAX_FINAL_STATE_CHARS).text
}

fn goal_summary_for(
    action_type: &GuiActionType,
    target_app_hint: Option<&str>,
    target_window_hint: Option<&str>,
    target_control_hint: Option<&str>,
    typed_text: Option<&str>,
    query_summary: Option<&str>,
    fallback_prompt_summary: &str,
) -> String {
    let summary = match action_type {
        GuiActionType::Observe => "Observe current GUI state".to_string(),
        GuiActionType::AnalyzePlan => "Analyze current GUI and create a safe plan".to_string(),
        GuiActionType::FocusInput => format!(
            "Focus {}",
            target_control_hint.unwrap_or("a visible input field")
        ),
        GuiActionType::TypeText => format!(
            "Type requested text{}",
            typed_text
                .map(|text| format!(": {text}"))
                .unwrap_or_default()
        ),
        GuiActionType::ClickControl => format!(
            "Click {}",
            target_control_hint.unwrap_or("the requested visible control")
        ),
        GuiActionType::BrowserSearch => query_summary
            .map(|query| format!("Search the browser for {query}"))
            .unwrap_or_else(|| "Clarify browser search query".to_string()),
        GuiActionType::BrowserNavigate => "Prepare browser navigation".to_string(),
        GuiActionType::FillForm => "Create and validate form-fill plan".to_string(),
        GuiActionType::OpenApp => format!("Open {}", target_app_hint.unwrap_or("target app")),
        GuiActionType::SwitchWindow => format!(
            "Switch to {}",
            target_window_hint
                .or(target_app_hint)
                .unwrap_or("target window")
        ),
        GuiActionType::Save => "Save requested GUI content or state".to_string(),
        GuiActionType::Download => "Download requested content safely".to_string(),
        GuiActionType::CopyContent => "Copy requested content safely".to_string(),
        GuiActionType::PasteContent => "Paste requested content safely".to_string(),
        GuiActionType::Recovery => "Recover GUI focus/state if safe".to_string(),
        GuiActionType::RiskApproval => "Prepare risky GUI action for approval".to_string(),
        GuiActionType::SafeAction => "Perform one safe GUI action".to_string(),
        GuiActionType::Unknown => "Clarify unsupported GUI request".to_string(),
    };
    let summary = if summary.trim().is_empty() {
        fallback_prompt_summary.to_string()
    } else {
        summary
    };
    sanitize_gui_text(&summary, MAX_GOAL_SUMMARY_CHARS).text
}

fn confidence_for(
    action_type: &GuiActionType,
    ambiguities: &[GuiGoalAmbiguity],
    target_control_hint: Option<&str>,
    query_summary: Option<&str>,
    typed_text: Option<&str>,
) -> f64 {
    let mut confidence: f64 = match action_type {
        GuiActionType::Observe => 0.94,
        GuiActionType::AnalyzePlan => 0.86,
        GuiActionType::FocusInput => 0.82,
        GuiActionType::TypeText => {
            if typed_text.is_some() {
                0.9
            } else {
                0.62
            }
        }
        GuiActionType::ClickControl => {
            if target_control_hint.is_some() {
                0.86
            } else {
                0.62
            }
        }
        GuiActionType::BrowserSearch => {
            if query_summary.is_some() {
                0.93
            } else {
                0.68
            }
        }
        GuiActionType::BrowserNavigate => 0.8,
        GuiActionType::FillForm => 0.78,
        GuiActionType::OpenApp | GuiActionType::SwitchWindow => 0.76,
        GuiActionType::Save
        | GuiActionType::Download
        | GuiActionType::CopyContent
        | GuiActionType::PasteContent => 0.78,
        GuiActionType::Recovery => 0.74,
        GuiActionType::RiskApproval => 0.88,
        GuiActionType::SafeAction => 0.72,
        GuiActionType::Unknown => 0.46,
    };
    confidence -= ambiguities.len() as f64 * 0.14;
    confidence.clamp(0.35, 0.98)
}

fn source_evidence_for(
    action_type: &GuiActionType,
    target_app_kind: Option<&str>,
    target_app_hint: Option<&str>,
    app_source: Option<&str>,
    target_control_hint: Option<&str>,
    query_summary: Option<&str>,
    typed_text: Option<&str>,
    risk_reasons: &[String],
) -> Vec<GuiGoalEvidence> {
    let mut evidence = vec![GuiGoalEvidence::new(
        "user_prompt",
        "action_type",
        format!("matched action {}", action_type.as_str()),
        if matches!(action_type, GuiActionType::Unknown) {
            0.42
        } else {
            0.9
        },
    )];
    if let Some(kind) = target_app_kind {
        evidence.push(GuiGoalEvidence::new(
            app_source.unwrap_or("heuristic"),
            "target_app_kind",
            kind,
            0.86,
        ));
    }
    if let Some(app) = target_app_hint {
        evidence.push(GuiGoalEvidence::new(
            app_source.unwrap_or("heuristic"),
            "target_app_hint",
            app,
            0.86,
        ));
    }
    if let Some(control) = target_control_hint {
        evidence.push(GuiGoalEvidence::new(
            "user_prompt",
            "target_control_hint",
            control,
            0.82,
        ));
    }
    if let Some(query) = query_summary {
        evidence.push(GuiGoalEvidence::new(
            "user_prompt",
            "query_summary",
            query,
            0.9,
        ));
    }
    if let Some(text) = typed_text {
        evidence.push(GuiGoalEvidence::new(
            "user_prompt",
            "text_payload_summary",
            text,
            0.88,
        ));
    }
    for reason in risk_reasons {
        evidence.push(GuiGoalEvidence::new(
            "heuristic",
            "risk_level",
            reason,
            0.94,
        ));
    }
    evidence
}
