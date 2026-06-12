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
            "FocusField",
            "TypeText",
            "PressKey",
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
    assert_eq!(summary["typed_steps"][2]["text_payload_summary"], "KRIA");
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
    assert!(report
        .step_results
        .iter()
        .any(|step| step.step_type == "FocusField" && step.target_resolution_required));
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
