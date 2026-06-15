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
        load_failed: false,
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
        safety_polish: false,
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

// ---------------------------------------------------------------------------
// Task 9.5 (Requirements 10, 11, 12, 13, 14, 15, 22, 23): the three hardened
// recovery behaviors under the `gui_cog_safety_polish` flag. Deterministic
// fixtures (no live desktop / display / backend), proving:
//   1. idempotent focus-loss → exactly one refocus retry, then stop on the 2nd
//   2. a NON-idempotent action is NEVER auto-retried (stop + report)
//   3. an unexpected dialog post-action → stop + report (no click-through)
//   4. a load failure → re-observe + explain (no blind retry, no fabrication)
//   5. flag OFF = byte-for-byte unchanged recovery routing
// ---------------------------------------------------------------------------

fn polished(mut i: GuiRecoveryInput) -> GuiRecoveryInput {
    i.safety_polish = true;
    i
}

#[test]
fn idempotent_focus_loss_allows_exactly_one_refocus_under_safety_polish() {
    // Focus moved away from a still-valid, still-present field. FocusField is
    // idempotent, so KRIA may re-focus the SAME target exactly once.
    let a = assess_recovery(&polished(input("FocusField", "low", false)));
    assert_eq!(a.failure_kind, "focus_lost");
    assert_eq!(a.recovery_action_kind, "RefocusSameTarget");
    assert_eq!(a.status, "recoverable");
    assert!(a.can_execute_recovery);
    assert_eq!(a.retry_count, 0);
    assert_eq!(a.max_retry_count, 1);
}

#[test]
fn idempotent_focus_loss_second_attempt_stops_under_safety_polish() {
    // The single bounded retry was already spent: a SECOND focus-loss recovery
    // must stop, never re-retry.
    let mut i = polished(input("FocusField", "low", false));
    i.retry_count = 1;
    let a = assess_recovery(&i);
    assert_eq!(a.status, "blocked");
    assert!(!a.can_execute_recovery);
    assert_eq!(a.recovery_action_kind, "Stop");
    assert_eq!(a.max_retry_count, 1);
}

#[test]
fn non_idempotent_openapp_relaunch_is_blocked_under_safety_polish() {
    // GAP FIX: OpenApp is NOT idempotent (re-launching can spawn a second
    // window). The legacy `idempotent_recoverable` helper wrongly treated it as
    // retryable. Under the flag, recovery is gated on the canonical
    // `default_idempotent_for`, so an OpenApp whose window never matched (and
    // is not an active-window re-switch) stops + reports instead of relaunching.
    let mut i = polished(input("OpenApp", "low", false));
    i.signals.verification_strategy = "result_visible".into();
    let a = assess_recovery(&i);
    assert_eq!(a.recovery_action_kind, "Stop");
    assert_eq!(a.status, "blocked");
    assert!(!a.can_execute_recovery);
}

#[test]
fn non_idempotent_openapp_relaunch_allowed_when_flag_off_unchanged() {
    // Flag OFF: legacy behavior is preserved byte-for-byte — the legacy helper
    // treats OpenApp as retryable, so it retries the idempotent action once.
    let mut i = input("OpenApp", "low", false);
    i.signals.verification_strategy = "result_visible".into();
    let a = assess_recovery(&i);
    assert_eq!(a.recovery_action_kind, "RetryIdempotentAction");
    assert!(a.can_execute_recovery);
}

#[test]
fn non_idempotent_click_failure_stops_under_safety_polish() {
    // A click (TypeText/ClickControl/etc.) is never idempotent: stop + report.
    let mut i = polished(input("ClickControl", "low", false));
    i.signals.verification_strategy = "result_visible".into();
    let a = assess_recovery(&i);
    assert_eq!(a.failure_kind, "verification_failed");
    assert_eq!(a.recovery_action_kind, "Stop");
    assert!(!a.can_execute_recovery);
}

#[test]
fn unexpected_dialog_stops_and_reports_no_click_through() {
    // An unexpected modal/dialog appeared after the action. KRIA must STOP and
    // report it — never click through or auto-dismiss it. Holds with the flag
    // ON and OFF (this is an existing always-on safety gate).
    for i in [
        input("FocusField", "low", false),
        polished(input("FocusField", "low", false)),
    ] {
        let mut i = i;
        i.signals.modal_present = true;
        let a = assess_recovery(&i);
        assert_eq!(a.failure_kind, "modal_appeared");
        assert_eq!(a.status, "blocked");
        assert_eq!(a.recovery_action_kind, "Stop");
        assert!(!a.can_execute_recovery);
        // The recovery action must not be an input-backend action (no click).
        assert_ne!(a.recovery_action_kind, "RetryIdempotentAction");
        assert!(!a.safe_explanation.is_empty());
    }
}

#[test]
fn load_failure_reobserves_and_explains_under_safety_polish() {
    // The expected page/window never became observable (load failure). KRIA
    // re-observes and EXPLAINS — it never blind-retries the open/switch and
    // never fabricates a success.
    let mut i = polished(input("OpenApp", "low", false));
    i.signals.load_failed = true;
    let a = assess_recovery(&i);
    assert_eq!(a.failure_kind, "load_failed");
    assert_eq!(a.recovery_action_kind, "ReObserve");
    assert_eq!(a.status, "needs_reobserve");
    assert!(a.can_execute_recovery);
    // Re-observe never touches the input backend, so no fabricated retry.
    assert!(a.safe_explanation.to_lowercase().contains("load"));
}

#[test]
fn load_failure_signal_is_ignored_when_flag_off_unchanged() {
    // Flag OFF: the load_failed signal is never set by the runtime, and even if
    // present it is ignored, so routing is unchanged (legacy OpenApp retry).
    let mut i = input("OpenApp", "low", false);
    i.signals.load_failed = true;
    i.signals.verification_strategy = "result_visible".into();
    let a = assess_recovery(&i);
    assert_ne!(a.failure_kind, "load_failed");
    assert_eq!(a.recovery_action_kind, "RetryIdempotentAction");
}

#[test]
fn switch_window_remains_idempotent_retry_under_safety_polish() {
    // SwitchWindow IS idempotent per the canonical classification, so a bounded
    // single retry is still allowed under the flag (no over-blocking).
    let mut i = polished(input("SwitchWindow", "low", false));
    i.signals.verification_strategy = "result_visible".into();
    let a = assess_recovery(&i);
    assert_eq!(a.recovery_action_kind, "RetryIdempotentAction");
    assert!(a.can_execute_recovery);
}
