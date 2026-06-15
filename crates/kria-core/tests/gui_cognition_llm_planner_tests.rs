use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use async_trait::async_trait;
use futures::Stream;
use kria_core::agent::gui_cognition::context::{
    GuiContext, GuiContextBuildRequest, GuiContextBuilder,
};
use kria_core::agent::gui_cognition::goal_contract::extract_gui_goal_contract;
use kria_core::agent::gui_cognition::llm_planner::{
    parse_llm_plan, validate_llm_plan, validate_plan_for_resolution, FixtureGuiLlmPlanner,
    GuiLlmPlan, GuiLlmPlanner, GuiLlmPlannerFixture, GuiLlmPlannerRequest, GuiPlanValidationStatus,
    LlmBackendGuiPlanner,
};
use kria_core::agent::gui_cognition::perception::{
    GuiAccessibilitySummary, GuiActiveWindowSummary, GuiBounds, GuiControlSummary,
    GuiCursorFocusSummary, GuiObservationCacheSummary, GuiObservationSnapshot,
    GuiObservationTimingSummary, GuiOcrDiagnostics, GuiPerceptionCapabilities, GuiSourceStatus,
};
use kria_core::llm::{ChatMessage, LlmBackend, LlmResponse, TokenUsage, ToolSchema};

fn control(role: &str, name: &str) -> GuiControlSummary {
    let mut control = GuiControlSummary::new(role, name, format!("/fixture/{role}/{name}"));
    control.bounds = Some(GuiBounds {
        x: 10,
        y: 20,
        width: 120,
        height: 30,
    });
    control.identity_confidence = 0.9;
    control.bounds_confidence = 0.9;
    control.state_confidence = 0.9;
    control.executable_confidence = 0.9;
    control.quality = "trusted".into();
    control
}

fn request_for_prompt(prompt: &str) -> GuiLlmPlannerRequest {
    let text_fields = vec![control("text", "Search")];
    let buttons = vec![
        control("push button", "Search"),
        control("push button", "Submit"),
    ];
    let control_count = text_fields.len() + buttons.len();
    let observation = GuiObservationSnapshot {
        observation_id: "obs-llm".into(),
        context_id: "ctx-llm".into(),
        timestamp_ms: 1,
        screen_hash: Some("screen-hash".into()),
        active_window_label: "Kria Browser".into(),
        active_window: GuiActiveWindowSummary {
            label: "Kria Browser".into(),
            app_name: Some("Browser".into()),
            source: "fixture".into(),
            confidence: 0.93,
            fallback_used: false,
            blocker: None,
            reliability: "reliable".into(),
            fallback_chain: Vec::new(),
            ..GuiActiveWindowSummary::default()
        },
        visible_windows: Vec::new(),
        visible_app_count: 1,
        monitors: Vec::new(),
        cursor_focus: GuiCursorFocusSummary::default(),
        accessibility: GuiAccessibilitySummary {
            available: true,
            node_count: control_count,
            control_count,
            omitted_node_count: 0,
            enabled_control_count: control_count,
            disabled_control_count: 0,
            visible_control_count: control_count,
            focused_control_count: 0,
            source: "fixture".into(),
            source_status: "healthy".into(),
            snapshot_total_ms: Some(12),
            skipped_app_count: 0,
            remediation: Vec::new(),
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
        other_controls: Vec::new(),
        visual_controls: Vec::new(),
        timing: GuiObservationTimingSummary::default(),
        cache: GuiObservationCacheSummary::default(),
    };
    let context = GuiContextBuilder::new().build(GuiContextBuildRequest::new(observation));
    let contract = extract_gui_goal_contract(prompt, Some(&context)).contract;
    GuiLlmPlannerRequest::from_context(
        &contract,
        &context,
        vec![
            "Resolve Search button".into(),
            "Verify screen changed".into(),
        ],
    )
}

fn request() -> GuiLlmPlannerRequest {
    request_for_prompt("Click the visible safe button named Search.")
}

fn context_for_request(req: &GuiLlmPlannerRequest) -> GuiContext {
    let text_fields = vec![control("text", "Search")];
    let buttons = vec![
        control("push button", "Search"),
        control("push button", "Submit"),
    ];
    let control_count = text_fields.len() + buttons.len();
    let observation = GuiObservationSnapshot {
        observation_id: req.observation_id.clone(),
        context_id: req.context_id.clone(),
        timestamp_ms: 1,
        screen_hash: Some("screen-hash".into()),
        active_window_label: "Kria Browser".into(),
        active_window: GuiActiveWindowSummary {
            label: "Kria Browser".into(),
            app_name: Some("Browser".into()),
            source: "fixture".into(),
            confidence: 0.93,
            fallback_used: false,
            blocker: None,
            reliability: "reliable".into(),
            fallback_chain: Vec::new(),
            ..GuiActiveWindowSummary::default()
        },
        visible_windows: Vec::new(),
        visible_app_count: 1,
        monitors: Vec::new(),
        cursor_focus: GuiCursorFocusSummary::default(),
        accessibility: GuiAccessibilitySummary {
            available: true,
            node_count: control_count,
            control_count,
            omitted_node_count: 0,
            enabled_control_count: control_count,
            disabled_control_count: 0,
            visible_control_count: control_count,
            focused_control_count: 0,
            source: "fixture".into(),
            source_status: "healthy".into(),
            snapshot_total_ms: Some(12),
            skipped_app_count: 0,
            remediation: Vec::new(),
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
        other_controls: Vec::new(),
        visual_controls: Vec::new(),
        timing: GuiObservationTimingSummary::default(),
        cache: GuiObservationCacheSummary::default(),
    };
    GuiContextBuilder::new().build(GuiContextBuildRequest::new(observation))
}

async fn fixture_plan(fixture: GuiLlmPlannerFixture) -> Result<GuiLlmPlan, String> {
    let planner = FixtureGuiLlmPlanner::new(fixture);
    let req = request();
    let raw = planner
        .plan(req)
        .await
        .map_err(|error| error.safe_reason())?;
    parse_llm_plan(&raw.content)
}

#[tokio::test]
async fn valid_fake_llm_json_plan_is_accepted() {
    let req = request();
    let plan = fixture_plan(GuiLlmPlannerFixture::ValidPlan)
        .await
        .expect("valid fixture parses");
    let report = validate_llm_plan(&plan, &req);
    assert_eq!(
        report.status,
        GuiPlanValidationStatus::Valid,
        "blocked reasons: {:?}",
        report.blocked_reasons
    );
    assert!(report.blocked_reasons.is_empty());
    assert!(!plan.typed_steps.is_empty());
    assert!(plan.typed_steps.iter().all(|step| !step.allowed_to_execute));
    assert!(plan
        .typed_steps
        .iter()
        .all(|step| !step.verification_strategy.trim().is_empty()));
}

#[tokio::test]
async fn invalid_json_and_prose_wrapper_are_rejected() {
    let raw = FixtureGuiLlmPlanner::new(GuiLlmPlannerFixture::InvalidJson)
        .plan(request())
        .await
        .unwrap();
    assert!(parse_llm_plan(&raw.content).is_err());

    let raw = FixtureGuiLlmPlanner::new(GuiLlmPlannerFixture::ProseWrapper)
        .plan(request())
        .await
        .unwrap();
    assert!(parse_llm_plan(&raw.content).is_err());
}

#[tokio::test]
async fn missing_verification_and_expected_state_are_rejected() {
    let raw = FixtureGuiLlmPlanner::new(GuiLlmPlannerFixture::MissingVerification)
        .plan(request())
        .await
        .unwrap();
    assert!(parse_llm_plan(&raw.content).is_err());

    let raw = FixtureGuiLlmPlanner::new(GuiLlmPlannerFixture::MissingExpectedState)
        .plan(request())
        .await
        .unwrap();
    assert!(parse_llm_plan(&raw.content).is_err());
}

#[tokio::test]
async fn unsafe_llm_outputs_are_blocked_by_validator() {
    for fixture in [
        GuiLlmPlannerFixture::UnsupportedAction,
        GuiLlmPlannerFixture::StaleContext,
        GuiLlmPlannerFixture::InventedTarget,
        GuiLlmPlannerFixture::RawCoordinates,
        GuiLlmPlannerFixture::OcrInjection,
    ] {
        let req = request();
        let plan = fixture_plan(fixture).await.expect("fixture parses");
        let report = validate_llm_plan(&plan, &req);
        assert_eq!(report.status, GuiPlanValidationStatus::Blocked);
        assert!(!report.blocked_reasons.is_empty());
    }
}

#[tokio::test]
async fn risky_submit_must_be_marked_approval_required() {
    let req = request_for_prompt("Prepare to click Submit, but require approval.");
    let planner = FixtureGuiLlmPlanner::new(GuiLlmPlannerFixture::RiskySubmit);
    let raw = planner.plan(req.clone()).await.unwrap();
    let mut plan = parse_llm_plan(&raw.content).expect("fixture parses");
    let report = validate_llm_plan(&plan, &req);
    assert_eq!(
        report.status,
        GuiPlanValidationStatus::Valid,
        "blocked reasons: {:?}",
        report.blocked_reasons
    );
    assert!(plan.requires_user_approval);
    assert_eq!(plan.risk_level, "high");
    assert_eq!(plan.typed_steps[0].step_type, "RequireApproval");
    assert_eq!(
        plan.typed_steps[0].verification_strategy,
        "approval_pending"
    );

    plan.requires_user_approval = false;
    let report = validate_llm_plan(&plan, &req);
    assert_eq!(report.status, GuiPlanValidationStatus::Blocked);
    assert!(report
        .blocked_reasons
        .iter()
        .any(|reason| reason.contains("approval")));
}

#[test]
fn deterministic_browser_search_summary_plan_is_typed_and_non_executable() {
    use kria_core::agent::gui_cognition::llm_planner::{
        plan_summary_json, typed_plan_steps, GuiPlannerSelection,
    };
    use kria_core::agent::gui_cognition::planner::intent_from_goal_contract;

    let req = request_for_prompt("Open browser, search KRIA, and summarize page");
    let lower = "open browser, search kria, and summarize page".to_string();
    let intent = intent_from_goal_contract(&lower, &req.contract, &lower);
    let context = {
        let text_fields = vec![control("text", "Search")];
        let buttons = vec![control("push button", "Search")];
        let observation = GuiObservationSnapshot {
            observation_id: req.observation_id.clone(),
            context_id: req.context_id.clone(),
            timestamp_ms: 1,
            screen_hash: Some("screen-hash".into()),
            active_window_label: "Kria Browser".into(),
            active_window: GuiActiveWindowSummary {
                label: "Kria Browser".into(),
                app_name: Some("Browser".into()),
                source: "fixture".into(),
                confidence: 0.93,
                fallback_used: false,
                blocker: None,
                reliability: "reliable".into(),
                fallback_chain: Vec::new(),
                ..GuiActiveWindowSummary::default()
            },
            visible_windows: Vec::new(),
            visible_app_count: 1,
            monitors: Vec::new(),
            cursor_focus: GuiCursorFocusSummary::default(),
            accessibility: GuiAccessibilitySummary::default(),
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
            other_controls: Vec::new(),
            visual_controls: Vec::new(),
            timing: GuiObservationTimingSummary::default(),
            cache: GuiObservationCacheSummary::default(),
        };
        GuiContextBuilder::new().build(GuiContextBuildRequest::new(observation))
    };
    let selection = GuiPlannerSelection::deterministic(&req, &intent, &context);
    let steps = typed_plan_steps(&selection.plan);
    let kinds = steps
        .iter()
        .map(|step| step.step_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            "OpenApp",
            "TypeText",
            "WaitForState",
            "SummarizeVisibleContent"
        ]
    );
    assert!(steps.iter().all(|step| !step.allowed_to_execute));
    assert!(steps
        .iter()
        .any(|step| step.verification_strategy == "result_visible"));
    let summary = plan_summary_json("plan-test", &selection);
    assert_eq!(summary["goal_action_type"], "browser_search");
    assert_eq!(summary["typed_steps"][0]["step_type"], "OpenApp");
    // det-2 (index 1) is the atomic Ctrl+L+type+Enter address-bar step.
    assert_eq!(summary["typed_steps"][1]["text_payload_summary"], "KRIA");
}

#[test]
fn step4_browser_search_plan_is_valid_for_target_resolution() {
    use kria_core::agent::gui_cognition::llm_planner::GuiPlannerSelection;
    use kria_core::agent::gui_cognition::planner::intent_from_goal_contract;

    let req = request_for_prompt("Open browser, search KRIA, and summarize page");
    let context = context_for_request(&req);
    let lower = "open browser, search kria, and summarize page".to_string();
    let intent = intent_from_goal_contract(&lower, &req.contract, &lower);
    let selection = GuiPlannerSelection::deterministic(&req, &intent, &context);
    let report = validate_plan_for_resolution(&selection.plan, &req, "plan-step4");

    assert_eq!(
        report.status,
        GuiPlanValidationStatus::Valid,
        "blocked reasons: {:?}",
        report.blocked_reasons
    );
    assert_eq!(
        report.readiness_status.as_deref(),
        Some("valid_for_resolution")
    );
    assert!(report.can_proceed_to_target_resolution);
    assert!(!report.can_execute);
    assert!(report.blocked_reasons.is_empty());
    // Task 2 (Issue #3): the default browser-search plan focuses + types + submits
    // in one atomic address-bar step (Ctrl+L inside the executor), so there is no
    // FocusField control-resolution step and no separately-gated focus step.
    assert!(report
        .step_results
        .iter()
        .any(|step| step.step_type == "TypeText"));
    assert!(report
        .step_results
        .iter()
        .all(|step| step.step_type != "FocusField"));
}

#[test]
fn step4_allows_redacted_secret_labels_without_treating_them_as_raw_leaks() {
    use kria_core::agent::gui_cognition::llm_planner::GuiPlannerSelection;
    use kria_core::agent::gui_cognition::planner::intent_from_goal_contract;

    let req = request_for_prompt("What is on my screen? Report active window.");
    let context = context_for_request(&req);
    let lower = "what is on my screen? report active window.".to_string();
    let intent = intent_from_goal_contract(&lower, &req.contract, &lower);
    let mut selection = GuiPlannerSelection::deterministic(&req, &intent, &context);
    selection.plan.typed_steps[0].target_app_hint = Some("Secrets App".into());
    selection.plan.typed_steps[0].target_window_hint = Some("Project password=[redacted]".into());

    let report = validate_plan_for_resolution(&selection.plan, &req, "plan-redacted-secret");
    assert_eq!(
        report.readiness_status.as_deref(),
        Some("valid_for_resolution"),
        "blocked reasons: {:?}",
        report.blocked_reasons
    );
    assert!(!report.can_execute);
}

#[test]
fn step4_form_fill_without_values_asks_clarification() {
    use kria_core::agent::gui_cognition::llm_planner::GuiPlannerSelection;
    use kria_core::agent::gui_cognition::planner::intent_from_goal_contract;

    let prompt =
        "A form is open. Fill the visible form fields, validate the values, and do not press Submit or Send.";
    let req = request_for_prompt(prompt);
    let context = context_for_request(&req);
    let lower = prompt.to_lowercase();
    let intent = intent_from_goal_contract(&lower, &req.contract, &lower);
    let selection = GuiPlannerSelection::deterministic(&req, &intent, &context);

    assert_eq!(selection.plan.typed_steps[0].step_type, "AskClarification");
    let report = validate_plan_for_resolution(&selection.plan, &req, "plan-form-clarify");
    assert_eq!(
        report.readiness_status.as_deref(),
        Some("needs_clarification"),
        "blocked reasons: {:?}",
        report.blocked_reasons
    );
    assert!(!report.requires_user_approval);
    assert!(!report.can_execute);
}

#[test]
fn step4_open_app_is_valid_without_visible_app() {
    use kria_core::agent::gui_cognition::llm_planner::GuiPlannerSelection;
    use kria_core::agent::gui_cognition::planner::intent_from_goal_contract;

    let req = request_for_prompt("Open Chrome");
    let context = context_for_request(&req);
    let lower = "open chrome".to_string();
    let intent = intent_from_goal_contract(&lower, &req.contract, &lower);
    let selection = GuiPlannerSelection::deterministic(&req, &intent, &context);
    let report = validate_plan_for_resolution(&selection.plan, &req, "plan-open-app");

    assert_eq!(
        report.readiness_status.as_deref(),
        Some("valid_for_resolution")
    );
    assert!(report.can_proceed_to_target_resolution);
    assert!(!report.can_execute);
}

/// Regression: the OpenApp step in every plan path (including browser
/// search/navigation flows) must carry a concrete app hint, so the executor
/// receives a real application name instead of falling back to the action kind
/// ("OpenApp"). This is the generic, data-driven fix — no per-app hardcoding.
#[test]
fn open_app_step_threads_concrete_app_hint_not_action_kind() {
    use kria_core::agent::gui_cognition::llm_planner::{typed_plan_steps, GuiPlannerSelection};
    use kria_core::agent::gui_cognition::planner::intent_from_goal_contract;

    for prompt in [
        "Open Google Chrome",
        "Open Google Chrome and search for the latest Ubuntu version",
        "Open Google Chrome and go to github.com",
    ] {
        let req = request_for_prompt(prompt);
        let context = context_for_request(&req);
        let lower = prompt.to_lowercase();
        let intent = intent_from_goal_contract(&lower, &req.contract, &lower);
        let selection = GuiPlannerSelection::deterministic(&req, &intent, &context);
        let steps = typed_plan_steps(&selection.plan);
        if let Some(open_step) = steps.iter().find(|s| s.step_type == "OpenApp") {
            let hint = open_step.target_app_hint.clone().unwrap_or_default();
            assert!(
                !hint.trim().is_empty(),
                "OpenApp step for prompt {prompt:?} must carry a target_app_hint"
            );
            assert_ne!(
                hint, "OpenApp",
                "app hint must never be the action kind for prompt {prompt:?}"
            );
        }
    }
}

#[tokio::test]
async fn step4_risky_submit_requires_approval_before_resolution() {
    let req = request_for_prompt("Prepare to click Submit, but require approval.");
    let planner = FixtureGuiLlmPlanner::new(GuiLlmPlannerFixture::RiskySubmit);
    let raw = planner.plan(req.clone()).await.unwrap();
    let plan = parse_llm_plan(&raw.content).expect("fixture parses");
    let report = validate_plan_for_resolution(&plan, &req, "plan-risky");

    assert_eq!(report.status, GuiPlanValidationStatus::ApprovalRequired);
    assert_eq!(
        report.readiness_status.as_deref(),
        Some("approval_required")
    );
    assert!(report.requires_user_approval);
    assert!(!report.can_proceed_to_target_resolution);
    assert!(!report.can_execute);
    assert!(report
        .step_results
        .iter()
        .any(|step| step.step_type == "RequireApproval"
            && step.status == "approval_required"
            && step.verification_present));
}

#[tokio::test]
async fn step4_rejects_raw_coordinates_and_missing_verification() {
    let req = request();
    let plan = fixture_plan(GuiLlmPlannerFixture::RawCoordinates)
        .await
        .expect("fixture parses");
    let report = validate_plan_for_resolution(&plan, &req, "plan-coordinates");
    assert_eq!(report.status, GuiPlanValidationStatus::Rejected);
    assert_eq!(report.readiness_status.as_deref(), Some("rejected"));
    assert!(!report.can_execute);

    let mut missing_verification = fixture_plan(GuiLlmPlannerFixture::ValidPlan)
        .await
        .expect("fixture parses");
    missing_verification.typed_steps[0]
        .verification_strategy
        .clear();
    let report =
        validate_plan_for_resolution(&missing_verification, &req, "plan-missing-verification");
    assert_eq!(report.readiness_status.as_deref(), Some("blocked"));
    assert!(report
        .blocked_reasons
        .iter()
        .any(|reason| reason.contains("verification_strategy")));
    assert!(!report.can_execute);
}

#[tokio::test]
async fn step4_missing_target_clarifies_and_risky_without_approval_blocks() {
    let req = request_for_prompt("Click the button");
    let raw = FixtureGuiLlmPlanner::new(GuiLlmPlannerFixture::ValidPlan)
        .plan(req.clone())
        .await
        .unwrap();
    let mut missing_target = parse_llm_plan(&raw.content).expect("fixture parses");
    missing_target.typed_steps[0].step_type = "ClickControl".into();
    missing_target.typed_steps[0].target_control_hint = None;
    missing_target.typed_steps[0].verification_strategy = "target_resolved".into();
    let report = validate_plan_for_resolution(&missing_target, &req, "plan-missing-target");
    assert_eq!(
        report.readiness_status.as_deref(),
        Some("needs_clarification"),
        "blocked reasons: {:?}",
        report.blocked_reasons
    );
    assert!(!report.can_execute);

    let req = request_for_prompt("Prepare to click Submit, but require approval.");
    let raw = FixtureGuiLlmPlanner::new(GuiLlmPlannerFixture::RiskySubmit)
        .plan(req.clone())
        .await
        .unwrap();
    let mut risky = parse_llm_plan(&raw.content).expect("fixture parses");
    risky.requires_user_approval = false;
    risky.typed_steps[0].step_type = "ClickControl".into();
    risky.typed_steps[0].requires_approval = false;
    risky.typed_steps[0].verification_strategy = "screen_changed".into();
    let report = validate_plan_for_resolution(&risky, &req, "plan-risky-no-approval");
    assert_eq!(report.readiness_status.as_deref(), Some("blocked"));
    assert!(report
        .blocked_reasons
        .iter()
        .any(|reason| reason.contains("approval")));
    assert!(!report.can_execute);
}

#[tokio::test]
async fn llm_goal_contradiction_is_blocked() {
    let req = request_for_prompt("Open Chrome and search for weather");
    let planner = FixtureGuiLlmPlanner::new(GuiLlmPlannerFixture::GoalContradiction);
    let raw = planner.plan(req.clone()).await.unwrap();
    let plan = parse_llm_plan(&raw.content).expect("fixture parses");
    let report = validate_llm_plan(&plan, &req);
    assert_eq!(report.status, GuiPlanValidationStatus::Blocked);
    assert!(report
        .blocked_reasons
        .iter()
        .any(|reason| reason.contains("contradicts")));
}

#[tokio::test]
async fn planner_request_does_not_include_raw_secrets_or_ocr_text() {
    let req = request();
    let safe = serde_json::to_string(&req.safe_json()).expect("request serializes");
    assert!(!safe.to_lowercase().contains("ignore previous instructions"));
    assert!(!safe.contains("password="));
    assert!(!safe.contains("api_key"));
    assert!(safe.contains("ocr_block_count"));
}

struct GrammarBackend {
    called: Arc<AtomicBool>,
    content: String,
}

#[async_trait]
impl LlmBackend for GrammarBackend {
    fn model_label(&self) -> &str {
        "grammar-fixture"
    }

    fn capabilities(&self) -> &[String] {
        &[]
    }

    fn is_configured(&self) -> bool {
        true
    }

    async fn chat(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[ToolSchema]>,
        _temperature: f32,
        _max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        anyhow::bail!("plain chat should not be used by GUI LLM planner test")
    }

    async fn chat_stream(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[ToolSchema]>,
        _temperature: f32,
        _max_tokens: u32,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = String> + Send>>> {
        Ok(Box::pin(futures::stream::empty()))
    }

    async fn health_check(&self) -> bool {
        true
    }

    async fn chat_with_grammar(
        &self,
        _messages: &[ChatMessage],
        json_schema: serde_json::Value,
        _temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        self.called.store(true, Ordering::SeqCst);
        assert_eq!(json_schema["type"], "object");
        assert_eq!(max_tokens, 1200);
        Ok(LlmResponse {
            content: self.content.clone(),
            model: "grammar-fixture".into(),
            usage: Some(TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            }),
            tool_calls: None,
        })
    }
}

#[tokio::test]
async fn live_adapter_uses_chat_with_grammar_schema_path() {
    let req = request();
    let raw = FixtureGuiLlmPlanner::new(GuiLlmPlannerFixture::ValidPlan)
        .plan(req.clone())
        .await
        .unwrap();
    let called = Arc::new(AtomicBool::new(false));
    let backend = Arc::new(GrammarBackend {
        called: called.clone(),
        content: raw.content,
    });
    let planner = LlmBackendGuiPlanner::new(backend);
    let response = planner.plan(req).await.expect("grammar backend succeeds");
    assert_eq!(response.model.as_deref(), Some("grammar-fixture"));
    assert!(called.load(Ordering::SeqCst));
}

// ── Task 2.1: gui_cog_smart_planner flag config (Requirement 1.2) ─────────────

#[test]
fn smart_planner_flag_defaults_off_and_parses_truthy_values() {
    use kria_core::agent::gui_cognition::llm_planner::GuiSmartPlannerConfig;

    // Default is OFF (single-attempt behavior preserved).
    assert!(!GuiSmartPlannerConfig::default().is_enabled());
    assert!(GuiSmartPlannerConfig::enabled().is_enabled());
    assert!(!GuiSmartPlannerConfig::disabled().is_enabled());

    // Unset env → OFF.
    let off = GuiSmartPlannerConfig::from_env_lookup(|_| None);
    assert!(!off.is_enabled());

    // Truthy values → ON.
    for value in ["1", "true", "TRUE", "yes", "on", " On "] {
        let cfg = GuiSmartPlannerConfig::from_env_lookup(|key| {
            if key == "KRIA_GUI_COG_SMART_PLANNER" {
                Some(value.to_string())
            } else {
                None
            }
        });
        assert!(cfg.is_enabled(), "value {value:?} should enable the flag");
    }

    // Non-truthy values stay OFF.
    for value in ["0", "false", "no", "off", "", "maybe"] {
        let cfg = GuiSmartPlannerConfig::from_env_lookup(|key| {
            if key == "KRIA_GUI_COG_SMART_PLANNER" {
                Some(value.to_string())
            } else {
                None
            }
        });
        assert!(!cfg.is_enabled(), "value {value:?} should NOT enable the flag");
    }
}

// ── Task 2.9: gui_cog_smart_planner gate flip — default ON + env rollback ─────

#[test]
fn smart_planner_default_on_when_env_absent_or_truthy() {
    use kria_core::agent::gui_cognition::llm_planner::GuiSmartPlannerConfig;

    // Task 2.9 gate flip: the live/desktop path defaults ON.
    let cfg = GuiSmartPlannerConfig::from_env_lookup_default_on(|_| None);
    assert!(
        cfg.is_enabled(),
        "smart planner should default ON when the env var is absent"
    );

    // Any truthy / non-rollback value keeps it ON.
    for value in ["1", "true", "YES", "On", "anything-else"] {
        let cfg = GuiSmartPlannerConfig::from_env_lookup_default_on(|key| {
            if key == "KRIA_GUI_COG_SMART_PLANNER" {
                Some(value.to_string())
            } else {
                None
            }
        });
        assert!(cfg.is_enabled(), "value {value:?} should keep the flag ON");
    }
}

#[test]
fn smart_planner_default_on_rolls_back_when_env_explicitly_falsy() {
    use kria_core::agent::gui_cognition::llm_planner::GuiSmartPlannerConfig;

    // Documented rollback: KRIA_GUI_COG_SMART_PLANNER=0/false/no/off/"".
    for value in ["0", "false", "no", "off", "", " OFF "] {
        let cfg = GuiSmartPlannerConfig::from_env_lookup_default_on(|key| {
            if key == "KRIA_GUI_COG_SMART_PLANNER" {
                Some(value.to_string())
            } else {
                None
            }
        });
        assert!(
            !cfg.is_enabled(),
            "explicit falsy value {value:?} should roll the flag back OFF"
        );
    }
}

#[test]
fn repair_feedback_is_sanitized_and_threaded_onto_request() {
    let req = request();
    assert!(req.repair_feedback.is_none());

    let repaired = req.clone().with_repair_feedback("step verification_strategy is missing");
    assert_eq!(
        repaired.repair_feedback.as_deref(),
        Some("step verification_strategy is missing")
    );

    // Empty/whitespace feedback collapses to None (no spurious repair message).
    let empty = req.with_repair_feedback("   ");
    assert!(empty.repair_feedback.is_none());
}

/// The single repair-retry MUST feed the prior validation error back to the
/// model as an extra instruction. This backend records the messages it receives
/// so we can assert the feedback is present on the repair attempt and absent on
/// the first attempt.
struct RecordingGrammarBackend {
    last_messages: std::sync::Mutex<Vec<String>>,
    content: String,
}

#[async_trait]
impl LlmBackend for RecordingGrammarBackend {
    fn model_label(&self) -> &str {
        "recording-grammar-fixture"
    }

    fn capabilities(&self) -> &[String] {
        &[]
    }

    fn is_configured(&self) -> bool {
        true
    }

    async fn chat(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[ToolSchema]>,
        _temperature: f32,
        _max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        anyhow::bail!("plain chat should not be used by GUI LLM planner test")
    }

    async fn chat_stream(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[ToolSchema]>,
        _temperature: f32,
        _max_tokens: u32,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = String> + Send>>> {
        Ok(Box::pin(futures::stream::empty()))
    }

    async fn health_check(&self) -> bool {
        true
    }

    async fn chat_with_grammar(
        &self,
        messages: &[ChatMessage],
        _json_schema: serde_json::Value,
        _temperature: f32,
        _max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        let mut guard = self.last_messages.lock().unwrap();
        *guard = messages.iter().map(|message| message.content.clone()).collect();
        Ok(LlmResponse {
            content: self.content.clone(),
            model: "recording-grammar-fixture".into(),
            usage: None,
            tool_calls: None,
        })
    }
}

#[tokio::test]
async fn repair_feedback_is_appended_as_planner_instruction() {
    let req = request();
    let raw = FixtureGuiLlmPlanner::new(GuiLlmPlannerFixture::ValidPlan)
        .plan(req.clone())
        .await
        .unwrap();
    let backend = Arc::new(RecordingGrammarBackend {
        last_messages: std::sync::Mutex::new(Vec::new()),
        content: raw.content,
    });
    let planner = LlmBackendGuiPlanner::new(backend.clone());

    // First attempt: no repair feedback → no extra repair instruction message.
    planner.plan(req.clone()).await.expect("first attempt");
    {
        let messages = backend.last_messages.lock().unwrap();
        assert_eq!(messages.len(), 2, "first attempt is system + user only");
        assert!(messages
            .iter()
            .all(|message| !message.contains("failed strict schema validation")));
    }

    // Repair attempt: the sanitized prior error is fed back.
    let repair_req = req.with_repair_feedback("step verification_strategy is missing or incompatible");
    planner.plan(repair_req).await.expect("repair attempt");
    {
        let messages = backend.last_messages.lock().unwrap();
        assert_eq!(messages.len(), 3, "repair attempt appends a feedback message");
        assert!(messages.iter().any(|message| {
            message.contains("failed strict schema validation")
                && message.contains("verification_strategy")
        }));
    }
}

#[tokio::test]
async fn sequenced_fixture_planner_varies_response_and_tracks_repair_calls() {
    use kria_core::agent::gui_cognition::llm_planner::SequencedFixtureGuiLlmPlanner;

    let planner = SequencedFixtureGuiLlmPlanner::new(vec![
        GuiLlmPlannerFixture::InvalidJson,
        GuiLlmPlannerFixture::ValidPlan,
    ]);
    let req = request();

    // First call: no repair feedback, returns invalid JSON.
    let first = planner.plan(req.clone()).await.unwrap();
    assert!(parse_llm_plan(&first.content).is_err());

    // Second call carries repair feedback and returns a valid plan.
    let repair_req = req.with_repair_feedback("prior error");
    let second = planner.plan(repair_req).await.unwrap();
    assert!(parse_llm_plan(&second.content).is_ok());

    assert_eq!(planner.call_count(), 2);
    assert_eq!(planner.repair_call_count(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2.2: model-capability validation + deterministic fallback quality bar
// (Requirements 1.2, 1.3, 1.5)
// ─────────────────────────────────────────────────────────────────────────────

use kria_core::agent::gui_cognition::goal_contract::{
    GuiActionType, GuiGoalContract, GuiGoalExtractionMode, GuiRiskLevel,
};
use kria_core::agent::gui_cognition::llm_planner::{
    deterministic_fallback_meets_quality_bar, deterministic_fallback_quality_ok,
    GuiPlannerCapability, GuiPlannerHealthSignal, GuiPlannerHealthTracker, GuiPlannerMode,
    GuiPlannerSelection, GUI_PLANNER_DEFECT_THRESHOLD,
};
use kria_core::agent::gui_cognition::planner::{GuiCognitionIntent, GuiCognitionIntentKind};

/// Configurable backend used to exercise the truthful capability signal.
struct CapabilityBackend {
    label: String,
    configured: bool,
    grammar: bool,
}

#[async_trait]
impl LlmBackend for CapabilityBackend {
    fn model_label(&self) -> &str {
        &self.label
    }

    fn capabilities(&self) -> &[String] {
        &[]
    }

    fn is_configured(&self) -> bool {
        self.configured
    }

    fn supports_grammar(&self) -> bool {
        self.grammar
    }

    async fn chat(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[ToolSchema]>,
        _temperature: f32,
        _max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        anyhow::bail!("not used")
    }

    async fn chat_stream(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[ToolSchema]>,
        _temperature: f32,
        _max_tokens: u32,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = String> + Send>>> {
        Ok(Box::pin(futures::stream::empty()))
    }

    async fn health_check(&self) -> bool {
        self.configured
    }
}

#[test]
fn capability_validated_model_is_accepted() {
    let backend = Arc::new(CapabilityBackend {
        label: "grammar-model".into(),
        configured: true,
        grammar: true,
    });
    let planner = LlmBackendGuiPlanner::new(backend);
    let capability = planner.capability();

    assert!(capability.is_grammar_capable());
    assert!(capability.grammar_capable);
    assert!(capability.configured);
    assert!(capability.supports_grammar);
    assert_eq!(capability.status, "capability_validated");
    assert_eq!(capability.model_label, "grammar-model");
}

#[test]
fn non_grammar_model_is_flagged_not_capable() {
    let backend = Arc::new(CapabilityBackend {
        label: "prose-only-model".into(),
        configured: true,
        grammar: false,
    });
    let planner = LlmBackendGuiPlanner::new(backend);
    let capability = planner.capability();

    assert!(!capability.is_grammar_capable());
    assert!(!capability.grammar_capable);
    assert!(capability.configured);
    assert!(!capability.supports_grammar);
    assert_eq!(capability.status, "not_grammar_capable");
}

#[test]
fn unconfigured_model_is_flagged_unconfigured() {
    let backend = Arc::new(CapabilityBackend {
        label: "offline-model".into(),
        configured: false,
        grammar: true,
    });
    let planner = LlmBackendGuiPlanner::new(backend);
    let capability = planner.capability();

    assert!(!capability.is_grammar_capable());
    assert!(!capability.configured);
    assert_eq!(capability.status, "unconfigured");
}

#[test]
fn absent_planner_capability_is_no_planner() {
    let capability = GuiPlannerCapability::absent();
    assert!(!capability.is_grammar_capable());
    assert_eq!(capability.status, "no_planner");
    assert_eq!(capability.model_label, "none");
}

#[test]
fn persistent_rejected_fallback_on_healthy_model_is_a_defect() {
    let capability = GuiPlannerCapability::validated("grammar-model");

    // A single rejection on a capable model is a defect *suspicion*.
    let single = GuiPlannerHealthSignal::evaluate(
        &capability,
        &GuiPlannerMode::DeterministicFallback,
        "rejected",
        1,
    );
    assert!(!single.is_defect);
    assert_eq!(single.status, "defect_suspected");
    assert!(single.rejected_fallback);
    assert!(single.grammar_capable);

    // A persistent run (>= threshold) is a confirmed defect (Requirement 1.5).
    let persistent = GuiPlannerHealthSignal::evaluate(
        &capability,
        &GuiPlannerMode::DeterministicFallback,
        "rejected_after_repair",
        GUI_PLANNER_DEFECT_THRESHOLD,
    );
    assert!(persistent.is_defect, "persistent rejection must be a defect");
    assert_eq!(persistent.status, "persistent_defect");
    assert!(persistent.should_report());
}

#[test]
fn rejected_fallback_on_non_grammar_model_is_expected_not_defect() {
    let capability = GuiPlannerCapability::not_grammar_capable("prose-only-model");
    let signal = GuiPlannerHealthSignal::evaluate(
        &capability,
        &GuiPlannerMode::DeterministicFallback,
        "rejected",
        GUI_PLANNER_DEFECT_THRESHOLD + 3,
    );
    // A non-grammar model falling back is the expected path, never a defect.
    assert!(!signal.is_defect);
    assert_eq!(signal.status, "healthy");
    assert!(!signal.should_report());
}

#[test]
fn completed_llm_plan_health_signal_is_healthy() {
    let capability = GuiPlannerCapability::validated("grammar-model");
    let signal = GuiPlannerHealthSignal::evaluate(
        &capability,
        &GuiPlannerMode::LlmAssisted,
        "completed",
        0,
    );
    assert!(!signal.is_defect);
    assert!(!signal.rejected_fallback);
    assert_eq!(signal.status, "healthy");
}

// ── Task 2.6: cross-turn persistence tracker (Requirement 1.5) ───────────────

#[test]
fn health_tracker_escalates_persistent_rejection_to_defect_on_healthy_model() {
    // A healthy, grammar-capable model that keeps falling back is a defect once
    // the fallback is *persistent* across turns (Requirement 1.5).
    let capability = GuiPlannerCapability::validated("grammar-model");
    let tracker = GuiPlannerHealthTracker::new();

    // Turn 1: first rejection → one-off suspicion, not yet a defect.
    let n1 = tracker.record(&GuiPlannerMode::DeterministicFallback, "rejected");
    assert_eq!(n1, 1);
    let s1 = GuiPlannerHealthSignal::evaluate(
        &capability,
        &GuiPlannerMode::DeterministicFallback,
        "rejected",
        n1,
    );
    assert_eq!(s1.status, "defect_suspected");
    assert!(!s1.is_defect);

    // Turn 2: a second consecutive rejection crosses the threshold → defect.
    let n2 = tracker.record(&GuiPlannerMode::DeterministicFallback, "rejected_after_repair");
    assert_eq!(n2, 2);
    assert!(n2 >= GUI_PLANNER_DEFECT_THRESHOLD);
    let s2 = GuiPlannerHealthSignal::evaluate(
        &capability,
        &GuiPlannerMode::DeterministicFallback,
        "rejected_after_repair",
        n2,
    );
    assert_eq!(s2.status, "persistent_defect");
    assert!(s2.is_defect, "persistent rejection on a healthy model is a defect");
    assert!(s2.should_report());
    assert_eq!(s2.consecutive_rejected_fallbacks, GUI_PLANNER_DEFECT_THRESHOLD);
}

#[test]
fn health_tracker_resets_streak_on_recovery() {
    let tracker = GuiPlannerHealthTracker::new();
    assert_eq!(tracker.record(&GuiPlannerMode::DeterministicFallback, "rejected"), 1);
    assert_eq!(
        tracker.record(&GuiPlannerMode::DeterministicFallback, "rejected"),
        2
    );
    assert_eq!(tracker.current(), 2);

    // A recovering turn (completed LLM plan) clears the persistent condition.
    assert_eq!(tracker.record(&GuiPlannerMode::LlmAssisted, "completed"), 0);
    assert_eq!(tracker.current(), 0);

    // A subsequent lone rejection is once again only a one-off suspicion.
    assert_eq!(tracker.record(&GuiPlannerMode::DeterministicFallback, "rejected"), 1);
}

#[test]
fn health_tracker_does_not_count_non_rejection_fallbacks() {
    // A provider/transport error or unavailable planner is NOT a
    // llm_rejected_fallback and must not advance the persistence streak.
    let tracker = GuiPlannerHealthTracker::new();
    assert_eq!(tracker.record(&GuiPlannerMode::DeterministicFallback, "rejected"), 1);
    // Provider error fallback uses a non-"rejected" llm_status → resets.
    assert_eq!(tracker.record(&GuiPlannerMode::DeterministicFallback, "provider_error"), 0);
    // A purely deterministic plan (no LLM wired) also resets.
    assert_eq!(tracker.record(&GuiPlannerMode::Deterministic, "unavailable"), 0);
    assert_eq!(tracker.current(), 0);
}

// ── Deterministic fallback quality bar ───────────────────────────────────────

fn quality_contract(action_type: GuiActionType) -> GuiGoalContract {
    let risky = matches!(action_type, GuiActionType::RiskApproval);
    GuiGoalContract {
        contract_id: "contract-qb".into(),
        observation_id: "obs-llm".into(),
        context_id: "ctx-llm".into(),
        prompt_hash: "hash-qb".into(),
        goal_summary: "quality bar goal".into(),
        intent_kind: "analyze_plan".into(),
        action_type,
        target_app_kind: Some("browser".into()),
        target_app_hint: Some("Firefox".into()),
        target_window_hint: Some("Firefox window".into()),
        target_control_hint: Some("Search box".into()),
        query_summary: Some("kria ai".into()),
        query_hash: Some("qhash".into()),
        text_payload_summary: Some("hello world".into()),
        text_payload_hash: Some("thash".into()),
        desired_final_state: "requested state is reached".into(),
        risk_level: if risky {
            GuiRiskLevel::High
        } else {
            GuiRiskLevel::Low
        },
        requires_user_approval: risky,
        ambiguities: Vec::new(),
        source_evidence: Vec::new(),
        extraction_confidence: 0.8,
        extractor_mode: GuiGoalExtractionMode::Deterministic,
        cross_app_clipboard: None,
        file_manager_select: None,
    }
}

fn quality_intent() -> GuiCognitionIntent {
    GuiCognitionIntent {
        kind: GuiCognitionIntentKind::AnalyzePlan,
        typed_text: None,
        control_name: None,
        requires_approval: false,
        risk_level: "low".into(),
        risk_reasons: Vec::new(),
    }
}

fn deterministic_plan_for(contract: GuiGoalContract) -> GuiLlmPlan {
    let mut req = request();
    req.contract = contract;
    let context = context_for_request(&req);
    let intent = quality_intent();
    GuiPlannerSelection::deterministic_fallback(
        &req, &intent, &context, true, "rejected", "test fallback",
    )
    .plan
}

#[test]
fn scroll_steps_thread_direction_marker_onto_scroll_step() {
    // Task 4 (Issue #5): the goal contract encodes the scroll DIRECTION as a
    // `scroll:<dir>` marker in target_control_hint; the deterministic planner
    // must thread it onto the typed Scroll step so it survives into the proposal
    // and ultimately the desktop GuiActionRequest.
    for marker in ["scroll:up", "scroll:down", "scroll:top", "scroll:bottom"] {
        let mut contract = quality_contract(GuiActionType::Scroll);
        contract.target_control_hint = Some(marker.to_string());
        let plan = deterministic_plan_for(contract);
        let scroll_step = plan
            .typed_steps
            .iter()
            .find(|step| step.step_type == "Scroll")
            .unwrap_or_else(|| panic!("expected a Scroll step for marker {marker}"));
        assert_eq!(
            scroll_step.target_control_hint.as_deref(),
            Some(marker),
            "Scroll step must carry the threaded direction marker {marker}"
        );
    }
}

#[test]
fn scroll_steps_without_marker_carry_no_direction_hint() {
    // Flag-OFF / no-direction: the contract carries no marker, so the Scroll
    // step's control hint stays None (byte-for-byte with prior behavior).
    let mut contract = quality_contract(GuiActionType::Scroll);
    contract.target_control_hint = None;
    let plan = deterministic_plan_for(contract);
    let scroll_step = plan
        .typed_steps
        .iter()
        .find(|step| step.step_type == "Scroll")
        .expect("expected a Scroll step");
    assert!(scroll_step.target_control_hint.is_none());
}

#[test]
fn deterministic_fallback_meets_quality_bar_for_every_action_type() {
    let action_types = [
        GuiActionType::Observe,
        GuiActionType::AnalyzePlan,
        GuiActionType::FocusInput,
        GuiActionType::TypeText,
        GuiActionType::ClearField,
        GuiActionType::SelectAll,
        GuiActionType::ClickControl,
        GuiActionType::SetCheckbox,
        GuiActionType::CloseDialog,
        GuiActionType::PressKey,
        GuiActionType::Scroll,
        GuiActionType::InAppSearch,
        GuiActionType::VerifyAndStop,
        GuiActionType::BrowserSearch,
        GuiActionType::BrowserNavigate,
        GuiActionType::FillForm,
        GuiActionType::OpenApp,
        GuiActionType::SwitchWindow,
        GuiActionType::Save,
        GuiActionType::Download,
        GuiActionType::CopyContent,
        GuiActionType::PasteContent,
        GuiActionType::Recovery,
        GuiActionType::RiskApproval,
        GuiActionType::SafeAction,
        GuiActionType::Unknown,
    ];

    for action_type in action_types {
        let label = action_type.as_str();
        let plan = deterministic_plan_for(quality_contract(action_type));
        let result = deterministic_fallback_meets_quality_bar(&plan);
        assert!(
            result.is_ok(),
            "deterministic fallback for '{label}' must meet the quality bar: {:?}",
            result.err()
        );
        assert!(!plan.typed_steps.is_empty(), "'{label}' produced no steps");
        assert!(plan.typed_steps.iter().all(|step| !step.allowed_to_execute));
    }
}

#[test]
fn deterministic_fallback_meets_quality_bar_for_common_combos() {
    // Combos that the deterministic planner expands into multiple verified steps.
    for action_type in [
        GuiActionType::BrowserSearch,  // open → focus → type → enter → wait → summarize
        GuiActionType::BrowserNavigate, // open → focus → type → enter → wait
        GuiActionType::TypeText,        // focus → type → verify
    ] {
        let label = action_type.as_str();
        let plan = deterministic_plan_for(quality_contract(action_type));
        assert!(
            plan.typed_steps.len() >= 3,
            "combo '{label}' should expand to multiple steps"
        );
        assert!(
            deterministic_fallback_quality_ok(&plan),
            "combo '{label}' must meet the quality bar: {:?}",
            deterministic_fallback_meets_quality_bar(&plan).err()
        );
    }
}

#[test]
fn missing_hint_action_falls_back_to_clarification_meeting_quality_bar() {
    // TypeText with no control hint and no payload → AskClarification, which is a
    // valid, complete deterministic outcome (Requirement 4.1), not an invalid step.
    let mut contract = quality_contract(GuiActionType::TypeText);
    contract.target_control_hint = None;
    contract.text_payload_summary = None;
    contract.text_payload_hash = None;
    contract.query_summary = None;
    contract.query_hash = None;

    let plan = deterministic_plan_for(contract);
    assert!(
        plan.typed_steps
            .iter()
            .any(|step| step.step_type == "AskClarification"),
        "missing payload should produce an AskClarification step"
    );
    assert!(
        deterministic_fallback_quality_ok(&plan),
        "clarification plan must meet the quality bar: {:?}",
        deterministic_fallback_meets_quality_bar(&plan).err()
    );
}

#[test]
fn quality_bar_rejects_action_kind_used_as_target() {
    // Construct an invalid plan that leaks the action kind into the target.
    let mut plan = deterministic_plan_for(quality_contract(GuiActionType::ClickControl));
    if let Some(step) = plan
        .typed_steps
        .iter_mut()
        .find(|step| step.step_type == "ClickControl")
    {
        step.target_control_hint = Some("ClickControl".into());
    }
    let result = deterministic_fallback_meets_quality_bar(&plan);
    assert!(result.is_err(), "action-kind-as-target must be rejected");
    assert!(result
        .unwrap_err()
        .iter()
        .any(|reason| reason.contains("action kind")));
}

#[test]
fn quality_bar_rejects_missing_verification_strategy() {
    let mut plan = deterministic_plan_for(quality_contract(GuiActionType::OpenApp));
    if let Some(step) = plan.typed_steps.first_mut() {
        step.verification_strategy = String::new();
    }
    let result = deterministic_fallback_meets_quality_bar(&plan);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .iter()
        .any(|reason| reason.contains("verification_strategy")));
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2.4: every supported intent maps to a complete, valid typed-step sequence
// (Requirements 1, 4, 5, 13). Each newly-mapped primitive must meet the
// deterministic fallback quality bar AND reach valid_for_resolution (or
// needs_clarification only when payload/target is genuinely missing).
// ─────────────────────────────────────────────────────────────────────────────

fn deterministic_selection_for(contract: GuiGoalContract) -> (GuiLlmPlannerRequest, GuiLlmPlan) {
    let mut req = request();
    req.contract = contract;
    let context = context_for_request(&req);
    let intent = quality_intent();
    let plan = GuiPlannerSelection::deterministic_fallback(
        &req, &intent, &context, true, "rejected", "test fallback",
    )
    .plan;
    (req, plan)
}

#[test]
fn task24_new_primitive_intents_meet_quality_bar_and_resolve() {
    // Each of these primitives previously fell through to clarification / blocked
    // because the deterministic planner did not map them. They must now produce a
    // complete, valid sequence that reaches valid_for_resolution.
    for action_type in [
        GuiActionType::ClearField,
        GuiActionType::SelectAll,
        GuiActionType::CopyContent,
        GuiActionType::PasteContent,
        GuiActionType::PressKey,
        GuiActionType::Scroll,
        GuiActionType::SetCheckbox,
        GuiActionType::CloseDialog,
        GuiActionType::InAppSearch,
        GuiActionType::VerifyAndStop,
    ] {
        let label = action_type.as_str();
        let (req, plan) = deterministic_selection_for(quality_contract(action_type));

        assert!(
            deterministic_fallback_quality_ok(&plan),
            "'{label}' must meet the quality bar: {:?}",
            deterministic_fallback_meets_quality_bar(&plan).err()
        );

        let report = validate_plan_for_resolution(&plan, &req, "plan-task24");
        assert_eq!(
            report.readiness_status.as_deref(),
            Some("valid_for_resolution"),
            "'{label}' must be valid_for_resolution; blockers: {:?}",
            report.blocked_reasons
        );
        assert!(report.can_proceed_to_target_resolution, "'{label}'");
        assert!(!report.can_execute, "'{label}' must not be executable at plan stage");

        // Property 1 / Requirement 1.4: the action kind is never used as a target.
        for step in &plan.typed_steps {
            for hint in [
                step.target_control_hint.as_deref(),
                step.target_app_hint.as_deref(),
                step.target_window_hint.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                assert_ne!(
                    hint, step.step_type,
                    "'{label}' leaked the action kind into a target hint"
                );
            }
        }
    }
}

#[test]
fn task24_clear_field_sequence_focuses_then_clears_then_verifies() {
    let (_req, plan) = deterministic_selection_for(quality_contract(GuiActionType::ClearField));
    let kinds = plan
        .typed_steps
        .iter()
        .map(|step| step.step_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(kinds, vec!["FocusField", "ClearField", "VerifyState"]);
}

#[test]
fn task24_copy_sequence_uses_clipboard_verification() {
    let (_req, plan) = deterministic_selection_for(quality_contract(GuiActionType::CopyContent));
    assert!(
        plan.typed_steps
            .iter()
            .any(|step| step.step_type == "Copy"
                && step.verification_strategy == "clipboard_changed"),
        "Copy step must verify clipboard_changed"
    );
}

#[test]
fn task24_in_app_search_expands_to_focus_type_enter_wait() {
    let (req, plan) = deterministic_selection_for(quality_contract(GuiActionType::InAppSearch));
    let kinds = plan
        .typed_steps
        .iter()
        .map(|step| step.step_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec!["FocusField", "TypeText", "PressKey", "WaitForState"]
    );
    // The TypeText step carries the threaded query payload (Requirement 4.1).
    let type_step = plan
        .typed_steps
        .iter()
        .find(|step| step.step_type == "TypeText")
        .expect("in-app search has a TypeText step");
    assert!(type_step
        .text_payload_summary
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty()));
    let report = validate_plan_for_resolution(&plan, &req, "plan-inapp");
    assert_eq!(
        report.readiness_status.as_deref(),
        Some("valid_for_resolution"),
        "blockers: {:?}",
        report.blocked_reasons
    );
}

#[test]
fn task24_in_app_search_without_query_asks_clarification() {
    let mut contract = quality_contract(GuiActionType::InAppSearch);
    contract.query_summary = None;
    contract.query_hash = None;
    contract.text_payload_summary = None;
    contract.text_payload_hash = None;

    let (req, plan) = deterministic_selection_for(contract);
    assert!(
        plan.typed_steps
            .iter()
            .any(|step| step.step_type == "AskClarification"),
        "missing in-app query should produce an AskClarification step"
    );
    assert!(
        deterministic_fallback_quality_ok(&plan),
        "clarification plan must meet the quality bar: {:?}",
        deterministic_fallback_meets_quality_bar(&plan).err()
    );
    let report = validate_plan_for_resolution(&plan, &req, "plan-inapp-clarify");
    assert_eq!(
        report.readiness_status.as_deref(),
        Some("needs_clarification"),
        "blockers: {:?}",
        report.blocked_reasons
    );
}

#[test]
fn task24_verify_and_stop_terminates_with_verify_state() {
    let (_req, plan) = deterministic_selection_for(quality_contract(GuiActionType::VerifyAndStop));
    let kinds = plan
        .typed_steps
        .iter()
        .map(|step| step.step_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds.last().copied(),
        Some("VerifyState"),
        "verify-and-stop must end on a terminal VerifyState step"
    );
    assert!(
        plan.typed_steps.iter().all(|step| !matches!(
            step.step_type.as_str(),
            "TypeText" | "ClickControl" | "PressKey" | "Copy" | "Paste" | "SetCheckbox"
        )),
        "verify-and-stop must not execute state-changing actions"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2.7 — T1: each intent → valid complete plan; no "prose or non-object";
// repair-retry path covered (Requirements 1, 4).
//
// These close the remaining 2.7 gaps left by Task 2.4 coverage:
//   (1) FULL intent coverage — *every* GuiActionType maps to a complete, valid
//       deterministic plan (quality bar + reaches valid_for_resolution /
//       approval_required, never blocked/rejected, every step verified, no
//       action-kind-as-target).
//   (2) Prose / non-object LLM output is REJECTED by parse_llm_plan with the
//       exact "LLM planner returned prose or non-object content" reason and is
//       NEVER lenient-scraped into a plan.
//   (3) The exactly-ONE repair-retry path through the runtime
//       (select_plan_with_optional_llm / select_plan_inner):
//         - flag OFF  → single attempt, immediate deterministic fallback, NO repair;
//         - flag ON   → first rejected → ONE repair-retry → repaired plan accepted;
//         - flag ON   → first rejected → repair also rejected → deterministic
//                       fallback, with AT MOST one repair call (no uncontrolled retries).
// ─────────────────────────────────────────────────────────────────────────────

/// (1) Every supported intent / GuiActionType deterministically maps to a
/// COMPLETE, VALID plan: it meets the deterministic quality bar, reaches a
/// resolvable readiness status (never `blocked`/`rejected`), every step carries a
/// `verification_strategy`, and the action kind is NEVER leaked into a target
/// hint (Property 1 / Requirement 1.4). `needs_clarification` is permitted only
/// because a complete contract should not trigger it — so we additionally assert
/// the status is one of the "complete plan" outcomes.
#[test]
fn task27_every_supported_intent_maps_to_valid_complete_plan() {
    let action_types = [
        GuiActionType::Observe,
        GuiActionType::AnalyzePlan,
        GuiActionType::FocusInput,
        GuiActionType::TypeText,
        GuiActionType::ClearField,
        GuiActionType::SelectAll,
        GuiActionType::ClickControl,
        GuiActionType::SetCheckbox,
        GuiActionType::CloseDialog,
        GuiActionType::PressKey,
        GuiActionType::Scroll,
        GuiActionType::InAppSearch,
        GuiActionType::VerifyAndStop,
        GuiActionType::BrowserSearch,
        GuiActionType::BrowserNavigate,
        GuiActionType::FillForm,
        GuiActionType::OpenApp,
        GuiActionType::SwitchWindow,
        GuiActionType::Save,
        GuiActionType::Download,
        GuiActionType::CopyContent,
        GuiActionType::PasteContent,
        GuiActionType::Recovery,
        GuiActionType::RiskApproval,
        GuiActionType::SafeAction,
        GuiActionType::Unknown,
    ];

    for action_type in action_types {
        let label = action_type.as_str();
        // Align the two payload-hash fields so the fixture contract is internally
        // consistent (the deterministic TypeText step threads `text_payload_hash`,
        // while the validator compares against `query_hash` first). Differing
        // fixture hashes would otherwise trip a spurious contradiction unrelated
        // to the property under test.
        let mut contract = quality_contract(action_type);
        contract.text_payload_hash = contract.query_hash.clone();
        let (req, plan) = deterministic_selection_for(contract);

        // Complete + valid deterministic plan.
        assert!(
            deterministic_fallback_quality_ok(&plan),
            "'{label}' must meet the deterministic quality bar: {:?}",
            deterministic_fallback_meets_quality_bar(&plan).err()
        );
        assert!(!plan.typed_steps.is_empty(), "'{label}' produced no steps");

        // Every step carries a verification strategy and is non-executable at the
        // plan stage (KRIA authority: plan never auto-executes).
        for step in &plan.typed_steps {
            assert!(
                !step.verification_strategy.trim().is_empty(),
                "'{label}' step {:?} missing verification_strategy",
                step.step_type
            );
            assert!(
                !step.allowed_to_execute,
                "'{label}' step {:?} must not be pre-authorized to execute",
                step.step_type
            );
            // Property 1 / Requirement 1.4: action kind never used as target.
            for hint in [
                step.target_control_hint.as_deref(),
                step.target_app_hint.as_deref(),
                step.target_window_hint.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                assert_ne!(
                    hint, step.step_type,
                    "'{label}' leaked the action kind into a target hint"
                );
            }
        }

        // Reaches a "complete plan" readiness — never blocked/rejected for a
        // fully-specified contract.
        let report = validate_plan_for_resolution(&plan, &req, "plan-task27-intent");
        let status = report.readiness_status.as_deref().unwrap_or("missing");
        assert!(
            matches!(
                status,
                "valid_for_resolution" | "approval_required" | "needs_clarification"
            ),
            "'{label}' must reach a complete-plan status, got {status:?}; blockers: {:?}",
            report.blocked_reasons
        );
        assert!(
            !report.can_execute,
            "'{label}' must not be executable at the plan stage"
        );
    }
}

/// (2) Prose / non-object outputs are rejected with the EXACT contract reason and
/// are NEVER lenient-scraped. Covers a prose wrapper, a JSON array, a bare JSON
/// string, a number, and free prose — each must fail parse with the same reason.
#[tokio::test]
async fn task27_prose_and_non_object_outputs_are_rejected_with_exact_reason() {
    const REASON: &str = "LLM planner returned prose or non-object content";

    // The fixture prose wrapper (valid JSON embedded in prose) is rejected, not
    // scraped out of the surrounding text.
    let prose = FixtureGuiLlmPlanner::new(GuiLlmPlannerFixture::ProseWrapper)
        .plan(request())
        .await
        .unwrap();
    let err = parse_llm_plan(&prose.content).expect_err("prose wrapper must be rejected");
    assert_eq!(err, REASON, "prose wrapper must fail with the exact reason");

    // Other non-object shapes are likewise rejected with the same reason.
    for raw in [
        "Sure! Here is your plan.",                 // pure prose
        "[ { \"step_type\": \"OpenApp\" } ]",        // JSON array, not an object
        "\"OpenApp\"",                               // bare JSON string
        "42",                                        // number
        "Plan: {\"steps\": []}",                     // prose-prefixed object
    ] {
        let err = parse_llm_plan(raw).expect_err("non-object content must be rejected");
        assert_eq!(
            err, REASON,
            "non-object input {raw:?} must fail with the exact reason"
        );
    }
}

// ── (3) Repair-retry path through the runtime (exactly ONE retry; flag-gated) ──

mod task27_repair_runtime {
    use super::*;
    use kria_core::agent::gui_cognition::executor::{
        GuiActionBackendStatus, GuiActionExecution, GuiActionExecutor, GuiActionRequest,
        GuiExecutionMode,
    };
    use kria_core::agent::gui_cognition::llm_planner::{
        GuiLlmPlanner, GuiSmartPlannerConfig, SequencedFixtureGuiLlmPlanner,
    };
    use kria_core::agent::gui_cognition::perception::{GuiPerceptionProvider, GuiProbeResult};
    use kria_core::agent::gui_cognition::{GuiCognitionRuntime, GuiTurnOutcome, GuiTurnRequest};

    const PROMPT: &str = "Open KRIA Workflow App and focus the visible search field";

    struct RuntimePerception {
        active_window: String,
    }

    #[async_trait]
    impl GuiPerceptionProvider for RuntimePerception {
        async fn get_active_window(&self) -> GuiProbeResult {
            GuiProbeResult::ok(serde_json::json!({
                "title": self.active_window,
                "app_name": self.active_window,
            }))
        }

        async fn get_desktop_state(&self) -> GuiProbeResult {
            GuiProbeResult::ok(serde_json::json!({
                "focused_window": self.active_window,
                "accessibility_operational": true,
                "applications": [self.active_window, "Browser"],
            }))
        }

        async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
            GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": true }))
        }

        async fn find_ui_elements(&self, role: &str) -> GuiProbeResult {
            let elements = match role {
                "text" => vec![serde_json::json!({
                    "role": "text",
                    "name": "Search",
                    "label": "Search",
                    "path": "/runtime/text/Search",
                    "control_id": "runtime-search-field",
                    "enabled": true,
                    "visible": true,
                    "focused": true,
                    "in_active_window": true,
                    "bounds": { "x": 10, "y": 20, "width": 240, "height": 32 },
                    "score": 0.9,
                    "identity_confidence": 0.9,
                    "bounds_confidence": 0.9,
                    "state_confidence": 0.9
                })],
                "push button" => vec![serde_json::json!({
                    "role": "push button",
                    "name": "Search",
                    "label": "Search",
                    "path": "/runtime/button/Search",
                    "control_id": "runtime-search-button",
                    "enabled": true,
                    "visible": true,
                    "focused": false,
                    "in_active_window": true,
                    "bounds": { "x": 280, "y": 20, "width": 90, "height": 32 },
                    "score": 0.9,
                    "identity_confidence": 0.9,
                    "bounds_confidence": 0.9,
                    "state_confidence": 0.9
                })],
                _ => Vec::new(),
            };
            GuiProbeResult::ok(serde_json::json!({ "elements": elements }))
        }

        async fn get_cursor_focus_state(&self) -> GuiProbeResult {
            GuiProbeResult::ok(serde_json::json!({
                "focused_window": self.active_window,
                "focused_app": self.active_window,
                "focused_control_id": "runtime-search-field",
                "focused_control_label": "Search",
                "focused_control_role": "text",
                "keyboard_focus_known": true,
                "text_cursor_known": true,
                "editable_target_known": true,
                "terminal_like": false,
                "focus_confidence": 0.9,
            }))
        }

        async fn capture_screenshot(&self) -> GuiProbeResult {
            GuiProbeResult::ok(serde_json::json!({
                "screen_hash": "runtime-screen-0",
                "byte_count": 16,
                "source": "fixture",
            }))
        }

        async fn focused_window_title(&self) -> Option<String> {
            Some(self.active_window.clone())
        }
    }

    struct RuntimeExecutor {
        backend: GuiActionBackendStatus,
    }

    #[async_trait]
    impl GuiActionExecutor for RuntimeExecutor {
        async fn action_backend_status(&self) -> GuiActionBackendStatus {
            self.backend.clone()
        }

        async fn execute(&self, request: GuiActionRequest) -> GuiActionExecution {
            GuiActionExecution::ok(
                "fixture_executor",
                serde_json::json!({ "executed": request.kind.as_str() }),
            )
        }
    }

    fn perception() -> RuntimePerception {
        RuntimePerception {
            active_window: "KRIA Workflow App".into(),
        }
    }

    fn executor() -> RuntimeExecutor {
        RuntimeExecutor {
            backend: GuiActionBackendStatus::available("fixture_executor"),
        }
    }

    fn turn_request() -> GuiTurnRequest {
        GuiTurnRequest {
            session_id: "session-27".into(),
            turn_id: "turn-27".into(),
            workflow_id: "workflow-27".into(),
            message: PROMPT.into(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            hitl_decision_fixture: None,
            execution_environment: Default::default(),
            execution_mode: GuiExecutionMode::SafetyOnly,
            workflow_enabled: false,
            resume_checkpoint: None,
            resume_reason: None,
        }
    }

    fn event_types(outcome: &GuiTurnOutcome) -> Vec<String> {
        outcome
            .events
            .iter()
            .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
            .map(str::to_string)
            .collect()
    }

    /// Flag OFF: a rejected first attempt falls back deterministically with NO
    /// repair-retry — single planner call, exactly the prior Step 1–12 behavior.
    #[tokio::test]
    async fn repair_retry_not_taken_when_flag_off() {
        let perception = perception();
        let executor = executor();
        let planner = SequencedFixtureGuiLlmPlanner::new(vec![
            GuiLlmPlannerFixture::InvalidJson,
            GuiLlmPlannerFixture::ValidPlan,
        ]);

        let runtime = GuiCognitionRuntime::new(&perception, &executor)
            .with_llm_planner(Some(&planner as &dyn GuiLlmPlanner))
            .with_smart_planner(GuiSmartPlannerConfig::disabled());

        let outcome = runtime.run_turn(turn_request()).await;
        let types = event_types(&outcome);

        assert_eq!(planner.call_count(), 1, "flag OFF must use a single attempt");
        assert_eq!(
            planner.repair_call_count(),
            0,
            "flag OFF must NOT perform a repair-retry"
        );
        assert!(
            !types.iter().any(|t| t == "LlmPlanRepairRetry"),
            "no repair-retry event when the flag is OFF; events: {types:?}"
        );
        assert!(
            types.iter().any(|t| t == "LlmPlanningFailed"),
            "the rejected first attempt is reported; events: {types:?}"
        );
    }

    /// Flag ON, repaired success: first rejected → exactly ONE repair-retry → the
    /// repaired plan is accepted (no post-repair failure event).
    #[tokio::test]
    async fn repair_retry_runs_once_and_accepts_repaired_plan_when_flag_on() {
        let perception = perception();
        let executor = executor();
        let planner = SequencedFixtureGuiLlmPlanner::new(vec![
            GuiLlmPlannerFixture::InvalidJson,
            GuiLlmPlannerFixture::ValidPlan,
        ]);

        let runtime = GuiCognitionRuntime::new(&perception, &executor)
            .with_llm_planner(Some(&planner as &dyn GuiLlmPlanner))
            .with_smart_planner(GuiSmartPlannerConfig::enabled());

        let outcome = runtime.run_turn(turn_request()).await;
        let types = event_types(&outcome);

        assert_eq!(planner.call_count(), 2, "first attempt + one repair-retry");
        assert_eq!(
            planner.repair_call_count(),
            1,
            "exactly ONE repair-retry (KRIA authority: no uncontrolled retries)"
        );
        assert!(
            types.iter().any(|t| t == "LlmPlanRepairRetry"),
            "a repair-retry event is emitted when the flag is ON; events: {types:?}"
        );
        // Repaired acceptance: there must be NO post-repair LlmPlanningFailed.
        assert!(
            !outcome.events.iter().any(|event| {
                event.get("type").and_then(serde_json::Value::as_str) == Some("LlmPlanningFailed")
                    && event.get("after_repair_retry").and_then(serde_json::Value::as_bool)
                        == Some(true)
            }),
            "repaired plan must be accepted (no post-repair failure); events: {types:?}"
        );
    }

    /// Flag ON, repair also fails: first rejected → ONE repair-retry → still
    /// rejected → deterministic fallback. The repair runs AT MOST once (never a
    /// second repair) and a post-repair failure is reported before falling back.
    #[tokio::test]
    async fn repair_retry_runs_at_most_once_then_falls_back_when_flag_on() {
        let perception = perception();
        let executor = executor();
        let planner = SequencedFixtureGuiLlmPlanner::new(vec![
            GuiLlmPlannerFixture::InvalidJson,
            GuiLlmPlannerFixture::InvalidJson,
            GuiLlmPlannerFixture::InvalidJson,
        ]);

        let runtime = GuiCognitionRuntime::new(&perception, &executor)
            .with_llm_planner(Some(&planner as &dyn GuiLlmPlanner))
            .with_smart_planner(GuiSmartPlannerConfig::enabled());

        let outcome = runtime.run_turn(turn_request()).await;
        let types = event_types(&outcome);

        assert_eq!(
            planner.call_count(),
            2,
            "first attempt + exactly one repair-retry, then deterministic fallback"
        );
        assert_eq!(
            planner.repair_call_count(),
            1,
            "AT MOST one repair-retry — never a second repair attempt"
        );
        assert!(
            types.iter().any(|t| t == "LlmPlanRepairRetry"),
            "a repair-retry event is emitted; events: {types:?}"
        );
        assert!(
            outcome.events.iter().any(|event| {
                event.get("type").and_then(serde_json::Value::as_str) == Some("LlmPlanningFailed")
                    && event.get("after_repair_retry").and_then(serde_json::Value::as_bool)
                        == Some(true)
            }),
            "the failed repair is reported before falling back; events: {types:?}"
        );
        // The deterministic fallback still yields a usable plan (KRIA always
        // produces a valid plan; the fallback is the safety net).
        let plan_created = outcome
            .events
            .iter()
            .find(|event| {
                event.get("type").and_then(serde_json::Value::as_str) == Some("PlanCreated")
            })
            .expect("a PlanCreated event is emitted after fallback");
        assert!(
            plan_created
                .get("step_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
                >= 1,
            "deterministic fallback plan must contain at least one step"
        );
    }

    // ── Task 0 (Requirement 0.4): structured planner — bounded re-ask ≤ 2 ──────
    use kria_core::agent::gui_cognition::llm_planner::GuiStructuredPlannerConfig;

    /// `gui_cog_structured_planner` ON: a first rejection followed by a second
    /// rejection is re-asked TWICE (each feeding the validation error back), and
    /// the second repair's valid plan is accepted. Proves the bound is 2 and the
    /// error is fed back across both re-asks.
    #[tokio::test]
    async fn structured_flag_allows_two_reasks_then_accepts() {
        let perception = perception();
        let executor = executor();
        let planner = SequencedFixtureGuiLlmPlanner::new(vec![
            GuiLlmPlannerFixture::InvalidJson,
            GuiLlmPlannerFixture::InvalidJson,
            GuiLlmPlannerFixture::ValidPlan,
        ]);

        let runtime = GuiCognitionRuntime::new(&perception, &executor)
            .with_llm_planner(Some(&planner as &dyn GuiLlmPlanner))
            .with_structured_planner(GuiStructuredPlannerConfig::enabled());

        let outcome = runtime.run_turn(turn_request()).await;
        let types = event_types(&outcome);

        assert_eq!(
            planner.call_count(),
            3,
            "first attempt + up to TWO re-asks under the structured flag"
        );
        assert_eq!(
            planner.repair_call_count(),
            2,
            "exactly TWO re-asks, each feeding the validation error back"
        );
        assert_eq!(
            outcome.response["gui_cognition"]["planner"]["mode"], "llm_schema",
            "the second repaired plan is accepted as an LLM plan"
        );
        assert!(
            !outcome.events.iter().any(|event| {
                event.get("type").and_then(serde_json::Value::as_str) == Some("LlmPlanningFailed")
                    && event
                        .get("after_repair_retry")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
            }),
            "the accepted repaired plan emits no post-repair failure; events: {types:?}"
        );
    }

    /// `gui_cog_structured_planner` ON: re-asks are bounded at 2. Four invalid
    /// responses → first attempt + exactly 2 re-asks → deterministic fallback. A
    /// fourth (valid) response is queued but MUST never be consumed.
    #[tokio::test]
    async fn structured_flag_bounds_reasks_at_two_then_falls_back() {
        let perception = perception();
        let executor = executor();
        let planner = SequencedFixtureGuiLlmPlanner::new(vec![
            GuiLlmPlannerFixture::InvalidJson,
            GuiLlmPlannerFixture::InvalidJson,
            GuiLlmPlannerFixture::InvalidJson,
            GuiLlmPlannerFixture::ValidPlan,
        ]);

        let runtime = GuiCognitionRuntime::new(&perception, &executor)
            .with_llm_planner(Some(&planner as &dyn GuiLlmPlanner))
            .with_structured_planner(GuiStructuredPlannerConfig::enabled());

        let outcome = runtime.run_turn(turn_request()).await;
        let types = event_types(&outcome);

        assert_eq!(
            planner.call_count(),
            3,
            "bounded to first attempt + at most TWO re-asks (never a third)"
        );
        assert_eq!(planner.repair_call_count(), 2, "at most TWO re-asks");
        assert_eq!(
            outcome.response["gui_cognition"]["planner"]["llm_status"], "rejected_after_repair",
            "exhausted re-ask budget falls back deterministically"
        );
        assert!(
            outcome.events.iter().any(|event| {
                event.get("type").and_then(serde_json::Value::as_str) == Some("LlmPlanningFailed")
                    && event
                        .get("after_repair_retry")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
            }),
            "the exhausted re-ask is reported before falling back; events: {types:?}"
        );
    }

    /// `gui_cog_structured_planner` OFF (and smart OFF): single attempt, NO
    /// re-ask — prior behavior byte-for-byte even though a valid repair response
    /// is queued.
    #[tokio::test]
    async fn structured_flag_off_is_single_attempt() {
        let perception = perception();
        let executor = executor();
        let planner = SequencedFixtureGuiLlmPlanner::new(vec![
            GuiLlmPlannerFixture::InvalidJson,
            GuiLlmPlannerFixture::ValidPlan,
        ]);

        let runtime = GuiCognitionRuntime::new(&perception, &executor)
            .with_llm_planner(Some(&planner as &dyn GuiLlmPlanner))
            .with_structured_planner(GuiStructuredPlannerConfig::disabled())
            .with_smart_planner(GuiSmartPlannerConfig::disabled());

        let outcome = runtime.run_turn(turn_request()).await;
        let types = event_types(&outcome);

        assert_eq!(planner.call_count(), 1, "flag OFF must use a single attempt");
        assert_eq!(planner.repair_call_count(), 0, "flag OFF must NOT re-ask");
        assert!(
            !types.iter().any(|t| t == "LlmPlanRepairRetry"),
            "no re-ask event when both flags are OFF; events: {types:?}"
        );
    }

    // ── Task 0.9 / 0.10: Planner Capability Ladder + honest layman notice ──────
    use kria_core::agent::gui_cognition::llm_planner::{
        GuiLlmPlannerError, GuiLlmPlannerRawResponse,
    };
    use kria_core::llm::StructuredOutputMode;
    use std::sync::atomic::AtomicUsize;

    /// A planner that delegates plan CONTENT to an inner `FixtureGuiLlmPlanner`,
    /// counts how many times it was invoked, and reports a CONFIGURABLE
    /// capability (label + structured mode). This lets the ladder's
    /// "grammar-capable + different backend" gate and the call-count assertions
    /// be exercised deterministically with no network.
    struct CountingLadderPlanner {
        inner: FixtureGuiLlmPlanner,
        calls: AtomicUsize,
        capability: GuiPlannerCapability,
    }

    impl CountingLadderPlanner {
        fn new(fixture: GuiLlmPlannerFixture, capability: GuiPlannerCapability) -> Self {
            Self {
                inner: FixtureGuiLlmPlanner::new(fixture),
                calls: AtomicUsize::new(0),
                capability,
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl GuiLlmPlanner for CountingLadderPlanner {
        async fn plan(
            &self,
            request: GuiLlmPlannerRequest,
        ) -> Result<GuiLlmPlannerRawResponse, GuiLlmPlannerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inner.plan(request).await
        }

        fn capability(&self) -> GuiPlannerCapability {
            self.capability.clone()
        }
    }

    /// Rung B: the configured planner is strictly rejected (after the bounded
    /// re-ask) and a grammar-capable LOCAL planner returns a valid plan ⇒ the
    /// FINAL plan is the local one, `ladder_rung = local_grammar_fallback`, and
    /// NO capability notice is emitted (an LLM rung produced the plan).
    #[tokio::test]
    async fn ladder_rung_b_uses_local_grammar_fallback_when_configured_rejected() {
        let perception = perception();
        let executor = executor();
        // Configured (e.g. cloud json_object) keeps producing invalid JSON.
        let configured = CountingLadderPlanner::new(
            GuiLlmPlannerFixture::InvalidJson,
            GuiPlannerCapability::structured_validated(
                "cloud-deepseek-v4-flash",
                StructuredOutputMode::JsonObject,
            ),
        );
        // Local grammar backend posts a real constraint → valid plan.
        let local = CountingLadderPlanner::new(
            GuiLlmPlannerFixture::ValidPlan,
            GuiPlannerCapability::validated("qwen-local-grammar"),
        );

        let runtime = GuiCognitionRuntime::new(&perception, &executor)
            .with_llm_planner(Some(&configured as &dyn GuiLlmPlanner))
            .with_local_grammar_planner(Some(&local as &dyn GuiLlmPlanner))
            .with_structured_planner(GuiStructuredPlannerConfig::enabled());

        let outcome = runtime.run_turn(turn_request()).await;
        let planner = &outcome.response["gui_cognition"]["planner"];

        assert_eq!(
            planner["ladder_rung"], "local_grammar_fallback",
            "the local grammar rung produced the final plan; planner: {planner}"
        );
        assert_eq!(
            planner["mode"], "llm_schema",
            "the local grammar plan is the accepted LLM plan"
        );
        assert!(
            planner.get("capability_notice").map_or(true, |v| v.is_null()),
            "no capability notice when an LLM rung produced the plan; planner: {planner}"
        );
        assert!(local.calls() >= 1, "local grammar planner must be consulted");
        assert!(
            !event_types(&outcome)
                .iter()
                .any(|t| t == "PlannerCapabilityNotice"),
            "no capability-notice event when the local rung succeeded"
        );
    }

    /// Rung C + notice: the configured planner is strictly rejected and NO local
    /// grammar backend is available ⇒ deterministic fallback, `ladder_rung =
    /// deterministic`, and an honest layman `capability_notice` is emitted (text
    /// mentions switching model; carries no hash/id/secret).
    #[tokio::test]
    async fn ladder_rung_c_emits_capability_notice_when_no_local_grammar_backend() {
        let perception = perception();
        let executor = executor();
        let configured = CountingLadderPlanner::new(
            GuiLlmPlannerFixture::InvalidJson,
            GuiPlannerCapability::structured_validated(
                "cloud-deepseek-v4-flash",
                StructuredOutputMode::JsonObject,
            ),
        );

        let runtime = GuiCognitionRuntime::new(&perception, &executor)
            .with_llm_planner(Some(&configured as &dyn GuiLlmPlanner))
            // No local grammar planner wired.
            .with_structured_planner(GuiStructuredPlannerConfig::enabled());

        let outcome = runtime.run_turn(turn_request()).await;
        let planner = &outcome.response["gui_cognition"]["planner"];

        assert_eq!(
            planner["ladder_rung"], "deterministic",
            "no LLM rung succeeded → deterministic; planner: {planner}"
        );
        assert_eq!(
            planner["mode"], "llm_rejected_fallback",
            "the configured LLM was rejected and drove the deterministic fallback"
        );
        let notice = &planner["capability_notice"];
        let message = notice["message"]
            .as_str()
            .expect("capability notice carries a message");
        assert!(
            message.contains("switch"),
            "layman text advises switching model: {message}"
        );
        assert!(
            message.contains("Settings → Model"),
            "layman text points to Settings → Model: {message}"
        );
        // Sanitized: a fixed layman string carries no numeric ids/hashes/secrets.
        assert!(
            !message.chars().any(|c| c.is_ascii_digit()),
            "sanitized layman notice carries no numeric ids/hashes: {message}"
        );
        let lower = message.to_ascii_lowercase();
        assert!(
            !lower.contains("hash")
                && !lower.contains("token")
                && !lower.contains("secret")
                && !lower.contains("api"),
            "notice must not surface hashes/ids/secrets: {message}"
        );
        assert!(
            event_types(&outcome)
                .iter()
                .any(|t| t == "PlannerCapabilityNotice"),
            "the capability notice is mirrored as an event"
        );
    }

    /// Happy path: the configured planner returns a valid plan ⇒ `ladder_rung =
    /// configured_llm`, no notice, and the local planner is NEVER consulted.
    #[tokio::test]
    async fn ladder_rung_a_configured_llm_does_not_consult_local() {
        let perception = perception();
        let executor = executor();
        let configured = CountingLadderPlanner::new(
            GuiLlmPlannerFixture::ValidPlan,
            GuiPlannerCapability::structured_validated(
                "cloud-capable",
                StructuredOutputMode::JsonObject,
            ),
        );
        let local = CountingLadderPlanner::new(
            GuiLlmPlannerFixture::ValidPlan,
            GuiPlannerCapability::validated("qwen-local-grammar"),
        );

        let runtime = GuiCognitionRuntime::new(&perception, &executor)
            .with_llm_planner(Some(&configured as &dyn GuiLlmPlanner))
            .with_local_grammar_planner(Some(&local as &dyn GuiLlmPlanner))
            .with_structured_planner(GuiStructuredPlannerConfig::enabled());

        let outcome = runtime.run_turn(turn_request()).await;
        let planner = &outcome.response["gui_cognition"]["planner"];

        assert_eq!(planner["ladder_rung"], "configured_llm");
        assert_eq!(planner["mode"], "llm_schema");
        assert!(
            planner.get("capability_notice").map_or(true, |v| v.is_null()),
            "no notice on the happy path; planner: {planner}"
        );
        assert_eq!(
            local.calls(),
            0,
            "the local fallback planner is NOT called when the configured plan is valid"
        );
    }

    /// Flag OFF: the ladder/notice do not run; no `ladder_rung`/`capability_notice`
    /// telemetry appears and the local fallback is never consulted (byte-for-byte
    /// prior selection behavior).
    #[tokio::test]
    async fn ladder_does_not_run_when_structured_flag_off() {
        let perception = perception();
        let executor = executor();
        let configured = CountingLadderPlanner::new(
            GuiLlmPlannerFixture::ValidPlan,
            GuiPlannerCapability::validated("configured"),
        );
        let local = CountingLadderPlanner::new(
            GuiLlmPlannerFixture::ValidPlan,
            GuiPlannerCapability::validated("qwen-local-grammar"),
        );

        let runtime = GuiCognitionRuntime::new(&perception, &executor)
            .with_llm_planner(Some(&configured as &dyn GuiLlmPlanner))
            .with_local_grammar_planner(Some(&local as &dyn GuiLlmPlanner))
            .with_structured_planner(GuiStructuredPlannerConfig::disabled())
            .with_smart_planner(GuiSmartPlannerConfig::disabled());

        let outcome = runtime.run_turn(turn_request()).await;
        let planner = &outcome.response["gui_cognition"]["planner"];

        assert!(
            planner.get("ladder_rung").map_or(true, |v| v.is_null()),
            "flag OFF: no ladder_rung telemetry; planner: {planner}"
        );
        assert!(
            planner.get("capability_notice").map_or(true, |v| v.is_null()),
            "flag OFF: no capability_notice telemetry; planner: {planner}"
        );
        assert_eq!(
            local.calls(),
            0,
            "flag OFF: the local fallback planner is never consulted"
        );
        assert!(
            !event_types(&outcome)
                .iter()
                .any(|t| t == "PlannerCapabilityNotice"),
            "flag OFF: no capability-notice event"
        );
    }
}

// ── Task 0 (Requirement 0.3): structured-capability mapping in the planner ────

/// A backend whose structured-output mode is configurable, used to exercise the
/// `gui_cog_structured_planner` capability mapping.
struct StructuredCapabilityBackend {
    mode: kria_core::llm::StructuredOutputMode,
}

#[async_trait]
impl LlmBackend for StructuredCapabilityBackend {
    fn model_label(&self) -> &str {
        "structured-fixture"
    }

    fn capabilities(&self) -> &[String] {
        &[]
    }

    fn is_configured(&self) -> bool {
        true
    }

    fn structured_output_mode(&self) -> kria_core::llm::StructuredOutputMode {
        self.mode
    }

    async fn chat(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[ToolSchema]>,
        _temperature: f32,
        _max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        anyhow::bail!("not used")
    }

    async fn chat_stream(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[ToolSchema]>,
        _temperature: f32,
        _max_tokens: u32,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = String> + Send>>> {
        Ok(Box::pin(futures::stream::empty()))
    }

    async fn health_check(&self) -> bool {
        true
    }
}

#[test]
fn structured_planner_reports_validated_for_json_object_mode() {
    use kria_core::agent::gui_cognition::llm_planner::GuiStructuredPlannerConfig;
    use kria_core::llm::StructuredOutputMode;

    let backend = Arc::new(StructuredCapabilityBackend {
        mode: StructuredOutputMode::JsonObject,
    });
    let planner = LlmBackendGuiPlanner::new(backend)
        .with_structured_config(GuiStructuredPlannerConfig::enabled());
    let capability = planner.capability();

    assert_eq!(capability.status, "capability_validated");
    assert!(capability.is_structured_capable());
    // json_object is NOT grammar, so the narrower grammar signal stays false.
    assert!(!capability.supports_grammar);
    assert_eq!(capability.structured_mode, "json_object");
}

#[test]
fn structured_planner_reports_validated_for_tool_calling_mode() {
    use kria_core::agent::gui_cognition::llm_planner::GuiStructuredPlannerConfig;
    use kria_core::llm::StructuredOutputMode;

    let backend = Arc::new(StructuredCapabilityBackend {
        mode: StructuredOutputMode::ToolCalling,
    });
    let planner = LlmBackendGuiPlanner::new(backend)
        .with_structured_config(GuiStructuredPlannerConfig::enabled());
    let capability = planner.capability();

    assert_eq!(capability.status, "capability_validated");
    assert!(capability.is_structured_capable());
    assert_eq!(capability.structured_mode, "tool_calling");
}

#[test]
fn structured_planner_reports_not_structured_capable_when_no_mode() {
    use kria_core::agent::gui_cognition::llm_planner::GuiStructuredPlannerConfig;
    use kria_core::llm::StructuredOutputMode;

    let backend = Arc::new(StructuredCapabilityBackend {
        mode: StructuredOutputMode::None,
    });
    let planner = LlmBackendGuiPlanner::new(backend)
        .with_structured_config(GuiStructuredPlannerConfig::enabled());
    let capability = planner.capability();

    assert_eq!(capability.status, "not_structured_capable");
    assert!(!capability.is_structured_capable());
    assert!(!capability.is_grammar_capable());
}

#[test]
fn structured_flag_off_preserves_legacy_grammar_capability_mapping() {
    use kria_core::llm::StructuredOutputMode;

    // With the structured flag OFF (default), a json_object backend that does
    // NOT support grammar maps to the legacy `not_grammar_capable` — byte-for-
    // byte prior behavior (Requirement 0.6).
    let backend = Arc::new(StructuredCapabilityBackend {
        mode: StructuredOutputMode::JsonObject,
    });
    let planner = LlmBackendGuiPlanner::new(backend);
    let capability = planner.capability();

    assert_eq!(capability.status, "not_grammar_capable");
    assert!(!capability.is_grammar_capable());
}

#[test]
fn local_grammar_mode_maps_to_supports_grammar_backcompat() {
    use kria_core::llm::StructuredOutputMode;

    // Grammar mode derives supports_grammar()==true via the trait default, and
    // the legacy (flag-OFF) capability mapping reports `capability_validated`.
    let backend = Arc::new(StructuredCapabilityBackend {
        mode: StructuredOutputMode::Grammar,
    });
    assert!(backend.supports_grammar());
    let planner = LlmBackendGuiPlanner::new(backend);
    assert_eq!(planner.capability().status, "capability_validated");
}

// ── Task 2 (Issue #3): auto-prerequisite for bare-primitive plans ──────────────
mod auto_prereq_tests {
    use kria_core::agent::gui_cognition::goal_contract::extract_gui_goal_contract;
    use kria_core::agent::gui_cognition::llm_planner::{
        apply_auto_prerequisite, AppObservability, AutoPrereqOutcome, GuiAutoPrereqConfig,
        GuiLlmPlan, GuiTypedPlanStep, AUTO_PREREQ_ENV_FLAG,
    };

    /// A bare typed step with an OPTIONAL app hint, mirroring a single primitive
    /// the user issued with no app prerequisite ("scroll down", "type hello").
    fn typed(step_type: &str, app_hint: Option<&str>, control: Option<&str>) -> GuiTypedPlanStep {
        GuiTypedPlanStep {
            step_id: format!("step-{step_type}"),
            step_type: step_type.into(),
            summary: format!("{step_type} bare primitive"),
            target_app_hint: app_hint.map(str::to_string),
            target_window_hint: None,
            target_control_hint: control.map(str::to_string),
            text_payload_summary: None,
            text_payload_hash: None,
            expected_precondition: "context observed".into(),
            expected_postcondition: "primitive applied".into(),
            verification_strategy: "screen_changed".into(),
            risk_level: "low".into(),
            requires_approval: false,
            idempotent: kria_core::agent::gui_cognition::default_idempotent_for(step_type),
            allowed_to_execute: false,
            confidence: 0.9,
            reason: "test".into(),
        }
    }

    fn plan(steps: Vec<GuiTypedPlanStep>) -> GuiLlmPlan {
        GuiLlmPlan {
            plan_id: Some("plan-prereq".into()),
            goal_contract_id: Some("goal-prereq".into()),
            observation_id: Some("obs-prereq".into()),
            context_id: Some("ctx-prereq".into()),
            prompt_hash: Some("prompt-hash".into()),
            goal_action_type: Some("scroll".into()),
            plan_status: Some("valid".into()),
            planner_mode: "deterministic".into(),
            plan_summary: "auto-prereq test plan".into(),
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

    /// A bare primitive whose target app is NOT present at all gets an OpenApp
    /// prerequisite PREPENDED in front of the primitive (Requirement 2). The
    /// prerequisite carries the inferred app via `target_app_hint`, uses the
    /// `window_visible` strategy, and is never auto-executed/approved.
    #[test]
    fn bare_scroll_with_app_hint_not_present_prepends_open_app() {
        let contract = extract_gui_goal_contract("scroll down", None).contract;
        let mut p = plan(vec![typed("Scroll", Some("Calculator"), None)]);

        let outcome =
            apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::NotPresent);

        assert_eq!(
            outcome,
            AutoPrereqOutcome::PrependedOpenApp("Calculator".into())
        );
        assert_eq!(p.typed_steps.len(), 2);
        let prereq = &p.typed_steps[0];
        assert_eq!(prereq.step_type, "OpenApp");
        assert_eq!(prereq.target_app_hint.as_deref(), Some("Calculator"));
        assert_eq!(prereq.verification_strategy, "window_visible");
        assert!(!prereq.allowed_to_execute);
        assert!(!prereq.requires_approval);
        // The original primitive is preserved AFTER the prerequisite.
        assert_eq!(p.typed_steps[1].step_type, "Scroll");
    }

    /// A standalone TypeText whose target app is VISIBLE but not focused gets a
    /// SwitchWindow prerequisite (window + app hint set) PREPENDED.
    #[test]
    fn bare_type_text_with_visible_app_prepends_switch_window() {
        let contract = extract_gui_goal_contract("type hello", None).contract;
        let mut p = plan(vec![typed("TypeText", Some("gedit"), Some("the field"))]);

        let outcome =
            apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::VisibleNotActive);

        assert_eq!(
            outcome,
            AutoPrereqOutcome::PrependedSwitchWindow("gedit".into())
        );
        assert_eq!(p.typed_steps.len(), 2);
        let prereq = &p.typed_steps[0];
        assert_eq!(prereq.step_type, "SwitchWindow");
        assert_eq!(prereq.target_app_hint.as_deref(), Some("gedit"));
        assert_eq!(prereq.target_window_hint.as_deref(), Some("gedit"));
        assert_eq!(prereq.verification_strategy, "window_visible");
        assert!(!prereq.allowed_to_execute);
    }

    /// When neither the step nor the contract names an app but the contract
    /// carries an app KIND, the generic label is inferred (editor → "text
    /// editor").
    #[test]
    fn bare_primitive_infers_generic_label_from_app_kind() {
        let mut contract = extract_gui_goal_contract("scroll down", None).contract;
        contract.target_app_hint = None;
        contract.target_app_kind = Some("editor".into());
        let mut p = plan(vec![typed("Scroll", None, None)]);

        let outcome =
            apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::NotPresent);

        assert_eq!(
            outcome,
            AutoPrereqOutcome::PrependedOpenApp("text editor".into())
        );
        assert_eq!(p.typed_steps[0].step_type, "OpenApp");
        assert_eq!(p.typed_steps[0].target_app_hint.as_deref(), Some("text editor"));
    }

    /// A bare primitive whose target app IS already the active window gets NO
    /// prerequisite (no extra step).
    #[test]
    fn bare_primitive_in_active_app_is_noop() {
        let contract = extract_gui_goal_contract("scroll down", None).contract;
        let mut p = plan(vec![typed("Scroll", Some("Calculator"), None)]);

        let outcome = apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::Active);

        assert_eq!(outcome, AutoPrereqOutcome::NoOp);
        assert_eq!(p.typed_steps.len(), 1);
        assert_eq!(p.typed_steps[0].step_type, "Scroll");
    }

    /// A plan that already starts with OpenApp/SwitchWindow is left unchanged
    /// (app-launch / multi-step plans are not double-prefixed).
    #[test]
    fn plan_with_existing_open_app_is_unchanged() {
        let contract = extract_gui_goal_contract("open chrome and search", None).contract;
        let open = typed("OpenApp", Some("Chrome"), None);
        let focus = typed("FocusField", Some("Chrome"), Some("address/search field"));
        let mut p = plan(vec![open, focus]);
        let before = p.typed_steps.clone();

        // Even with a NotPresent observe, an existing OpenApp suppresses prepend.
        let outcome =
            apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::NotPresent);

        assert_eq!(outcome, AutoPrereqOutcome::NoOp);
        assert_eq!(p.typed_steps.len(), before.len());
        assert_eq!(p.typed_steps[0].step_type, "OpenApp");
        assert_eq!(p.typed_steps[1].step_type, "FocusField");
    }

    /// A bare primitive with NO inferable app (no step hint, no contract hint,
    /// no kind) → the plan is replaced with a single AskClarification step and
    /// has NO executable step.
    #[test]
    fn bare_primitive_without_inferable_app_clarifies() {
        let mut contract = extract_gui_goal_contract("scroll down", None).contract;
        contract.target_app_hint = None;
        contract.target_app_kind = None;
        let mut p = plan(vec![typed("Scroll", None, None)]);

        let outcome =
            apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::NotPresent);

        assert_eq!(outcome, AutoPrereqOutcome::Clarified);
        assert_eq!(p.typed_steps.len(), 1);
        assert_eq!(p.typed_steps[0].step_type, "AskClarification");
        assert!(p
            .typed_steps
            .iter()
            .all(|step| step.step_type != "Scroll" && !step.allowed_to_execute));
        assert!(p.clarification_question.is_some());
    }

    /// A leading approval gate stays FIRST; the prerequisite is inserted AFTER
    /// the RequireApproval (+ its approval WaitForState) so order is sane.
    #[test]
    fn prerequisite_inserted_after_leading_approval_gate() {
        let contract = extract_gui_goal_contract("scroll down", None).contract;
        let mut approval = typed("RequireApproval", None, None);
        approval.verification_strategy = "approval_pending".into();
        let mut wait = typed("WaitForState", None, None);
        wait.verification_strategy = "approval_pending".into();
        let mut p = plan(vec![approval, wait, typed("Scroll", Some("Calculator"), None)]);

        let outcome =
            apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::NotPresent);

        assert!(matches!(outcome, AutoPrereqOutcome::PrependedOpenApp(_)));
        assert_eq!(p.typed_steps[0].step_type, "RequireApproval");
        assert_eq!(p.typed_steps[1].step_type, "WaitForState");
        assert_eq!(p.typed_steps[2].step_type, "OpenApp");
        assert_eq!(p.typed_steps[3].step_type, "Scroll");
    }

    /// Flag-OFF (gui_cog_auto_prereq disabled) → the runtime never calls the
    /// pass, so the plan is byte-for-byte identical. This test models the exact
    /// runtime gate (`if cfg.is_enabled() { apply_auto_prerequisite(..) }`).
    #[test]
    fn flag_off_leaves_plan_byte_for_byte_unchanged() {
        let contract = extract_gui_goal_contract("scroll down", None).contract;
        let original = plan(vec![typed("Scroll", Some("Calculator"), None)]);
        let mut p = original.clone();

        let cfg = GuiAutoPrereqConfig::disabled();
        assert!(!cfg.is_enabled());
        if cfg.is_enabled() {
            apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::NotPresent);
        }

        let before = serde_json::to_string(&original.typed_steps).unwrap();
        let after = serde_json::to_string(&p.typed_steps).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn auto_prereq_config_default_off_and_env_lookup() {
        // Default OFF.
        assert!(!GuiAutoPrereqConfig::default().is_enabled());
        assert!(!GuiAutoPrereqConfig::disabled().is_enabled());
        assert!(GuiAutoPrereqConfig::enabled().is_enabled());

        // Unset env → OFF for from_env_lookup.
        assert!(!GuiAutoPrereqConfig::from_env_lookup(|_| None).is_enabled());
        for value in ["1", "true", "YES", "on", " On "] {
            let cfg = GuiAutoPrereqConfig::from_env_lookup(|key| {
                (key == AUTO_PREREQ_ENV_FLAG).then(|| value.to_string())
            });
            assert!(cfg.is_enabled(), "expected ON for {value:?}");
        }
        for value in ["0", "false", "no", "off", "", "maybe"] {
            let cfg = GuiAutoPrereqConfig::from_env_lookup(|key| {
                (key == AUTO_PREREQ_ENV_FLAG).then(|| value.to_string())
            });
            assert!(!cfg.is_enabled(), "expected OFF for {value:?}");
        }

        // Default-ON path: ON unless explicitly falsy (rollback switch).
        assert!(GuiAutoPrereqConfig::from_env_lookup_default_on(|_| None).is_enabled());
        for value in ["0", "false", "no", "off", "", " OFF "] {
            let cfg = GuiAutoPrereqConfig::from_env_lookup_default_on(|key| {
                (key == AUTO_PREREQ_ENV_FLAG).then(|| value.to_string())
            });
            assert!(!cfg.is_enabled(), "expected rollback OFF for {value:?}");
        }
        for value in ["1", "true", "anything-else"] {
            let cfg = GuiAutoPrereqConfig::from_env_lookup_default_on(|key| {
                (key == AUTO_PREREQ_ENV_FLAG).then(|| value.to_string())
            });
            assert!(cfg.is_enabled(), "expected ON for {value:?}");
        }
    }

    // ── Case B (Task 2 refinement): clarification-collapsed plans ──────────────
    //
    // The real live failures were plans that COLLAPSED to a single
    // AskClarification for app-named bare primitives, so case A's prepend never
    // fired. Case B converts such a clarification into OpenApp/SwitchWindow + the
    // contract's primitive, but ONLY when an app is inferable and (for text
    // primitives) a text payload exists.
    use kria_core::agent::gui_cognition::goal_contract::GuiActionType;

    /// A single-AskClarification plan, mirroring a plan that collapsed because
    /// the deterministic/LLM planner clarified instead of opening the app.
    fn clarification_plan() -> GuiLlmPlan {
        let mut step = typed("AskClarification", None, None);
        step.verification_strategy = "clarification_requested".into();
        step.expected_postcondition = "clarification is requested".into();
        let mut p = plan(vec![step]);
        p.clarification_question = Some("Which application should I act in?".into());
        p
    }

    /// "Open the text editor and type hello world": a clarification-collapsed
    /// TypeText with app kind=editor + a text payload but NO control hint →
    /// converts to [OpenApp(text editor), FocusField/TypeText("visible text
    /// input"), VerifyState]. The TypeText primitive is present, carries the
    /// generic control + the payload, and stays non-executable/un-approved.
    #[test]
    fn clarification_typetext_with_app_and_payload_converts_to_open_app_plus_primitive() {
        let mut contract = extract_gui_goal_contract("open the text editor and type hello world", None).contract;
        contract.action_type = GuiActionType::TypeText;
        contract.target_app_hint = None;
        contract.target_app_kind = Some("editor".into());
        contract.target_control_hint = None;
        contract.text_payload_summary = Some("hello world".into());
        let mut p = clarification_plan();

        let outcome =
            apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::NotPresent);

        assert_eq!(
            outcome,
            AutoPrereqOutcome::ConvertedClarification("text editor".into())
        );
        // No clarification remains; the app prerequisite leads.
        assert!(p.clarification_question.is_none());
        assert_eq!(p.typed_steps[0].step_type, "OpenApp");
        assert_eq!(p.typed_steps[0].target_app_hint.as_deref(), Some("text editor"));
        assert_eq!(p.typed_steps[0].verification_strategy, "window_visible");
        // The TypeText primitive is present with the generic control + payload.
        let type_step = p
            .typed_steps
            .iter()
            .find(|s| s.step_type == "TypeText")
            .expect("a TypeText primitive is emitted");
        assert_eq!(type_step.target_control_hint.as_deref(), Some("visible text input"));
        assert!(type_step.text_payload_summary.is_some());
        // No step is auto-executable or auto-approved; no AskClarification left.
        assert!(p.typed_steps.iter().all(|s| !s.allowed_to_execute));
        assert!(p.typed_steps.iter().all(|s| s.step_type != "AskClarification"));
    }

    /// "Focus the address bar in the browser": a clarification-collapsed
    /// FocusInput with app kind=browser + control "address bar" → converts to
    /// [OpenApp(browser), FocusField("address bar"), VerifyState].
    #[test]
    fn clarification_focusinput_in_browser_converts_to_open_app_plus_focus() {
        let mut contract = extract_gui_goal_contract("focus the address bar in the browser", None).contract;
        contract.action_type = GuiActionType::FocusInput;
        contract.target_app_hint = None;
        contract.target_app_kind = Some("browser".into());
        contract.target_control_hint = Some("address bar".into());
        let mut p = clarification_plan();

        let outcome =
            apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::NotPresent);

        assert_eq!(
            outcome,
            AutoPrereqOutcome::ConvertedClarification("browser".into())
        );
        assert_eq!(p.typed_steps[0].step_type, "OpenApp");
        assert_eq!(p.typed_steps[0].target_app_hint.as_deref(), Some("browser"));
        let focus = p
            .typed_steps
            .iter()
            .find(|s| s.step_type == "FocusField")
            .expect("a FocusField primitive is emitted");
        assert_eq!(focus.target_control_hint.as_deref(), Some("address bar"));
        assert!(p.typed_steps.iter().all(|s| !s.allowed_to_execute));
    }

    /// A VISIBLE-but-not-active app uses a SwitchWindow prerequisite instead of
    /// OpenApp (same observability helper as case A).
    #[test]
    fn clarification_focusinput_in_visible_browser_uses_switch_window() {
        let mut contract = extract_gui_goal_contract("focus the address bar in the browser", None).contract;
        contract.action_type = GuiActionType::FocusInput;
        contract.target_app_hint = Some("browser".into());
        contract.target_control_hint = Some("address bar".into());
        let mut p = clarification_plan();

        let outcome =
            apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::VisibleNotActive);

        assert_eq!(
            outcome,
            AutoPrereqOutcome::ConvertedClarification("browser".into())
        );
        assert_eq!(p.typed_steps[0].step_type, "SwitchWindow");
        assert_eq!(p.typed_steps[0].target_window_hint.as_deref(), Some("browser"));
    }

    /// "Open settings and search for sound": a clarification-collapsed
    /// InAppSearch with app kind=settings + a query → converts to
    /// [OpenApp(settings), <in-app search sequence>].
    #[test]
    fn clarification_in_app_search_with_settings_converts_to_open_app_plus_search() {
        let mut contract = extract_gui_goal_contract("open settings and search for sound", None).contract;
        contract.action_type = GuiActionType::InAppSearch;
        contract.target_app_hint = None;
        contract.target_app_kind = Some("settings".into());
        contract.query_summary = Some("sound".into());
        let mut p = clarification_plan();

        let outcome =
            apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::NotPresent);

        assert_eq!(
            outcome,
            AutoPrereqOutcome::ConvertedClarification("settings".into())
        );
        assert_eq!(p.typed_steps[0].step_type, "OpenApp");
        assert_eq!(p.typed_steps[0].target_app_hint.as_deref(), Some("settings"));
        // The in-app search emits an executable primitive (focus + type the query).
        assert!(p
            .typed_steps
            .iter()
            .any(|s| s.step_type == "TypeText" || s.step_type == "FocusField"));
        assert!(p.typed_steps.iter().all(|s| !s.allowed_to_execute));
    }

    /// A TypeText with NO text payload AND no control → the clarification is the
    /// correct ask (the user never said WHAT to type) and is kept unchanged.
    #[test]
    fn clarification_typetext_without_payload_stays_clarification() {
        let mut contract = extract_gui_goal_contract("open the text editor and type", None).contract;
        contract.action_type = GuiActionType::TypeText;
        contract.target_app_hint = None;
        contract.target_app_kind = Some("editor".into());
        contract.target_control_hint = None;
        contract.text_payload_summary = None;
        contract.text_payload_hash = None;
        let mut p = clarification_plan();

        let outcome =
            apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::NotPresent);

        assert_eq!(outcome, AutoPrereqOutcome::NoOp);
        assert_eq!(p.typed_steps.len(), 1);
        assert_eq!(p.typed_steps[0].step_type, "AskClarification");
        assert!(p.clarification_question.is_some());
    }

    /// A primitive (Scroll) with NO inferable app (no hint, no kind) → the
    /// clarification is the correct ask and is kept unchanged.
    #[test]
    fn clarification_primitive_without_inferable_app_stays_clarification() {
        let mut contract = extract_gui_goal_contract("scroll down", None).contract;
        contract.action_type = GuiActionType::Scroll;
        contract.target_app_hint = None;
        contract.target_app_kind = None;
        let mut p = clarification_plan();

        let outcome =
            apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::NotPresent);

        assert_eq!(outcome, AutoPrereqOutcome::NoOp);
        assert_eq!(p.typed_steps.len(), 1);
        assert_eq!(p.typed_steps[0].step_type, "AskClarification");
    }

    /// Flag-OFF → the runtime never calls the pass, so a clarification-collapsed
    /// plan stays a clarification byte-for-byte (models the runtime gate).
    #[test]
    fn flag_off_leaves_clarification_collapsed_plan_unchanged() {
        let mut contract = extract_gui_goal_contract("open the text editor and type hello world", None).contract;
        contract.action_type = GuiActionType::TypeText;
        contract.target_app_kind = Some("editor".into());
        contract.text_payload_summary = Some("hello world".into());
        let original = clarification_plan();
        let mut p = original.clone();

        let cfg = GuiAutoPrereqConfig::disabled();
        assert!(!cfg.is_enabled());
        if cfg.is_enabled() {
            apply_auto_prerequisite(&mut p, &contract, |_app| AppObservability::NotPresent);
        }

        let before = serde_json::to_string(&original.typed_steps).unwrap();
        let after = serde_json::to_string(&p.typed_steps).unwrap();
        assert_eq!(before, after);
        assert_eq!(p.typed_steps[0].step_type, "AskClarification");
    }
}
