//! Task 9.4 (Requirements 10, 11, 12, 13, 14, 15, 22, 23): ambiguity → ask
//! (never guess), boundaries strictly respected, and verify-and-stop terminates
//! after verification.
//!
//! This module HARDENS and PROVES three KRIA runtime-authority invariants and
//! makes each one inspectable via ADDITIVE telemetry. Every function here is
//! pure (no I/O, no display, no network) so the behaviors are CI-safe and
//! deterministic. The runtime calls these ONLY when the `gui_cog_safety_polish`
//! flag is ON (Task 9, default OFF until the wave gate 9.7). While the flag is
//! OFF none of this code runs and the turn is byte-for-byte unchanged.
//!
//! The three behaviors:
//!
//! 1. AMBIGUITY → ASK, NEVER GUESS — when target resolution finds multiple
//!    candidates (ambiguous) or the goal is under-specified, the runtime PAUSES
//!    and asks for clarification. It NEVER picks one candidate by guessing. The
//!    [`ambiguity_no_guess_event`] telemetry makes the decision inspectable
//!    (candidate count + reason + the explicit `no_guess` flag + decision
//!    point).
//!
//! 2. BOUNDARIES STRICTLY RESPECTED — an action outside the requested scope (a
//!    destructive verb in a non-destructive task, a target not named/observed,
//!    an out-of-scope app) is refused/blocked. [`assess_action_boundary`]
//!    deterministically classifies whether a proposed action stays within the
//!    requested capability boundary; [`boundary_check_event`] records the
//!    decision (and, on a crossing, the refusal).
//!
//! 3. VERIFY-AND-STOP TERMINATES AFTER VERIFICATION — a verify-and-stop intent
//!    (Requirement 13) observes → verifies the requested condition → then STOPS
//!    with NO further action. [`is_verify_and_stop_plan`] recognizes the
//!    Observe→VerifyState terminal shape and [`verify_and_stop_event`] asserts
//!    the turn ended after verification with zero state-changing actions
//!    executed.

use super::workflow_runtime::{side_effect_is_risky, side_effect_kind_for};

/// Where in the turn an ambiguity-no-guess pause was decided. Used only for the
/// additive telemetry so the decision is inspectable; never alters control flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiAmbiguityDecisionPoint {
    /// Plan validation surfaced an under-specified / clarification-needed plan
    /// before any execution.
    PlanValidation,
    /// A plan step explicitly requires clarification before continuing.
    WorkflowClarification,
    /// Per-step re-observe found the expected target present but with multiple
    /// matches (Task 3.4) → pause + ask.
    PerStepReobserve,
}

impl GuiAmbiguityDecisionPoint {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlanValidation => "plan_validation",
            Self::WorkflowClarification => "workflow_clarification",
            Self::PerStepReobserve => "per_step_reobserve",
        }
    }
}

/// Build the additive `AmbiguityNoGuess` telemetry event (Task 9.4, flag-ON
/// only).
///
/// KRIA never resolves an ambiguous/under-specified target by guessing: it
/// pauses and asks. This event makes that decision inspectable — the candidate
/// count that triggered the pause, the sanitized reason, the decision point,
/// and the explicit `no_guess: true` flag. It carries no raw prompt, secret, or
/// coordinates and never alters control flow (purely observational).
pub fn ambiguity_no_guess_event(
    candidate_count: usize,
    reason: &str,
    decision_point: GuiAmbiguityDecisionPoint,
    clarification_question: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "type": "AmbiguityNoGuess",
        "decision": "ask",
        "no_guess": true,
        "candidate_count": candidate_count,
        "reason": sanitize(reason),
        "decision_point": decision_point.as_str(),
        "clarification_question": clarification_question.map(sanitize),
        "can_execute": false,
    })
}

/// The kind of capability-boundary crossing detected for a proposed action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiBoundaryCrossing {
    /// The proposed action has a destructive / external-submit side effect
    /// (delete / send / pay / submit / purchase) that the request neither asked
    /// for nor approval-gated — KRIA would escalate beyond the requested scope.
    DestructiveBeyondScope,
    /// The proposed action targets an app/window different from the one the
    /// request named — KRIA would act on an out-of-scope app.
    OutOfScopeApp,
    /// The proposed action would act on a target that was never uniquely
    /// resolved from real observation — KRIA would act on an unobserved target.
    UnobservedTarget,
}

impl GuiBoundaryCrossing {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DestructiveBeyondScope => "destructive_beyond_scope",
            Self::OutOfScopeApp => "out_of_scope_app",
            Self::UnobservedTarget => "unobserved_target",
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::DestructiveBeyondScope => {
                "Proposed action has a destructive side effect the request did not authorize."
            }
            Self::OutOfScopeApp => {
                "Proposed action targets an app/window outside the requested scope."
            }
            Self::UnobservedTarget => {
                "Proposed action would act on a target that was not uniquely observed."
            }
        }
    }
}

/// The inputs needed to assess whether a proposed action stays within the
/// requested capability boundary. All fields are sanitized descriptors — never
/// raw secrets, prompts, or coordinates.
#[derive(Debug, Clone)]
pub struct GuiBoundaryInput<'a> {
    /// The requested action type from the goal contract.
    pub requested_action_type: &'a str,
    /// The requested risk level from the goal contract.
    pub requested_risk_level: &'a str,
    /// Whether the request explicitly authorized an approval-gated action. When
    /// true the user is in the loop, so a destructive action is in-scope (it
    /// still flows through the HITL gate).
    pub requested_approval: bool,
    /// The app the request named, if any (lowercased comparison).
    pub requested_app: Option<&'a str>,
    /// The proposed action type from the bound proposal.
    pub proposed_action_type: &'a str,
    /// The proposed action's risk level.
    pub proposed_risk_level: &'a str,
    /// The app the proposed/resolved target belongs to, if known.
    pub proposed_app: Option<&'a str>,
    /// Whether the proposed action requires a target (most executable actions do;
    /// pure Observe/VerifyState do not).
    pub requires_target: bool,
    /// Whether the proposed target was uniquely resolved from real observation.
    pub target_resolved: bool,
}

/// The outcome of a capability-boundary assessment (Task 9.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiBoundaryAssessment {
    pub within_bounds: bool,
    pub crossing: Option<GuiBoundaryCrossing>,
    pub reason: Option<String>,
    pub requested_action_type: String,
    pub proposed_action_type: String,
}

impl GuiBoundaryAssessment {
    /// Whether the action must be refused because it crosses the boundary.
    pub fn must_refuse(&self) -> bool {
        !self.within_bounds
    }
}

/// Whether `action_type` classifies as a destructive / external-submit side
/// effect (delete / send / pay / submit / purchase / git). Destructiveness here
/// is driven by the action TYPE, NOT the risk level: a risk-escalated but
/// in-scope action (e.g. a click the safety system flags as risky) is handled by
/// the approval gate, not refused as a boundary crossing. Reuses the existing
/// Step 11 side-effect classifier (with a neutral risk level) so the boundary
/// definition stays consistent with the duplicate-risky-action guard.
fn is_destructive_side_effect(action_type: &str, _risk_level: &str) -> bool {
    side_effect_is_risky(side_effect_kind_for(action_type, "low"))
}

/// Case-insensitive app-scope match: two app descriptors are considered the same
/// app when either contains the other (handles "Chrome" vs "Google Chrome").
fn apps_match(requested: &str, proposed: &str) -> bool {
    let requested = requested.trim().to_ascii_lowercase();
    let proposed = proposed.trim().to_ascii_lowercase();
    if requested.is_empty() || proposed.is_empty() {
        return true;
    }
    requested.contains(&proposed) || proposed.contains(&requested)
}

/// Deterministically assess whether a proposed action stays within the requested
/// capability boundary (Task 9.4, Requirements 11, 12, 15, 22).
///
/// KRIA is capability-first and bounded: it must NEVER perform an action outside
/// the requested scope. This classifies three out-of-scope crossings — a
/// destructive verb beyond a non-destructive request, an out-of-scope app, and
/// an unobserved target — and otherwise reports the action as within bounds.
/// The check is conservative: when the request itself is destructive or
/// approval-gated, a destructive proposal is in-scope (the user asked / will be
/// asked). Pure function: no I/O, no side effects.
pub fn assess_action_boundary(input: &GuiBoundaryInput<'_>) -> GuiBoundaryAssessment {
    let requested_action_type = input.requested_action_type.to_string();
    let proposed_action_type = input.proposed_action_type.to_string();

    let request_authorizes_destructive = input.requested_approval
        || is_destructive_side_effect(input.requested_action_type, input.requested_risk_level);
    let proposal_is_destructive =
        is_destructive_side_effect(input.proposed_action_type, input.proposed_risk_level);

    // 1. A destructive action the request neither asked for nor approval-gated.
    if proposal_is_destructive && !request_authorizes_destructive {
        return crossing(
            GuiBoundaryCrossing::DestructiveBeyondScope,
            requested_action_type,
            proposed_action_type,
        );
    }

    // 2. The proposed target belongs to an app the request did not name.
    if let (Some(requested_app), Some(proposed_app)) = (input.requested_app, input.proposed_app) {
        if !apps_match(requested_app, proposed_app) {
            return crossing(
                GuiBoundaryCrossing::OutOfScopeApp,
                requested_action_type,
                proposed_action_type,
            );
        }
    }

    // 3. An action that needs a target but has no uniquely observed one.
    if input.requires_target && !input.target_resolved {
        return crossing(
            GuiBoundaryCrossing::UnobservedTarget,
            requested_action_type,
            proposed_action_type,
        );
    }

    GuiBoundaryAssessment {
        within_bounds: true,
        crossing: None,
        reason: None,
        requested_action_type,
        proposed_action_type,
    }
}

fn crossing(
    kind: GuiBoundaryCrossing,
    requested_action_type: String,
    proposed_action_type: String,
) -> GuiBoundaryAssessment {
    GuiBoundaryAssessment {
        within_bounds: false,
        crossing: Some(kind),
        reason: Some(kind.reason().to_string()),
        requested_action_type,
        proposed_action_type,
    }
}

/// Build the additive `BoundaryCheck` telemetry event (Task 9.4, flag-ON only).
///
/// Records the boundary decision for a proposed action so it is inspectable: the
/// requested vs proposed action type, whether the action stayed within the
/// requested capability boundary, and — on a crossing — the crossing kind, the
/// sanitized reason, and that the action was refused. Carries no raw prompt,
/// secret, or coordinates.
pub fn boundary_check_event(assessment: &GuiBoundaryAssessment) -> serde_json::Value {
    serde_json::json!({
        "type": "BoundaryCheck",
        "within_bounds": assessment.within_bounds,
        "refused": assessment.must_refuse(),
        "crossing_kind": assessment.crossing.map(GuiBoundaryCrossing::as_str),
        "reason": assessment.reason.as_deref().map(sanitize),
        "requested_action_type": assessment.requested_action_type,
        "proposed_action_type": assessment.proposed_action_type,
        "can_execute": false,
    })
}

/// Whether a plan's typed step types form a verify-and-stop plan (Requirement
/// 13): one or more non-state-changing observe/verify steps whose terminal step
/// is a `VerifyState`, and which contains NO state-changing (executable) step.
///
/// This recognizes the Observe→VerifyState contract built by
/// `verify_and_stop_steps` (Task 2.4) without assuming an exact length, so a
/// planner that emits extra leading Observe/WaitForState steps still qualifies
/// as long as nothing changes state and the terminal step verifies.
pub fn is_verify_and_stop_plan(step_types: &[String]) -> bool {
    let Some(last) = step_types.last() else {
        return false;
    };
    if last != "VerifyState" {
        return false;
    }
    step_types.iter().all(|step_type| is_observe_or_verify(step_type))
}

/// Whether a step type is a non-state-changing observe/verify step. These never
/// call the executor (no action is started), so a plan composed only of these
/// cannot change GUI state.
fn is_observe_or_verify(step_type: &str) -> bool {
    matches!(
        step_type,
        "Observe" | "WaitForState" | "VerifyState" | "SummarizeVisibleContent"
    )
}

/// Build the additive `VerifyAndStopTerminated` telemetry event (Task 9.4,
/// Requirement 13, flag-ON only).
///
/// Asserts that a verify-and-stop turn observed → verified the requested
/// condition → then STOPPED with no further action. It surfaces the count of
/// state-changing actions executed during the turn (which MUST be zero for a
/// verify-and-stop plan) and the terminal step type, so the "stop after
/// verification" contract is inspectable. Purely observational.
pub fn verify_and_stop_event(
    state_changing_actions_executed: usize,
    terminal_step_type: &str,
    run_status: &str,
) -> serde_json::Value {
    serde_json::json!({
        "type": "VerifyAndStopTerminated",
        "verified_then_stopped": state_changing_actions_executed == 0,
        "state_changing_actions_executed": state_changing_actions_executed,
        "terminal_step_type": terminal_step_type,
        "run_status": run_status,
        "can_execute": false,
    })
}

/// Build the additive `RecoveryDecision` telemetry event (Task 9.5,
/// Requirements 10, 11, 12, 13, 14, 15, 22, 23, flag-ON only).
///
/// Makes each recovery decision inspectable so the three hardened recovery
/// behaviors can be proven from the event stream alone:
///
/// - `idempotent_gated` — the canonical idempotency classification of the
///   failed action ([`default_idempotent_for`](super::default_idempotent_for)).
///   Any input-backend retry is gated on this; a non-idempotent action is never
///   auto-retried.
/// - `single_retry_respected` — the bounded single-retry rule: `retry_count`
///   never exceeds `max_retry_count` (== 1).
/// - `unexpected_dialog_stop` — an unexpected modal/dialog appeared post-action,
///   so KRIA stops and reports it (never clicks through).
/// - `load_failure_explain` — the expected page/window did not load, so KRIA
///   re-observes and explains (never blind-retries / fabricates success).
///
/// Carries no raw prompt, secret, or coordinates and never alters control flow
/// (purely observational). The runtime emits it ONLY when the
/// `gui_cog_safety_polish` flag is ON; while OFF the event is never produced and
/// the turn is byte-for-byte unchanged.
#[allow(clippy::too_many_arguments)]
pub fn recovery_decision_event(
    recovery_action_kind: &str,
    failure_kind: &str,
    status: &str,
    idempotent_gated: bool,
    can_execute_recovery: bool,
    retry_count: u32,
    max_retry_count: u32,
) -> serde_json::Value {
    serde_json::json!({
        "type": "RecoveryDecision",
        "recovery_action_kind": recovery_action_kind,
        "failure_kind": failure_kind,
        "status": status,
        "idempotent_gated": idempotent_gated,
        "retry_count": retry_count,
        "max_retry_count": max_retry_count,
        "single_retry_respected": retry_count <= max_retry_count && max_retry_count <= 1,
        "unexpected_dialog_stop": failure_kind == "modal_appeared",
        "load_failure_explain": failure_kind == "load_failed",
        "can_execute_recovery": can_execute_recovery,
        "can_execute": false,
    })
}

/// Compact, length-bounded sanitization for telemetry text (mirrors the
/// runtime's `sanitize_event_text`): strips newlines, collapses whitespace, and
/// caps length so a raw payload can never leak through telemetry.
fn sanitize(value: &str) -> String {
    value
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(180)
        .collect()
}

#[cfg(test)]
mod tests {
    //! Task 9.4 CI-safe unit tests: ambiguity-no-guess telemetry, the
    //! capability-boundary classifier + refusal, and the verify-and-stop
    //! terminal recognition. No live desktop, display, or backend required.

    use super::*;

    fn base_input<'a>() -> GuiBoundaryInput<'a> {
        GuiBoundaryInput {
            requested_action_type: "click_control",
            requested_risk_level: "low",
            requested_approval: false,
            requested_app: Some("Text Editor"),
            proposed_action_type: "click_control",
            proposed_risk_level: "low",
            proposed_app: Some("Text Editor"),
            requires_target: true,
            target_resolved: true,
        }
    }

    #[test]
    fn within_bounds_for_matching_non_destructive_action() {
        let assessment = assess_action_boundary(&base_input());
        assert!(assessment.within_bounds);
        assert!(!assessment.must_refuse());
        assert!(assessment.crossing.is_none());
    }

    #[test]
    fn refuses_destructive_action_beyond_non_destructive_request() {
        let mut input = base_input();
        // The request was a benign click; the proposal would delete/submit.
        input.proposed_action_type = "submit_form";
        input.proposed_risk_level = "high";
        let assessment = assess_action_boundary(&input);
        assert!(assessment.must_refuse());
        assert_eq!(
            assessment.crossing,
            Some(GuiBoundaryCrossing::DestructiveBeyondScope)
        );
        let event = boundary_check_event(&assessment);
        assert_eq!(event["within_bounds"], false);
        assert_eq!(event["refused"], true);
        assert_eq!(event["crossing_kind"], "destructive_beyond_scope");
    }

    #[test]
    fn destructive_action_is_in_scope_when_request_is_approval_gated() {
        let mut input = base_input();
        input.requested_approval = true;
        input.proposed_action_type = "submit_form";
        input.proposed_risk_level = "high";
        let assessment = assess_action_boundary(&input);
        // The user is in the loop (approval-gated), so it is NOT a boundary
        // crossing — it still flows through the HITL gate.
        assert!(assessment.within_bounds, "{assessment:?}");
    }

    #[test]
    fn refuses_out_of_scope_app() {
        let mut input = base_input();
        input.requested_app = Some("Text Editor");
        input.proposed_app = Some("Online Banking");
        let assessment = assess_action_boundary(&input);
        assert!(assessment.must_refuse());
        assert_eq!(assessment.crossing, Some(GuiBoundaryCrossing::OutOfScopeApp));
    }

    #[test]
    fn matching_app_with_different_naming_is_in_scope() {
        let mut input = base_input();
        input.requested_app = Some("Chrome");
        input.proposed_app = Some("Google Chrome");
        let assessment = assess_action_boundary(&input);
        assert!(assessment.within_bounds);
    }

    #[test]
    fn refuses_unobserved_target() {
        let mut input = base_input();
        input.requires_target = true;
        input.target_resolved = false;
        let assessment = assess_action_boundary(&input);
        assert!(assessment.must_refuse());
        assert_eq!(
            assessment.crossing,
            Some(GuiBoundaryCrossing::UnobservedTarget)
        );
    }

    #[test]
    fn no_target_required_action_is_in_bounds_without_resolution() {
        let mut input = base_input();
        input.requires_target = false;
        input.target_resolved = false;
        let assessment = assess_action_boundary(&input);
        assert!(assessment.within_bounds);
    }

    #[test]
    fn ambiguity_event_is_no_guess_and_asks() {
        let event = ambiguity_no_guess_event(
            3,
            "Multiple matching buttons/controls were found.",
            GuiAmbiguityDecisionPoint::PerStepReobserve,
            Some("Which exact visible target should I use?"),
        );
        assert_eq!(event["type"], "AmbiguityNoGuess");
        assert_eq!(event["decision"], "ask");
        assert_eq!(event["no_guess"], true);
        assert_eq!(event["candidate_count"], 3);
        assert_eq!(event["decision_point"], "per_step_reobserve");
        assert_eq!(event["can_execute"], false);
    }

    #[test]
    fn verify_and_stop_plan_recognizes_observe_then_verify_terminal() {
        let plan = vec!["Observe".to_string(), "VerifyState".to_string()];
        assert!(is_verify_and_stop_plan(&plan));

        let with_wait = vec![
            "Observe".to_string(),
            "WaitForState".to_string(),
            "VerifyState".to_string(),
        ];
        assert!(is_verify_and_stop_plan(&with_wait));
    }

    #[test]
    fn plan_with_state_changing_step_is_not_verify_and_stop() {
        let plan = vec![
            "Observe".to_string(),
            "ClickControl".to_string(),
            "VerifyState".to_string(),
        ];
        assert!(!is_verify_and_stop_plan(&plan));
    }

    #[test]
    fn plan_not_terminating_in_verify_is_not_verify_and_stop() {
        let plan = vec!["Observe".to_string(), "SummarizeVisibleContent".to_string()];
        assert!(!is_verify_and_stop_plan(&plan));
    }

    #[test]
    fn verify_and_stop_event_asserts_zero_state_changing_actions() {
        let event = verify_and_stop_event(0, "VerifyState", "completed");
        assert_eq!(event["type"], "VerifyAndStopTerminated");
        assert_eq!(event["verified_then_stopped"], true);
        assert_eq!(event["state_changing_actions_executed"], 0);
        assert_eq!(event["terminal_step_type"], "VerifyState");

        // Defensive: if any state-changing action HAD executed, the contract is
        // violated and the flag is false.
        let violated = verify_and_stop_event(1, "VerifyState", "completed");
        assert_eq!(violated["verified_then_stopped"], false);
    }

    #[test]
    fn recovery_decision_event_surfaces_idempotent_gate_and_single_retry() {
        // Idempotent focus-loss: refocus once, gated on idempotency, bounded.
        let event = recovery_decision_event(
            "RefocusSameTarget",
            "focus_lost",
            "recoverable",
            true,
            true,
            0,
            1,
        );
        assert_eq!(event["type"], "RecoveryDecision");
        assert_eq!(event["recovery_action_kind"], "RefocusSameTarget");
        assert_eq!(event["idempotent_gated"], true);
        assert_eq!(event["single_retry_respected"], true);
        assert_eq!(event["unexpected_dialog_stop"], false);
        assert_eq!(event["load_failure_explain"], false);
        assert_eq!(event["can_execute"], false);
    }

    #[test]
    fn recovery_decision_event_flags_non_idempotent_stop() {
        // A non-idempotent action is never auto-retried: idempotent_gated false,
        // recovery stops.
        let event = recovery_decision_event(
            "Stop",
            "verification_failed",
            "blocked",
            false,
            false,
            0,
            1,
        );
        assert_eq!(event["idempotent_gated"], false);
        assert_eq!(event["recovery_action_kind"], "Stop");
        assert_eq!(event["can_execute_recovery"], false);
    }

    #[test]
    fn recovery_decision_event_flags_unexpected_dialog_stop() {
        let event =
            recovery_decision_event("Stop", "modal_appeared", "blocked", true, false, 0, 1);
        assert_eq!(event["unexpected_dialog_stop"], true);
        assert_eq!(event["recovery_action_kind"], "Stop");
    }

    #[test]
    fn recovery_decision_event_flags_load_failure_explain() {
        let event = recovery_decision_event(
            "ReObserve",
            "load_failed",
            "needs_reobserve",
            false,
            true,
            0,
            1,
        );
        assert_eq!(event["load_failure_explain"], true);
        assert_eq!(event["recovery_action_kind"], "ReObserve");
    }
}
