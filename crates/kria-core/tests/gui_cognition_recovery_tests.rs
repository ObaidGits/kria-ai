use kria_core::agent::gui_cognition::recovery::{
    assess_recovery, should_attempt_recovery, GuiRecoveryInput, GuiRecoverySignals,
};

fn signals() -> GuiRecoverySignals {
    GuiRecoverySignals {
        backend_success: true,
        verification_status: "verification_failed".into(),
        verification_strategy: "focused_control".into(),
        matched_expected_state: false,
        target_still_present: true,
        target_identity_matches: true,
        modal_present: false,
        active_window_known: true,
        reresolve_candidate_count: 1,
        context_stale: false,
    }
}

fn input(action_type: &str, risk_level: &str, requires_approval: bool) -> GuiRecoveryInput {
    GuiRecoveryInput {
        recovery_id: "recovery-1".into(),
        execution_id: "execution-1".into(),
        verification_id: "verification-1".into(),
        proposal_id: "proposal-1".into(),
        proposal_hash: "proposal-hash".into(),
        target_hash: "target-hash".into(),
        action_type: action_type.into(),
        risk_level: risk_level.into(),
        requires_user_approval: requires_approval,
        hitl_denied: false,
        hitl_stale: false,
        retry_count: 0,
        prompt_hash: "prompt-hash".into(),
        signals: signals(),
    }
}

#[test]
fn verification_verified_skips_recovery() {
    assert!(!should_attempt_recovery("verified"));
    assert!(should_attempt_recovery("verification_failed"));
    assert!(should_attempt_recovery("inconclusive"));
    assert!(should_attempt_recovery("blocked"));
}

#[test]
fn focus_lost_recoverable_refocus_same_target() {
    let a = assess_recovery(&input("FocusField", "low", false));
    assert_eq!(a.failure_kind, "focus_lost");
    assert_eq!(a.status, "recoverable");
    assert_eq!(a.recovery_action_kind, "RefocusSameTarget");
    assert!(a.can_recover);
    assert!(a.can_execute_recovery);
}

#[test]
fn wrong_window_recoverable_switch_back_for_low_risk() {
    let mut i = input("OpenApp", "low", false);
    i.signals.verification_strategy = "active_window_match".into();
    let a = assess_recovery(&i);
    assert_eq!(a.failure_kind, "wrong_window");
    assert_eq!(a.recovery_action_kind, "SwitchBackToWindow");
    assert!(a.can_execute_recovery);
}

#[test]
fn target_missing_blocks_when_not_resolvable() {
    let mut i = input("FocusField", "low", false);
    i.signals.target_still_present = false;
    i.signals.reresolve_candidate_count = 0;
    let a = assess_recovery(&i);
    assert_eq!(a.failure_kind, "target_missing");
    assert_eq!(a.status, "blocked");
    assert!(!a.can_execute_recovery);
    assert_eq!(a.recovery_action_kind, "Stop");
}

#[test]
fn target_ambiguous_asks_clarification() {
    let mut i = input("FocusField", "low", false);
    i.signals.target_still_present = false;
    i.signals.reresolve_candidate_count = 2;
    let a = assess_recovery(&i);
    assert_eq!(a.failure_kind, "target_ambiguous");
    assert_eq!(a.status, "needs_clarification");
    assert_eq!(a.recovery_action_kind, "AskClarification");
    assert!(!a.can_execute_recovery);
}

#[test]
fn target_moved_asks_clarification() {
    let mut i = input("FocusField", "low", false);
    i.signals.target_still_present = true;
    i.signals.target_identity_matches = false;
    let a = assess_recovery(&i);
    assert_eq!(a.failure_kind, "target_moved");
    assert_eq!(a.recovery_action_kind, "AskClarification");
    assert!(!a.can_execute_recovery);
}

#[test]
fn modal_appeared_blocks_and_explains() {
    let mut i = input("FocusField", "low", false);
    i.signals.modal_present = true;
    let a = assess_recovery(&i);
    assert_eq!(a.failure_kind, "modal_appeared");
    assert_eq!(a.status, "blocked");
    assert!(!a.can_execute_recovery);
    assert!(!a.safe_explanation.is_empty());
}

#[test]
fn backend_failed_blocks_without_blind_retry() {
    let mut i = input("FocusField", "low", false);
    i.signals.backend_success = false;
    i.signals.verification_status = "blocked".into();
    let a = assess_recovery(&i);
    assert_eq!(a.failure_kind, "backend_failed");
    assert_eq!(a.status, "blocked");
    assert_eq!(a.recovery_action_kind, "Stop");
    assert!(!a.can_execute_recovery);
}

#[test]
fn high_risk_action_never_auto_recovers() {
    let a = assess_recovery(&input("ClickControl", "high", true));
    assert_eq!(a.failure_kind, "unsafe_to_retry");
    assert_eq!(a.status, "blocked");
    assert!(!a.can_execute_recovery);
    assert!(a.requires_user_approval);
}

#[test]
fn denied_hitl_never_recovers() {
    let mut i = input("FocusField", "low", false);
    i.hitl_denied = true;
    let a = assess_recovery(&i);
    assert_eq!(a.failure_kind, "unsafe_to_retry");
    assert_eq!(a.status, "blocked");
    assert!(!a.can_execute_recovery);
}

#[test]
fn stale_approval_never_recovers() {
    let mut i = input("FocusField", "low", false);
    i.hitl_stale = true;
    let a = assess_recovery(&i);
    assert_eq!(a.failure_kind, "unsafe_to_retry");
    assert_eq!(a.status, "blocked");
    assert!(!a.can_execute_recovery);
}

#[test]
fn retry_count_limited_to_one() {
    let mut i = input("FocusField", "low", false);
    i.retry_count = 1;
    let a = assess_recovery(&i);
    assert_eq!(a.status, "blocked");
    assert!(!a.can_execute_recovery);
    assert_eq!(a.max_retry_count, 1);
}

#[test]
fn inconclusive_verification_reobserves_safely() {
    let mut i = input("ClickControl", "low", false);
    i.signals.verification_status = "inconclusive".into();
    let a = assess_recovery(&i);
    assert_eq!(a.failure_kind, "verification_inconclusive");
    assert_eq!(a.recovery_action_kind, "ReObserve");
    assert!(a.can_execute_recovery);
}

#[test]
fn non_idempotent_click_failure_stops_safely() {
    let mut i = input("ClickControl", "low", false);
    i.signals.verification_strategy = "result_visible".into();
    let a = assess_recovery(&i);
    assert_eq!(a.failure_kind, "verification_failed");
    assert_eq!(a.recovery_action_kind, "Stop");
    assert!(!a.can_execute_recovery);
}

#[test]
fn recovery_assessment_does_not_leak_secrets() {
    let mut i = input("FocusField", "low", false);
    i.prompt_hash = "prompt-hash".into();
    let a = assess_recovery(&i);
    let serialized = serde_json::to_string(&a.summary_json()).unwrap();
    assert!(!serialized.contains("password"));
    assert!(!serialized.contains("SECRET"));
    assert!(!serialized.contains("raw_prompt"));
    // Event payload also carries no raw text.
    let event = serde_json::to_string(&a.event_payload()).unwrap();
    assert!(event.contains("RecoveryAssessmentCompleted"));
    assert!(!event.contains("SECRET"));
}

#[test]
fn recovery_blocked_assessment_is_not_executable() {
    // A blocked assessment must never advertise an executable recovery action,
    // so the runtime emits RecoveryBlocked with no RecoveryActionStarted.
    let a = assess_recovery(&input("ClickControl", "critical", true));
    assert!(!a.can_execute_recovery);
    assert_eq!(a.recovery_action_kind, "Stop");
}

#[test]
fn safe_refocus_recovery_is_executable() {
    let a = assess_recovery(&input("FocusField", "low", false));
    assert!(a.can_execute_recovery);
    assert_eq!(a.recovery_action_kind, "RefocusSameTarget");
}
