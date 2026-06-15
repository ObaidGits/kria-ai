//! Task 4 (Issue #5): surface-scroll resolution + execution gating.
//!
//! A `Scroll` step is a SURFACE action — it scrolls the active focused
//! window/viewport and needs NO named control. These tests prove:
//!  - Scroll resolves to a `scrollable` surface (status `resolved`,
//!    `resolved_target: None`) when an active/visible window exists (flag ON).
//!  - Scroll blocks honestly when no scrollable surface is observable (flag ON),
//!    never blind-scrolling.
//!  - The executor precondition treats a resolved surface scroll (no
//!    `resolved_target`, no bounds) as executable when primitives ON, and still
//!    blocks for "resolved target missing" when primitives OFF (byte-for-byte).
//!  - Flag-OFF resolution reproduces the legacy `_` fallback blocked_result
//!    byte-for-byte.
//!
//! Verification (`screen_changed`) is covered in `gui_cognition_verification_tests.rs`.

use kria_core::agent::gui_cognition::context::GuiContext;
use kria_core::agent::gui_cognition::executor::{
    build_execution_request_from_proposal, validate_execution_preconditions, GuiActionBackendStatus,
    GuiExecutionAuthorizationSource, GuiExecutionMode, GuiPayloadVault,
};
use kria_core::agent::gui_cognition::llm_planner::{
    GuiLlmPlan, GuiPlanValidationReport, GuiTypedPlanStep,
};
use kria_core::agent::gui_cognition::perception::{
    GuiAccessibilitySummary, GuiActiveWindowSummary, GuiCursorFocusSummary,
    GuiObservationCacheSummary, GuiObservationSnapshot, GuiObservationTimingSummary,
    GuiOcrDiagnostics, GuiPerceptionCapabilities, GuiSourceStatus, GuiWindowSummary,
};
use kria_core::agent::gui_cognition::resolver::{
    resolve_plan_targets, GuiTargetResolutionSummary,
};
use kria_core::agent::gui_cognition::safety_hitl::{proposal_hash, GuiActionProposal};

const FLAG: &str = "KRIA_GUI_COG_PRIMITIVES";

/// RAII guard that sets `KRIA_GUI_COG_PRIMITIVES` for the duration of a test and
/// restores the previous value on drop. Env vars are process-global, so the
/// flag tests run `#[serial]`.
struct EnvGuard {
    prev: Option<String>,
}

impl EnvGuard {
    fn set(value: &str) -> Self {
        let prev = std::env::var(FLAG).ok();
        std::env::set_var(FLAG, value);
        Self { prev }
    }

    fn clear() -> Self {
        let prev = std::env::var(FLAG).ok();
        std::env::remove_var(FLAG);
        Self { prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var(FLAG, v),
            None => std::env::remove_var(FLAG),
        }
    }
}

/// Build a context that either has an observable scrollable surface (a known
/// active window and/or visible windows) or none at all.
fn context_with_surface(active_window_known: bool, visible_windows: usize) -> GuiContext {
    let active_window = if active_window_known {
        GuiActiveWindowSummary {
            label: "Fixture App".into(),
            app_name: Some("Fixture App".into()),
            source: "fixture".into(),
            confidence: 0.95,
            fallback_used: false,
            blocker: None,
            reliability: "reliable".into(),
            authority_status: "available".into(),
            fallback_chain: Vec::new(),
            ..GuiActiveWindowSummary::default()
        }
    } else {
        // Default == label "unknown", app_name None: no observable active window.
        GuiActiveWindowSummary::default()
    };
    let windows: Vec<GuiWindowSummary> = (0..visible_windows)
        .map(|idx| GuiWindowSummary {
            title: format!("Window {idx}"),
            app_name: Some(format!("App {idx}")),
            bounds: None,
            focused: idx == 0,
            visible: true,
            monitor_id: None,
            source: "fixture".into(),
        })
        .collect();
    let visible_app_count = windows.len();
    GuiContext::from_observation(GuiObservationSnapshot {
        observation_id: "obs-scroll".into(),
        context_id: "ctx-scroll".into(),
        timestamp_ms: 1,
        screen_hash: Some("screen-hash".into()),
        active_window_label: active_window.label.clone(),
        active_window,
        visible_windows: windows,
        visible_app_count,
        monitors: Vec::new(),
        cursor_focus: GuiCursorFocusSummary::default(),
        accessibility: GuiAccessibilitySummary {
            available: true,
            source: "fixture".into(),
            source_status: "healthy".into(),
            overall_status: "healthy".into(),
            overall_confidence: 0.94,
            ..GuiAccessibilitySummary::default()
        },
        ocr_blocks: Vec::new(),
        ocr_diagnostics: GuiOcrDiagnostics::default(),
        capabilities: GuiPerceptionCapabilities {
            active_window: GuiSourceStatus::available("fixture"),
            desktop_state: GuiSourceStatus::available("fixture"),
            accessibility: GuiSourceStatus::available("fixture"),
            screenshot: GuiSourceStatus::available("fixture"),
            ocr: GuiSourceStatus::blocked("fixture", "ocr unavailable"),
            monitor: GuiSourceStatus::blocked("fixture", "monitor unavailable"),
            cursor_focus: GuiSourceStatus::blocked("fixture", "focus unavailable"),
        },
        accessibility_ok: true,
        ocr_available: false,
        screenshot_available: true,
        active_window_probe_ok: true,
        desktop_state_probe_ok: true,
        capabilities_probe_ok: true,
        text_fields: Vec::new(),
        buttons: Vec::new(),
        dialogs: Vec::new(),
        other_controls: Vec::new(),
        visual_controls: Vec::new(),
        timing: GuiObservationTimingSummary::default(),
        cache: GuiObservationCacheSummary::default(),
    })
}

fn scroll_plan() -> GuiLlmPlan {
    GuiLlmPlan {
        plan_id: Some("plan-scroll".into()),
        goal_contract_id: Some("goal-scroll".into()),
        observation_id: Some("obs-scroll".into()),
        context_id: Some("ctx-scroll".into()),
        prompt_hash: Some("prompt-hash".into()),
        goal_action_type: Some("scroll".into()),
        plan_status: Some("valid".into()),
        planner_mode: "deterministic".into(),
        plan_summary: "scroll the active view".into(),
        confidence: 0.9,
        risk_level: "low".into(),
        requires_user_approval: false,
        ambiguity_count: 0,
        validation_errors: Vec::new(),
        source_evidence: Vec::new(),
        steps: Vec::new(),
        typed_steps: vec![scroll_step()],
        clarification_question: None,
    }
}

fn scroll_step() -> GuiTypedPlanStep {
    GuiTypedPlanStep {
        step_id: "step-scroll".into(),
        step_type: "Scroll".into(),
        summary: "Scroll the active view down".into(),
        // A surface scroll has no named app/window/control target.
        target_app_hint: None,
        target_window_hint: None,
        target_control_hint: None,
        text_payload_summary: None,
        text_payload_hash: None,
        expected_precondition: "a scrollable view is active".into(),
        expected_postcondition: "the viewport scrolls as requested".into(),
        verification_strategy: "screen_changed".into(),
        risk_level: "low".into(),
        requires_approval: false,
        idempotent: kria_core::agent::gui_cognition::default_idempotent_for("Scroll"),
        allowed_to_execute: false,
        confidence: 0.9,
        reason: "scroll test".into(),
    }
}

fn validation() -> GuiPlanValidationReport {
    let mut report = GuiPlanValidationReport::valid();
    report.validation_id = Some("validation-scroll".into());
    report.plan_id = Some("plan-scroll".into());
    report.goal_contract_id = Some("goal-scroll".into());
    report.context_id = Some("ctx-scroll".into());
    report.prompt_hash = Some("prompt-hash".into());
    report
}

// ── Resolver: surface scroll resolves when a surface is observable ───────────

#[test]
#[serial_test::serial]
fn scroll_resolves_to_scrollable_surface_when_active_window_known() {
    let _guard = EnvGuard::set("1");
    let ctx = context_with_surface(true, 0);
    let summary = resolve_plan_targets(&scroll_plan(), &validation(), &ctx, "plan-scroll");

    assert_eq!(summary.status, "resolved");
    let result = &summary.results[0];
    assert_eq!(result.step_type, "Scroll");
    assert_eq!(result.status, "resolved");
    assert_eq!(result.target_kind, "scrollable");
    assert_eq!(result.target_query, "active scrollable surface");
    // A surface has no control — there must be NO resolved_target.
    assert!(result.resolved_target.is_none());
    assert!(result.can_proceed_to_safety_gate);
    assert_eq!(result.can_execute, false);
    assert!(result.blockers.is_empty());
    assert!(result.confidence >= 0.8 - f64::EPSILON);
    assert!(result
        .source_evidence
        .iter()
        .any(|e| e.contains("scrollable surface")));
}

#[test]
#[serial_test::serial]
fn scroll_resolves_to_scrollable_surface_with_only_visible_windows() {
    let _guard = EnvGuard::set("on");
    // No KNOWN active window, but at least one visible window → surface observable.
    let ctx = context_with_surface(false, 2);
    let summary = resolve_plan_targets(&scroll_plan(), &validation(), &ctx, "plan-scroll");

    let result = &summary.results[0];
    assert_eq!(result.status, "resolved");
    assert_eq!(result.target_kind, "scrollable");
    assert!(result.resolved_target.is_none());
    assert!(result.can_proceed_to_safety_gate);
}

// ── Resolver: honest block when no surface is observable ─────────────────────

#[test]
#[serial_test::serial]
fn scroll_blocks_honestly_when_no_surface_observable() {
    let _guard = EnvGuard::clear(); // absent ⇒ default ON
    // No known active window AND no visible windows → nothing to scroll.
    let ctx = context_with_surface(false, 0);
    let summary = resolve_plan_targets(&scroll_plan(), &validation(), &ctx, "plan-scroll");

    let result = &summary.results[0];
    assert_eq!(result.status, "blocked");
    assert_eq!(result.target_kind, "scrollable");
    assert!(result.resolved_target.is_none());
    assert_eq!(result.can_execute, false);
    assert!(!result.can_proceed_to_safety_gate);
    assert!(
        result
            .blockers
            .iter()
            .any(|b| b.contains("No scrollable surface is observable")),
        "expected honest no-surface block, got {:#?}",
        result.blockers
    );
}

// ── Resolver: flag-OFF byte-for-byte (Scroll stays filtered / skipped) ───────

#[test]
#[serial_test::serial]
fn scroll_flag_off_is_byte_for_byte_skipped_as_today() {
    let _guard = EnvGuard::set("0");
    // Byte-for-byte: with the primitive OFF, a `Scroll` step is filtered out of
    // target resolution exactly as before this change — it emits NO resolution
    // result and the summary is "skipped" (it never reaches the new arm). The
    // downstream executor then blocks it for "resolved target missing"
    // (asserted by `surface_scroll_is_blocked_when_primitives_disabled`), which
    // is the "still blocked as today" end-state.
    let ctx = context_with_surface(true, 1);
    let summary = resolve_plan_targets(&scroll_plan(), &validation(), &ctx, "plan-scroll");

    assert!(
        summary.results.is_empty(),
        "flag-OFF Scroll must emit no resolution result, got {:#?}",
        summary.results
    );
    assert_eq!(summary.status, "skipped");
    assert!(summary.resolved_target.is_none());
    assert_eq!(summary.can_execute, false);
}

// ── Executor precondition: surface scroll executability gating ───────────────

fn surface_scroll_resolution() -> GuiTargetResolutionSummary {
    GuiTargetResolutionSummary {
        resolution_id: "resolution-scroll".into(),
        plan_id: "plan-scroll".into(),
        validation_id: Some("validation-scroll".into()),
        goal_contract_id: Some("goal-scroll".into()),
        context_id: "ctx-scroll".into(),
        observation_id: "obs-scroll".into(),
        status: "resolved".into(),
        results: Vec::new(),
        // Surface scroll: a resolved surface with NO control target / bounds.
        resolved_target: None,
        can_proceed_to_safety_gate: true,
        can_execute: false,
        blocker_count: 0,
        blockers: Vec::new(),
        ambiguity_count: 0,
        ambiguity_reasons: Vec::new(),
        confidence: 0.8,
        prompt_hash: Some("prompt-hash".into()),
    }
}

fn scroll_proposal() -> GuiActionProposal {
    let mut proposal = GuiActionProposal {
        proposal_schema_version: 1,
        proposal_id: "proposal-scroll".into(),
        request_id: "request-scroll".into(),
        session_id: "session-scroll".into(),
        workflow_id: "workflow-scroll".into(),
        goal_contract_id: "goal-scroll".into(),
        plan_id: "plan-scroll".into(),
        validation_id: Some("validation-scroll".into()),
        resolution_id: Some("resolution-scroll".into()),
        context_id: "ctx-scroll".into(),
        observation_id: "obs-scroll".into(),
        step_id: "step-scroll".into(),
        action_type: "Scroll".into(),
        target_hash: "scroll-surface-hash".into(),
        target_control_id: None,
        target_label: None,
        target_role: Some("scrollable".into()),
        target_bounds: None,
        text_payload_summary: None,
        text_payload_hash: None,
        expected_precondition: "a scrollable view is active".into(),
        expected_postcondition: "the viewport scrolls as requested".into(),
        risk_level: "low".into(),
        risk_reasons: vec!["scroll risk".into()],
        requires_user_approval: false,
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
fn surface_scroll_is_executable_when_primitives_enabled() {
    let proposal = scroll_proposal();
    let resolution = surface_scroll_resolution();
    let mut vault = GuiPayloadVault::default();
    let request = build_execution_request_from_proposal(
        &proposal,
        &resolution,
        GuiExecutionAuthorizationSource::SafeNoApprovalRequired,
        None,
        &mut vault,
        1_500,
    );

    // Primitives ON: a resolved surface scroll (no resolved_target, no bounds)
    // must NOT be blocked for "resolved target missing".
    let report = validate_execution_preconditions(
        GuiExecutionMode::ExecuteFixture,
        &request,
        &proposal,
        &resolution,
        &GuiActionBackendStatus::available("fixture_executor"),
        None,
        &vault,
        1_500,
        true,
    );
    assert!(
        report.can_start_action,
        "surface scroll should be executable, blockers: {:#?}",
        report.blockers
    );
    assert!(report
        .blockers
        .iter()
        .all(|b| !b.contains("resolved target missing")));
}

#[test]
fn surface_scroll_is_blocked_when_primitives_disabled() {
    let proposal = scroll_proposal();
    let resolution = surface_scroll_resolution();
    let mut vault = GuiPayloadVault::default();
    let request = build_execution_request_from_proposal(
        &proposal,
        &resolution,
        GuiExecutionAuthorizationSource::SafeNoApprovalRequired,
        None,
        &mut vault,
        1_500,
    );

    // Primitives OFF: Scroll stays a strict control action → a missing
    // resolved_target blocks exactly as before (byte-for-byte).
    let report = validate_execution_preconditions(
        GuiExecutionMode::ExecuteFixture,
        &request,
        &proposal,
        &resolution,
        &GuiActionBackendStatus::available("fixture_executor"),
        None,
        &vault,
        1_500,
        false,
    );
    assert!(!report.can_start_action);
    assert!(
        report
            .blockers
            .iter()
            .any(|b| b.contains("resolved target missing")),
        "expected legacy 'resolved target missing' block, got {:#?}",
        report.blockers
    );
}
