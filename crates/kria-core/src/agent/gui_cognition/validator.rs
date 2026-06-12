use super::context::GuiContext;
use super::planner::{GuiCognitionIntent, GuiCognitionIntentKind};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GuiValidationStatus {
    Valid,
    Blocked,
    NeedsApproval,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiValidationReport {
    pub status: GuiValidationStatus,
    pub reasons: Vec<String>,
}

pub fn is_potentially_terminal_window(active_window: &str) -> bool {
    let lower = active_window.to_lowercase();
    ["terminal", "konsole", "gnome-terminal", "xterm", "shell"]
        .iter()
        .any(|needle| lower.contains(needle))
}

pub fn validate_intent(intent: &GuiCognitionIntent, context: &GuiContext) -> GuiValidationReport {
    let mut reasons = Vec::new();
    match intent.kind {
        GuiCognitionIntentKind::TypeText if intent.typed_text.is_none() => {
            reasons.push("No quoted text was provided for typing.".into());
        }
        GuiCognitionIntentKind::ClickControl if intent.control_name.is_none() => {
            reasons.push("No button/control name was provided.".into());
        }
        GuiCognitionIntentKind::TypeText
            if is_potentially_terminal_window(&context.observation.active_window_label)
                || context.active_window_is_terminal_like() =>
        {
            reasons.push(
                "active window looks like a terminal/shell, so blind typing is blocked".into(),
            );
        }
        _ => {}
    }

    if !reasons.is_empty() {
        return GuiValidationReport {
            status: GuiValidationStatus::Blocked,
            reasons,
        };
    }

    if intent.requires_approval {
        return GuiValidationReport {
            status: GuiValidationStatus::NeedsApproval,
            reasons: intent.risk_reasons.clone(),
        };
    }

    GuiValidationReport {
        status: GuiValidationStatus::Valid,
        reasons,
    }
}
