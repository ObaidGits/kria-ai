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
    ClearField,
    SelectAll,
    ClickControl,
    SetCheckbox,
    CloseDialog,
    PressKey,
    Scroll,
    InAppSearch,
    VerifyAndStop,
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
            Self::ClearField => "clear_field",
            Self::SelectAll => "select_all",
            Self::ClickControl => "click_control",
            Self::SetCheckbox => "set_checkbox",
            Self::CloseDialog => "close_dialog",
            Self::PressKey => "press_key",
            Self::Scroll => "scroll",
            Self::InAppSearch => "in_app_search",
            Self::VerifyAndStop => "verify_and_stop",
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

/// Task 8.2 (Requirements 6, 7, 8) — cross-app clipboard COMBO descriptor.
///
/// Captures BOTH endpoints of a copy→switch→paste combo ("copy X from A and
/// paste into B") so the deterministic planner can thread the SOURCE app (where
/// the copy happens) AND the TARGET app (where the paste lands) into the typed
/// step sequence — neither of which the single [`GuiGoalContract::target_app_hint`]
/// (just the first app mention) can express on its own.
///
/// This is populated ONLY when the `gui_cog_crossapp` flag is ON: the runtime
/// calls [`GuiGoalContract::enrich_cross_app_clipboard`] behind the flag. While
/// the flag is OFF the contract's `cross_app_clipboard` stays `None`, so the
/// deterministic planner falls back to the existing single copy/paste primitive
/// plan and the contract is byte-for-byte unchanged.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CrossAppClipboardCombo {
    /// App-kind tag of the SOURCE app the content is copied from (e.g. `browser`).
    pub source_app_kind: Option<String>,
    /// Human-facing label of the SOURCE app (e.g. `Chrome`).
    pub source_app_hint: Option<String>,
    /// App-kind tag of the TARGET app the content is pasted into (e.g. `editor`).
    pub target_app_kind: Option<String>,
    /// Human-facing label of the TARGET app (e.g. `VS Code`).
    pub target_app_hint: Option<String>,
    /// Window hint for the TARGET app (used by the SwitchWindow step).
    pub target_window_hint: Option<String>,
    /// Control hint for the TARGET input the paste lands in.
    pub target_control_hint: Option<String>,
    /// Sanitized, credential-redacted summary of the copied content (never the
    /// raw secret — see [`redact_inline_credential`]).
    pub content_summary: Option<String>,
    /// Stable hash of the copied-content summary.
    pub content_hash: Option<String>,
}

/// Task 8.3 (Requirements 6, 7, 8) — NON-DESTRUCTIVE file-manager select flow.
///
/// Captures a "navigate the file manager → select the newest/first file → show
/// its name" intent so the deterministic planner can emit the complete typed
/// sequence OpenApp(file manager) → Observe(list files) → FocusField(select the
/// resolved file entry) → SummarizeVisibleContent(report the name).
///
/// The flow is strictly NON-DESTRUCTIVE: selecting + reading the name ONLY — no
/// delete / move / rename. [`detect_file_manager_select_flow`] returns `None`
/// (so the prompt falls back to the existing safety-gated path) the moment any
/// destructive verb is present, so a destructive request can never ride this
/// flow.
///
/// The "newest/first" choice is an ORDER/POSITION-based selection ([`Self::selection`])
/// resolved against the OBSERVED file-entry controls at resolution time — never
/// an invented filename. This is populated ONLY when the `gui_cog_crossapp` flag
/// is ON (the runtime calls [`GuiGoalContract::enrich_file_manager_select`]
/// behind the flag); while OFF the field stays `None` and the contract/plan are
/// byte-for-byte unchanged.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileManagerSelectFlow {
    /// App-kind tag of the file manager (always `file_manager`).
    pub app_kind: Option<String>,
    /// Human-facing label of the file manager app (e.g. `file manager`).
    pub app_hint: Option<String>,
    /// Optional folder the file manager should be navigated to, data-driven from
    /// the prompt's OWN wording (`None` = the current / default folder).
    pub folder_hint: Option<String>,
    /// Which observed file entry to select by ORDER/POSITION — e.g. `newest`,
    /// `first`, `oldest`, `last`. Resolved against the OBSERVED file list at
    /// resolution time; never an invented filename.
    pub selection: String,
    /// Positional control hint for the selection step (e.g. "newest file entry").
    pub selection_control_hint: Option<String>,
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
    /// Task 8.2 (Requirements 6, 7, 8): the cross-app clipboard COMBO endpoints,
    /// set ONLY when the `gui_cog_crossapp` flag is ON (the runtime enriches it
    /// behind the flag). `None` (the default) on the flag-OFF path, so the
    /// contract is byte-for-byte unchanged while the flag is OFF.
    #[serde(default)]
    pub cross_app_clipboard: Option<CrossAppClipboardCombo>,
    /// Task 8.3 (Requirements 6, 7, 8): the NON-DESTRUCTIVE file-manager select
    /// flow, set ONLY when the `gui_cog_crossapp` flag is ON (the runtime
    /// enriches it behind the flag). `None` (the default) on the flag-OFF path,
    /// so the contract is byte-for-byte unchanged while the flag is OFF.
    #[serde(default)]
    pub file_manager_select: Option<FileManagerSelectFlow>,
    /// Multi-action fix: the sanitized FULL user instruction (bounded), so the
    /// LLM planner can decompose EVERY requested action — not just the primary
    /// one captured by `action_type`/`goal_summary`. `goal_summary` stays the
    /// short templated phrase (used by deterministic/display); this carries the
    /// complete intent (e.g. "Open Chrome and create a new tab"). `#[serde(default)]`
    /// keeps older serialized contracts loadable; `None` falls back to the prior
    /// behavior (planner sees only `goal_summary`).
    #[serde(default)]
    pub full_instruction: Option<String>,
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

    /// Task 6.2 (Requirement 5/15): redact a secret typed payload destined for a
    /// password / secure-entry field from EVERY user-facing text field, replacing
    /// the raw value with a redacted placeholder. The runtime calls this (behind
    /// the `gui_cog_primitives` flag) once a secure-field target is detected and
    /// BEFORE any contract event is emitted, so the raw value never reaches the
    /// goal summary, desired final state, payload summary, or source evidence.
    /// The value-derived `text_payload_hash` is preserved so the secret flag is
    /// forced downstream (the payload vault rejects the placeholder, so the value
    /// is never typed or read back). No-op when there is no typed payload.
    pub fn redact_secret_payload(&mut self) {
        let placeholder = super::executor::GUI_SECRET_FIELD_PLACEHOLDER;
        let mut secrets: Vec<String> = Vec::new();
        if let Some(value) = self.text_payload_summary.as_ref() {
            let trimmed = value.trim();
            if !trimmed.is_empty() && value != placeholder {
                secrets.push(value.clone());
            }
        }
        if secrets.is_empty() {
            return;
        }
        self.text_payload_summary = Some(placeholder.to_string());

        let scrub = |text: &mut String| {
            for secret in &secrets {
                if !secret.is_empty() && text.contains(secret.as_str()) {
                    *text = text.replace(secret.as_str(), placeholder);
                }
            }
        };
        scrub(&mut self.goal_summary);
        scrub(&mut self.desired_final_state);
        if let Some(instruction) = self.full_instruction.as_mut() {
            scrub(instruction);
        }
        for evidence in &mut self.source_evidence {
            scrub(&mut evidence.summary);
        }
    }

    /// Task 8.2 (Requirements 6, 7, 8): detect a cross-app clipboard COMBO
    /// ("copy X from A and paste into B") from the sanitized prompt and, when
    /// found, populate [`Self::cross_app_clipboard`] with the SOURCE/TARGET app
    /// hints + the copied-content hint. The deterministic planner reads that
    /// descriptor to emit the complete typed sequence
    /// Copy(source)→SwitchWindow(target)→FocusField(target input)→Paste→VerifyState.
    ///
    /// Data-driven (no per-app hardcoding): the endpoints come from the prompt's
    /// OWN app mentions. No-op (leaves the field `None`) unless the prompt names
    /// BOTH a copy and a paste action across TWO DISTINCT apps, so a single copy
    /// or a single paste keeps its existing single-primitive plan.
    ///
    /// The runtime calls this ONLY when the `gui_cog_crossapp` flag is ON, so the
    /// contract — and every event derived from it — is byte-for-byte unchanged
    /// while the flag is OFF. The `action_type` is intentionally left untouched
    /// (the combo is expressed purely in the planned steps), so no event shape
    /// changes even on the flag-ON path.
    pub fn enrich_cross_app_clipboard(&mut self, prompt: &str) {
        if let Some(combo) = detect_cross_app_clipboard_combo(prompt) {
            self.cross_app_clipboard = Some(combo);
        }
    }

    /// Task 8.3 (Requirements 6, 7, 8): detect a NON-DESTRUCTIVE file-manager
    /// select flow ("open the file manager and select the newest/first file and
    /// tell me its name") from the sanitized prompt and, when found, populate
    /// [`Self::file_manager_select`] with the file-manager app hint + selection
    /// ordering + optional folder. The deterministic planner reads that
    /// descriptor to emit the complete typed sequence OpenApp(file manager) →
    /// Observe(list files) → FocusField(select the resolved entry) →
    /// SummarizeVisibleContent(report the name).
    ///
    /// Data-driven (no per-app hardcoding): the file-manager endpoint comes from
    /// the prompt's OWN app mention and the selection from its OWN ordering
    /// wording. Strictly NON-DESTRUCTIVE: [`detect_file_manager_select_flow`]
    /// returns `None` the moment any destructive verb is present, so a delete /
    /// move / rename request keeps its existing safety-gated path and never rides
    /// this flow.
    ///
    /// The runtime calls this ONLY when the `gui_cog_crossapp` flag is ON, so the
    /// contract — and every event derived from it — is byte-for-byte unchanged
    /// while the flag is OFF. The `action_type` is intentionally left untouched
    /// (the flow is expressed purely in the planned steps).
    pub fn enrich_file_manager_select(&mut self, prompt: &str) {
        if let Some(flow) = detect_file_manager_select_flow(prompt) {
            self.file_manager_select = Some(flow);
        }
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
        smart_planner_vocab_enabled() && explicit_ask_on_ambiguity(&lower),
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

    // Task 4 (Issue #5): scroll direction threading. Behind the
    // `gui_cog_primitives` flag (default-ON), encode the requested scroll
    // DIRECTION into the contract's `target_control_hint` (e.g. "scroll:down")
    // so it survives into the typed `Scroll` step → proposal → desktop
    // `GuiActionRequest`, where the executor picks the paging/arrow keys. This
    // is injected ONLY at contract-construction time (after every extraction
    // helper has run), so the goal summary / evidence / confidence are
    // unchanged. Flag-OFF (an explicit falsy value) yields `None` from the pure
    // helper, leaving `target_control_hint` byte-for-byte unchanged.
    let target_control_hint = if matches!(action_type, GuiActionType::Scroll) {
        scroll_direction_marker_for(prompt, gui_cog_primitives_enabled())
            .or(target_control_hint)
    } else {
        target_control_hint
    };

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
        // Task 8.2: cross-app clipboard combo is OFF by default; the runtime
        // enriches it only when the `gui_cog_crossapp` flag is ON.
        cross_app_clipboard: None,
        // Task 8.3: file-manager select flow is OFF by default; the runtime
        // enriches it only when the `gui_cog_crossapp` flag is ON.
        file_manager_select: None,
        // Multi-action fix: carry the sanitized FULL instruction so the LLM
        // planner can decompose every requested action, not just the primary
        // one reflected in `goal_summary`.
        full_instruction: Some(safe_prompt.text.clone()),
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

    // Issue #3 / Task 2.3: app inference from intent for single-window desktop
    // utilities the base vocabulary historically missed. Without "calculator"
    // here, `resolve_app_hint` falls back to the ACTIVE WINDOW app, so "Open the
    // calculator" was poisoned into "observe <active window> is open" (the live
    // open-app miss). Flag-gated by `gui_cog_smart_planner` (default-ON in the
    // desktop); flag-OFF leaves the vocabulary byte-for-byte unchanged.
    const EXTENDED_APPS: &[AppMention] = &[
        AppMention {
            kind: "calculator",
            label: "calculator",
            aliases: &["calculator", "calc", "gnome calculator"],
        },
        // Issue #6 / Task 6: system settings. Without this, "Open settings" /
        // "Open system settings" fell back to the ACTIVE WINDOW app (the same
        // poisoning the calculator entry fixes), so settings never opened. The
        // label "settings" resolves via `app_registry` ("settings" →
        // `gnome-control-center`, OS-detected). Flag-gated by
        // `gui_cog_smart_planner` (shared with the calculator entry); flag-OFF
        // leaves the vocabulary byte-for-byte unchanged.
        AppMention {
            kind: "settings",
            label: "settings",
            aliases: &[
                "system settings",
                "settings",
                "control center",
                "control centre",
                "system preferences",
                "gnome settings",
            ],
        },
    ];

    let mut mentions = Vec::new();
    let extended = smart_planner_vocab_enabled();
    let candidates = APPS.iter().chain(
        EXTENDED_APPS
            .iter()
            .take(if extended { EXTENDED_APPS.len() } else { 0 }),
    );
    for app in candidates {
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

/// Issue #3 / Task 2.3: whether the extended single-window app vocabulary is
/// active. Shares the Task 2 `gui_cog_smart_planner` flag
/// (`KRIA_GUI_COG_SMART_PLANNER`), default-ON (absent ⇒ ON, matching the
/// desktop's default-on wiring); an explicit falsy value
/// (`0`/`false`/`no`/`off`/empty) is the documented rollback to the prior
/// (calculator-less) vocabulary.
fn smart_planner_vocab_enabled() -> bool {
    match std::env::var("KRIA_GUI_COG_SMART_PLANNER") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "0" | "false" | "no" | "off" | "")
        }
        Err(_) => true,
    }
}

/// Task 4 (Issue #5): whether the GUI cognition primitives are active. Mirrors
/// the `gui_cog_primitives` default-ON env contract (`KRIA_GUI_COG_PRIMITIVES`)
/// used across the executor and resolver: an absent value ⇒ ON; an explicit
/// falsy value (`0`/`false`/`no`/`off`/empty) is the documented rollback that
/// suppresses the scroll-direction marker so the contract is byte-for-byte
/// unchanged. The extractor is a free function (no `GuiPrimitivesConfig`), so
/// this local helper reads the same env flag (matching `resolver.rs`'s
/// `primitives_surface_scroll_enabled` pattern).
fn gui_cog_primitives_enabled() -> bool {
    match std::env::var("KRIA_GUI_COG_PRIMITIVES") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "0" | "false" | "no" | "off" | "")
        }
        Err(_) => true,
    }
}

/// Task 4 (Issue #5): map a scroll prompt to a DIRECTION marker carried in
/// `target_control_hint` (e.g. `scroll:down`). The desktop executor reads this
/// marker to choose paging/arrow keys. Precedence is most-specific-first so a
/// "scroll up to the top" resolves to `scroll:top` and "scroll down to the
/// bottom" to `scroll:bottom`; a bare up/down resolves to `scroll:up`/
/// `scroll:down`; anything unrecognized defaults to `scroll:down`. Data-driven
/// from the prompt's own wording — never per-app hardcoded.
fn scroll_direction_marker(lower: &str) -> String {
    let direction = if contains_any(lower, &["bottom", " end"]) {
        "bottom"
    } else if contains_any(lower, &["top", "beginning"]) {
        "top"
    } else if phrase_present(lower, "up") {
        "up"
    } else if phrase_present(lower, "down") {
        "down"
    } else {
        "down"
    };
    format!("scroll:{direction}")
}

/// Task 4 (Issue #5): pure, explicitly flag-gated scroll-direction extraction.
/// Returns `Some("scroll:<dir>")` ONLY when `enabled` (the `gui_cog_primitives`
/// flag is ON); when `enabled` is `false` it returns `None`, which is the
/// byte-for-byte flag-OFF behavior (no direction marker is ever produced). Public
/// so flag-OFF vs flag-ON can be asserted directly in tests without racing the
/// process-global env var (mirrors `capture_trailing_typed_payload`).
pub fn scroll_direction_marker_for(prompt: &str, enabled: bool) -> Option<String> {
    if !enabled {
        return None;
    }
    Some(scroll_direction_marker(&normalize_prompt(prompt)))
}

/// Task 8.2 (Requirements 6, 7, 8): detect a cross-app clipboard COMBO from the
/// prompt. Returns `Some` only when the prompt names BOTH a copy and a paste
/// action across TWO DISTINCT apps (the combo is cross-app by definition); a
/// same-app or single-action prompt returns `None` so the existing single
/// copy/paste primitive plan is preserved. Data-driven from the prompt's own app
/// mentions — never per-app hardcoded.
fn detect_cross_app_clipboard_combo(prompt: &str) -> Option<CrossAppClipboardCombo> {
    let lower = normalize_prompt(prompt);
    // Both halves of the combo must be named: copy (the source action) AND paste
    // (the target action).
    if !(phrase_present(&lower, "copy") && phrase_present(&lower, "paste")) {
        return None;
    }
    // The combo spans two endpoints: the SOURCE app (first mention) and the
    // TARGET app (last mention). Require them to be DISTINCT — a same-app
    // copy→paste is not a cross-app combo.
    let mentions = extract_app_mentions(&lower);
    if mentions.len() < 2 {
        return None;
    }
    let source = mentions.first()?;
    let target = mentions.last()?;
    if source.label == target.label {
        return None;
    }
    // Copied-content hint: a quoted segment when present, sanitized and
    // credential-redacted so a secret is never echoed (Requirement 8 / privacy).
    let content = extract_first_quoted_segment(prompt)
        .map(|value| redact_inline_credential(&sanitize_gui_text(&value, MAX_HINT_CHARS).text))
        .filter(|value| !value.trim().is_empty());
    let content_hash = content.as_ref().map(|value| stable_hash(value));
    Some(CrossAppClipboardCombo {
        source_app_kind: Some(source.kind.to_string()),
        source_app_hint: Some(source.label.to_string()),
        target_app_kind: Some(target.kind.to_string()),
        target_app_hint: Some(target.label.to_string()),
        target_window_hint: Some(target.label.to_string()),
        target_control_hint: Some("visible text input".to_string()),
        content_summary: content,
        content_hash,
    })
}

/// Task 8.3 (Requirements 6, 7, 8): detect a NON-DESTRUCTIVE file-manager select
/// flow from the prompt. Returns `Some` only when the prompt names the file
/// manager AND a selection ordering (newest / most recent / first ...) AND a
/// select-or-show-name intent, and contains NO destructive verb. A destructive
/// request (delete / move / rename / trash ...) returns `None` so it keeps its
/// existing safety-gated path — selecting + reading a name is the ONLY behavior
/// this flow ever expresses. Data-driven from the prompt's own app mention +
/// ordering wording — never per-app or per-filename hardcoded.
fn detect_file_manager_select_flow(prompt: &str) -> Option<FileManagerSelectFlow> {
    let lower = normalize_prompt(prompt);

    // Strictly NON-DESTRUCTIVE: any destructive verb routes through the safety
    // gate, never this flow. Bail BEFORE recognizing the flow so a destructive
    // request can never ride the non-destructive select path.
    const DESTRUCTIVE_VERBS: &[&str] = &[
        "delete", "remove", "move ", "rename", "trash", "erase", "discard",
        "shred", "wipe", "overwrite", "cut ", "drag",
    ];
    if contains_any(&lower, DESTRUCTIVE_VERBS) {
        return None;
    }

    // The file manager must be named (data-driven from the prompt's own mention).
    let mentions = extract_app_mentions(&lower);
    let fm = mentions.iter().find(|app| app.kind == "file_manager")?;

    // A selection ORDERING must be named — the "newest/first" choice drives an
    // order/position selection against the OBSERVED file list (never a filename).
    let selection = if contains_any(
        &lower,
        &["newest", "most recent", "most recently", "latest", "last modified", "recently added"],
    ) {
        "newest"
    } else if phrase_present(&lower, "first") {
        "first"
    } else if phrase_present(&lower, "oldest") {
        "oldest"
    } else if phrase_present(&lower, "last") {
        "last"
    } else {
        return None;
    };

    // A select / show-name intent must be present (otherwise it is a plain open).
    let selects = contains_any(&lower, &["select", "pick", "choose", "highlight", "click on"]);
    let shows_name = contains_any(
        &lower,
        &["name", "called", "tell me", "what file", "which file", "what is it"],
    );
    if !(selects || shows_name) {
        return None;
    }

    let folder_hint = extract_file_manager_folder_hint(prompt);
    Some(FileManagerSelectFlow {
        app_kind: Some(fm.kind.to_string()),
        app_hint: Some(fm.label.to_string()),
        folder_hint,
        selection: selection.to_string(),
        selection_control_hint: Some(format!("{selection} file entry")),
    })
}

/// Extract an optional folder name the file manager should be navigated to,
/// data-driven from the prompt's OWN wording ("in the Downloads folder", "folder
/// named Reports"). Returns `None` (the current/default folder) when no folder is
/// named. Sanitized; never a fabricated path.
fn extract_file_manager_folder_hint(prompt: &str) -> Option<String> {
    let patterns = [
        r#"(?i)\b(?:in|inside|from|under|within)\s+(?:the\s+)?([A-Za-z0-9][A-Za-z0-9 _./-]{0,48}?)\s+(?:folder|directory|dir)\b"#,
        r#"(?i)\b(?:folder|directory)\s+(?:named|called)\s+["']?([A-Za-z0-9][A-Za-z0-9 _./-]{0,48})"#,
    ];
    for pattern in patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(cap) = re.captures(prompt) {
                if let Some(matched) = cap.get(1) {
                    let value = sanitize_gui_text(matched.as_str(), MAX_HINT_CHARS)
                        .text
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
    }
    None
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
    extract_named_control(prompt, lower)
        .or_else(|| {
            if contains_any(
                lower,
                &["search/input", "search field", "input field", "text field"],
            ) {
                Some("visible text input".into())
            } else {
                None
            }
        })
        .or_else(|| extract_focus_control_hint(prompt, lower))
}

/// Fix 2 (gui_cog_smart_planner, default-ON): extract the named control of a
/// "focus the <control> in/on the <app>" / "click into the <control>" intent
/// (e.g. "focus the address bar in the browser" → "address bar"). Runs ONLY as
/// a fallback after the existing control-hint logic, so already-handled shapes
/// (e.g. "Focus the visible search/input field." → "visible text input") are
/// byte-for-byte unchanged. Flag-OFF returns `None`, leaving the prior behavior
/// exactly. Sanitized via the same sanitizer the file already uses.
fn extract_focus_control_hint(prompt: &str, lower: &str) -> Option<String> {
    if !smart_planner_vocab_enabled() {
        return None;
    }
    let focus_verb = contains_any(
        lower,
        &[
            "focus ",
            "click into ",
            "put cursor in",
            "put the cursor in",
            "place cursor in",
            "place the cursor in",
        ],
    ) || (lower.contains("select the") && lower.contains("field"));
    if !focus_verb {
        return None;
    }
    let re = regex::Regex::new(
        r"(?i)(?:focus(?:\s+on)?|click\s+into|put(?:\s+the)?\s+cursor\s+in|place(?:\s+the)?\s+cursor\s+in|select)\s+the\s+(.+?)(?:\s+(?:in|on)\s+the\s+.+)?$",
    )
    .ok()?;
    let cap = re.captures(prompt)?;
    let value = sanitize_gui_text(cap.get(1)?.as_str(), MAX_HINT_CHARS)
        .text
        .trim()
        .trim_end_matches(['.', ',', ';', ':'])
        .trim()
        .to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Fix 2 (gui_cog_smart_planner): whether a prompt is a "focus a named control
/// in/on the <app>" intent ("focus the address bar in the browser", "click into
/// the username field") that should classify as [`GuiActionType::FocusInput`].
/// Gated by the focus verb plus a successfully extracted control hint, so it only
/// ADDS classification for shapes that currently fall through to `Unknown`; it
/// never fires for the existing "Focus the visible search/input field." path
/// (already FocusInput) or for plain "click the button" (no focus verb).
/// Flag-OFF returns `false`, preserving prior behavior byte-for-byte.
fn is_focus_control_in_app_intent(lower: &str, target_control_hint: Option<&str>) -> bool {
    if !smart_planner_vocab_enabled() {
        return false;
    }
    let focus_verb = contains_any(
        lower,
        &[
            "focus ",
            "click into ",
            "put cursor in",
            "put the cursor in",
            "place cursor in",
            "place the cursor in",
        ],
    ) || (lower.contains("select the") && lower.contains("field"));
    focus_verb && target_control_hint.is_some()
}

fn extract_text_payload(prompt: &str, lower: &str, quoted: Option<&str>) -> Option<String> {
    if let Some(value) = quoted {
        return Some(redact_inline_credential(&sanitize_gui_text(value, MAX_HINT_CHARS).text));
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
                        return Some(redact_inline_credential(&value));
                    }
                }
            }
        }
    }
    // Fix 1 (gui_cog_smart_planner, default-ON): capture text that trails a
    // mid-sentence `type|write|enter` verb (e.g. "open the text editor and type
    // hello world"), which the destination-anchored patterns above miss. Only
    // reached when no quoted segment and no existing pattern matched, so the
    // already-working phrasings stay byte-for-byte unchanged. Flag-OFF returns
    // None here, preserving the prior behavior exactly.
    if let Some(value) = capture_trailing_typed_payload(prompt, smart_planner_vocab_enabled()) {
        return Some(value);
    }
    if contains_any(lower, &["type ", "write ", "enter "]) {
        None
    } else {
        quoted.map(|value| redact_inline_credential(&sanitize_gui_text(value, MAX_HINT_CHARS).text))
    }
}

/// Fix 1 (gui_cog_smart_planner): capture the typed text that follows a
/// mid-sentence `type|write|enter` verb when the destination-anchored patterns
/// in [`extract_text_payload`] do not match (e.g. "open the text editor and type
/// hello world"). PURE + explicitly flag-gated (the `smart_planner_enabled`
/// param) so the flag-OFF branch is unit-testable without env races: when
/// `false` it returns `None`, leaving the extractor byte-for-byte identical to
/// the prior behavior.
///
/// Behavior:
/// - A bare destination verb with no payload (e.g. "type into the field" — the
///   first trailing word is a location preposition) returns `None`, so the
///   clarification ambiguity is preserved.
/// - A trailing "into/in/on/to the <control>" clause is stripped, so
///   "type quarterly report into the search box" yields "quarterly report".
/// - The captured text is routed through the SAME sanitizer/credential-redactor
///   the function already uses ([`clean_slot_value`] + [`redact_inline_credential`]),
///   so secrets are never bypassed.
pub fn capture_trailing_typed_payload(prompt: &str, smart_planner_enabled: bool) -> Option<String> {
    if !smart_planner_enabled {
        return None;
    }
    let verb_re = regex::Regex::new(r"(?i)\b(?:type|enter|write)\s+(.+)$").ok()?;
    let cap = verb_re.captures(prompt)?;
    let tail = cap.get(1)?.as_str().trim();
    if tail.is_empty() {
        return None;
    }
    // Destination-only (no payload): the verb is immediately followed by a
    // location preposition ("type into the field"). Stay None so the existing
    // missing-text clarification is preserved.
    let first_word = tail
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(first_word.as_str(), "into" | "in" | "on" | "to") {
        return None;
    }
    // Strip a trailing "into/in/on/to the <control>" destination clause so the
    // payload is the text only (mirrors the destination-anchored patterns).
    let payload = match regex::Regex::new(r"(?i)\s+(?:into|in|on|to)\b.*$") {
        Ok(re) => re.replace(tail, "").into_owned(),
        Err(_) => tail.to_string(),
    };
    let value = clean_slot_value(&payload, &[]);
    if !useful_text_payload(&value) {
        return None;
    }
    Some(redact_inline_credential(&value))
}

/// Redact a secret value that immediately follows a credential keyword in
/// natural language (e.g. `type the password hunter2 into ...`). The
/// `key: value` / `key=value` redaction in [`sanitize_gui_text`] does not catch
/// the whitespace-separated natural-language form, so a typed credential payload
/// would otherwise be echoed verbatim into events/logs. This keeps the keyword
/// (so the field intent stays legible) and masks the secret token, satisfying
/// the password/secret non-echo guarantee (Requirement 5.10, Property 7).
///
/// Scoped to typed-text payloads only (not window titles / control labels), so
/// benign labels such as "Password Manager" are never affected.
fn redact_inline_credential(value: &str) -> String {
    let pattern =
        r"(?i)\b(password|passwd|passphrase|secret|credential|token|api[ _-]?key|otp|pin)\b\s+\S+";
    match regex::Regex::new(pattern) {
        Ok(re) => re.replace_all(value, "$1 [redacted]").into_owned(),
        Err(_) => value.to_string(),
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
    // Issue #6 / Task 6: merely OPENING or SEARCHING the settings app is GREEN —
    // only actually CHANGING a setting is risky. Without this, "Open system
    // settings" (or "…security settings") matched the "system setting" phrase and
    // was wrongly gated as a system change requiring approval, so settings never
    // opened. Scope these two phrases to an actual change-intent verb. Flag-gated
    // by `gui_cog_smart_planner`; flag-OFF keeps the prior (always-risky) match.
    let scope_settings_change = smart_planner_vocab_enabled();
    let settings_change_intent = contains_any(
        lower,
        &[
            "change", "modify", "toggle", "enable", "disable", "turn on", "turn off",
            "adjust", "configure", "reset", "switch on", "switch off", "set the",
            "update the", "uncheck", "check the",
        ],
    );
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
        let is_settings_phrase = needle == "system setting" || needle == "security setting";
        if is_settings_phrase && scope_settings_change && !settings_change_intent {
            // Open/search/show settings — not a setting change.
            continue;
        }
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
    } else if is_verify_and_stop_intent(lower) {
        GuiActionType::VerifyAndStop
    } else if is_scroll_intent(lower) {
        GuiActionType::Scroll
    } else if is_select_all_intent(lower) {
        GuiActionType::SelectAll
    } else if is_clear_field_intent(lower) {
        GuiActionType::ClearField
    } else if is_checkbox_intent(lower) {
        GuiActionType::SetCheckbox
    } else if is_close_dialog_intent(lower) {
        GuiActionType::CloseDialog
    } else if is_press_key_intent(lower, typed_text) {
        GuiActionType::PressKey
    } else if is_in_app_search_intent(lower, target_app_hint) {
        GuiActionType::InAppSearch
    } else if is_browser_search_intent(lower, query_summary, target_app_hint) {
        GuiActionType::BrowserSearch
    } else if is_browser_navigation_intent(lower) {
        GuiActionType::BrowserNavigate
    } else if contains_any(lower, &["type ", "enter ", "write "]) || typed_text.is_some() {
        GuiActionType::TypeText
    } else if (lower.contains("focus") && contains_any(lower, &["input", "field", "search", "text"]))
        || is_focus_control_in_app_intent(lower, target_control_hint)
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
    } else if contains_any(
        lower,
        &[
            "switch window",
            "switch to",
            "to the front",
            "to front",
            "bring forward",
            "bring to the foreground",
            "to the foreground",
            "raise the window",
        ],
    ) {
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
    // A "type X into [the] ... field / input / box" is an in-app TYPE into a
    // NAMED control (normal typing), NOT a web/browser search — even if the text
    // or the field is called "search". (A browser search says "search for X" /
    // "search the web", or types into the "address bar", never a named in-app
    // "field".)
    if lower.contains("type")
        && contains_any(lower, &["field", "input", "textbox", "text box"])
        && !contains_any(lower, &["address bar", "address-bar"])
    {
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

/// Verify-and-stop family (Requirement 13): the prompt asks to confirm/verify a
/// state and then terminate without further actions. Detected by a verify/confirm
/// phrase paired with an explicit stop/terminate instruction. Data-driven (no
/// per-app hardcoding).
fn is_verify_and_stop_intent(lower: &str) -> bool {
    let verify_phrase = contains_any(
        lower,
        &[
            "verify",
            "confirm that",
            "confirm the",
            "check that",
            "make sure",
            "ensure that",
        ],
    );
    let stop_phrase = contains_any(
        lower,
        &[
            "and stop",
            "then stop",
            "and terminate",
            "without doing anything",
            "without any further action",
            "without further action",
            "do not act",
            "do nothing else",
        ],
    );
    verify_phrase && stop_phrase
}

/// In-app search family (Requirement 5.9): searching inside a non-browser app's
/// own search field (settings/preferences/file manager/menu). Distinguished from
/// browser search by the in-app context keywords and the absence of a browser
/// app target. Data-driven (no per-app hardcoding).
fn is_in_app_search_intent(lower: &str, _target_app_hint: Option<&str>) -> bool {
    // Distinguish from browser search by the PROMPT's own browser keywords (never
    // the ambient active-window context, which may already be a browser).
    let prompt_targets_browser = contains_any(
        lower,
        &[
            "browser", "chrome", "firefox", "edge", "safari", "chromium", "website",
            "web page", "webpage", "url", "the web",
        ],
    );
    if prompt_targets_browser {
        return false;
    }
    let has_search = contains_any(lower, &["search", "find", "look for", "filter"]);
    let in_app_context = contains_any(
        lower,
        &[
            "settings",
            "preferences",
            "system settings",
            "control panel",
            "file manager",
            "files app",
            "the files",
            "nautilus",
            "the menu",
            "this app",
            "in-app",
            "within the app",
        ],
    );
    has_search && in_app_context
}

/// Scroll family (Requirement 5.5).
fn is_scroll_intent(lower: &str) -> bool {
    lower.contains("scroll")
}

/// Select-all family (Requirement 5.2). Matches only explicit "select all" style
/// phrases — never the unrelated "selected" word (e.g. "copy the selected text").
fn is_select_all_intent(lower: &str) -> bool {
    contains_any(
        lower,
        &[
            "select all",
            "select everything",
            "select the whole",
            "highlight all",
            "highlight everything",
            "ctrl+a",
            "ctrl + a",
        ],
    )
}

/// Clear-field family (Requirement 5.2).
fn is_clear_field_intent(lower: &str) -> bool {
    if contains_any(
        lower,
        &[
            "clear the field",
            "clear field",
            "clear the input",
            "clear input",
            "clear the text",
            "clear text",
            "clear the search",
            "clear search",
            "empty the field",
            "empty the input",
            "erase the field",
        ],
    ) {
        return true;
    }
    lower.contains("clear")
        && contains_any(
            lower,
            &[
                "field",
                "input",
                "textbox",
                "text box",
                "search box",
                "search bar",
                "address bar",
            ],
        )
}

/// Checkbox family (Requirement 5.7).
fn is_checkbox_intent(lower: &str) -> bool {
    if contains_any(lower, &["checkbox", "check box", "uncheck", "untick"]) {
        return true;
    }
    contains_any(lower, &["check the", "tick the", "toggle the", "toggle "])
        && contains_any(
            lower,
            &["box", "option", "checkbox", "remember", "agree", "terms", "consent"],
        )
}

/// Dialog-close family (Requirement 5.8). Requires an explicit close/dismiss verb
/// so a passive "confirmation dialog" observation stays an observe request.
fn is_close_dialog_intent(lower: &str) -> bool {
    contains_any(lower, &["close", "dismiss", "cancel"])
        && contains_any(lower, &["dialog", "popup", "pop-up", "modal", "prompt box"])
}

/// Key-press / shortcut family (Requirement 5.4). Excludes prompts that are
/// primarily typing text so a "type X" combo keeps its TypeText classification.
fn is_press_key_intent(lower: &str, typed_text: Option<&str>) -> bool {
    if typed_text.is_some() || contains_any(lower, &["type ", "write ", "enter the text"]) {
        return false;
    }
    let press_verb = contains_any(
        lower,
        &["press ", "hit ", "keystroke", "keyboard shortcut", "shortcut", "key combo"],
    );
    let key_token = contains_any(
        lower,
        &[
            "enter",
            "return",
            "escape",
            "esc",
            "tab",
            "ctrl",
            "control+",
            "cmd",
            "command+",
            "space",
            "arrow",
            "delete",
            "backspace",
            "f5",
            "page down",
            "page up",
            "key",
        ],
    );
    press_verb && key_token
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
        GuiActionType::ClearField => "clear_field",
        GuiActionType::SelectAll => "select_all",
        GuiActionType::ClickControl => "click_control",
        GuiActionType::SetCheckbox => "set_checkbox",
        GuiActionType::CloseDialog => "close_dialog",
        GuiActionType::PressKey => "press_key",
        GuiActionType::Scroll => "scroll",
        GuiActionType::InAppSearch => "in_app_search",
        GuiActionType::VerifyAndStop => "verify_and_stop",
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

/// Issue #7 / Task 7: whether the prompt EXPLICITLY instructs KRIA to ask the
/// user when the target is ambiguous / has multiple matches (e.g. "…if there are
/// multiple … ask me first", "…if several reports match ask me which one"). Both
/// an ask-verb and a multiplicity/ambiguity conditional must be present, so a
/// plain unconditional action is unaffected.
fn explicit_ask_on_ambiguity(lower: &str) -> bool {
    let asks = contains_any(
        lower,
        &["ask me", "ask first", "ask which", "ask you", "ask before"],
    );
    let conditional = contains_any(
        lower,
        &[
            "multiple", "ambiguous", "more than one", "several", "many matches",
            "matches the name", "match the name", "reports match", "files match",
            "if the field is", "if there are", "if there is more",
        ],
    );
    asks && conditional
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
    explicit_ask_on_ambiguity: bool,
) -> Vec<GuiGoalAmbiguity> {
    let mut ambiguities = Vec::new();
    // Issue #7 / Task 7: the user EXPLICITLY asked to be asked when the target is
    // ambiguous / there are multiple matches ("…if multiple/several/ambiguous,
    // ask me first/which"). Honor that instruction deterministically — record an
    // ambiguity so the plan clarifies and NEVER guesses/executes an ambiguous
    // target. Flag-gated by `gui_cog_smart_planner`; flag-OFF byte-for-byte.
    if explicit_ask_on_ambiguity {
        ambiguities.push(GuiGoalAmbiguity::new(
            "explicit_ask_on_ambiguity",
            Some("target"),
            "The request asks to be consulted if the target is ambiguous or has multiple matches; clarification is required before acting.",
        ));
    }
    if matches!(action_type, GuiActionType::Unknown) {
        ambiguities.push(GuiGoalAmbiguity::new(
            "unsupported_goal",
            Some("action_type"),
            "The prompt does not describe a supported GUI cognition goal clearly enough.",
        ));
    }
    if matches!(
        action_type,
        GuiActionType::BrowserSearch | GuiActionType::InAppSearch
    ) && query_summary.is_none()
    {
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
        GuiActionType::ClearField => "target field is cleared and verified empty".to_string(),
        GuiActionType::SelectAll => {
            "all text in the focused field is selected and verified".to_string()
        }
        GuiActionType::SetCheckbox => {
            "labeled checkbox reflects the requested state and is verified".to_string()
        }
        GuiActionType::CloseDialog => "the active dialog is closed and verified".to_string(),
        GuiActionType::PressKey => {
            "requested key/shortcut is pressed and the screen change verified".to_string()
        }
        GuiActionType::Scroll => "the viewport is scrolled and the change verified".to_string(),
        GuiActionType::InAppSearch => query_summary
            .map(|query| format!("in-app search results visible for {query}"))
            .unwrap_or_else(|| {
                "in-app search results visible after query is clarified".to_string()
            }),
        GuiActionType::VerifyAndStop => {
            "requested state is verified and the workflow stops with no further action".to_string()
        }
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
        GuiActionType::ClearField => format!(
            "Clear {}",
            target_control_hint.unwrap_or("the requested visible field")
        ),
        GuiActionType::SelectAll => format!(
            "Select all text in {}",
            target_control_hint.unwrap_or("the focused field")
        ),
        GuiActionType::SetCheckbox => format!(
            "Set {}",
            target_control_hint.unwrap_or("the labeled checkbox")
        ),
        GuiActionType::CloseDialog => "Close the active dialog".to_string(),
        GuiActionType::PressKey => "Press the requested key or shortcut".to_string(),
        GuiActionType::Scroll => "Scroll the active view".to_string(),
        GuiActionType::InAppSearch => query_summary
            .map(|query| format!("Search within the app for {query}"))
            .unwrap_or_else(|| "Clarify in-app search query".to_string()),
        GuiActionType::VerifyAndStop => "Verify the requested state and stop".to_string(),
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
        GuiActionType::ClearField | GuiActionType::SelectAll => {
            if target_control_hint.is_some() {
                0.84
            } else {
                0.7
            }
        }
        GuiActionType::SetCheckbox => {
            if target_control_hint.is_some() {
                0.84
            } else {
                0.66
            }
        }
        GuiActionType::CloseDialog => 0.82,
        GuiActionType::PressKey => 0.82,
        GuiActionType::Scroll => 0.82,
        GuiActionType::InAppSearch => {
            if query_summary.is_some() {
                0.88
            } else {
                0.66
            }
        }
        GuiActionType::VerifyAndStop => 0.86,
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
