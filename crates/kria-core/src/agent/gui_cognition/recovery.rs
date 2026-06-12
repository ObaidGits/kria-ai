#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiBlocker {
    pub kind: String,
    pub reason: String,
    pub candidate_count: Option<usize>,
    pub target_name: Option<String>,
    pub options: Vec<String>,
    pub clarification_question: Option<String>,
}

impl GuiBlocker {
    pub fn new(kind: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            reason: reason.into(),
            candidate_count: None,
            target_name: None,
            options: Vec::new(),
            clarification_question: None,
        }
    }

    pub fn with_candidate_count(mut self, count: usize) -> Self {
        self.candidate_count = Some(count);
        self
    }

    pub fn with_target_name(mut self, target_name: impl Into<String>) -> Self {
        self.target_name = Some(target_name.into());
        self
    }

    pub fn with_options(mut self, options: Vec<String>) -> Self {
        self.options = options;
        self
    }

    pub fn with_clarification(mut self, question: impl Into<String>) -> Self {
        self.clarification_question = Some(question.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Step 9: Recovery Loop
// ---------------------------------------------------------------------------

use super::executor::GuiActionKind;

pub const RECOVERY_MAX_RETRY_COUNT: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiRecoveryFailureKind {
    FocusLost,
    WrongWindow,
    TargetMissing,
    TargetMoved,
    TargetAmbiguous,
    ModalAppeared,
    StaleContext,
    BackendFailed,
    VerificationFailed,
    VerificationInconclusive,
    UnsafeToRetry,
}

impl GuiRecoveryFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FocusLost => "focus_lost",
            Self::WrongWindow => "wrong_window",
            Self::TargetMissing => "target_missing",
            Self::TargetMoved => "target_moved",
            Self::TargetAmbiguous => "target_ambiguous",
            Self::ModalAppeared => "modal_appeared",
            Self::StaleContext => "stale_context",
            Self::BackendFailed => "backend_failed",
            Self::VerificationFailed => "verification_failed",
            Self::VerificationInconclusive => "verification_inconclusive",
            Self::UnsafeToRetry => "unsafe_to_retry",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiRecoveryActionKind {
    ReObserve,
    RefocusSameTarget,
    SwitchBackToWindow,
    ReResolveTarget,
    RetryIdempotentAction,
    AskClarification,
    Stop,
}

impl GuiRecoveryActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReObserve => "ReObserve",
            Self::RefocusSameTarget => "RefocusSameTarget",
            Self::SwitchBackToWindow => "SwitchBackToWindow",
            Self::ReResolveTarget => "ReResolveTarget",
            Self::RetryIdempotentAction => "RetryIdempotentAction",
            Self::AskClarification => "AskClarification",
            Self::Stop => "Stop",
        }
    }

    /// Recovery actions that re-touch the GUI through the input backend. These
    /// are only allowed for bounded, idempotent recovery and must never run for
    /// risky actions.
    pub fn uses_input_backend(self) -> bool {
        matches!(
            self,
            Self::RefocusSameTarget | Self::SwitchBackToWindow | Self::RetryIdempotentAction
        )
    }

    pub fn is_executable_recovery(self) -> bool {
        matches!(self, Self::ReObserve) || self.uses_input_backend()
    }
}

/// Deterministic signals derived from the post-action observation / verification.
/// Recovery never reads raw prompt/OCR/clipboard text; only these bounded
/// structural signals drive the decision.
#[derive(Debug, Clone)]
pub struct GuiRecoverySignals {
    pub backend_success: bool,
    pub verification_status: String,
    pub verification_strategy: String,
    pub matched_expected_state: bool,
    pub target_still_present: bool,
    pub target_identity_matches: bool,
    pub modal_present: bool,
    pub active_window_known: bool,
    pub reresolve_candidate_count: usize,
    pub context_stale: bool,
}

#[derive(Debug, Clone)]
pub struct GuiRecoveryInput {
    pub recovery_id: String,
    pub execution_id: String,
    pub verification_id: String,
    pub proposal_id: String,
    pub proposal_hash: String,
    pub target_hash: String,
    pub action_type: String,
    pub risk_level: String,
    pub requires_user_approval: bool,
    pub hitl_denied: bool,
    pub hitl_stale: bool,
    pub retry_count: u32,
    pub prompt_hash: String,
    pub signals: GuiRecoverySignals,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiRecoveryAssessment {
    pub recovery_id: String,
    pub execution_id: String,
    pub verification_id: String,
    pub proposal_id: String,
    pub proposal_hash: String,
    pub target_hash: String,
    pub action_type: String,
    pub failure_kind: String,
    pub status: String,
    pub proposed_recovery_step: String,
    pub recovery_action_kind: String,
    pub requires_user_approval: bool,
    pub can_recover: bool,
    pub can_execute_recovery: bool,
    pub retry_count: u32,
    pub max_retry_count: u32,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub safe_explanation: String,
    pub recovery_hint: Option<String>,
    pub prompt_hash: String,
}

impl GuiRecoveryAssessment {
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "recovery_id": self.recovery_id,
            "execution_id": self.execution_id,
            "verification_id": self.verification_id,
            "proposal_id": self.proposal_id,
            "proposal_hash": self.proposal_hash,
            "target_hash": self.target_hash,
            "action_type": self.action_type,
            "failure_kind": self.failure_kind,
            "status": self.status,
            "proposed_recovery_step": self.proposed_recovery_step,
            "recovery_action_kind": self.recovery_action_kind,
            "requires_user_approval": self.requires_user_approval,
            "can_recover": self.can_recover,
            "can_execute_recovery": self.can_execute_recovery,
            "retry_count": self.retry_count,
            "max_retry_count": self.max_retry_count,
            "blockers": self.blockers,
            "warnings": self.warnings,
            "safe_explanation": self.safe_explanation,
            "recovery_hint": self.recovery_hint,
            "prompt_hash": self.prompt_hash,
        })
    }

    pub fn event_payload(&self) -> serde_json::Value {
        let mut payload = self.summary_json();
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                "type".into(),
                serde_json::Value::String("RecoveryAssessmentCompleted".into()),
            );
        }
        payload
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiRecoveryResult {
    pub recovery_id: String,
    pub execution_id: String,
    pub status: String,
    pub recovery_action_kind: String,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub backend_used: String,
    pub post_recovery_observation_id: Option<String>,
    pub post_recovery_context_id: Option<String>,
    pub verification_result: String,
    pub safe_error_summary: Option<String>,
    pub next_recommended_state: String,
    pub can_retry_original_action: bool,
    pub can_continue_workflow: bool,
    pub prompt_hash: String,
}

impl GuiRecoveryResult {
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "recovery_id": self.recovery_id,
            "execution_id": self.execution_id,
            "status": self.status,
            "recovery_action_kind": self.recovery_action_kind,
            "started_at_ms": self.started_at_ms,
            "completed_at_ms": self.completed_at_ms,
            "backend_used": self.backend_used,
            "post_recovery_observation_id": self.post_recovery_observation_id,
            "post_recovery_context_id": self.post_recovery_context_id,
            "verification_result": self.verification_result,
            "safe_error_summary": self.safe_error_summary,
            "next_recommended_state": self.next_recommended_state,
            "can_retry_original_action": self.can_retry_original_action,
            "can_continue_workflow": self.can_continue_workflow,
            "prompt_hash": self.prompt_hash,
        })
    }

    pub fn started_event_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "RecoveryActionStarted",
            "recovery_id": self.recovery_id,
            "execution_id": self.execution_id,
            "recovery_action_kind": self.recovery_action_kind,
            "backend_used": self.backend_used,
            "prompt_hash": self.prompt_hash,
        })
    }

    pub fn completed_event_payload(&self) -> serde_json::Value {
        let mut payload = self.summary_json();
        if let Some(object) = payload.as_object_mut() {
            let event_type = if self.status == "recovered" {
                "RecoveryActionCompleted"
            } else {
                "RecoveryBlocked"
            };
            object.insert(
                "type".into(),
                serde_json::Value::String(event_type.into()),
            );
        }
        payload
    }
}

pub fn recovery_blocked_event(assessment: &GuiRecoveryAssessment) -> serde_json::Value {
    serde_json::json!({
        "type": "RecoveryBlocked",
        "recovery_id": assessment.recovery_id,
        "execution_id": assessment.execution_id,
        "failure_kind": assessment.failure_kind,
        "status": assessment.status,
        "recovery_action_kind": assessment.recovery_action_kind,
        "requires_user_approval": assessment.requires_user_approval,
        "blockers": assessment.blockers,
        "safe_explanation": assessment.safe_explanation,
        "recovery_hint": assessment.recovery_hint,
        "prompt_hash": assessment.prompt_hash,
    })
}

fn risky_action(action_type: &str, risk_level: &str, requires_user_approval: bool) -> bool {
    let risk = risk_level.trim().to_ascii_lowercase();
    if risk == "high" || risk == "critical" || requires_user_approval {
        return true;
    }
    // Semantic risky verbs in case the action type carries them.
    let action = action_type.trim().to_ascii_lowercase();
    [
        "submit", "send", "delete", "pay", "install", "system", "git",
    ]
    .iter()
    .any(|verb| action.contains(verb))
}

fn idempotent_recoverable(kind: &GuiActionKind) -> bool {
    matches!(
        kind,
        GuiActionKind::OpenApp | GuiActionKind::SwitchWindow | GuiActionKind::FocusField
    )
}

/// Returns true only when verification did not confirm the expected state, i.e.
/// a recovery assessment should run. Verified actions never trigger recovery.
pub fn should_attempt_recovery(verification_status: &str) -> bool {
    !matches!(verification_status.trim().to_ascii_lowercase().as_str(), "verified")
}

fn blocked(
    input: &GuiRecoveryInput,
    failure_kind: GuiRecoveryFailureKind,
    status: &str,
    recovery_action_kind: GuiRecoveryActionKind,
    requires_user_approval: bool,
    blockers: Vec<String>,
    explanation: &str,
) -> GuiRecoveryAssessment {
    GuiRecoveryAssessment {
        recovery_id: input.recovery_id.clone(),
        execution_id: input.execution_id.clone(),
        verification_id: input.verification_id.clone(),
        proposal_id: input.proposal_id.clone(),
        proposal_hash: input.proposal_hash.clone(),
        target_hash: input.target_hash.clone(),
        action_type: input.action_type.clone(),
        failure_kind: failure_kind.as_str().into(),
        status: status.into(),
        proposed_recovery_step: recovery_action_kind.as_str().into(),
        recovery_action_kind: recovery_action_kind.as_str().into(),
        requires_user_approval,
        can_recover: false,
        can_execute_recovery: false,
        retry_count: input.retry_count,
        max_retry_count: RECOVERY_MAX_RETRY_COUNT,
        blockers,
        warnings: Vec::new(),
        safe_explanation: explanation.into(),
        recovery_hint: Some(
            "Re-observe and confirm a safe target before any manual retry; KRIA will not blind-retry.".into(),
        ),
        prompt_hash: input.prompt_hash.clone(),
    }
}

fn recoverable(
    input: &GuiRecoveryInput,
    failure_kind: GuiRecoveryFailureKind,
    status: &str,
    recovery_action_kind: GuiRecoveryActionKind,
    explanation: &str,
) -> GuiRecoveryAssessment {
    GuiRecoveryAssessment {
        recovery_id: input.recovery_id.clone(),
        execution_id: input.execution_id.clone(),
        verification_id: input.verification_id.clone(),
        proposal_id: input.proposal_id.clone(),
        proposal_hash: input.proposal_hash.clone(),
        target_hash: input.target_hash.clone(),
        action_type: input.action_type.clone(),
        failure_kind: failure_kind.as_str().into(),
        status: status.into(),
        proposed_recovery_step: recovery_action_kind.as_str().into(),
        recovery_action_kind: recovery_action_kind.as_str().into(),
        requires_user_approval: false,
        can_recover: true,
        can_execute_recovery: true,
        retry_count: input.retry_count,
        max_retry_count: RECOVERY_MAX_RETRY_COUNT,
        blockers: Vec::new(),
        warnings: Vec::new(),
        safe_explanation: explanation.into(),
        recovery_hint: Some("KRIA will attempt one bounded, safe recovery action.".into()),
        prompt_hash: input.prompt_hash.clone(),
    }
}

/// Deterministic recovery assessment. Classifies the failure and decides whether
/// a single bounded, safe recovery action is allowed. Risky / denied / stale /
/// ambiguous / moved-target / modal cases always stop.
pub fn assess_recovery(input: &GuiRecoveryInput) -> GuiRecoveryAssessment {
    let action_kind = GuiActionKind::from_action_type(&input.action_type);
    let signals = &input.signals;

    // 1. Hard safety gates that always stop (never auto-recover).
    if input.hitl_denied {
        return blocked(
            input,
            GuiRecoveryFailureKind::UnsafeToRetry,
            "blocked",
            GuiRecoveryActionKind::Stop,
            false,
            vec!["denied HITL approval cannot be auto-recovered".into()],
            "This action was denied at approval, so KRIA will not retry it.",
        );
    }
    if input.hitl_stale {
        return blocked(
            input,
            GuiRecoveryFailureKind::UnsafeToRetry,
            "blocked",
            GuiRecoveryActionKind::Stop,
            false,
            vec!["stale or invalidated approval cannot authorize recovery".into()],
            "The prior approval is stale, so KRIA will not retry without a fresh decision.",
        );
    }
    if risky_action(&input.action_type, &input.risk_level, input.requires_user_approval) {
        return blocked(
            input,
            GuiRecoveryFailureKind::UnsafeToRetry,
            "blocked",
            GuiRecoveryActionKind::Stop,
            true,
            vec!["risky/high-impact action is never auto-recovered".into()],
            "This is a risky action, so KRIA stops and will not retry it automatically.",
        );
    }

    // 2. One recovery attempt only.
    if input.retry_count >= RECOVERY_MAX_RETRY_COUNT {
        return blocked(
            input,
            GuiRecoveryFailureKind::UnsafeToRetry,
            "blocked",
            GuiRecoveryActionKind::Stop,
            false,
            vec!["recovery was already attempted once".into()],
            "KRIA already attempted one safe recovery and will not retry again.",
        );
    }

    // 3. Backend failure -> no blind retry.
    if !signals.backend_success || signals.verification_status == "blocked" {
        return blocked(
            input,
            GuiRecoveryFailureKind::BackendFailed,
            "blocked",
            GuiRecoveryActionKind::Stop,
            false,
            vec!["deterministic action backend failed".into()],
            "The action backend failed, so KRIA stops instead of blind-retrying.",
        );
    }

    // 4. A modal / dialog appeared -> pause and explain.
    if signals.modal_present {
        return blocked(
            input,
            GuiRecoveryFailureKind::ModalAppeared,
            "blocked",
            GuiRecoveryActionKind::Stop,
            false,
            vec!["a dialog or modal became visible".into()],
            "A dialog appeared after the action, so KRIA pauses for you to review it.",
        );
    }

    // 5. Stale context -> safe re-observe only (no input-backend action).
    if signals.context_stale {
        let mut assessment = recoverable(
            input,
            GuiRecoveryFailureKind::StaleContext,
            "needs_reobserve",
            GuiRecoveryActionKind::ReObserve,
            "Context looked stale, so KRIA re-observes before doing anything else.",
        );
        assessment.can_recover = true;
        assessment.can_execute_recovery = true;
        return assessment;
    }

    // 6. Inconclusive verification -> safe re-observe only.
    if signals.verification_status == "inconclusive" {
        return recoverable(
            input,
            GuiRecoveryFailureKind::VerificationInconclusive,
            "needs_reobserve",
            GuiRecoveryActionKind::ReObserve,
            "Verification was inconclusive, so KRIA re-observes the screen safely.",
        );
    }

    // 7. verification_failed classification.
    // Target no longer present after re-observe.
    if !signals.target_still_present {
        if signals.reresolve_candidate_count > 1 {
            return blocked(
                input,
                GuiRecoveryFailureKind::TargetAmbiguous,
                "needs_clarification",
                GuiRecoveryActionKind::AskClarification,
                false,
                vec!["multiple matching targets were found".into()],
                "Several similar targets are visible, so KRIA asks you which one to use.",
            );
        }
        return blocked(
            input,
            GuiRecoveryFailureKind::TargetMissing,
            "blocked",
            GuiRecoveryActionKind::Stop,
            false,
            vec!["the resolved target is no longer present".into()],
            "The target is no longer on screen and cannot be re-resolved safely, so KRIA stops.",
        );
    }

    // Target present but identity changed -> do not guess.
    if !signals.target_identity_matches {
        return blocked(
            input,
            GuiRecoveryFailureKind::TargetMoved,
            "needs_clarification",
            GuiRecoveryActionKind::AskClarification,
            false,
            vec!["target identity changed after the action".into()],
            "The target looks different now, so KRIA asks for clarification instead of guessing.",
        );
    }

    // Ambiguous duplicates even though one still matches.
    if signals.reresolve_candidate_count > 1 {
        return blocked(
            input,
            GuiRecoveryFailureKind::TargetAmbiguous,
            "needs_clarification",
            GuiRecoveryActionKind::AskClarification,
            false,
            vec!["multiple matching targets were found".into()],
            "Several similar targets are visible, so KRIA asks you which one to use.",
        );
    }

    // Target present + identity stable: safe idempotent recovery is possible.
    match signals.verification_strategy.as_str() {
        "focused_control" if action_kind == GuiActionKind::FocusField => recoverable(
            input,
            GuiRecoveryFailureKind::FocusLost,
            "recoverable",
            GuiRecoveryActionKind::RefocusSameTarget,
            "Focus moved away, so KRIA re-focuses the same still-valid field once.",
        ),
        "active_window_match"
            if matches!(action_kind, GuiActionKind::OpenApp | GuiActionKind::SwitchWindow)
                && signals.active_window_known =>
        {
            recoverable(
                input,
                GuiRecoveryFailureKind::WrongWindow,
                "recoverable",
                GuiRecoveryActionKind::SwitchBackToWindow,
                "The intended window is still visible, so KRIA switches back to it once.",
            )
        }
        _ if idempotent_recoverable(&action_kind) => recoverable(
            input,
            GuiRecoveryFailureKind::VerificationFailed,
            "recoverable",
            GuiRecoveryActionKind::RetryIdempotentAction,
            "The action is idempotent and the target is still valid, so KRIA retries it once.",
        ),
        _ => blocked(
            input,
            GuiRecoveryFailureKind::VerificationFailed,
            "blocked",
            GuiRecoveryActionKind::Stop,
            false,
            vec!["non-idempotent action is not safe to auto-retry".into()],
            "This action is not safe to repeat automatically, so KRIA stops and recommends re-planning.",
        ),
    }
}
