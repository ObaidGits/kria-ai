use kria_core::agent::gui_cognition::executor::{
    build_execution_request_from_proposal, validate_execution_preconditions, GuiActionBackendStatus,
    GuiExecutionAuthorizationSource, GuiExecutionMode, GuiPayloadVault,
};
use kria_core::agent::gui_cognition::perception::GuiBounds;
use kria_core::agent::gui_cognition::resolver::{
    GuiResolvedTarget, GuiTargetResolutionSummary,
};
use kria_core::agent::gui_cognition::safety_hitl::{
    decision_for, proposal_hash, GuiActionProposal,
};

fn target() -> GuiResolvedTarget {
    GuiResolvedTarget {
        control_id: "control-search".into(),
        target_hash: "target-hash-1".into(),
        label: "Search".into(),
        role: "push button".into(),
        target_kind: "button".into(),
        app_hint: Some("Chrome".into()),
        window_hint: Some("Chrome".into()),
        bounds: Some(GuiBounds {
            x: 10,
            y: 20,
            width: 80,
            height: 30,
        }),
        enabled: true,
        visible: true,
        focused: false,
        source: "accessibility".into(),
    }
}

fn resolution(target: GuiResolvedTarget) -> GuiTargetResolutionSummary {
    GuiTargetResolutionSummary {
        resolution_id: "resolution-1".into(),
        plan_id: "plan-1".into(),
        validation_id: Some("validation-1".into()),
        goal_contract_id: Some("goal-1".into()),
        context_id: "context-1".into(),
        observation_id: "observation-1".into(),
        status: "resolved".into(),
        results: Vec::new(),
        resolved_target: Some(target),
        can_proceed_to_safety_gate: true,
        can_execute: false,
        blocker_count: 0,
        blockers: Vec::new(),
        ambiguity_count: 0,
        ambiguity_reasons: Vec::new(),
        confidence: 0.92,
        prompt_hash: Some("prompt-hash".into()),
    }
}

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
        target_control_id: Some("control-search".into()),
        target_label: Some("Search".into()),
        target_role: Some("push button".into()),
        target_bounds: Some(GuiBounds {
            x: 10,
            y: 20,
            width: 80,
            height: 30,
        }),
        text_payload_summary: None,
        text_payload_hash: None,
        expected_precondition: "Chrome".into(),
        expected_postcondition: "result visible".into(),
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

#[test]
fn authorized_low_risk_proposal_passes_preconditions() {
    let proposal = proposal("ClickControl", "low", false);
    let resolved = resolution(target());
    let mut vault = GuiPayloadVault::default();
    let request = build_execution_request_from_proposal(
        &proposal,
        &resolved,
        GuiExecutionAuthorizationSource::SafeNoApprovalRequired,
        None,
        &mut vault,
        1_500,
    );

    let report = validate_execution_preconditions(
        GuiExecutionMode::ExecuteFixture,
        &request,
        &proposal,
        &resolved,
        &GuiActionBackendStatus::available("fixture_executor"),
        None,
        &vault,
        1_500,
    );

    assert!(report.can_start_action);
    assert!(report.blockers.is_empty());
    let serialized = serde_json::to_string(&request.summary_json()).unwrap();
    assert!(!serialized.contains("raw_prompt"));
}

#[test]
fn safety_only_backend_block_and_hash_mismatch_block_before_action() {
    let proposal = proposal("ClickControl", "low", false);
    let resolved = resolution(target());
    let mut vault = GuiPayloadVault::default();
    let mut request = build_execution_request_from_proposal(
        &proposal,
        &resolved,
        GuiExecutionAuthorizationSource::SafeNoApprovalRequired,
        None,
        &mut vault,
        1_500,
    );

    let safety_only = validate_execution_preconditions(
        GuiExecutionMode::SafetyOnly,
        &request,
        &proposal,
        &resolved,
        &GuiActionBackendStatus::available("fixture_executor"),
        None,
        &vault,
        1_500,
    );
    assert!(!safety_only.can_start_action);
    assert!(safety_only.blockers.iter().any(|value| value.contains("safety_only")));

    request.proposal_hash = "wrong".into();
    let mismatch = validate_execution_preconditions(
        GuiExecutionMode::ExecuteFixture,
        &request,
        &proposal,
        &resolved,
        &GuiActionBackendStatus::available("fixture_executor"),
        None,
        &vault,
        1_500,
    );
    assert!(!mismatch.can_start_action);
    assert!(mismatch
        .blockers
        .iter()
        .any(|value| value.contains("proposal_hash mismatch")));

    let backend_blocked = validate_execution_preconditions(
        GuiExecutionMode::ExecuteFixture,
        &build_execution_request_from_proposal(
            &proposal,
            &resolved,
            GuiExecutionAuthorizationSource::SafeNoApprovalRequired,
            None,
            &mut GuiPayloadVault::default(),
            1_500,
        ),
        &proposal,
        &resolved,
        &GuiActionBackendStatus::blocked("blocked", "global safety halt is engaged", "wayland"),
        None,
        &GuiPayloadVault::default(),
        1_500,
    );
    assert!(!backend_blocked.can_start_action);
    assert!(backend_blocked
        .blockers
        .iter()
        .any(|value| value.contains("global safety halt")));
}

#[test]
fn risky_action_requires_matching_hitl_decision() {
    let proposal = proposal("ClickControl", "high", true);
    let resolved = resolution(target());
    let mut vault = GuiPayloadVault::default();
    let request = build_execution_request_from_proposal(
        &proposal,
        &resolved,
        GuiExecutionAuthorizationSource::HitlApproved,
        None,
        &mut vault,
        1_500,
    );
    let denied = decision_for(&proposal, "denied", 1_500, Some("denied"));

    let denied_report = validate_execution_preconditions(
        GuiExecutionMode::ExecuteFixture,
        &request,
        &proposal,
        &resolved,
        &GuiActionBackendStatus::available("fixture_executor"),
        Some(&denied),
        &vault,
        1_500,
    );
    assert!(!denied_report.can_start_action);

    let approved = decision_for(&proposal, "approved", 1_500, None);
    let request = build_execution_request_from_proposal(
        &proposal,
        &resolved,
        GuiExecutionAuthorizationSource::HitlApproved,
        Some(approved.decision_id.clone()),
        &mut GuiPayloadVault::default(),
        1_500,
    );
    let approved_report = validate_execution_preconditions(
        GuiExecutionMode::ExecuteFixture,
        &request,
        &proposal,
        &resolved,
        &GuiActionBackendStatus::available("fixture_executor"),
        Some(&approved),
        &GuiPayloadVault::default(),
        1_500,
    );
    assert!(approved_report.can_start_action);
}

#[test]
fn target_identity_payload_and_secret_outputs_are_enforced() {
    let mut proposal = proposal("TypeText", "medium", false);
    proposal.text_payload_summary = Some("KRIA".into());
    proposal.text_payload_hash = Some("payload-hash".into());
    proposal.proposal_hash = proposal_hash(&proposal);
    let resolved = resolution(target());
    let mut vault = GuiPayloadVault::default();
    let request = build_execution_request_from_proposal(
        &proposal,
        &resolved,
        GuiExecutionAuthorizationSource::SafeNoApprovalRequired,
        None,
        &mut vault,
        1_500,
    );
    assert!(request.text_payload_handle.is_some());
    let report = validate_execution_preconditions(
        GuiExecutionMode::ExecuteFixture,
        &request,
        &proposal,
        &resolved,
        &GuiActionBackendStatus::available("fixture_executor"),
        None,
        &vault,
        1_500,
    );
    assert!(report.can_start_action);

    let mut changed_target = target();
    changed_target.control_id = "other-control".into();
    let changed_resolution = resolution(changed_target);
    let mismatch = validate_execution_preconditions(
        GuiExecutionMode::ExecuteFixture,
        &request,
        &proposal,
        &changed_resolution,
        &GuiActionBackendStatus::available("fixture_executor"),
        None,
        &vault,
        1_500,
    );
    assert!(!mismatch.can_start_action);
    assert!(mismatch
        .blockers
        .iter()
        .any(|value| value.contains("stable target identity mismatch")));

    let mut secret = proposal.clone();
    secret.text_payload_summary = Some("[redacted]".into());
    secret.proposal_hash = proposal_hash(&secret);
    let secret_request = build_execution_request_from_proposal(
        &secret,
        &resolved,
        GuiExecutionAuthorizationSource::SafeNoApprovalRequired,
        None,
        &mut GuiPayloadVault::default(),
        1_500,
    );
    assert!(secret_request.text_payload_handle.is_none());
    let serialized = serde_json::to_string(&secret_request.summary_json()).unwrap();
    assert!(!serialized.contains("SECRET123"));
}
