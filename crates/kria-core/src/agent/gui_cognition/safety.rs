use super::planner::GuiCognitionIntent;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GuiSafetyStatus {
    Allowed,
    RequiresApproval,
    Blocked,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiSafetyDecision {
    pub status: GuiSafetyStatus,
    pub risk_level: String,
    pub reasons: Vec<String>,
}

pub fn safety_for_intent(intent: &GuiCognitionIntent) -> GuiSafetyDecision {
    if intent.requires_approval {
        GuiSafetyDecision {
            status: GuiSafetyStatus::RequiresApproval,
            risk_level: intent.risk_level.clone(),
            reasons: if intent.risk_reasons.is_empty() {
                vec!["user requested approval before action".into()]
            } else {
                intent.risk_reasons.clone()
            },
        }
    } else {
        GuiSafetyDecision {
            status: GuiSafetyStatus::Allowed,
            risk_level: "low".into(),
            reasons: Vec::new(),
        }
    }
}

impl GuiSafetyStatus {
    pub fn as_event_status(&self) -> &'static str {
        match self {
            Self::Allowed => "Allowed",
            Self::RequiresApproval => "RequiresApproval",
            Self::Blocked => "Blocked",
        }
    }
}
