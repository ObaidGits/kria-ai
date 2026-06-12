use kria_core::agent::gui_cognition::context::GuiContext;
use kria_core::agent::gui_cognition::llm_planner::{
    GuiLlmPlan, GuiPlanValidationReport, GuiTypedPlanStep,
};
use kria_core::agent::gui_cognition::perception::{
    GuiAccessibilitySummary, GuiActiveWindowSummary, GuiBounds, GuiControlSummary,
    GuiCursorFocusSummary, GuiObservationCacheSummary, GuiObservationSnapshot,
    GuiObservationTimingSummary, GuiOcrDiagnostics, GuiPerceptionCapabilities, GuiSourceStatus,
};
use kria_core::agent::gui_cognition::resolver::resolve_plan_targets;

fn control(role: &str, name: &str) -> GuiControlSummary {
    let mut control = GuiControlSummary::new(role, name, format!("/fixture/{role}/{name}"));
    control.bounds = Some(GuiBounds {
        x: 10,
        y: 20,
        width: 140,
        height: 32,
    });
    control.in_active_window = true;
    control.identity_confidence = 0.92;
    control.bounds_confidence = 0.94;
    control.state_confidence = 0.95;
    control.executable_confidence = 0.92;
    control.confidence = 0.94;
    control.quality = "trusted".into();
    control.sources = vec!["accessibility".into(), "control_fusion".into()];
    control
}

fn context_with(
    text_fields: Vec<GuiControlSummary>,
    buttons: Vec<GuiControlSummary>,
    other_controls: Vec<GuiControlSummary>,
    focus: GuiCursorFocusSummary,
) -> GuiContext {
    GuiContext::from_observation(GuiObservationSnapshot {
        observation_id: "obs-target".into(),
        context_id: "ctx-target".into(),
        timestamp_ms: 1,
        screen_hash: Some("screen-hash".into()),
        active_window_label: "Fixture App".into(),
        active_window: GuiActiveWindowSummary {
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
        },
        visible_windows: Vec::new(),
        visible_app_count: 1,
        monitors: Vec::new(),
        cursor_focus: focus,
        accessibility: GuiAccessibilitySummary {
            available: true,
            node_count: text_fields.len() + buttons.len() + other_controls.len(),
            control_count: text_fields.len() + buttons.len() + other_controls.len(),
            enabled_control_count: text_fields.len() + buttons.len() + other_controls.len(),
            visible_control_count: text_fields.len() + buttons.len() + other_controls.len(),
            focused_control_count: 0,
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
        text_fields,
        buttons,
        dialogs: Vec::new(),
        other_controls,
        visual_controls: Vec::new(),
        timing: GuiObservationTimingSummary::default(),
        cache: GuiObservationCacheSummary::default(),
    })
}

fn plan_with_steps(steps: Vec<GuiTypedPlanStep>) -> GuiLlmPlan {
    GuiLlmPlan {
        plan_id: Some("plan-target".into()),
        goal_contract_id: Some("goal-target".into()),
        observation_id: Some("obs-target".into()),
        context_id: Some("ctx-target".into()),
        prompt_hash: Some("prompt-hash".into()),
        goal_action_type: Some("click_control".into()),
        plan_status: Some("valid".into()),
        planner_mode: "deterministic".into(),
        plan_summary: "target resolver test plan".into(),
        confidence: 0.9,
        risk_level: "low".into(),
        requires_user_approval: false,
        ambiguity_count: 0,
        validation_errors: Vec::new(),
        source_evidence: Vec::new(),
        steps: Vec::new(),
        typed_steps: steps,
        clarification_question: None,
    }
}

fn validation() -> GuiPlanValidationReport {
    let mut report = GuiPlanValidationReport::valid();
    report.validation_id = Some("validation-target".into());
    report.plan_id = Some("plan-target".into());
    report.goal_contract_id = Some("goal-target".into());
    report.context_id = Some("ctx-target".into());
    report.prompt_hash = Some("prompt-hash".into());
    report
}

fn step(step_type: &str, target: Option<&str>) -> GuiTypedPlanStep {
    GuiTypedPlanStep {
        step_id: format!("step-{step_type}"),
        step_type: step_type.into(),
        summary: format!("{step_type} test step"),
        target_app_hint: Some("Fixture App".into()),
        target_window_hint: Some("Fixture App".into()),
        target_control_hint: target.map(str::to_string),
        text_payload_summary: None,
        text_payload_hash: None,
        expected_precondition: "current GUI context observed".into(),
        expected_postcondition: "target resolved".into(),
        verification_strategy: "target_resolved".into(),
        risk_level: "low".into(),
        requires_approval: false,
        allowed_to_execute: false,
        confidence: 0.9,
        reason: "test".into(),
    }
}

#[test]
fn search_button_resolves_with_high_confidence() {
    let ctx = context_with(
        Vec::new(),
        vec![control("push button", "Search")],
        Vec::new(),
        GuiCursorFocusSummary::default(),
    );
    let plan = plan_with_steps(vec![step("ClickControl", Some("Search button"))]);
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-target");

    assert_eq!(summary.status, "resolved");
    assert_eq!(summary.can_execute, false);
    assert!(summary.can_proceed_to_safety_gate);
    assert!(summary.confidence >= 0.85, "{summary:#?}");
    let target = summary.resolved_target.expect("resolved target");
    assert_eq!(target.label, "Search");
    assert_eq!(target.role, "push button");
    assert_eq!(target.target_kind, "button");
    assert!(!target.control_id.is_empty());
    assert!(!target.target_hash.is_empty());
    assert!(target.bounds.is_some());
}

#[test]
fn search_input_resolves_as_editable() {
    let ctx = context_with(
        vec![control("text", "Search")],
        Vec::new(),
        Vec::new(),
        GuiCursorFocusSummary::default(),
    );
    let plan = plan_with_steps(vec![step("FocusField", Some("Search box"))]);
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-target");

    assert_eq!(summary.status, "resolved");
    assert!(summary.confidence >= 0.85, "{summary:#?}");
    let target = summary.resolved_target.expect("resolved target");
    assert_eq!(target.target_kind, "text_field");
    assert_eq!(target.role, "text");
}

#[test]
fn browser_search_future_controls_defer_after_open_app() {
    let ctx = context_with(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        GuiCursorFocusSummary::default(),
    );
    let mut open = step("OpenApp", None);
    open.target_app_hint = Some("browser".into());
    open.target_window_hint = None;

    let mut focus = step("FocusField", Some("address/search field"));
    focus.target_app_hint = Some("browser".into());
    focus.target_window_hint = None;

    let mut typed = step("TypeText", Some("address/search field"));
    typed.target_app_hint = Some("browser".into());
    typed.target_window_hint = None;
    typed.text_payload_summary = Some("KRIA".into());
    typed.text_payload_hash = Some("query-hash".into());

    let mut enter = step("PressKey", None);
    enter.target_app_hint = Some("browser".into());
    enter.target_window_hint = None;

    let mut plan = plan_with_steps(vec![open, focus, typed, enter]);
    plan.goal_action_type = Some("browser_search".into());
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-target");

    assert_eq!(summary.status, "resolved");
    assert_eq!(summary.can_execute, false);
    assert!(summary.can_proceed_to_safety_gate);
    assert_eq!(summary.results[0].step_type, "OpenApp");
    assert_eq!(summary.results[0].status, "resolved");
    assert!(summary
        .results
        .iter()
        .skip(1)
        .all(|result| result.status == "deferred"));
}

#[test]
fn multiple_search_buttons_are_ambiguous() {
    let ctx = context_with(
        Vec::new(),
        vec![
            control("push button", "Search"),
            control("push button", "Search"),
        ],
        Vec::new(),
        GuiCursorFocusSummary::default(),
    );
    let plan = plan_with_steps(vec![step("ClickControl", Some("Search button"))]);
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-target");

    assert_eq!(summary.status, "ambiguous");
    assert_eq!(summary.can_proceed_to_safety_gate, false);
    assert!(summary.ambiguity_count > 0);
    assert_eq!(summary.results[0].candidates.len(), 2);
}

#[test]
fn generic_click_button_clarifies() {
    let ctx = context_with(
        Vec::new(),
        vec![control("push button", "Search")],
        Vec::new(),
        GuiCursorFocusSummary::default(),
    );
    let plan = plan_with_steps(vec![step("ClickControl", Some("button"))]);
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-target");

    assert_eq!(summary.status, "needs_clarification");
    assert_eq!(summary.can_execute, false);
}

#[test]
fn hidden_disabled_and_missing_bounds_targets_are_rejected() {
    let mut disabled = control("push button", "Search");
    disabled.enabled = false;
    let ctx = context_with(
        Vec::new(),
        vec![disabled],
        Vec::new(),
        GuiCursorFocusSummary::default(),
    );
    let plan = plan_with_steps(vec![step("ClickControl", Some("Search button"))]);
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-target");
    assert_eq!(summary.status, "blocked");
    assert!(summary
        .blockers
        .iter()
        .any(|reason| reason.contains("hidden or disabled")));

    let mut no_bounds = control("push button", "Search");
    no_bounds.bounds = None;
    let ctx = context_with(
        Vec::new(),
        vec![no_bounds],
        Vec::new(),
        GuiCursorFocusSummary::default(),
    );
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-target");
    assert_eq!(summary.status, "blocked");
    assert!(summary
        .blockers
        .iter()
        .any(|reason| reason.contains("no bounds")));
}

#[test]
fn ocr_and_visual_only_targets_do_not_resolve_action_target() {
    let mut ocr_only = control("push button", "Search");
    ocr_only.source = "ocr_label_evidence".into();
    ocr_only.sources = vec!["ocr_label_evidence".into()];
    ocr_only.quality = "not_executable".into();
    let ctx = context_with(
        Vec::new(),
        Vec::new(),
        vec![ocr_only],
        GuiCursorFocusSummary::default(),
    );
    let plan = plan_with_steps(vec![step("ClickControl", Some("Search button"))]);
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-target");
    assert_eq!(summary.status, "blocked");
    assert!(summary
        .blockers
        .iter()
        .any(|reason| reason.contains("ocr_only")));

    let mut visual_only = control("push button", "Search");
    visual_only.source = "visual_detector".into();
    visual_only.sources = vec!["visual_detector".into()];
    visual_only.quality = "not_executable".into();
    let ctx = context_with(
        Vec::new(),
        Vec::new(),
        vec![visual_only],
        GuiCursorFocusSummary::default(),
    );
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-target");
    assert_eq!(summary.status, "blocked");
    assert!(summary
        .blockers
        .iter()
        .any(|reason| reason.contains("visual-only")));
}

#[test]
fn type_text_uses_prior_focusfield_resolution() {
    let ctx = context_with(
        vec![control("text", "Search")],
        Vec::new(),
        Vec::new(),
        GuiCursorFocusSummary::default(),
    );
    let mut type_step = step("TypeText", None);
    type_step.text_payload_summary = Some("KRIA".into());
    type_step.text_payload_hash = Some("hash-kria".into());
    let plan = plan_with_steps(vec![step("FocusField", Some("Search box")), type_step]);
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-target");

    assert_eq!(summary.status, "resolved");
    assert_eq!(summary.results.len(), 2);
    assert_eq!(summary.results[1].status, "resolved");
    assert_eq!(
        summary.results[0]
            .resolved_target
            .as_ref()
            .unwrap()
            .control_id,
        summary.results[1]
            .resolved_target
            .as_ref()
            .unwrap()
            .control_id
    );
}

#[test]
fn press_key_blocks_when_focus_unknown_and_raw_coordinates_reject() {
    let ctx = context_with(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        GuiCursorFocusSummary::default(),
    );
    let plan = plan_with_steps(vec![step("PressKey", None)]);
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-target");
    assert_eq!(summary.status, "blocked");
    assert_eq!(summary.can_execute, false);

    let ctx = context_with(
        Vec::new(),
        vec![control("push button", "Search")],
        Vec::new(),
        GuiCursorFocusSummary::default(),
    );
    let plan = plan_with_steps(vec![step("ClickControl", Some("x=100 y=200"))]);
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-target");
    assert_eq!(summary.status, "rejected");
}

#[test]
fn secret_labels_are_redacted_and_results_never_execute() {
    let ctx = context_with(
        Vec::new(),
        vec![control("push button", "api_key=SECRET123")],
        Vec::new(),
        GuiCursorFocusSummary::default(),
    );
    let plan = plan_with_steps(vec![step("ClickControl", Some("api_key=SECRET123 button"))]);
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-target");
    let serialized = serde_json::to_string(&summary).expect("summary serializes");

    assert!(!serialized.contains("SECRET123"));
    assert!(serialized.contains("[redacted]"));
    assert_eq!(summary.can_execute, false);
    assert!(summary.results.iter().all(|result| !result.can_execute));
}
