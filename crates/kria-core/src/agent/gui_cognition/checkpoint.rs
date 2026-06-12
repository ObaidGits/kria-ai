//! Step 11: Checkpoint / Resume.
//!
//! A checkpoint persists safe workflow progress (hashes, IDs, safe summaries
//! only). On resume KRIA must re-observe and revalidate before any action: a
//! checkpoint can restore state, but it cannot restore trust. Default behavior
//! fails closed.

use super::perception::{sanitize_gui_text, stable_hash};
use super::safety_hitl::GuiHitlDecision;
use super::workflow_runtime::{side_effect_is_risky, GuiWorkflowRun, GuiWorkflowStepReceipt};

pub const CHECKPOINT_SCHEMA_VERSION: u32 = 1;

fn safe(value: &str) -> String {
    sanitize_gui_text(value, 180).text
}

fn safe_opt(value: &Option<String>) -> Option<String> {
    value.as_ref().map(|item| safe(item))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiWorkflowCheckpoint {
    pub checkpoint_schema_version: u32,
    pub checkpoint_id: String,
    pub workflow_run_id: String,
    pub session_id: String,
    pub workflow_id: String,
    pub turn_id: String,
    pub goal_contract_id: String,
    pub plan_id: String,
    pub prompt_hash: String,
    pub current_step_index: usize,
    pub step_count: usize,
    pub step_states: Vec<serde_json::Value>,
    pub completed_step_receipts: Vec<GuiWorkflowStepReceipt>,
    pub pending_step_id: Option<String>,
    pub pending_proposal_id: Option<String>,
    pub pending_proposal_hash: Option<String>,
    pub pending_target_hash: Option<String>,
    pub pending_stable_target_identity_hash: Option<String>,
    pub pending_hitl_request_id: Option<String>,
    pub approved_decision_id: Option<String>,
    pub approved_decision_hash: Option<String>,
    pub last_observation_id: String,
    pub last_context_id: String,
    pub last_screen_hash_prefix: Option<String>,
    pub last_active_window_hash: Option<String>,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub risk_level: String,
    pub requires_user_approval: bool,
    pub checkpoint_hash: String,
    pub source_evidence: Vec<String>,
    pub can_resume: bool,
    pub can_execute: bool,
}

/// Inputs describing the pending (not-yet-completed) step at checkpoint time.
#[derive(Debug, Clone, Default)]
pub struct GuiCheckpointPending {
    pub pending_step_id: Option<String>,
    pub pending_proposal_id: Option<String>,
    pub pending_proposal_hash: Option<String>,
    pub pending_target_hash: Option<String>,
    pub pending_stable_target_identity_hash: Option<String>,
    pub pending_hitl_request_id: Option<String>,
    pub approved_decision_id: Option<String>,
    pub approved_decision_hash: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn build_checkpoint(
    run: &GuiWorkflowRun,
    pending: &GuiCheckpointPending,
    last_observation_id: &str,
    last_context_id: &str,
    last_screen_hash_prefix: Option<String>,
    last_active_window_hash: Option<String>,
    created_at_ms: i64,
    ttl_ms: i64,
) -> GuiWorkflowCheckpoint {
    let checkpoint_id = format!(
        "checkpoint-{}",
        stable_hash(&format!(
            "{}|{}|{}",
            run.workflow_run_id, run.current_step_index, created_at_ms
        ))
    );
    let mut checkpoint = GuiWorkflowCheckpoint {
        checkpoint_schema_version: CHECKPOINT_SCHEMA_VERSION,
        checkpoint_id,
        workflow_run_id: safe(&run.workflow_run_id),
        session_id: safe(&run.session_id),
        workflow_id: safe(&run.workflow_id),
        turn_id: safe(&run.turn_id),
        goal_contract_id: safe(&run.goal_contract_id),
        plan_id: safe(&run.plan_id),
        prompt_hash: safe(&run.prompt_hash),
        current_step_index: run.current_step_index,
        step_count: run.step_count,
        step_states: run
            .step_states
            .iter()
            .map(|state| state.summary_json())
            .collect(),
        completed_step_receipts: run.completed_step_receipts.clone(),
        pending_step_id: safe_opt(&pending.pending_step_id),
        pending_proposal_id: safe_opt(&pending.pending_proposal_id),
        pending_proposal_hash: safe_opt(&pending.pending_proposal_hash),
        pending_target_hash: safe_opt(&pending.pending_target_hash),
        pending_stable_target_identity_hash: safe_opt(&pending.pending_stable_target_identity_hash),
        pending_hitl_request_id: safe_opt(&pending.pending_hitl_request_id),
        approved_decision_id: safe_opt(&pending.approved_decision_id),
        approved_decision_hash: safe_opt(&pending.approved_decision_hash),
        last_observation_id: safe(last_observation_id),
        last_context_id: safe(last_context_id),
        last_screen_hash_prefix: last_screen_hash_prefix.map(|value| safe(&value)),
        last_active_window_hash: last_active_window_hash.map(|value| safe(&value)),
        created_at_ms,
        expires_at_ms: created_at_ms.saturating_add(ttl_ms),
        risk_level: safe(&run.risk_level),
        requires_user_approval: run.requires_user_approval,
        checkpoint_hash: String::new(),
        source_evidence: vec![
            "gui_workflow_run".into(),
            "completed_step_receipts".into(),
            "last_observation".into(),
        ],
        can_resume: run.status == "running" || run.status == "paused",
        can_execute: false,
    };
    checkpoint.checkpoint_hash = checkpoint_hash(&checkpoint);
    checkpoint
}

/// Deterministic checkpoint integrity hash over safe state only.
pub fn checkpoint_hash(checkpoint: &GuiWorkflowCheckpoint) -> String {
    let receipts = checkpoint
        .completed_step_receipts
        .iter()
        .map(|receipt| receipt.receipt_hash.clone())
        .collect::<Vec<_>>()
        .join(",");
    stable_hash(&format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        checkpoint.checkpoint_schema_version,
        checkpoint.workflow_run_id,
        checkpoint.session_id,
        checkpoint.plan_id,
        checkpoint.goal_contract_id,
        checkpoint.prompt_hash,
        checkpoint.current_step_index,
        checkpoint.step_count,
        receipts,
        checkpoint.pending_step_id.as_deref().unwrap_or(""),
        checkpoint.pending_proposal_hash.as_deref().unwrap_or(""),
        checkpoint.pending_target_hash.as_deref().unwrap_or(""),
        checkpoint.approved_decision_hash.as_deref().unwrap_or(""),
        checkpoint.last_observation_id,
        checkpoint.last_context_id,
    ))
}

impl GuiWorkflowCheckpoint {
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    pub fn saved_event_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "WorkflowCheckpointSaved",
            "checkpoint_id": self.checkpoint_id,
            "checkpoint_hash_prefix": self.checkpoint_hash.chars().take(12).collect::<String>(),
            "workflow_run_id": self.workflow_run_id,
            "current_step_index": self.current_step_index,
            "step_count": self.step_count,
            "completed_step_count": self.completed_step_receipts.len(),
            "pending_step_id": self.pending_step_id,
            "requires_user_approval": self.requires_user_approval,
            "can_resume": self.can_resume,
            "can_execute": false,
            "prompt_hash": self.prompt_hash,
        })
    }

    /// Returns the completed receipt for a step id, if present.
    pub fn completed_receipt(&self, step_id: &str) -> Option<&GuiWorkflowStepReceipt> {
        self.completed_step_receipts
            .iter()
            .find(|receipt| receipt.step_id == step_id)
    }
}

// ── Resume ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiWorkflowResumeRequest {
    pub resume_id: String,
    pub checkpoint_id: String,
    pub workflow_run_id: String,
    pub session_id: String,
    pub requested_at_ms: i64,
    pub current_observation_id: String,
    pub current_context_id: String,
    pub current_screen_hash_prefix: Option<String>,
    pub reason: String,
    pub prompt_hash: String,
}

/// Bounded structural signals from the post-resume re-observation.
#[derive(Debug, Clone)]
pub struct GuiResumeObservationSignals {
    pub current_screen_hash_prefix: Option<String>,
    pub current_active_window_hash: Option<String>,
    pub pending_target_still_present: bool,
    pub pending_target_identity_matches: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiWorkflowResumeResult {
    pub resume_id: String,
    pub checkpoint_id: String,
    pub workflow_run_id: String,
    pub status: String,
    pub next_step_id: Option<String>,
    pub next_step_index: usize,
    pub invalidated_approvals: Vec<String>,
    pub duplicate_action_guards: Vec<String>,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub safe_explanation: String,
    pub can_continue_workflow: bool,
    pub can_execute: bool,
    pub prompt_hash: String,
}

pub const RESUME_RESUMED: &str = "resumed";
pub const RESUME_NEEDS_REOBSERVE: &str = "needs_reobserve";
pub const RESUME_NEEDS_APPROVAL: &str = "needs_approval";
pub const RESUME_STALE_REJECTED: &str = "stale_rejected";
pub const RESUME_TARGET_MISMATCH: &str = "target_mismatch_rejected";
pub const RESUME_APPROVAL_INVALIDATED: &str = "approval_invalidated";
pub const RESUME_DUPLICATE_BLOCKED: &str = "duplicate_action_blocked";
pub const RESUME_BLOCKED: &str = "blocked";

impl GuiWorkflowResumeResult {
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    pub fn validated_event_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "WorkflowResumeValidated",
            "resume_id": self.resume_id,
            "checkpoint_id": self.checkpoint_id,
            "workflow_run_id": self.workflow_run_id,
            "status": self.status,
            "next_step_id": self.next_step_id,
            "next_step_index": self.next_step_index,
            "warnings": self.warnings,
            "can_continue_workflow": self.can_continue_workflow,
            "can_execute": false,
            "prompt_hash": self.prompt_hash,
        })
    }

    pub fn rejected_event_payload(&self) -> serde_json::Value {
        let event_type = match self.status.as_str() {
            RESUME_APPROVAL_INVALIDATED => "WorkflowApprovalInvalidated",
            RESUME_DUPLICATE_BLOCKED => "WorkflowDuplicateActionBlocked",
            _ => "WorkflowResumeRejected",
        };
        serde_json::json!({
            "type": event_type,
            "resume_id": self.resume_id,
            "checkpoint_id": self.checkpoint_id,
            "workflow_run_id": self.workflow_run_id,
            "status": self.status,
            "invalidated_approvals": self.invalidated_approvals,
            "duplicate_action_guards": self.duplicate_action_guards,
            "blockers": self.blockers,
            "safe_explanation": self.safe_explanation,
            "can_continue_workflow": false,
            "can_execute": false,
            "prompt_hash": self.prompt_hash,
        })
    }

    pub fn is_validated(&self) -> bool {
        matches!(self.status.as_str(), RESUME_RESUMED)
    }
}

/// Deterministic resume validation. A checkpoint can restore state but not
/// trust: this re-validates integrity, identity, freshness, approval, and the
/// duplicate-risky-action guard before allowing continuation. Fails closed.
#[allow(clippy::too_many_arguments)]
pub fn validate_resume(
    checkpoint: &GuiWorkflowCheckpoint,
    request: &GuiWorkflowResumeRequest,
    signals: &GuiResumeObservationSignals,
    recomputed_checkpoint_hash: &str,
    hitl_decision: Option<&GuiHitlDecision>,
    now_ms: i64,
) -> GuiWorkflowResumeResult {
    let mut result = GuiWorkflowResumeResult {
        resume_id: request.resume_id.clone(),
        checkpoint_id: checkpoint.checkpoint_id.clone(),
        workflow_run_id: checkpoint.workflow_run_id.clone(),
        status: RESUME_BLOCKED.into(),
        next_step_id: checkpoint.pending_step_id.clone(),
        next_step_index: checkpoint.current_step_index,
        invalidated_approvals: Vec::new(),
        duplicate_action_guards: Vec::new(),
        blockers: Vec::new(),
        warnings: Vec::new(),
        safe_explanation: String::new(),
        can_continue_workflow: false,
        can_execute: false,
        prompt_hash: checkpoint.prompt_hash.clone(),
    };

    // 1. Integrity.
    if recomputed_checkpoint_hash != checkpoint.checkpoint_hash {
        result.blockers.push("checkpoint integrity hash mismatch".into());
        result.safe_explanation = "The checkpoint failed its integrity check, so KRIA will not resume from it.".into();
        return result;
    }
    // 2. Identity binding.
    if request.workflow_run_id != checkpoint.workflow_run_id
        || request.session_id != checkpoint.session_id
    {
        result.blockers.push("workflow/session binding mismatch".into());
        result.safe_explanation = "The resume request does not match this workflow, so KRIA will not resume.".into();
        return result;
    }
    // 3. Expiry.
    if now_ms > checkpoint.expires_at_ms {
        result.status = RESUME_STALE_REJECTED.into();
        result.blockers.push("checkpoint expired".into());
        result.safe_explanation = "The checkpoint expired, so KRIA will not resume; please restart the task.".into();
        return result;
    }

    let pending_step_id = checkpoint.pending_step_id.clone();
    let risky = checkpoint.requires_user_approval
        || matches!(checkpoint.risk_level.as_str(), "high" | "critical");

    // 4. Duplicate risky action guard: never replay a completed risky step.
    if let Some(step_id) = pending_step_id.as_deref() {
        if let Some(receipt) = checkpoint.completed_receipt(step_id) {
            if side_effect_is_risky(&receipt.side_effect_kind) {
                result.status = RESUME_DUPLICATE_BLOCKED.into();
                result
                    .duplicate_action_guards
                    .push(format!("{} already completed", receipt.side_effect_kind));
                result.safe_explanation = "This risky step already completed once; KRIA will not repeat it.".into();
                return result;
            }
        }
    }

    // 5. Screen / context freshness.
    let screen_changed = match (
        checkpoint.last_screen_hash_prefix.as_deref(),
        signals.current_screen_hash_prefix.as_deref(),
    ) {
        (Some(before), Some(after)) => before != after,
        _ => false,
    };

    // 6. Pending target stable identity.
    if checkpoint.pending_target_hash.is_some() {
        if !signals.pending_target_still_present || !signals.pending_target_identity_matches {
            result.status = RESUME_TARGET_MISMATCH.into();
            result.blockers.push("pending target identity changed".into());
            result.safe_explanation = "The pending target is no longer the same on screen, so KRIA will not resume into it.".into();
            return result;
        }
    }

    // 7. Approval handling for risky pending steps.
    if risky {
        match hitl_decision {
            Some(decision) if decision.decision == "denied" => {
                result.status = RESUME_BLOCKED.into();
                result.blockers.push("approval was denied".into());
                result.safe_explanation = "The pending risky step was denied, so KRIA will not resume it.".into();
                return result;
            }
            Some(decision)
                if decision.decision == "approved"
                    && decision.can_authorize_step7
                    && Some(decision.proposal_hash.as_str())
                        == checkpoint.pending_proposal_hash.as_deref()
                    && Some(decision.target_hash.as_str())
                        == checkpoint.pending_target_hash.as_deref()
                    && now_ms <= checkpoint.expires_at_ms
                    && !screen_changed =>
            {
                result.status = RESUME_RESUMED.into();
                result.can_continue_workflow = true;
                result.safe_explanation =
                    "Fresh matching approval validated; KRIA resumes the same pending step.".into();
                return result;
            }
            Some(_) => {
                result.status = RESUME_APPROVAL_INVALIDATED.into();
                result
                    .invalidated_approvals
                    .push("approval no longer matches the pending proposal/target".into());
                result.safe_explanation = "The approval no longer matches the pending action, so KRIA invalidated it.".into();
                return result;
            }
            None => {
                if screen_changed {
                    result.status = RESUME_APPROVAL_INVALIDATED.into();
                    result
                        .invalidated_approvals
                        .push("screen changed before approval".into());
                    result.safe_explanation = "The screen changed before approval, so KRIA invalidated the pending approval.".into();
                    return result;
                }
                result.status = RESUME_NEEDS_APPROVAL.into();
                result.warnings.push("risky pending step still needs approval".into());
                result.safe_explanation = "The pending risky step still needs a fresh approval before KRIA can resume.".into();
                return result;
            }
        }
    }

    // 8. Safe pending step.
    if screen_changed {
        result.status = RESUME_NEEDS_REOBSERVE.into();
        result.warnings.push("screen changed; re-observe before continuing".into());
        result.safe_explanation = "The screen changed since the checkpoint, so KRIA re-observes before continuing.".into();
        return result;
    }
    result.status = RESUME_RESUMED.into();
    result.can_continue_workflow = true;
    result.safe_explanation = "Checkpoint validated against the current screen; KRIA resumes the next safe step.".into();
    result
}
