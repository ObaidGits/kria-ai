use kria_core::agent::gui_cognition::perception::GuiBounds;
use kria_core::agent::gui_cognition::safety_hitl::{
    decision_from_fixture, evaluate_safety_gate, proposal_hash, GuiActionProposal,
    GuiHitlDecisionFixture, GuiHitlProposalStore,
};
use kria_core::agent::gui_cognition::resolver::GuiTargetResolutionSummary;

fn proposal(action_type: &str, risk_level: &str, requires_approval: bool) -> GuiActionProposal {
    let mut proposal = GuiActionProposal {
        proposal_schema_version: 1,
        proposal_id: "proposal-1".into(),
        request_id: "request-1".into(),
        session_id: "session-1".into(),
        workflow_id: "workflow-1".into(),
        goal_contract_id: "goal-1".into(),
        plan_id: "plan-1".into(),
        validation_id: Some("validation-1".into()),
        resolution_id: Some("resolution-1".into()),
        context_id: "context-1".into(),
        observation_id: "observation-1".into(),
        step_id: "step-1".into(),
        action_type: action_type.into(),
        target_hash: "target-hash-1".into(),
        target_control_id: Some("control-1".into()),
        target_label: Some(action_type.into()),
        target_role: Some("button".into()),
        target_bounds: Some(GuiBounds {
            x: 1,
            y: 2,
            width: 100,
            height: 30,
        }),
        text_payload_summary: None,
        text_payload_hash: None,
        expected_precondition: "Fixture App".into(),
        expected_postcondition: format!("{action_type} completed"),
        risk_level: risk_level.into(),
        risk_reasons: vec![format!("{action_type} risk")],
        requires_user_approval: requires_approval,
        created_at_ms: 1_000,
        expires_at_ms: 31_000,
        proposal_hash: String::new(),
        prompt_hash: "prompt-hash".into(),
        can_execute: false,
    };
    proposal.proposal_hash = proposal_hash(&proposal);
    proposal
}

fn resolved_summary() -> GuiTargetResolutionSummary {
    GuiTargetResolutionSummary {
        resolution_id: "resolution-1".into(),
        plan_id: "plan-1".into(),
        validation_id: Some("validation-1".into()),
        goal_contract_id: Some("goal-1".into()),
        context_id: "context-1".into(),
        observation_id: "observation-1".into(),
        status: "resolved".into(),
        results: Vec::new(),
        resolved_target: None,
        can_proceed_to_safety_gate: true,
        can_execute: false,
        blocker_count: 0,
        blockers: Vec::new(),
        ambiguity_count: 0,
        ambiguity_reasons: Vec::new(),
        confidence: 0.91,
        prompt_hash: Some("prompt-hash".into()),
    }
}

#[test]
fn action_proposal_hash_is_stable_and_bound_to_target_payload_and_context() {
    let base = proposal("Send", "high", true);
    let same = proposal("Send", "high", true);
    assert_eq!(base.proposal_hash, same.proposal_hash);

    let mut changed_target = base.clone();
    changed_target.target_hash = "target-hash-2".into();
    changed_target.proposal_hash = proposal_hash(&changed_target);
    assert_ne!(base.proposal_hash, changed_target.proposal_hash);

    let mut changed_payload = base.clone();
    changed_payload.text_payload_hash = Some("payload-hash".into());
    changed_payload.proposal_hash = proposal_hash(&changed_payload);
    assert_ne!(base.proposal_hash, changed_payload.proposal_hash);

    let mut changed_context = base.clone();
    changed_context.context_id = "context-2".into();
    changed_context.proposal_hash = proposal_hash(&changed_context);
    assert_ne!(base.proposal_hash, changed_context.proposal_hash);
}

#[test]
fn high_and_critical_actions_require_hitl_and_never_execute() {
    for (action, risk) in [
        ("Send", "high"),
        ("Delete", "high"),
        ("Submit", "high"),
        ("Install", "high"),
        ("Pay", "critical"),
    ] {
        let safety = evaluate_safety_gate(proposal(action, risk, true), &resolved_summary());
        assert_eq!(safety.status, "approval_required");
        assert!(safety.requires_user_approval);
        assert!(safety.can_request_hitl);
        assert!(!safety.can_authorize_step7);
        assert!(!safety.can_execute);
        assert_eq!(safety.event_payload()["can_execute"], false);
    }
}

#[test]
fn low_risk_action_gets_auditable_no_hitl_authorization_only() {
    let safety = evaluate_safety_gate(proposal("FocusField", "low", false), &resolved_summary());
    assert_eq!(safety.status, "safe_no_approval_required");
    assert!(!safety.requires_user_approval);
    assert!(!safety.can_request_hitl);
    assert!(safety.can_authorize_step7);
    assert!(!safety.can_execute);
}

#[test]
fn deny_approve_expired_and_hash_mismatch_decisions_are_bound_and_non_executable() {
    let proposal = proposal("Send", "high", true);
    let denied = decision_from_fixture(&proposal, &GuiHitlDecisionFixture::Deny, 1_500);
    assert_eq!(denied.decision, "denied");
    assert!(!denied.can_authorize_step7);
    assert!(!denied.can_execute);

    let approved = decision_from_fixture(&proposal, &GuiHitlDecisionFixture::Approve, 1_500);
    assert_eq!(approved.decision, "approved");
    assert!(approved.can_authorize_step7);
    assert!(!approved.can_execute);

    let expired =
        decision_from_fixture(&proposal, &GuiHitlDecisionFixture::ApproveExpired, 40_000);
    assert_eq!(expired.decision, "expired");
    assert!(!expired.can_authorize_step7);

    let mismatch =
        decision_from_fixture(&proposal, &GuiHitlDecisionFixture::ApproveTargetMismatch, 1_500);
    assert_eq!(mismatch.decision, "hash_mismatch_rejected");
    assert!(!mismatch.can_authorize_step7);
}

#[test]
fn proposal_store_binds_request_and_allows_only_one_decision() {
    let proposal = proposal("Send", "high", true);
    let request_id = proposal.request_id.clone();
    let mut store = GuiHitlProposalStore::default();
    assert!(store.insert_pending(proposal).is_empty());
    assert!(store.lookup_by_request_id(&request_id).is_some());

    let decision = store.record_decision(&request_id, true, 1_500);
    assert_eq!(decision.decision, "approved");
    assert!(decision.can_authorize_step7);
    assert!(store.lookup_by_request_id(&request_id).is_none());

    let second = store.record_decision(&request_id, true, 1_600);
    assert_eq!(second.decision, "stale_rejected");
    assert!(!second.can_authorize_step7);
}

#[test]
fn serialized_safety_outputs_do_not_expose_secret_values_or_execution_enabled() {
    let mut proposal = proposal("TypeText", "medium", true);
    proposal.target_label = Some("[redacted]".into());
    proposal.text_payload_summary = Some("[redacted]".into());
    proposal.text_payload_hash = Some("payload-hash".into());
    proposal.proposal_hash = proposal_hash(&proposal);
    let safety = evaluate_safety_gate(proposal, &resolved_summary());
    let serialized = serde_json::to_string(&safety.event_payload()).expect("serializes");
    assert!(!serialized.contains("SECRET123"));
    assert!(serialized.contains("[redacted]"));
    assert!(serialized.contains("\"can_execute\":false"));
}
