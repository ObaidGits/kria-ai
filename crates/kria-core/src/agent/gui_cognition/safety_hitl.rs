use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use super::context::GuiContext;
use super::goal_contract::{GuiActionType, GuiGoalContract};
use super::llm_planner::{typed_plan_steps, GuiLlmPlan, GuiPlanValidationReport, GuiTypedPlanStep};
use super::perception::{sanitize_gui_text, stable_hash, GuiBounds};
use super::resolver::{GuiResolvedTarget, GuiTargetResolutionSummary};

const PROPOSAL_SCHEMA_VERSION: u32 = 1;
const DISPLAY_LIMIT: usize = 160;
const ID_LIMIT: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiHitlDecisionFixture {
    Approve,
    Deny,
    ApproveExpired,
    ApproveTargetMismatch,
    ApproveProposalMismatch,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiActionProposal {
    pub proposal_schema_version: u32,
    pub proposal_id: String,
    pub request_id: String,
    pub session_id: String,
    pub workflow_id: String,
    pub goal_contract_id: String,
    pub plan_id: String,
    pub validation_id: Option<String>,
    pub resolution_id: Option<String>,
    pub context_id: String,
    pub observation_id: String,
    pub step_id: String,
    pub action_type: String,
    pub target_hash: String,
    pub target_control_id: Option<String>,
    pub target_label: Option<String>,
    pub target_role: Option<String>,
    pub target_bounds: Option<GuiBounds>,
    pub text_payload_summary: Option<String>,
    pub text_payload_hash: Option<String>,
    pub expected_precondition: String,
    pub expected_postcondition: String,
    pub risk_level: String,
    pub risk_reasons: Vec<String>,
    pub requires_user_approval: bool,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub proposal_hash: String,
    pub prompt_hash: String,
    pub can_execute: bool,
}

impl GuiActionProposal {
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "proposal_schema_version": self.proposal_schema_version,
            "proposal_id": self.proposal_id,
            "request_id": self.request_id,
            "goal_contract_id": self.goal_contract_id,
            "plan_id": self.plan_id,
            "validation_id": self.validation_id,
            "resolution_id": self.resolution_id,
            "context_id": self.context_id,
            "observation_id": self.observation_id,
            "step_id": self.step_id,
            "action_type": self.action_type,
            "target_hash": self.target_hash,
            "target_control_id": self.target_control_id,
            "target_label": self.target_label,
            "target_role": self.target_role,
            "target_bounds": self.target_bounds,
            "text_payload_summary": self.text_payload_summary,
            "text_payload_hash": self.text_payload_hash,
            "expected_precondition": self.expected_precondition,
            "expected_postcondition": self.expected_postcondition,
            "risk_level": self.risk_level,
            "risk_reasons": self.risk_reasons,
            "requires_user_approval": self.requires_user_approval,
            "created_at_ms": self.created_at_ms,
            "expires_at_ms": self.expires_at_ms,
            "proposal_hash": self.proposal_hash,
            "prompt_hash": self.prompt_hash,
            "can_execute": false,
        })
    }

    pub fn modal_metadata(&self, consequence: &str) -> serde_json::Value {
        serde_json::json!({
            "kind": "gui_cognition_action_proposal",
            "proposal_id": self.proposal_id,
            "workflow_id": self.workflow_id,
            "action_kind": self.action_type,
            "target_label": self.target_label,
            "target_role": self.target_role,
            "active_window": self.expected_precondition,
            "risk_level": self.risk_level,
            "consequence": sanitize_text(consequence, DISPLAY_LIMIT),
            "evidence_summary": "Bound to current GUI Cognition proposal",
            "action_hash": self.proposal_hash,
            "proposal_hash": self.proposal_hash,
            "target_hash": self.target_hash,
            "expires_at_ms": self.expires_at_ms,
            "can_execute": false,
        })
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiSafetyGateResult {
    pub safety_gate_id: String,
    pub proposal_id: String,
    pub request_id: String,
    pub proposal_hash: String,
    pub target_hash: String,
    pub status: String,
    pub risk_level: String,
    pub requires_user_approval: bool,
    pub approval_reason: Option<String>,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub can_request_hitl: bool,
    pub can_authorize_step7: bool,
    pub can_execute: bool,
    pub source_evidence: Vec<String>,
    pub prompt_hash: String,
    pub proposal: GuiActionProposal,
}

impl GuiSafetyGateResult {
    pub fn event_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "SafetyGateCompleted",
            "safety_gate_id": self.safety_gate_id,
            "proposal_id": self.proposal_id,
            "request_id": self.request_id,
            "proposal_hash": self.proposal_hash,
            "target_hash": self.target_hash,
            "status": self.legacy_status(),
            "safety_status": self.status,
            "risk_level": self.risk_level,
            "reasons": self.reasons(),
            "risk_reasons": self.proposal.risk_reasons,
            "requires_user_approval": self.requires_user_approval,
            "approval_reason": self.approval_reason,
            "blockers": self.blockers,
            "warnings": self.warnings,
            "can_request_hitl": self.can_request_hitl,
            "can_authorize_step7": self.can_authorize_step7,
            "can_execute": false,
            "source_evidence": self.source_evidence,
            "prompt_hash": self.prompt_hash,
            "action_type": self.proposal.action_type,
            "target_label": self.proposal.target_label,
            "target_role": self.proposal.target_role,
            "expected_postcondition": self.proposal.expected_postcondition,
            "expires_at_ms": self.proposal.expires_at_ms,
            "proposal": self.proposal.summary_json(),
        })
    }

    pub fn summary_json(&self) -> serde_json::Value {
        let mut payload = self.event_payload();
        if let Some(object) = payload.as_object_mut() {
            object.remove("type");
        }
        payload
    }

    pub fn hitl_required_event(&self) -> serde_json::Value {
        let reason = self
            .approval_reason
            .clone()
            .unwrap_or_else(|| "GUI action requires human approval".into());
        serde_json::json!({
            "type": "HitlRequired",
            "request_id": self.request_id,
            "proposal_id": self.proposal_id,
            "proposal_hash": self.proposal_hash,
            "target_hash": self.target_hash,
            "action_type": self.proposal.action_type,
            "target_label": self.proposal.target_label,
            "target_role": self.proposal.target_role,
            "risk_level": self.risk_level,
            "reason": reason,
            "risk_reasons": self.proposal.risk_reasons,
            "expected_postcondition": self.proposal.expected_postcondition,
            "expires_at_ms": self.proposal.expires_at_ms,
            "requires_user_approval": true,
            "can_authorize_step7": false,
            "can_execute": false,
            "prompt_hash": self.prompt_hash,
            "approval_request": {
                "requestId": self.request_id,
                "toolName": "GuiCognitionActionProposal",
                "riskLevel": self.risk_level,
                "reason": reason,
                "args": {
                    "gui_cognition": self.proposal.modal_metadata(&self.proposal.expected_postcondition)
                }
            }
        })
    }

    fn legacy_status(&self) -> &'static str {
        match self.status.as_str() {
            "safe_no_approval_required" => "Allowed",
            "approval_required" => "RequiresApproval",
            "blocked" => "Blocked",
            "rejected" => "Rejected",
            "stale" => "Stale",
            _ => "Blocked",
        }
    }

    fn reasons(&self) -> Vec<String> {
        if !self.blockers.is_empty() {
            return self.blockers.clone();
        }
        if !self.proposal.risk_reasons.is_empty() {
            return self.proposal.risk_reasons.clone();
        }
        self.approval_reason
            .clone()
            .map(|reason| vec![reason])
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiHitlDecision {
    pub decision_id: String,
    pub request_id: String,
    pub proposal_id: String,
    pub proposal_hash: String,
    pub target_hash: String,
    pub decision: String,
    pub decided_at_ms: i64,
    pub decision_reason: Option<String>,
    pub actor: String,
    pub user_visible_summary_hash: String,
    pub can_authorize_step7: bool,
    pub can_execute: bool,
}

impl GuiHitlDecision {
    pub fn event_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "HitlDecisionRecorded",
            "decision_id": self.decision_id,
            "request_id": self.request_id,
            "proposal_id": self.proposal_id,
            "proposal_hash": self.proposal_hash,
            "target_hash": self.target_hash,
            "decision": self.decision,
            "decided_at_ms": self.decided_at_ms,
            "decision_reason": self.decision_reason,
            "actor": self.actor,
            "user_visible_summary_hash": self.user_visible_summary_hash,
            "can_authorize_step7": self.can_authorize_step7,
            "can_execute": false,
        })
    }

    pub fn invalidated_event_payload(&self) -> serde_json::Value {
        let mut payload = self.event_payload();
        if let Some(object) = payload.as_object_mut() {
            object.insert("type".into(), serde_json::json!("HitlDecisionInvalidated"));
        }
        payload
    }

    pub fn summary_json(&self) -> serde_json::Value {
        let mut payload = self.event_payload();
        if let Some(object) = payload.as_object_mut() {
            object.remove("type");
        }
        payload
    }
}

#[derive(Debug, Clone)]
pub struct PendingGuiActionProposal {
    pub proposal: GuiActionProposal,
    pub decided: bool,
}

#[derive(Debug, Default, Clone)]
pub struct GuiHitlProposalStore {
    by_request_id: HashMap<String, PendingGuiActionProposal>,
    active_by_session: HashMap<String, String>,
}

impl GuiHitlProposalStore {
    pub fn insert_pending(&mut self, proposal: GuiActionProposal) -> Vec<GuiHitlDecision> {
        let now_ms = proposal.created_at_ms;
        let mut invalidated = Vec::new();
        if let Some(old_request_id) = self
            .active_by_session
            .insert(proposal.session_id.clone(), proposal.request_id.clone())
        {
            if old_request_id != proposal.request_id {
                if let Some(old) = self.by_request_id.remove(&old_request_id) {
                    invalidated.push(decision_for(
                        &old.proposal,
                        "stale_rejected",
                        now_ms,
                        Some("replaced by newer GUI Cognition proposal"),
                    ));
                }
            }
        }
        self.by_request_id.insert(
            proposal.request_id.clone(),
            PendingGuiActionProposal {
                proposal,
                decided: false,
            },
        );
        invalidated
    }

    pub fn lookup_by_request_id(&self, request_id: &str) -> Option<&GuiActionProposal> {
        self.by_request_id
            .get(request_id)
            .map(|pending| &pending.proposal)
    }

    pub fn record_decision(
        &mut self,
        request_id: &str,
        approved: bool,
        now_ms: i64,
    ) -> GuiHitlDecision {
        let Some(mut pending) = self.by_request_id.remove(request_id) else {
            return unknown_request_decision(request_id, now_ms);
        };
        if pending.decided {
            return decision_for(
                &pending.proposal,
                "stale_rejected",
                now_ms,
                Some("proposal was already decided"),
            );
        }
        pending.decided = true;
        self.active_by_session.remove(&pending.proposal.session_id);
        if !approved {
            return decision_for(&pending.proposal, "denied", now_ms, Some("User denied"));
        }
        validate_approval(&pending.proposal, now_ms)
    }

    pub fn expire_old_proposals(&mut self, now_ms: i64) -> Vec<GuiHitlDecision> {
        let expired = self
            .by_request_id
            .iter()
            .filter_map(|(request_id, pending)| {
                (now_ms > pending.proposal.expires_at_ms).then(|| request_id.clone())
            })
            .collect::<Vec<_>>();
        expired
            .into_iter()
            .filter_map(|request_id| self.by_request_id.remove(&request_id))
            .map(|pending| {
                self.active_by_session.remove(&pending.proposal.session_id);
                decision_for(&pending.proposal, "expired", now_ms, Some("proposal expired"))
            })
            .collect()
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

pub fn build_action_proposal(
    session_id: &str,
    workflow_id: &str,
    goal_contract: &GuiGoalContract,
    plan_id: &str,
    plan: &GuiLlmPlan,
    validation: &GuiPlanValidationReport,
    target_resolution: &GuiTargetResolutionSummary,
    context: &GuiContext,
    created_at_ms: i64,
) -> GuiActionProposal {
    build_action_proposal_for_step(
        session_id,
        workflow_id,
        goal_contract,
        plan_id,
        selected_action_step(plan),
        validation,
        target_resolution,
        context,
        created_at_ms,
    )
}

/// Step 10: build a proposal for a specific typed step (instead of the plan's
/// first executable step). Used by the multi-step workflow runtime so each step
/// gets its own immutable bound proposal.
#[allow(clippy::too_many_arguments)]
pub fn build_action_proposal_for_step(
    session_id: &str,
    workflow_id: &str,
    goal_contract: &GuiGoalContract,
    plan_id: &str,
    selected_step: Option<GuiTypedPlanStep>,
    validation: &GuiPlanValidationReport,
    target_resolution: &GuiTargetResolutionSummary,
    context: &GuiContext,
    created_at_ms: i64,
) -> GuiActionProposal {
    let step = selected_step;
    let target = selected_target(target_resolution);
    let action_type = action_type_for(goal_contract, step.as_ref(), target.as_ref());
    let risk_level = normalized_risk(
        validation
            .risk_level
            .as_deref()
            .unwrap_or(goal_contract.risk_level.as_str()),
    );
    let risk_reasons = risk_reasons(goal_contract, step.as_ref(), target.as_ref());
    let requires_user_approval = approval_required_for(
        &risk_level,
        goal_contract.requires_user_approval
            || validation.requires_user_approval
            || step.as_ref().is_some_and(|value| value.requires_approval),
        step.as_ref(),
        target.as_ref(),
    );
    let ttl_ms = ttl_for_risk(&risk_level, requires_user_approval);
    let target_hash = target
        .as_ref()
        .map(|target| target.target_hash.clone())
        .unwrap_or_else(|| {
            stable_hash(&format!(
                "no_target|{}|{}|{}|{}",
                plan_id, context.context_id, context.observation_id, action_type
            ))
        });
    let target_label = target
        .as_ref()
        .and_then(|target| nonempty(sanitize_text(&target.label, DISPLAY_LIMIT)))
        .or_else(|| {
            // App-launch / window-switch actions resolve an application or window,
            // not an on-screen control, so the target resolver returns no control.
            // Thread the app/window name from the plan step or goal contract so the
            // executor receives a real name instead of falling back to the action
            // kind. This is generic (data-driven), never per-app hardcoded.
            if matches!(action_type.as_str(), "OpenApp" | "SwitchWindow") {
                step.as_ref()
                    .and_then(|step| step.target_app_hint.clone())
                    .or_else(|| step.as_ref().and_then(|step| step.target_window_hint.clone()))
                    .or_else(|| goal_contract.target_app_hint.clone())
                    .or_else(|| goal_contract.target_window_hint.clone())
                    .and_then(|value| nonempty(sanitize_text(&value, DISPLAY_LIMIT)))
            } else {
                None
            }
        });
    let target_role = target
        .as_ref()
        .and_then(|target| nonempty(sanitize_text(&target.role, 80)));
    let target_control_id = target
        .as_ref()
        .and_then(|target| nonempty(sanitize_text(&target.control_id, ID_LIMIT)));
    let target_bounds = target.as_ref().and_then(|target| target.bounds.clone());
    let text_payload_summary = step
        .as_ref()
        .and_then(|step| step.text_payload_summary.clone())
        .or_else(|| goal_contract.text_payload_summary.clone())
        .or_else(|| goal_contract.query_summary.clone())
        .map(|value| sanitize_text(&value, DISPLAY_LIMIT));
    let text_payload_hash = step
        .as_ref()
        .and_then(|step| step.text_payload_hash.clone())
        .or_else(|| goal_contract.text_payload_hash.clone())
        .or_else(|| goal_contract.query_hash.clone())
        .map(|value| sanitize_text(&value, 96));
    let expected_precondition = step
        .as_ref()
        .map(|step| step.expected_precondition.clone())
        .unwrap_or_else(|| context.observation.active_window_display())
        .pipe(|value| sanitize_text(&value, DISPLAY_LIMIT));
    let expected_postcondition = step
        .as_ref()
        .map(|step| step.expected_postcondition.clone())
        .unwrap_or_else(|| goal_contract.desired_final_state.clone())
        .pipe(|value| sanitize_text(&value, DISPLAY_LIMIT));

    let proposal_id = format!("gui-proposal-{}", stable_hash(&format!("{session_id}|{workflow_id}|{plan_id}|{}|{}", target_resolution.resolution_id, action_type)));
    let request_id = format!("gui-hitl-{}", stable_hash(&format!("request|{proposal_id}|{}", goal_contract.prompt_hash)));
    let mut proposal = GuiActionProposal {
        proposal_schema_version: PROPOSAL_SCHEMA_VERSION,
        proposal_id,
        request_id,
        session_id: sanitize_text(session_id, ID_LIMIT),
        workflow_id: sanitize_text(workflow_id, ID_LIMIT),
        goal_contract_id: sanitize_text(&goal_contract.contract_id, ID_LIMIT),
        plan_id: sanitize_text(plan_id, ID_LIMIT),
        validation_id: validation
            .validation_id
            .clone()
            .map(|value| sanitize_text(&value, ID_LIMIT)),
        resolution_id: Some(sanitize_text(&target_resolution.resolution_id, ID_LIMIT)),
        context_id: sanitize_text(&context.context_id, ID_LIMIT),
        observation_id: sanitize_text(&context.observation_id, ID_LIMIT),
        step_id: step
            .as_ref()
            .map(|step| sanitize_text(&step.step_id, ID_LIMIT))
            .unwrap_or_else(|| "proposal_step".into()),
        action_type,
        target_hash,
        target_control_id,
        target_label,
        target_role,
        target_bounds,
        text_payload_summary,
        text_payload_hash,
        expected_precondition,
        expected_postcondition,
        risk_level,
        risk_reasons,
        requires_user_approval,
        created_at_ms,
        expires_at_ms: created_at_ms.saturating_add(ttl_ms),
        proposal_hash: String::new(),
        prompt_hash: sanitize_text(&goal_contract.prompt_hash, 96),
        can_execute: false,
    };
    proposal.proposal_hash = proposal_hash(&proposal);
    proposal
}

pub fn evaluate_safety_gate(
    proposal: GuiActionProposal,
    target_resolution: &GuiTargetResolutionSummary,
) -> GuiSafetyGateResult {
    let mut blockers = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    if !target_resolution.can_proceed_to_safety_gate && !proposal.requires_user_approval {
        blockers.push(
            target_resolution
                .blockers
                .first()
                .cloned()
                .or_else(|| target_resolution.ambiguity_reasons.first().cloned())
                .unwrap_or_else(|| "Target resolution is not ready for safety gate".into()),
        );
    } else if !target_resolution.can_proceed_to_safety_gate {
        warnings.push(
            "target metadata is incomplete; approval is metadata-only and Step 7 must revalidate"
                .into(),
        );
    }
    if proposal.proposal_hash.trim().is_empty() {
        blockers.push("proposal hash missing".into());
    }
    if proposal.target_hash.trim().is_empty() {
        blockers.push("target hash missing".into());
    }
    if proposal.risk_level == "unknown" {
        blockers.push("risk level unknown".into());
    }
    if proposal.requires_user_approval {
        warnings.push("human approval required before Step 7 authorization".into());
    }

    let status = if !blockers.is_empty() {
        "blocked"
    } else if proposal.requires_user_approval {
        "approval_required"
    } else {
        "safe_no_approval_required"
    };
    let can_request_hitl = status == "approval_required";
    let can_authorize_step7 = status == "safe_no_approval_required";
    let approval_reason = if proposal.requires_user_approval {
        Some(approval_reason(&proposal))
    } else {
        None
    };

    GuiSafetyGateResult {
        safety_gate_id: format!("safety-{}", proposal.proposal_id),
        proposal_id: proposal.proposal_id.clone(),
        request_id: proposal.request_id.clone(),
        proposal_hash: proposal.proposal_hash.clone(),
        target_hash: proposal.target_hash.clone(),
        status: status.into(),
        risk_level: proposal.risk_level.clone(),
        requires_user_approval: proposal.requires_user_approval,
        approval_reason,
        blockers: blockers.into_iter().map(|value| sanitize_text(&value, DISPLAY_LIMIT)).collect(),
        warnings: warnings.into_iter().map(|value| sanitize_text(&value, DISPLAY_LIMIT)).collect(),
        can_request_hitl,
        can_authorize_step7,
        can_execute: false,
        source_evidence: vec![
            "gui_goal_contract".into(),
            "plan_validation".into(),
            "target_resolution".into(),
        ],
        prompt_hash: proposal.prompt_hash.clone(),
        proposal,
    }
}

pub fn validate_approval(proposal: &GuiActionProposal, now_ms: i64) -> GuiHitlDecision {
    if now_ms > proposal.expires_at_ms {
        return decision_for(proposal, "expired", now_ms, Some("proposal expired"));
    }
    decision_for(proposal, "approved", now_ms, None)
}

pub fn decision_for(
    proposal: &GuiActionProposal,
    decision: &str,
    decided_at_ms: i64,
    reason: Option<&str>,
) -> GuiHitlDecision {
    let can_authorize_step7 = decision == "approved" && decided_at_ms <= proposal.expires_at_ms;
    GuiHitlDecision {
        decision_id: format!("decision-{}", stable_hash(&format!("{}|{}|{}", proposal.proposal_id, decision, decided_at_ms))),
        request_id: proposal.request_id.clone(),
        proposal_id: proposal.proposal_id.clone(),
        proposal_hash: proposal.proposal_hash.clone(),
        target_hash: proposal.target_hash.clone(),
        decision: decision.into(),
        decided_at_ms,
        decision_reason: reason.map(|value| sanitize_text(value, DISPLAY_LIMIT)),
        actor: "local_user".into(),
        user_visible_summary_hash: stable_hash(&format!(
            "{}|{}|{}|{}",
            proposal.action_type,
            proposal.target_label.as_deref().unwrap_or(""),
            proposal.target_role.as_deref().unwrap_or(""),
            proposal.expected_postcondition
        )),
        can_authorize_step7,
        can_execute: false,
    }
}

pub fn decision_from_fixture(
    proposal: &GuiActionProposal,
    fixture: &GuiHitlDecisionFixture,
    now_ms: i64,
) -> GuiHitlDecision {
    match fixture {
        GuiHitlDecisionFixture::Approve => validate_approval(proposal, now_ms),
        GuiHitlDecisionFixture::Deny => decision_for(proposal, "denied", now_ms, Some("User denied")),
        GuiHitlDecisionFixture::ApproveExpired => {
            decision_for(proposal, "expired", proposal.expires_at_ms.saturating_add(1), Some("proposal expired"))
        }
        GuiHitlDecisionFixture::ApproveTargetMismatch => decision_for(
            proposal,
            "hash_mismatch_rejected",
            now_ms,
            Some("target hash mismatch"),
        ),
        GuiHitlDecisionFixture::ApproveProposalMismatch => decision_for(
            proposal,
            "hash_mismatch_rejected",
            now_ms,
            Some("proposal hash mismatch"),
        ),
    }
}

pub fn proposal_hash(proposal: &GuiActionProposal) -> String {
    stable_hash(&format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        proposal.proposal_schema_version,
        proposal.goal_contract_id,
        proposal.plan_id,
        proposal.validation_id.as_deref().unwrap_or(""),
        proposal.resolution_id.as_deref().unwrap_or(""),
        proposal.context_id,
        proposal.observation_id,
        proposal.step_id,
        proposal.action_type,
        proposal.target_hash,
        proposal.text_payload_hash.as_deref().unwrap_or(""),
        proposal.risk_level,
        proposal.requires_user_approval,
        proposal.expected_postcondition,
    ))
}

pub fn unknown_request_decision(request_id: &str, now_ms: i64) -> GuiHitlDecision {
    let safe_request_id = sanitize_text(request_id, ID_LIMIT);
    GuiHitlDecision {
        decision_id: format!("decision-unknown-{}", stable_hash(&safe_request_id)),
        request_id: safe_request_id,
        proposal_id: "unknown".into(),
        proposal_hash: "unknown".into(),
        target_hash: "unknown".into(),
        decision: "stale_rejected".into(),
        decided_at_ms: now_ms,
        decision_reason: Some("unknown request_id".into()),
        actor: "local_user".into(),
        user_visible_summary_hash: stable_hash("unknown"),
        can_authorize_step7: false,
        can_execute: false,
    }
}

fn selected_action_step(plan: &GuiLlmPlan) -> Option<GuiTypedPlanStep> {
    typed_plan_steps(plan)
        .into_iter()
        .find(|step| {
            matches!(
                step.step_type.as_str(),
                "ClickControl"
                    | "TypeText"
                    | "PressKey"
                    | "OpenApp"
                    | "SwitchWindow"
                    | "FocusField"
                    | "BrowserNavigate"
                    | "Save"
                    | "Download"
                    | "Copy"
                    | "Paste"
                    | "RequireApproval"
            )
        })
        .or_else(|| typed_plan_steps(plan).into_iter().next())
}

fn selected_target(summary: &GuiTargetResolutionSummary) -> Option<GuiResolvedTarget> {
    summary.resolved_target.clone().or_else(|| {
        summary
            .results
            .iter()
            .find_map(|result| result.resolved_target.clone())
    })
}

fn action_type_for(
    contract: &GuiGoalContract,
    step: Option<&GuiTypedPlanStep>,
    target: Option<&GuiResolvedTarget>,
) -> String {
    let mut action = match contract.action_type {
        GuiActionType::RiskApproval => risk_action_from_text(
            &format!(
                "{} {} {}",
                contract.goal_summary,
                contract.target_control_hint.clone().unwrap_or_default(),
                target.map(|value| value.label.clone()).unwrap_or_default()
            ),
        )
        .unwrap_or_else(|| "RiskApproval".into()),
        _ => step
            .map(|step| step.step_type.clone())
            .unwrap_or_else(|| contract.action_type.as_str().into()),
    };
    action = normalize_action_type(&action);
    let risk_escalated = risk_action_from_text(&format!(
        "{} {} {}",
        action,
        contract.goal_summary,
        target.map(|value| value.label.clone()).unwrap_or_default()
    ));
    risk_escalated.unwrap_or(action)
}

fn normalize_action_type(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "click_control" | "clickcontrol" => "ClickControl",
        "type_text" | "typetext" | "fillfield" => "TypeText",
        "open_app" | "openapp" => "OpenApp",
        "switch_window" | "switchwindow" => "SwitchWindow",
        "focus_input" | "focusfield" | "focus_field" => "FocusField",
        "browser_navigate" | "browsernavigate" => "BrowserNavigate",
        "copy_content" | "copy" => "Copy",
        "paste_content" | "paste" => "Paste",
        "download" => "Download",
        "save" => "Save",
        "presskey" | "press_key" => "PressKey",
        "send" => "Send",
        "submit" => "Submit",
        "delete" | "remove" => "Delete",
        "pay" | "payment" | "purchase" => "Pay",
        "install" => "Install",
        "systemchange" | "system_change" => "SystemChange",
        _ => return sanitize_text(value, 80),
    }
    .into()
}

fn risk_action_from_text(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let pairs = [
        ("pay", "Pay"),
        ("payment", "Pay"),
        ("purchase", "Pay"),
        ("send", "Send"),
        ("submit", "Submit"),
        ("confirm order", "Submit"),
        ("delete", "Delete"),
        ("remove", "Delete"),
        ("archive", "Delete"),
        ("install", "Install"),
        ("system", "SystemChange"),
        ("security", "SystemChange"),
        ("git push", "SystemChange"),
        ("git merge", "SystemChange"),
        ("git rebase", "SystemChange"),
    ];
    pairs
        .iter()
        .find_map(|(needle, action)| lower.contains(needle).then(|| (*action).to_string()))
}

fn risk_reasons(
    contract: &GuiGoalContract,
    step: Option<&GuiTypedPlanStep>,
    target: Option<&GuiResolvedTarget>,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if contract.requires_user_approval {
        reasons.push("goal contract requires user approval".into());
    }
    if let Some(step) = step {
        if step.requires_approval {
            reasons.push(format!("plan step {} requires approval", sanitize_text(&step.step_type, 80)));
        }
    }
    let target_text = target
        .map(|target| format!("{} {}", target.label, target.role))
        .unwrap_or_default();
    if let Some(action) = risk_action_from_text(&format!("{} {}", contract.goal_summary, target_text)) {
        reasons.push(format!("risky action detected: {action}"));
    }
    if reasons.is_empty() {
        reasons.push(format!("risk level: {}", contract.risk_level.as_str()));
    }
    reasons
        .into_iter()
        .map(|value| sanitize_text(&value, DISPLAY_LIMIT))
        .collect()
}

fn approval_required_for(
    risk_level: &str,
    existing_requirement: bool,
    step: Option<&GuiTypedPlanStep>,
    target: Option<&GuiResolvedTarget>,
) -> bool {
    if existing_requirement || matches!(risk_level, "high" | "critical") {
        return true;
    }
    if risk_level == "medium" {
        let text = format!(
            "{} {} {}",
            step.map(|step| step.summary.clone()).unwrap_or_default(),
            target.map(|target| target.label.clone()).unwrap_or_default(),
            target.map(|target| target.role.clone()).unwrap_or_default()
        )
        .to_ascii_lowercase();
        return [
            "password",
            "token",
            "api key",
            "terminal",
            "email",
            "payment",
            "system",
            "security",
        ]
        .iter()
        .any(|needle| text.contains(needle));
    }
    false
}

fn normalized_risk(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "low" | "green" => "low",
        "medium" | "yellow" => "medium",
        "high" | "red" => "high",
        "critical" | "black" => "critical",
        _ => "unknown",
    }
    .into()
}

fn ttl_for_risk(risk_level: &str, requires_approval: bool) -> i64 {
    if !requires_approval {
        return 60_000;
    }
    match risk_level {
        "high" | "critical" => 30_000,
        "medium" => 60_000,
        _ => 30_000,
    }
}

fn approval_reason(proposal: &GuiActionProposal) -> String {
    if proposal.risk_reasons.is_empty() {
        format!("{} requires human approval", proposal.action_type)
    } else {
        proposal.risk_reasons.join("; ")
    }
}

fn sanitize_text(value: &str, limit: usize) -> String {
    sanitize_gui_text(value, limit).text
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}
