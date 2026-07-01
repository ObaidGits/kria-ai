use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use super::perception::{sanitize_gui_text, stable_hash, GuiBounds};

const DISPLAY_LIMIT: usize = 160;
const ID_LIMIT: usize = 120;

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
                decision_for(
                    &pending.proposal,
                    "expired",
                    now_ms,
                    Some("proposal expired"),
                )
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
        decision_id: format!(
            "decision-{}",
            stable_hash(&format!(
                "{}|{}|{}",
                proposal.proposal_id, decision, decided_at_ms
            ))
        ),
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

fn sanitize_text(value: &str, limit: usize) -> String {
    sanitize_gui_text(value, limit).text
}
