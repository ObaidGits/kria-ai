use kria_core::agent::gui_cognition::checkpoint::{
    build_checkpoint, checkpoint_hash, validate_resume, GuiCheckpointPending,
    GuiResumeObservationSignals, GuiWorkflowResumeRequest,
};
use kria_core::agent::gui_cognition::llm_planner::GuiTypedPlanStep;
use kria_core::agent::gui_cognition::safety_hitl::GuiHitlDecision;
use kria_core::agent::gui_cognition::workflow_runtime::{
    compute_receipt_hash, side_effect_kind_for, GuiWorkflowRun, GuiWorkflowStepReceipt,
};

fn typed_step(step_id: &str, step_type: &str) -> GuiTypedPlanStep {
    serde_json::from_value(serde_json::json!({
        "step_id": step_id,
        "step_type": step_type,
        "summary": format!("{step_type} step"),
        "expected_precondition": "precondition",
        "expected_postcondition": "postcondition",
        "verification_strategy": "screen_changed",
        "risk_level": "low",
        "requires_approval": false,
        "allowed_to_execute": false,
        "confidence": 0.9,
        "reason": "test"
    }))
    .expect("typed step")
}

fn receipt(step_id: &str, index: usize, side_effect: &str) -> GuiWorkflowStepReceipt {
    GuiWorkflowStepReceipt {
        receipt_id: format!("receipt-{index}"),
        workflow_run_id: "workflow-run-1".into(),
        step_id: step_id.into(),
        step_index: index,
        step_type: "ClickControl".into(),
        status: "completed".into(),
        proposal_id: Some(format!("proposal-{index}")),
        action_type: Some("ClickControl".into()),
        risk_level: Some("low".into()),
        side_effect_kind: side_effect.into(),
        target_hash: Some(format!("target-hash-{index}")),
        proposal_hash: Some(format!("proposal-hash-{index}")),
        execution_id: Some(format!("execution-{index}")),
        verification_id: Some(format!("verification-{index}")),
        verification_status: Some("verified".into()),
        recovery_id: None,
        recovery_status: None,
        started_at_ms: 1_000,
        completed_at_ms: 1_100,
        safe_summary: "step done".into(),
        receipt_hash: compute_receipt_hash(
            "workflow-run-1",
            step_id,
            index,
            Some(&format!("proposal-hash-{index}")),
            Some(&format!("execution-{index}")),
            Some("verified"),
        ),
        prompt_hash: "prompt-hash".into(),
    }
}

fn run(risk: &str, requires_approval: bool) -> GuiWorkflowRun {
    let steps = vec![
        typed_step("s0", "OpenApp"),
        typed_step("s1", "FocusField"),
        typed_step("s2", "ClickControl"),
    ];
    let mut run = GuiWorkflowRun::new(
        "session-1",
        "workflow-1",
        "turn-1",
        "goal-1",
        "plan-1",
        "context-1",
        &steps,
        risk,
        requires_approval,
        "execute_fixture",
        "prompt-hash",
    );
    run.current_step_index = 1;
    run
}

fn pending() -> GuiCheckpointPending {
    GuiCheckpointPending {
        pending_step_id: Some("s1".into()),
        pending_proposal_id: Some("proposal-1".into()),
        pending_proposal_hash: Some("proposal-hash-1".into()),
        pending_target_hash: Some("target-hash-1".into()),
        pending_stable_target_identity_hash: Some("identity-1".into()),
        pending_hitl_request_id: Some("request-1".into()),
        approved_decision_id: None,
        approved_decision_hash: None,
    }
}

fn checkpoint(risk: &str, requires_approval: bool) -> kria_core::agent::gui_cognition::checkpoint::GuiWorkflowCheckpoint {
    build_checkpoint(
        &run(risk, requires_approval),
        &pending(),
        "obs-1",
        "context-1",
        Some("screenAA".into()),
        Some("windowAA".into()),
        10_000,
        60_000,
    )
}

fn resume_request_for(
    cp: &kria_core::agent::gui_cognition::checkpoint::GuiWorkflowCheckpoint,
) -> GuiWorkflowResumeRequest {
    GuiWorkflowResumeRequest {
        resume_id: "resume-1".into(),
        checkpoint_id: cp.checkpoint_id.clone(),
        workflow_run_id: cp.workflow_run_id.clone(),
        session_id: cp.session_id.clone(),
        requested_at_ms: 20_000,
        current_observation_id: "obs-2".into(),
        current_context_id: "context-2".into(),
        current_screen_hash_prefix: Some("screenAA".into()),
        reason: "user_resume".into(),
        prompt_hash: "prompt-hash".into(),
    }
}

fn signals_ok() -> GuiResumeObservationSignals {
    GuiResumeObservationSignals {
        current_screen_hash_prefix: Some("screenAA".into()),
        current_active_window_hash: Some("windowAA".into()),
        pending_target_still_present: true,
        pending_target_identity_matches: true,
    }
}

fn decision(decision: &str, proposal_hash: &str, target_hash: &str, authorize: bool) -> GuiHitlDecision {
    GuiHitlDecision {
        decision_id: "decision-1".into(),
        request_id: "request-1".into(),
        proposal_id: "proposal-1".into(),
        proposal_hash: proposal_hash.into(),
        target_hash: target_hash.into(),
        decision: decision.into(),
        decided_at_ms: 19_000,
        decision_reason: None,
        actor: "local_user".into(),
        user_visible_summary_hash: "summary".into(),
        can_authorize_step7: authorize,
        can_execute: false,
    }
}

// The pending step (s1) must NOT have a completed receipt for resume tests; the
// run() helper leaves receipts empty by default.

#[test]
fn checkpoint_serializes_without_raw_prompt_or_secrets() {
    let cp = checkpoint("low", false);
    let serialized = serde_json::to_string(&cp.summary_json()).unwrap();
    assert!(!serialized.contains("raw_prompt"));
    assert!(!serialized.contains("password"));
    assert!(!serialized.contains("screenshot"));
    assert_eq!(cp.can_execute, false);
    assert!(cp.can_resume);
}

#[test]
fn checkpoint_hash_stable_for_same_safe_state() {
    let cp = checkpoint("low", false);
    assert_eq!(cp.checkpoint_hash, checkpoint_hash(&cp));
}

#[test]
fn checkpoint_hash_changes_on_step_receipt_change() {
    let cp = checkpoint("low", false);
    let mut tampered = cp.clone();
    tampered
        .completed_step_receipts
        .push(receipt("s0", 0, "local_ui"));
    assert_ne!(cp.checkpoint_hash, checkpoint_hash(&tampered));
}

#[test]
fn checkpoint_saves_completed_step_receipt() {
    let mut run = run("low", false);
    run.completed_step_receipts.push(receipt("s0", 0, "local_ui"));
    let cp = build_checkpoint(&run, &pending(), "obs-1", "context-1", None, None, 10_000, 60_000);
    assert_eq!(cp.completed_step_receipts.len(), 1);
    assert_eq!(cp.completed_step_receipts[0].step_id, "s0");
}

#[test]
fn side_effect_classification_flags_risky_actions() {
    assert_eq!(side_effect_kind_for("Submit", "high"), "external_submit");
    assert_eq!(side_effect_kind_for("Delete", "low"), "destructive");
    assert_eq!(side_effect_kind_for("Pay", "low"), "payment");
    assert_eq!(side_effect_kind_for("Install", "low"), "install_system");
    assert_eq!(side_effect_kind_for("ClickControl", "low"), "local_ui");
}

#[test]
fn resume_rejects_integrity_mismatch() {
    let cp = checkpoint("low", false);
    let result = validate_resume(&cp, &resume_request_for(&cp), &signals_ok(), "wrong-hash", None, 20_000);
    assert_eq!(result.status, "blocked");
    assert!(!result.can_continue_workflow);
}

#[test]
fn resume_rejects_expired_checkpoint() {
    let cp = checkpoint("low", false);
    let result = validate_resume(&cp, &resume_request_for(&cp), &signals_ok(), &cp.checkpoint_hash, None, 999_999);
    assert_eq!(result.status, "stale_rejected");
}

#[test]
fn resume_rejects_context_mismatch() {
    let cp = checkpoint("low", false);
    let mut signals = signals_ok();
    signals.current_screen_hash_prefix = Some("screenZZ".into());
    let result = validate_resume(&cp, &resume_request_for(&cp), &signals, &cp.checkpoint_hash, None, 20_000);
    assert_eq!(result.status, "needs_reobserve");
    assert!(!result.can_continue_workflow);
}

#[test]
fn resume_rejects_target_hash_mismatch() {
    let cp = checkpoint("low", false);
    let mut signals = signals_ok();
    signals.pending_target_still_present = false;
    let result = validate_resume(&cp, &resume_request_for(&cp), &signals, &cp.checkpoint_hash, None, 20_000);
    assert_eq!(result.status, "target_mismatch_rejected");
}

#[test]
fn resume_rejects_stable_identity_mismatch() {
    let cp = checkpoint("low", false);
    let mut signals = signals_ok();
    signals.pending_target_identity_matches = false;
    let result = validate_resume(&cp, &resume_request_for(&cp), &signals, &cp.checkpoint_hash, None, 20_000);
    assert_eq!(result.status, "target_mismatch_rejected");
}

#[test]
fn resume_invalidates_hash_mismatch_approval() {
    let cp = checkpoint("high", true);
    let decision = decision("approved", "WRONG-HASH", "target-hash-1", true);
    let result = validate_resume(&cp, &resume_request_for(&cp), &signals_ok(), &cp.checkpoint_hash, Some(&decision), 20_000);
    assert_eq!(result.status, "approval_invalidated");
    assert!(!result.invalidated_approvals.is_empty());
}

#[test]
fn resume_invalidates_approval_after_screen_change() {
    let cp = checkpoint("high", true);
    let mut signals = signals_ok();
    signals.current_screen_hash_prefix = Some("screenZZ".into());
    // No decision yet, but the screen changed: a pending approval is invalidated.
    let result = validate_resume(&cp, &resume_request_for(&cp), &signals, &cp.checkpoint_hash, None, 20_000);
    assert_eq!(result.status, "approval_invalidated");
}

#[test]
fn resume_accepts_fresh_matching_approval() {
    let cp = checkpoint("high", true);
    let decision = decision("approved", "proposal-hash-1", "target-hash-1", true);
    let result = validate_resume(&cp, &resume_request_for(&cp), &signals_ok(), &cp.checkpoint_hash, Some(&decision), 20_000);
    assert_eq!(result.status, "resumed");
    assert!(result.can_continue_workflow);
    assert_eq!(result.next_step_id.as_deref(), Some("s1"));
}

#[test]
fn denied_approval_blocks_resume() {
    let cp = checkpoint("high", true);
    let decision = decision("denied", "proposal-hash-1", "target-hash-1", false);
    let result = validate_resume(&cp, &resume_request_for(&cp), &signals_ok(), &cp.checkpoint_hash, Some(&decision), 20_000);
    assert_eq!(result.status, "blocked");
    assert!(!result.can_continue_workflow);
}

#[test]
fn pending_safe_step_resumes_after_reobserve() {
    let cp = checkpoint("low", false);
    let result = validate_resume(&cp, &resume_request_for(&cp), &signals_ok(), &cp.checkpoint_hash, None, 20_000);
    assert_eq!(result.status, "resumed");
    assert!(result.can_continue_workflow);
    assert_eq!(result.next_step_index, 1);
}

#[test]
fn pending_risky_step_requires_fresh_hitl() {
    let cp = checkpoint("high", true);
    let result = validate_resume(&cp, &resume_request_for(&cp), &signals_ok(), &cp.checkpoint_hash, None, 20_000);
    assert_eq!(result.status, "needs_approval");
    assert!(!result.can_continue_workflow);
}

#[test]
fn completed_risky_step_not_replayed() {
    let mut run = run("high", true);
    // The pending step itself is already completed as an external submit.
    run.completed_step_receipts.push(receipt("s1", 1, "external_submit"));
    let cp = build_checkpoint(&run, &pending(), "obs-1", "context-1", Some("screenAA".into()), None, 10_000, 60_000);
    let decision = decision("approved", "proposal-hash-1", "target-hash-1", true);
    let result = validate_resume(&cp, &resume_request_for(&cp), &signals_ok(), &cp.checkpoint_hash, Some(&decision), 20_000);
    assert_eq!(result.status, "duplicate_action_blocked");
    assert!(!result.duplicate_action_guards.is_empty());
    assert!(!result.can_continue_workflow);
}

#[test]
fn checkpoint_can_continue_only_to_next_incomplete_step() {
    let cp = checkpoint("low", false);
    let result = validate_resume(&cp, &resume_request_for(&cp), &signals_ok(), &cp.checkpoint_hash, None, 20_000);
    assert_eq!(result.next_step_index, cp.current_step_index);
    assert_eq!(result.next_step_id, cp.pending_step_id);
}
