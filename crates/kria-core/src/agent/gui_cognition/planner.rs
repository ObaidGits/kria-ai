use super::goal_contract::{extract_gui_goal_contract, GuiGoalContract, GuiRiskLevel};
use super::perception::GuiObservationSnapshot;

pub use super::goal_contract::{contains_any, extract_first_quoted_segment, extract_named_control};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GuiCognitionIntentKind {
    Observe,
    AnalyzePlan,
    FocusInput,
    TypeText,
    ClickControl,
    BrowserSearchPlan,
    FillFormPlan,
    AmbiguityCheck,
    TargetAvailabilityCheck,
    SafeAction,
    FocusRecovery,
    RiskApproval,
}

impl GuiCognitionIntentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::AnalyzePlan => "analyze_plan",
            Self::FocusInput => "focus_input",
            Self::TypeText => "type_text",
            Self::ClickControl => "click_control",
            Self::BrowserSearchPlan => "browser_search_plan",
            Self::FillFormPlan => "fill_form_plan",
            Self::AmbiguityCheck => "ambiguity_check",
            Self::TargetAvailabilityCheck => "target_availability_check",
            Self::SafeAction => "safe_action",
            Self::FocusRecovery => "focus_recovery",
            Self::RiskApproval => "risk_approval",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiCognitionIntent {
    pub kind: GuiCognitionIntentKind,
    pub typed_text: Option<String>,
    pub control_name: Option<String>,
    pub requires_approval: bool,
    pub risk_level: String,
    pub risk_reasons: Vec<String>,
}

pub fn classify_gui_cognition_prompt(prompt: &str) -> GuiCognitionIntent {
    let lower = prompt.to_lowercase();
    let report = extract_gui_goal_contract(prompt, None);
    intent_from_goal_contract(prompt, &report.contract, &lower)
}

pub fn intent_from_goal_contract(
    prompt: &str,
    contract: &GuiGoalContract,
    lower: &str,
) -> GuiCognitionIntent {
    let typed_text = extract_first_quoted_segment(prompt);
    let control_name =
        extract_named_control(prompt, lower).or_else(|| contract.target_control_hint.clone());
    let risk_reasons = risk_reasons_from_prompt(&lower, contract.requires_user_approval);

    GuiCognitionIntent {
        kind: intent_kind_from_contract(&contract.intent_kind),
        typed_text,
        control_name,
        requires_approval: contract.requires_user_approval,
        risk_level: risk_level_from_contract(&contract.risk_level),
        risk_reasons,
    }
}

fn intent_kind_from_contract(intent_kind: &str) -> GuiCognitionIntentKind {
    match intent_kind {
        "analyze_plan" => GuiCognitionIntentKind::AnalyzePlan,
        "focus_input" => GuiCognitionIntentKind::FocusInput,
        "type_text" => GuiCognitionIntentKind::TypeText,
        "click_control" => GuiCognitionIntentKind::ClickControl,
        "browser_search" | "browser_search_plan" | "browser_navigate" => {
            GuiCognitionIntentKind::BrowserSearchPlan
        }
        "fill_form_plan" => GuiCognitionIntentKind::FillFormPlan,
        "target_availability_check" => GuiCognitionIntentKind::TargetAvailabilityCheck,
        "ambiguity_check" => GuiCognitionIntentKind::AmbiguityCheck,
        "focus_recovery" => GuiCognitionIntentKind::FocusRecovery,
        "safe_action" => GuiCognitionIntentKind::SafeAction,
        "risk_approval" => GuiCognitionIntentKind::RiskApproval,
        "save" | "download" | "copy_content" | "paste_content" => {
            GuiCognitionIntentKind::AnalyzePlan
        }
        "unknown" => GuiCognitionIntentKind::TargetAvailabilityCheck,
        _ => GuiCognitionIntentKind::Observe,
    }
}

fn risk_level_from_contract(risk_level: &GuiRiskLevel) -> String {
    risk_level.as_str().to_string()
}

fn risk_reasons_from_prompt(lower: &str, requires_user_approval: bool) -> Vec<String> {
    let mut reasons = Vec::new();
    for (needle, reason) in [
        ("delete", "destructive delete/remove action"),
        ("remove", "destructive delete/remove action"),
        ("archive", "destructive or state-changing archive action"),
        ("submit", "external submit action"),
        ("send", "external communication action"),
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
            && !reasons.iter().any(|item| item == reason)
        {
            reasons.push(reason.to_string());
        }
    }
    if reasons.is_empty() && requires_user_approval {
        vec!["risky GUI action requires approval".into()]
    } else {
        reasons
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

pub fn gui_plan_steps(
    intent: &GuiCognitionIntent,
    observation: &GuiObservationSnapshot,
) -> Vec<String> {
    match intent.kind {
        GuiCognitionIntentKind::Observe => vec![
            "Observe active window and exposed GUI controls".into(),
            "Report accessibility/OCR availability without executing actions".into(),
        ],
        GuiCognitionIntentKind::AnalyzePlan => vec![
            "Use the observed active window and accessible controls as context".into(),
            "Classify requested actions by risk before execution".into(),
            "Resolve exact target before any click or typing".into(),
            "Verify the result after every allowed action".into(),
        ],
        GuiCognitionIntentKind::FocusInput => vec![
            "Find a visible accessible text/search input".into(),
            "Block if there are zero, multiple, or unlabeled targets".into(),
            "Focus the resolved field and re-observe".into(),
        ],
        GuiCognitionIntentKind::TypeText => vec![
            "Find or use the focused visible text field".into(),
            "Reject credential/risky text entry without approval".into(),
            "Type through AT-SPI and re-observe the screen".into(),
        ],
        GuiCognitionIntentKind::ClickControl => vec![
            "Resolve the named button/control from accessibility data".into(),
            "Block on missing or ambiguous targets".into(),
            "Click only if risk is low, then re-observe".into(),
        ],
        GuiCognitionIntentKind::BrowserSearchPlan => vec![
            "Open or switch to a browser window".into(),
            "Resolve the browser search/address field".into(),
            "Enter the search query".into(),
            "Require approval before any risky external submit".into(),
        ],
        GuiCognitionIntentKind::FillFormPlan => vec![
            "Map visible form fields from accessibility labels".into(),
            "Validate field values before typing".into(),
            "Require approval before submit/send actions".into(),
        ],
        GuiCognitionIntentKind::AmbiguityCheck => vec![
            "Search for matching controls".into(),
            "If multiple strong matches exist, ask for clarification".into(),
            "Do not guess between similar controls".into(),
        ],
        GuiCognitionIntentKind::TargetAvailabilityCheck => vec![
            "Check whether the requested target is specified and visible".into(),
            "Block when the target is missing, ambiguous, or not exposed by accessibility".into(),
            "Explain the blocker instead of guessing".into(),
        ],
        GuiCognitionIntentKind::SafeAction => {
            if observation.text_fields.len() == 1 {
                vec![
                    "Use the single visible text field as a safe focus target".into(),
                    "Focus it, re-observe, and report verification".into(),
                ]
            } else {
                vec![
                    "Observe the GUI state".into(),
                    "Pick no action unless one low-risk target is uniquely resolvable".into(),
                ]
            }
        }
        GuiCognitionIntentKind::FocusRecovery => vec![
            "Detect current active/focused window".into(),
            "Recover focus only when the intended target is unambiguous".into(),
            "Stop safely if focus recovery could affect the wrong app".into(),
        ],
        GuiCognitionIntentKind::RiskApproval => vec![
            "Prepare a bounded action proposal".into(),
            "Classify risk and explain consequence".into(),
            "Pause for explicit user approval before execution".into(),
        ],
    }
}
