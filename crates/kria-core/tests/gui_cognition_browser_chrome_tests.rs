//! Task 7.1 (Requirements 5, 9, 26) — browser **chrome-UI** targeting T1/T2.
//!
//! CI-safe: no live KRIA desktop API, no display, no network. Two tiers:
//!
//!   * **T1 (unit):** the `gui_cog_browser` flag mirrors the established
//!     `GuiPrimitivesConfig` pattern (Default OFF, `enabled`/`disabled`,
//!     `from_env*` truthy/falsy rollback); browser-app detection reads observed
//!     window identity; chrome-hint classification maps natural phrasings
//!     ("address bar"/"reload"/"new tab"/"find"/…) to the right
//!     [`BrowserChromeControl`]; and `resolve_browser_chrome_target` maps each
//!     hint to a REAL observed control (never invents one) ONLY when the flag is
//!     ON and the active app is a browser — non-browser apps and flag-OFF both
//!     yield `None`.
//!
//!   * **T2 (fixture pipeline, no display):** the role/label the helper returns
//!     for each chrome control is actually RESOLVABLE by the existing resolver
//!     ([`resolve_plan_targets`]) from a fixture browser observation, and a
//!     non-browser observation leaves the same prompts un-targeted by the
//!     browser layer (flag-OFF / non-browser unchanged).
//!
//! KRIA authority invariants: the helper never executes, never invents a
//! control, never uses coordinates — it only bridges a hint to an observed
//! control's role+label so the resolver stays the single resolution authority.

use kria_core::agent::gui_cognition::browser::{
    classify_browser_chrome_hint, is_browser_app, resolve_browser_chrome_target,
    BrowserChromeControl, GuiBrowserConfig, BROWSER_ENV_FLAG,
};
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

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures (mirror gui_cognition_target_resolver_tests.rs style).
// ─────────────────────────────────────────────────────────────────────────────

fn control(role: &str, name: &str) -> GuiControlSummary {
    let mut control = GuiControlSummary::new(role, name, format!("/fixture/{role}/{name}"));
    control.bounds = Some(GuiBounds {
        x: 10,
        y: 20,
        width: 200,
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

fn context_with(app: &str, controls: Vec<GuiControlSummary>) -> GuiContext {
    let count = controls.len();
    GuiContext::from_observation(GuiObservationSnapshot {
        observation_id: "obs-browser".into(),
        context_id: "ctx-browser".into(),
        timestamp_ms: 1,
        screen_hash: Some("screen-hash".into()),
        active_window_label: app.into(),
        active_window: GuiActiveWindowSummary {
            label: app.into(),
            app_name: Some(app.into()),
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
        cursor_focus: GuiCursorFocusSummary::default(),
        accessibility: GuiAccessibilitySummary {
            available: true,
            node_count: count,
            control_count: count,
            enabled_control_count: count,
            visible_control_count: count,
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
        // All chrome controls live in `other_controls`; the context builder
        // fuses every control list, so role-based resolution still works.
        text_fields: Vec::new(),
        buttons: Vec::new(),
        dialogs: Vec::new(),
        other_controls: controls,
        visual_controls: Vec::new(),
        timing: GuiObservationTimingSummary::default(),
        cache: GuiObservationCacheSummary::default(),
    })
}

/// A representative real browser chrome accessibility tree.
fn browser_chrome_controls() -> Vec<GuiControlSummary> {
    vec![
        control("entry", "Address and search bar"),
        control("push button", "Reload this page"),
        control("push button", "Back"),
        control("push button", "Forward"),
        control("push button", "Stop"),
        control("push button", "New Tab"),
        control("page tab", "KRIA Docs"),
        control("entry", "Find"),
    ]
}

fn browser_context() -> GuiContext {
    context_with("Google Chrome", browser_chrome_controls())
}

fn plan_with_steps(steps: Vec<GuiTypedPlanStep>) -> GuiLlmPlan {
    GuiLlmPlan {
        plan_id: Some("plan-browser".into()),
        goal_contract_id: Some("goal-browser".into()),
        observation_id: Some("obs-browser".into()),
        context_id: Some("ctx-browser".into()),
        prompt_hash: Some("prompt-hash".into()),
        goal_action_type: Some("click_control".into()),
        plan_status: Some("valid".into()),
        planner_mode: "deterministic".into(),
        plan_summary: "browser chrome resolver test plan".into(),
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
    report.validation_id = Some("validation-browser".into());
    report.plan_id = Some("plan-browser".into());
    report.goal_contract_id = Some("goal-browser".into());
    report.context_id = Some("ctx-browser".into());
    report.prompt_hash = Some("prompt-hash".into());
    report
}

fn click_step(target: &str) -> GuiTypedPlanStep {
    GuiTypedPlanStep {
        step_id: "step-click".into(),
        step_type: "ClickControl".into(),
        summary: "click chrome control".into(),
        target_app_hint: Some("Google Chrome".into()),
        target_window_hint: Some("Google Chrome".into()),
        target_control_hint: Some(target.into()),
        text_payload_summary: None,
        text_payload_hash: None,
        expected_precondition: "current GUI context observed".into(),
        expected_postcondition: "target resolved".into(),
        verification_strategy: "result_visible".into(),
        risk_level: "low".into(),
        requires_approval: false,
        idempotent: kria_core::agent::gui_cognition::default_idempotent_for("ClickControl"),
        allowed_to_execute: false,
        confidence: 0.9,
        reason: "test".into(),
    }
}

fn focus_step(target: &str) -> GuiTypedPlanStep {
    GuiTypedPlanStep {
        step_id: "step-focus".into(),
        step_type: "FocusField".into(),
        summary: "focus chrome field".into(),
        target_app_hint: Some("Google Chrome".into()),
        target_window_hint: Some("Google Chrome".into()),
        target_control_hint: Some(target.into()),
        text_payload_summary: None,
        text_payload_hash: None,
        expected_precondition: "current GUI context observed".into(),
        expected_postcondition: "target resolved".into(),
        verification_strategy: "focused_control".into(),
        risk_level: "low".into(),
        requires_approval: false,
        idempotent: kria_core::agent::gui_cognition::default_idempotent_for("FocusField"),
        allowed_to_execute: false,
        confidence: 0.9,
        reason: "test".into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// T1 — flag pattern + classification + recognition (no pipeline).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t1_flag_default_off_and_constructors_mirror_pattern() {
    assert!(!GuiBrowserConfig::default().is_enabled());
    assert!(GuiBrowserConfig::enabled().is_enabled());
    assert!(!GuiBrowserConfig::disabled().is_enabled());
}

#[test]
fn t1_flag_from_env_truthy_and_default_on_rollback() {
    // from_env: OFF unless truthy.
    assert!(!GuiBrowserConfig::from_env_lookup(|_| None).is_enabled());
    for truthy in ["1", "true", "yes", "on", "ON", " True "] {
        assert!(
            GuiBrowserConfig::from_env_lookup(|k| (k == BROWSER_ENV_FLAG)
                .then(|| truthy.to_string()))
            .is_enabled(),
            "{truthy:?} must enable the flag"
        );
    }
    // from_env_default_on: ON unless explicitly falsy (rollback switch).
    assert!(GuiBrowserConfig::from_env_lookup_default_on(|_| None).is_enabled());
    for falsy in ["0", "false", "no", "off", ""] {
        assert!(
            !GuiBrowserConfig::from_env_lookup_default_on(|k| (k == BROWSER_ENV_FLAG)
                .then(|| falsy.to_string()))
            .is_enabled(),
            "{falsy:?} must roll the default-on flag back OFF"
        );
    }
}

#[test]
fn t1_browser_app_detection_reads_observed_identity() {
    for app in ["Google Chrome", "Chromium", "Mozilla Firefox", "Microsoft Edge", "Brave"] {
        let ctx = context_with(app, Vec::new());
        assert!(is_browser_app(&ctx.active_window), "{app} must be a browser");
    }
    for app in ["Files", "GNOME Terminal", "Text Editor", "Calculator"] {
        let ctx = context_with(app, Vec::new());
        assert!(
            !is_browser_app(&ctx.active_window),
            "{app} must NOT be a browser"
        );
    }
}

#[test]
fn t1_chrome_hint_classification() {
    use BrowserChromeControl::*;
    let cases: &[(&str, BrowserChromeControl)] = &[
        ("address bar", AddressBar),
        ("the URL bar", AddressBar),
        ("omnibox", AddressBar),
        ("open a new tab", NewTab),
        ("the second tab", Tab),
        ("go back", Back),
        ("back", Back),
        ("forward", Forward),
        ("reload", Reload),
        ("refresh the page", Reload),
        ("stop loading", Stop),
        ("find", Find),
        ("find in page", Find),
    ];
    for (hint, expected) in cases {
        assert_eq!(
            classify_browser_chrome_hint(hint),
            Some(*expected),
            "hint {hint:?} must classify as {expected:?}"
        );
    }
    // Not a chrome control.
    assert_eq!(classify_browser_chrome_hint("the login button on the page"), None);
    assert_eq!(classify_browser_chrome_hint(""), None);
}

#[test]
fn t1_resolve_maps_each_chrome_hint_to_a_real_observed_control() {
    let ctx = browser_context();
    let on = GuiBrowserConfig::enabled();

    let cases: &[(&str, BrowserChromeControl, &str, &str)] = &[
        ("address bar", BrowserChromeControl::AddressBar, "entry", "Address and search bar"),
        ("reload", BrowserChromeControl::Reload, "push button", "Reload this page"),
        ("back", BrowserChromeControl::Back, "push button", "Back"),
        ("forward", BrowserChromeControl::Forward, "push button", "Forward"),
        ("stop loading", BrowserChromeControl::Stop, "push button", "Stop"),
        ("new tab", BrowserChromeControl::NewTab, "push button", "New Tab"),
        ("find", BrowserChromeControl::Find, "entry", "Find"),
    ];
    for (hint, control, role, label) in cases {
        let m = resolve_browser_chrome_target(&on, &ctx, hint)
            .unwrap_or_else(|| panic!("hint {hint:?} must match a real observed control"));
        assert_eq!(m.control, *control, "hint {hint:?}");
        assert_eq!(m.role, *role, "hint {hint:?} role");
        assert_eq!(m.label, *label, "hint {hint:?} label");
        assert_eq!(m.target_hint(), *label);
        assert!(m.bounds.is_some(), "matched control carries observed bounds");
    }

    // An individual tab resolves by role to the observed page tab.
    let tab = resolve_browser_chrome_target(&on, &ctx, "switch to the tab").expect("tab matches");
    assert_eq!(tab.control, BrowserChromeControl::Tab);
    assert_eq!(tab.role, "page tab");
    assert_eq!(tab.label, "KRIA Docs");
}

#[test]
fn t1_never_invents_a_control_when_absent() {
    // A browser with NO chrome controls observed → no fabrication.
    let empty = context_with("Google Chrome", Vec::new());
    let on = GuiBrowserConfig::enabled();
    assert!(resolve_browser_chrome_target(&on, &empty, "address bar").is_none());
    assert!(resolve_browser_chrome_target(&on, &empty, "reload").is_none());
}

#[test]
fn t1_flag_off_and_non_browser_are_unaffected() {
    let ctx = browser_context();
    // Flag OFF → never resolves, even on a browser with the controls present.
    let off = GuiBrowserConfig::disabled();
    assert!(resolve_browser_chrome_target(&off, &ctx, "address bar").is_none());
    assert!(resolve_browser_chrome_target(&off, &ctx, "reload").is_none());

    // Flag ON but a non-browser app → never resolves (non-browser unaffected).
    let on = GuiBrowserConfig::enabled();
    let files = context_with("Files", browser_chrome_controls());
    assert!(resolve_browser_chrome_target(&on, &files, "address bar").is_none());
    assert!(resolve_browser_chrome_target(&on, &files, "reload").is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// T2 — the helper's role/label is resolvable by the existing resolver.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t2_resolved_chrome_label_is_resolvable_by_the_resolver() {
    let ctx = browser_context();
    let on = GuiBrowserConfig::enabled();

    // Address bar → editable → FocusField resolves it.
    let addr = resolve_browser_chrome_target(&on, &ctx, "address bar").expect("address bar");
    let plan = plan_with_steps(vec![focus_step(addr.target_hint())]);
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-browser");
    assert_eq!(summary.status, "resolved", "{summary:#?}");
    let target = summary.resolved_target.expect("resolved address bar");
    assert_eq!(target.label, "Address and search bar");
    assert_eq!(target.role, "entry");
    assert_eq!(summary.can_execute, false);

    // Reload button → button → ClickControl resolves it.
    let reload = resolve_browser_chrome_target(&on, &ctx, "reload").expect("reload");
    let plan = plan_with_steps(vec![click_step(reload.target_hint())]);
    let summary = resolve_plan_targets(&plan, &validation(), &ctx, "plan-browser");
    assert_eq!(summary.status, "resolved", "{summary:#?}");
    let target = summary.resolved_target.expect("resolved reload");
    assert_eq!(target.label, "Reload this page");
    assert_eq!(target.target_kind, "button");
    assert_eq!(summary.can_execute, false);
}

#[test]
fn t2_non_browser_observation_is_not_targeted_by_browser_layer() {
    // Same chrome-looking controls, but the active app is the file manager:
    // the browser layer refuses to target them (Task 7.1 is browser-scoped).
    let files = context_with("Files", browser_chrome_controls());
    let on = GuiBrowserConfig::enabled();
    assert!(resolve_browser_chrome_target(&on, &files, "reload").is_none());
    assert!(resolve_browser_chrome_target(&on, &files, "address bar").is_none());
}
