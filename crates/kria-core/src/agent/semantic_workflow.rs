//! Bounded semantic workflow metadata for GUI workflow intelligence.
//!
//! This module is intentionally metadata-only. It extracts normalized workflow
//! semantics and fidelity requirements from an existing [`GuiTaskSpec`] plus the
//! user text, but it does not plan, execute, verify, recover, or mutate policy.

use crate::agent::intent_compiler::{Ambiguity, ContentClass, GuiTaskSpec, TargetRef, Verb};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskFamily {
    Coding,
    Browser,
    Media,
    DocumentEditing,
    Spreadsheet,
    Communication,
    FileManagement,
    SystemTerminal,
    Research,
    General,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppClass {
    Ide,
    Browser,
    Terminal,
    Spreadsheet,
    DocumentEditor,
    Media,
    Communication,
    FileManager,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppAnchorStrength {
    Incidental,
    Preferred,
    Required,
    SafetyCredential,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppSemanticRole {
    WorkSurface,
    ExecutionSurface,
    ReviewSurface,
    AccountSessionSurface,
    MediaSurface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityExpectation {
    None,
    ResultVisible,
    AppVisible,
    WorkflowVisible,
    HumanObserved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationRequirement {
    None,
    SurfaceResult,
    EvidenceSummary,
    HumanInspection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationRequirement {
    None,
    Optional,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmbiguitySeverity {
    None,
    Harmless,
    Reversible,
    WorkflowChanging,
    Visibility,
    Identity,
    AccountSession,
    Destructive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowSafetyClass {
    Safe,
    Reversible,
    ReviewRequired,
    DestructiveOrExternal,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowFidelityTier {
    MinimalResultFidelity,
    AppContextFidelity,
    WorkflowStageFidelity,
    HumanObservedFidelity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FidelityDegradationPolicy {
    NoFallback,
    AskBeforeFallback,
    ExplainFallback,
    SilentFallbackAllowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartialCompletionPolicy {
    ReportPartial,
    PauseForRecovery,
    FailClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackTolerance {
    Unknown,
    Allowed,
    AskFirst,
    Forbidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppAnchor {
    pub label: String,
    pub app_class: AppClass,
    pub strength: AppAnchorStrength,
    pub roles: Vec<AppSemanticRole>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowExpectationMetadata {
    pub strongest_app_anchor: AppAnchorStrength,
    pub visibility: VisibilityExpectation,
    pub collaboration: CollaborationRequirement,
    pub fallback_tolerance: FallbackTolerance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticWorkflowTrace {
    pub signal_labels: Vec<String>,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticWorkflowFrame {
    pub task_family: TaskFamily,
    pub app_anchors: Vec<AppAnchor>,
    pub visibility_expectation: VisibilityExpectation,
    pub observation_requirement: ObservationRequirement,
    pub collaboration_requirement: CollaborationRequirement,
    pub ambiguity_level: AmbiguitySeverity,
    pub safety_class: WorkflowSafetyClass,
    pub expectation: WorkflowExpectationMetadata,
    pub trace: SemanticWorkflowTrace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowFidelityResolution {
    pub requested_fidelity: WorkflowFidelityTier,
    pub minimum_acceptable_fidelity: WorkflowFidelityTier,
    pub environment_available_fidelity: Option<WorkflowFidelityTier>,
    pub planned_fidelity: WorkflowFidelityTier,
    pub fallback_adjusted_fidelity: Option<WorkflowFidelityTier>,
    pub degradation_policy: FidelityDegradationPolicy,
    pub partial_completion_policy: PartialCompletionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticWorkflowAnalysis {
    pub frame: SemanticWorkflowFrame,
    pub fidelity: WorkflowFidelityResolution,
}

/// Deterministically extracts semantic workflow metadata.
///
/// This function is trace-only in Phase 1. It must not influence execution
/// behavior until later phases explicitly consume the returned contract data.
pub fn analyze_semantic_workflow(
    spec: &GuiTaskSpec,
    raw_user_text: &str,
) -> SemanticWorkflowAnalysis {
    let normalized = normalize_text(raw_user_text);
    let app_anchors = extract_app_anchors(spec, &normalized);
    let task_family = classify_task_family(spec, &normalized, &app_anchors);
    let collaboration_requirement = classify_collaboration_requirement(&normalized);
    let ambiguity_level = classify_ambiguity(spec, &normalized);
    let safety_class = classify_safety(&normalized, ambiguity_level);
    let visibility_expectation = classify_visibility(
        spec,
        &normalized,
        &app_anchors,
        collaboration_requirement,
        ambiguity_level,
    );
    let observation_requirement = classify_observation(
        &normalized,
        visibility_expectation,
        collaboration_requirement,
    );
    let strongest_app_anchor = strongest_app_anchor(&app_anchors);
    let fallback_tolerance = classify_fallback_tolerance(
        strongest_app_anchor,
        visibility_expectation,
        collaboration_requirement,
        ambiguity_level,
    );
    let expectation = WorkflowExpectationMetadata {
        strongest_app_anchor,
        visibility: visibility_expectation,
        collaboration: collaboration_requirement,
        fallback_tolerance,
    };
    let trace = build_trace(
        task_family,
        &app_anchors,
        visibility_expectation,
        ambiguity_level,
        safety_class,
        &normalized,
    );
    let frame = SemanticWorkflowFrame {
        task_family,
        app_anchors,
        visibility_expectation,
        observation_requirement,
        collaboration_requirement,
        ambiguity_level,
        safety_class,
        expectation,
        trace,
    };
    let fidelity = resolve_fidelity(&frame);
    SemanticWorkflowAnalysis { frame, fidelity }
}

fn normalize_text(raw_user_text: &str) -> String {
    raw_user_text
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch.is_ascii_whitespace() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_app_anchors(spec: &GuiTaskSpec, normalized: &str) -> Vec<AppAnchor> {
    let mut anchors = Vec::new();
    for target in &spec.targets {
        match target {
            TargetRef::App(app) => {
                let app_class = classify_app_class(app);
                anchors.push(AppAnchor {
                    label: audit_safe_label(app),
                    app_class,
                    strength: classify_anchor_strength(app_class, normalized),
                    roles: classify_roles(app_class, normalized),
                });
            }
            TargetRef::Url(url) => {
                anchors.push(AppAnchor {
                    label: audit_safe_label(url),
                    app_class: AppClass::Browser,
                    strength: classify_anchor_strength(AppClass::Browser, normalized),
                    roles: classify_roles(AppClass::Browser, normalized),
                });
            }
            TargetRef::File(_) | TargetRef::Element(_) => {}
        }
    }

    if anchors.is_empty() {
        if contains_any(
            normalized,
            &[" vs code ", " vscode ", " visual studio code "],
        ) {
            anchors.push(AppAnchor {
                label: "ide".to_string(),
                app_class: AppClass::Ide,
                strength: classify_anchor_strength(AppClass::Ide, normalized),
                roles: classify_roles(AppClass::Ide, normalized),
            });
        } else if contains_word(normalized, "browser") {
            anchors.push(AppAnchor {
                label: "browser".to_string(),
                app_class: AppClass::Browser,
                strength: classify_anchor_strength(AppClass::Browser, normalized),
                roles: classify_roles(AppClass::Browser, normalized),
            });
        } else if contains_word(normalized, "terminal") {
            anchors.push(AppAnchor {
                label: "terminal".to_string(),
                app_class: AppClass::Terminal,
                strength: classify_anchor_strength(AppClass::Terminal, normalized),
                roles: classify_roles(AppClass::Terminal, normalized),
            });
        }
    }

    anchors
}

fn classify_app_class(app: &str) -> AppClass {
    let app = normalize_text(app);
    if contains_any(
        &app,
        &[
            "code",
            "vscode",
            "visual studio code",
            "intellij",
            "pycharm",
            "webstorm",
            "clion",
            "cursor",
            "zed",
            "sublime",
            "atom",
        ],
    ) {
        AppClass::Ide
    } else if contains_any(
        &app,
        &[
            "chrome", "chromium", "firefox", "brave", "edge", "safari", "browser",
        ],
    ) {
        AppClass::Browser
    } else if contains_any(&app, &["terminal", "console", "konsole", "xterm", "shell"]) {
        AppClass::Terminal
    } else if contains_any(&app, &["excel", "calc", "spreadsheet"]) {
        AppClass::Spreadsheet
    } else if contains_any(
        &app,
        &["word", "writer", "document", "gedit", "notepad", "kate"],
    ) {
        AppClass::DocumentEditor
    } else if contains_any(&app, &["youtube", "vlc", "spotify", "media", "music"]) {
        AppClass::Media
    } else if contains_any(
        &app,
        &["gmail", "mail", "email", "slack", "whatsapp", "teams"],
    ) {
        AppClass::Communication
    } else if contains_any(
        &app,
        &["files", "nautilus", "dolphin", "finder", "explorer"],
    ) {
        AppClass::FileManager
    } else {
        AppClass::Unknown
    }
}

fn classify_anchor_strength(app_class: AppClass, normalized: &str) -> AppAnchorStrength {
    if matches!(
        app_class,
        AppClass::Browser | AppClass::Communication | AppClass::Media
    ) && contains_any(
        normalized,
        &[
            "login",
            "log in",
            "sign in",
            "account",
            "profile",
            "upload",
            "send",
            "my playlist",
            "my account",
            "private",
            "personal",
        ],
    ) {
        return AppAnchorStrength::SafetyCredential;
    }
    if contains_any(normalized, &["if possible", "prefer", "preferred", "maybe"]) {
        AppAnchorStrength::Preferred
    } else {
        AppAnchorStrength::Required
    }
}

fn classify_roles(app_class: AppClass, normalized: &str) -> Vec<AppSemanticRole> {
    let mut roles = match app_class {
        AppClass::Ide => vec![AppSemanticRole::WorkSurface],
        AppClass::Terminal => vec![AppSemanticRole::ExecutionSurface],
        AppClass::Browser => vec![AppSemanticRole::WorkSurface],
        AppClass::Spreadsheet | AppClass::DocumentEditor | AppClass::FileManager => {
            vec![AppSemanticRole::WorkSurface]
        }
        AppClass::Media => vec![AppSemanticRole::MediaSurface],
        AppClass::Communication => vec![AppSemanticRole::ReviewSurface],
        AppClass::Unknown => vec![AppSemanticRole::WorkSurface],
    };

    if contains_run_intent(normalized) && !roles.contains(&AppSemanticRole::ExecutionSurface) {
        roles.push(AppSemanticRole::ExecutionSurface);
    }
    if contains_account_or_session_intent(normalized)
        && !roles.contains(&AppSemanticRole::AccountSessionSurface)
    {
        roles.push(AppSemanticRole::AccountSessionSurface);
    }
    if contains_review_intent(normalized) && !roles.contains(&AppSemanticRole::ReviewSurface) {
        roles.push(AppSemanticRole::ReviewSurface);
    }
    roles
}

fn classify_task_family(
    spec: &GuiTaskSpec,
    normalized: &str,
    app_anchors: &[AppAnchor],
) -> TaskFamily {
    if has_app_class(app_anchors, AppClass::Ide) || content_is_code_like(spec, normalized) {
        TaskFamily::Coding
    } else if has_app_class(app_anchors, AppClass::Media)
        || contains_media_workflow_intent(normalized)
    {
        TaskFamily::Media
    } else if has_app_class(app_anchors, AppClass::Browser)
        || spec
            .targets
            .iter()
            .any(|target| matches!(target, TargetRef::Url(_)))
    {
        TaskFamily::Browser
    } else if has_app_class(app_anchors, AppClass::Spreadsheet) {
        TaskFamily::Spreadsheet
    } else if has_app_class(app_anchors, AppClass::Communication)
        || contains_any(normalized, &["email", "mail", "message", "send", "draft"])
    {
        TaskFamily::Communication
    } else if has_app_class(app_anchors, AppClass::Terminal) {
        TaskFamily::SystemTerminal
    } else if has_app_class(app_anchors, AppClass::DocumentEditor) {
        TaskFamily::DocumentEditing
    } else if has_app_class(app_anchors, AppClass::FileManager)
        || contains_any(normalized, &["folder", "directory", "rename", "move file"])
    {
        TaskFamily::FileManagement
    } else if contains_any(
        normalized,
        &["research", "summarize", "find sources", "cite"],
    ) {
        TaskFamily::Research
    } else {
        TaskFamily::General
    }
}

fn content_is_code_like(spec: &GuiTaskSpec, normalized: &str) -> bool {
    match spec.content.as_ref() {
        Some(ContentClass::Generated { hint, language }) => {
            language.is_some()
                || contains_any(
                    &normalize_text(hint),
                    &["program", "script", "code", "function", "class", "python"],
                )
        }
        Some(ContentClass::Literal(text)) => contains_any(
            &normalize_text(text),
            &["def ", "function ", "class ", "import ", "#include"],
        ),
        None => contains_any(
            normalized,
            &[
                "program", "script", "code", "function", "debug", "test", "compile",
            ],
        ),
    }
}

fn classify_visibility(
    spec: &GuiTaskSpec,
    normalized: &str,
    app_anchors: &[AppAnchor],
    collaboration: CollaborationRequirement,
    ambiguity: AmbiguitySeverity,
) -> VisibilityExpectation {
    if matches!(collaboration, CollaborationRequirement::Required)
        || matches!(
            ambiguity,
            AmbiguitySeverity::AccountSession | AmbiguitySeverity::Destructive
        )
    {
        return VisibilityExpectation::HumanObserved;
    }

    let app_visible = !app_anchors.is_empty()
        && (matches!(
            spec.primary_verb,
            Verb::Open | Verb::Switch | Verb::Other(_)
        ) || app_anchors
            .iter()
            .any(|anchor| anchor.strength != AppAnchorStrength::Incidental));
    let workflow_visible =
        contains_any(
            normalized,
            &[
                "show output",
                "show me output",
                "show the output",
                "run it there",
                "open terminal",
                "visibly",
                "visible",
                "see it work",
                "see if it works",
            ],
        ) || (contains_run_intent(normalized) && app_visible && contains_show_intent(normalized));

    if workflow_visible {
        VisibilityExpectation::WorkflowVisible
    } else if app_visible {
        VisibilityExpectation::AppVisible
    } else if contains_show_intent(normalized) {
        VisibilityExpectation::ResultVisible
    } else {
        VisibilityExpectation::None
    }
}

fn classify_observation(
    normalized: &str,
    visibility: VisibilityExpectation,
    collaboration: CollaborationRequirement,
) -> ObservationRequirement {
    if matches!(collaboration, CollaborationRequirement::Required)
        || matches!(visibility, VisibilityExpectation::HumanObserved)
    {
        ObservationRequirement::HumanInspection
    } else if matches!(
        visibility,
        VisibilityExpectation::ResultVisible
            | VisibilityExpectation::AppVisible
            | VisibilityExpectation::WorkflowVisible
    ) {
        ObservationRequirement::SurfaceResult
    } else if contains_any(normalized, &["verify", "check", "test", "run"]) {
        ObservationRequirement::EvidenceSummary
    } else {
        ObservationRequirement::None
    }
}

fn classify_collaboration_requirement(normalized: &str) -> CollaborationRequirement {
    if contains_any(
        normalized,
        &[
            "ask me",
            "approve",
            "approval",
            "let me review",
            "let me check",
            "manual",
            "before sending",
            "before send",
        ],
    ) {
        CollaborationRequirement::Required
    } else if contains_any(normalized, &["review", "confirm"]) {
        CollaborationRequirement::Optional
    } else {
        CollaborationRequirement::None
    }
}

fn classify_ambiguity(spec: &GuiTaskSpec, normalized: &str) -> AmbiguitySeverity {
    if contains_destructive_intent(normalized) {
        return AmbiguitySeverity::Destructive;
    }
    if contains_account_or_session_intent(normalized) {
        return AmbiguitySeverity::AccountSession;
    }
    if spec.ambiguities.iter().any(|amb| {
        matches!(
            amb,
            Ambiguity::FileNotSpecified | Ambiguity::MultipleTargetsPossible
        )
    }) {
        return AmbiguitySeverity::Identity;
    }
    if spec
        .ambiguities
        .iter()
        .any(|amb| matches!(amb, Ambiguity::ContentScopeUnclear))
    {
        return AmbiguitySeverity::WorkflowChanging;
    }
    if spec
        .ambiguities
        .iter()
        .any(|amb| matches!(amb, Ambiguity::AppNotSpecified))
    {
        return AmbiguitySeverity::Visibility;
    }
    if contains_deictic_reference(normalized) && spec.targets.is_empty() && spec.content.is_none() {
        return AmbiguitySeverity::Identity;
    }
    AmbiguitySeverity::None
}

fn classify_safety(normalized: &str, ambiguity: AmbiguitySeverity) -> WorkflowSafetyClass {
    if matches!(ambiguity, AmbiguitySeverity::Destructive) {
        WorkflowSafetyClass::DestructiveOrExternal
    } else if matches!(ambiguity, AmbiguitySeverity::AccountSession) {
        WorkflowSafetyClass::ReviewRequired
    } else if contains_any(normalized, &["create", "write", "edit", "run", "open"]) {
        WorkflowSafetyClass::Reversible
    } else {
        WorkflowSafetyClass::Safe
    }
}

fn classify_fallback_tolerance(
    strongest_anchor: AppAnchorStrength,
    visibility: VisibilityExpectation,
    collaboration: CollaborationRequirement,
    ambiguity: AmbiguitySeverity,
) -> FallbackTolerance {
    if matches!(
        ambiguity,
        AmbiguitySeverity::Destructive | AmbiguitySeverity::AccountSession
    ) || matches!(collaboration, CollaborationRequirement::Required)
    {
        FallbackTolerance::Forbidden
    } else if matches!(strongest_anchor, AppAnchorStrength::Required)
        || matches!(
            visibility,
            VisibilityExpectation::AppVisible | VisibilityExpectation::WorkflowVisible
        )
    {
        FallbackTolerance::AskFirst
    } else if matches!(visibility, VisibilityExpectation::None) {
        FallbackTolerance::Allowed
    } else {
        FallbackTolerance::Unknown
    }
}

fn resolve_fidelity(frame: &SemanticWorkflowFrame) -> WorkflowFidelityResolution {
    let mut requested = WorkflowFidelityTier::MinimalResultFidelity;
    if frame
        .app_anchors
        .iter()
        .any(|anchor| anchor.strength != AppAnchorStrength::Incidental)
        || matches!(
            frame.visibility_expectation,
            VisibilityExpectation::AppVisible
        )
    {
        requested = requested.max(WorkflowFidelityTier::AppContextFidelity);
    }
    if matches!(
        frame.visibility_expectation,
        VisibilityExpectation::WorkflowVisible | VisibilityExpectation::ResultVisible
    ) {
        requested = requested.max(WorkflowFidelityTier::WorkflowStageFidelity);
    }
    if matches!(
        frame.visibility_expectation,
        VisibilityExpectation::HumanObserved
    ) || matches!(
        frame.collaboration_requirement,
        CollaborationRequirement::Required
    ) || matches!(
        frame.ambiguity_level,
        AmbiguitySeverity::AccountSession | AmbiguitySeverity::Destructive
    ) {
        requested = requested.max(WorkflowFidelityTier::HumanObservedFidelity);
    }

    let degradation_policy = match requested {
        WorkflowFidelityTier::MinimalResultFidelity => {
            FidelityDegradationPolicy::SilentFallbackAllowed
        }
        WorkflowFidelityTier::AppContextFidelity => FidelityDegradationPolicy::AskBeforeFallback,
        WorkflowFidelityTier::WorkflowStageFidelity => FidelityDegradationPolicy::ExplainFallback,
        WorkflowFidelityTier::HumanObservedFidelity => FidelityDegradationPolicy::AskBeforeFallback,
    };
    let partial_completion_policy = match requested {
        WorkflowFidelityTier::HumanObservedFidelity => PartialCompletionPolicy::PauseForRecovery,
        WorkflowFidelityTier::WorkflowStageFidelity | WorkflowFidelityTier::AppContextFidelity => {
            PartialCompletionPolicy::ReportPartial
        }
        WorkflowFidelityTier::MinimalResultFidelity => PartialCompletionPolicy::ReportPartial,
    };

    WorkflowFidelityResolution {
        requested_fidelity: requested,
        minimum_acceptable_fidelity: requested,
        environment_available_fidelity: None,
        planned_fidelity: requested,
        fallback_adjusted_fidelity: None,
        degradation_policy,
        partial_completion_policy,
    }
}

fn build_trace(
    task_family: TaskFamily,
    app_anchors: &[AppAnchor],
    visibility: VisibilityExpectation,
    ambiguity: AmbiguitySeverity,
    safety: WorkflowSafetyClass,
    normalized: &str,
) -> SemanticWorkflowTrace {
    let mut signal_labels = Vec::new();
    signal_labels.push(format!("task_family:{:?}", task_family));
    signal_labels.push(format!("visibility:{:?}", visibility));
    signal_labels.push(format!("ambiguity:{:?}", ambiguity));
    signal_labels.push(format!("safety:{:?}", safety));
    if !app_anchors.is_empty() {
        signal_labels.push(format!("app_anchors:{}", app_anchors.len()));
    }
    if contains_run_intent(normalized) {
        signal_labels.push("run_intent".to_string());
    }
    if contains_show_intent(normalized) {
        signal_labels.push("show_intent".to_string());
    }
    if contains_account_or_session_intent(normalized) {
        signal_labels.push("account_or_session_intent".to_string());
    }
    SemanticWorkflowTrace {
        signal_labels,
        explanation: "deterministic_phase_1_metadata_only".to_string(),
    }
}

fn strongest_app_anchor(app_anchors: &[AppAnchor]) -> AppAnchorStrength {
    if app_anchors
        .iter()
        .any(|anchor| anchor.strength == AppAnchorStrength::SafetyCredential)
    {
        AppAnchorStrength::SafetyCredential
    } else if app_anchors
        .iter()
        .any(|anchor| anchor.strength == AppAnchorStrength::Required)
    {
        AppAnchorStrength::Required
    } else if app_anchors
        .iter()
        .any(|anchor| anchor.strength == AppAnchorStrength::Preferred)
    {
        AppAnchorStrength::Preferred
    } else {
        AppAnchorStrength::Incidental
    }
}

fn audit_safe_label(value: &str) -> String {
    let normalized = normalize_text(value);
    let mut label = normalized
        .split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join(" ");
    if label.len() > 64 {
        label.truncate(64);
    }
    if label.is_empty() {
        "target".to_string()
    } else {
        label
    }
}

fn has_app_class(app_anchors: &[AppAnchor], app_class: AppClass) -> bool {
    app_anchors
        .iter()
        .any(|anchor| anchor.app_class == app_class)
}

fn contains_word(text: &str, needle: &str) -> bool {
    text.split_whitespace().any(|word| word == needle)
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle.trim()))
}

fn contains_show_intent(normalized: &str) -> bool {
    contains_any(normalized, &["show", "output", "see", "display", "visible"])
}

fn contains_run_intent(normalized: &str) -> bool {
    contains_any(
        normalized,
        &["run", "execute", "try", "test", "compile", "launch"],
    )
}

fn contains_review_intent(normalized: &str) -> bool {
    contains_any(
        normalized,
        &["review", "approve", "approval", "confirm", "let me check"],
    )
}

fn contains_account_or_session_intent(normalized: &str) -> bool {
    contains_any(
        normalized,
        &[
            "login",
            "log in",
            "sign in",
            "signin",
            "account",
            "profile",
            "upload",
            "my playlist",
            "my playlists",
            "my account",
            "my profile",
            "my watch later",
            "my liked",
            "my videos",
            "my music",
            "my photos",
            "my files",
            "private",
            "personal",
            "checkout",
            "payment",
            "authenticated",
        ],
    )
}

fn contains_media_workflow_intent(normalized: &str) -> bool {
    [
        "play", "watch", "listen", "song", "music", "video", "playlist",
    ]
    .iter()
    .any(|word| contains_word(normalized, word))
        || contains_any(normalized, &["youtube", "spotify", "vlc"])
}

fn contains_destructive_intent(normalized: &str) -> bool {
    contains_any(
        normalized,
        &[
            "delete",
            "remove",
            "overwrite",
            "uninstall",
            "shutdown",
            "reboot",
            "format",
            "send",
            "pay",
            "purchase",
            "buy",
        ],
    )
}

fn contains_deictic_reference(normalized: &str) -> bool {
    contains_any(normalized, &["this", "that", "there", "it", "these"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn spec(
        primary_verb: Verb,
        targets: Vec<TargetRef>,
        content: Option<ContentClass>,
    ) -> GuiTaskSpec {
        GuiTaskSpec {
            primary_verb,
            targets,
            content,
            declared_preconditions: Vec::new(),
            declared_success_criteria: Vec::new(),
            ambiguities: Vec::new(),
        }
    }

    #[test]
    fn open_code_run_show_output_requires_workflow_stage_fidelity() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("VS Code".to_string())],
            Some(ContentClass::Generated {
                hint: "pascal triangle program".to_string(),
                language: Some("python".to_string()),
            }),
        );
        let analysis = analyze_semantic_workflow(
            &spec,
            "open code and write a program to print pascal triangle and run it and show output",
        );

        assert_eq!(analysis.frame.task_family, TaskFamily::Coding);
        assert_eq!(
            analysis.frame.visibility_expectation,
            VisibilityExpectation::WorkflowVisible
        );
        assert_eq!(
            analysis.fidelity.requested_fidelity,
            WorkflowFidelityTier::WorkflowStageFidelity
        );
        assert!(analysis
            .frame
            .app_anchors
            .iter()
            .any(|anchor| anchor.app_class == AppClass::Ide
                && anchor.strength == AppAnchorStrength::Required));
    }

    #[test]
    fn simple_code_generation_allows_minimal_result_fidelity() {
        let spec = spec(
            Verb::Run,
            Vec::new(),
            Some(ContentClass::Generated {
                hint: "python program".to_string(),
                language: Some("python".to_string()),
            }),
        );
        let analysis = analyze_semantic_workflow(&spec, "write a python program that prints hello");

        assert_eq!(analysis.frame.task_family, TaskFamily::Coding);
        assert_eq!(
            analysis.frame.visibility_expectation,
            VisibilityExpectation::None
        );
        assert_eq!(
            analysis.fidelity.requested_fidelity,
            WorkflowFidelityTier::MinimalResultFidelity
        );
    }

    #[test]
    fn browser_account_workflow_requires_human_observed_fidelity() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("Firefox".to_string())],
            None,
        );
        let analysis = analyze_semantic_workflow(
            &spec,
            "open browser and login to my account and upload this file",
        );

        assert_eq!(analysis.frame.task_family, TaskFamily::Browser);
        assert_eq!(
            analysis.frame.ambiguity_level,
            AmbiguitySeverity::AccountSession
        );
        assert_eq!(
            analysis.fidelity.requested_fidelity,
            WorkflowFidelityTier::HumanObservedFidelity
        );
    }

    #[test]
    fn public_media_workflow_is_media_without_account_session_ambiguity() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("YouTube".to_string())],
            None,
        );
        let analysis = analyze_semantic_workflow(&spec, "open youtube and play lo fi music");

        assert_eq!(analysis.frame.task_family, TaskFamily::Media);
        assert_eq!(analysis.frame.ambiguity_level, AmbiguitySeverity::None);
        assert_eq!(
            analysis.fidelity.requested_fidelity,
            WorkflowFidelityTier::AppContextFidelity
        );
    }

    #[test]
    fn personal_media_workflow_requires_human_observed_fidelity() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("YouTube".to_string())],
            None,
        );
        let analysis = analyze_semantic_workflow(&spec, "open youtube and play my playlist");

        assert_eq!(analysis.frame.task_family, TaskFamily::Media);
        assert_eq!(
            analysis.frame.ambiguity_level,
            AmbiguitySeverity::AccountSession
        );
        assert_eq!(
            analysis.fidelity.requested_fidelity,
            WorkflowFidelityTier::HumanObservedFidelity
        );
    }

    #[test]
    fn file_identity_ambiguity_is_visible_in_frame() {
        let mut spec = spec(Verb::Open, vec![TargetRef::File(PathBuf::from("x"))], None);
        spec.ambiguities.push(Ambiguity::FileNotSpecified);

        let analysis = analyze_semantic_workflow(&spec, "open the file");

        assert_eq!(analysis.frame.ambiguity_level, AmbiguitySeverity::Identity);
    }

    #[test]
    fn analysis_serializes_to_json() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("Terminal".to_string())],
            None,
        );
        let analysis = analyze_semantic_workflow(&spec, "open terminal and run df -h");
        let json = serde_json::to_string(&analysis).expect("analysis is serializable");
        let roundtrip: SemanticWorkflowAnalysis =
            serde_json::from_str(&json).expect("analysis is deserializable");

        assert_eq!(roundtrip, analysis);
    }
}
