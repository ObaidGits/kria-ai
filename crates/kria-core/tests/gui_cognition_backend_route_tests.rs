use async_trait::async_trait;

use kria_core::agent::gui_cognition::context::GuiContext;
use kria_core::agent::gui_cognition::executor::{
    sanitized_execution_evidence, select_gui_action_backend, GuiActionBackendStatus,
    GuiActionExecution, GuiActionExecutor, GuiActionRequest, GuiBackendProbeInput,
    GuiExecutionMode,
};
use kria_core::agent::gui_cognition::perception::{
    controls_from_probe_result, GuiAccessibilitySummary, GuiActiveWindowSummary, GuiControlSummary,
    GuiCursorFocusSummary, GuiObservationCacheSummary, GuiObservationSnapshot,
    GuiObservationTimingSummary, GuiOcrDiagnostics, GuiPerceptionCapabilities,
    GuiPerceptionProvider, GuiProbeResult, GuiSourceStatus,
};
use kria_core::agent::gui_cognition::planner::{
    classify_gui_cognition_prompt, extract_first_quoted_segment, extract_named_control,
    gui_plan_steps, GuiCognitionIntent, GuiCognitionIntentKind,
};
use kria_core::agent::gui_cognition::recovery::GuiBlocker;
use kria_core::agent::gui_cognition::resolver::{
    resolve_button, resolve_type_text_target, resolve_unique_text_field, TargetResolution,
};
use kria_core::agent::gui_cognition::safety::{safety_for_intent, GuiSafetyStatus};
use kria_core::agent::gui_cognition::execution_environment::GuiExecutionEnvironment;
use kria_core::agent::gui_cognition::safety_hitl::GuiHitlDecisionFixture;
use kria_core::agent::gui_cognition::validator::{validate_intent, GuiValidationStatus};
use kria_core::agent::gui_cognition::verifier::{verify_post_action, GuiSafetyPolishConfig};
use kria_core::agent::gui_cognition::{GuiCognitionRuntime, GuiTurnRequest};
use kria_core::agent::gui_cognition::llm_planner::{
    GuiLlmPlanner, GuiLlmPlannerFixture, GuiSmartPlannerConfig, SequencedFixtureGuiLlmPlanner,
};
fn control(role: &str, name: &str) -> GuiControlSummary {
    let mut control = GuiControlSummary::new(role, name, format!("/root/{role}/{name}"));
    control.bounds = Some(kria_core::agent::gui_cognition::perception::GuiBounds {
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

fn observation(
    active_window: &str,
    text_fields: Vec<GuiControlSummary>,
    buttons: Vec<GuiControlSummary>,
) -> GuiObservationSnapshot {
    GuiObservationSnapshot {
        observation_id: "obs-1".into(),
        context_id: "ctx-1".into(),
        timestamp_ms: 1,
        screen_hash: Some("screen-hash".into()),
        active_window_label: active_window.into(),
        active_window: GuiActiveWindowSummary {
            label: active_window.into(),
            app_name: Some(active_window.into()),
            source: "test".into(),
            confidence: 0.95,
            fallback_used: false,
            blocker: None,
            reliability: "reliable".into(),
            fallback_chain: Vec::new(),
            ..GuiActiveWindowSummary::default()
        },
        visible_windows: Vec::new(),
        visible_app_count: 2,
        monitors: Vec::new(),
        cursor_focus: GuiCursorFocusSummary::default(),
        accessibility: GuiAccessibilitySummary {
            available: true,
            node_count: text_fields.len() + buttons.len(),
            control_count: text_fields.len() + buttons.len(),
            omitted_node_count: 0,
            enabled_control_count: text_fields.len() + buttons.len(),
            disabled_control_count: 0,
            visible_control_count: text_fields.len() + buttons.len(),
            focused_control_count: 0,
            source: "test".into(),
            source_status: "healthy".into(),
            snapshot_total_ms: Some(12),
            skipped_app_count: 0,
            remediation: Vec::new(),
            ..GuiAccessibilitySummary::default()
        },
        ocr_blocks: Vec::new(),
        ocr_diagnostics: GuiOcrDiagnostics::default(),
        capabilities: GuiPerceptionCapabilities {
            active_window: GuiSourceStatus::available("test"),
            desktop_state: GuiSourceStatus::available("test"),
            accessibility: GuiSourceStatus::available("test"),
            screenshot: GuiSourceStatus::available("test"),
            ocr: GuiSourceStatus::blocked("test", "ocr unavailable"),
            monitor: GuiSourceStatus::blocked("test", "monitor unavailable"),
            cursor_focus: GuiSourceStatus::blocked("test", "focus unavailable"),
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
    }
}

#[derive(Clone)]
struct FakePerception {
    active_title: String,
    text_fields: Vec<GuiControlSummary>,
    buttons: Vec<GuiControlSummary>,
    dialogs: Vec<GuiControlSummary>,
}

impl FakePerception {
    fn new(text_fields: Vec<GuiControlSummary>, buttons: Vec<GuiControlSummary>) -> Self {
        Self {
            active_title: "Test App".into(),
            text_fields,
            buttons,
            dialogs: Vec::new(),
        }
    }

    fn element_probe(controls: &[GuiControlSummary]) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "elements": controls
                .iter()
                .map(|control| serde_json::json!({
                    "role": control.role,
                    "name": control.name,
                    "path": control.path,
                    "control_id": control.control_id,
                    "bounds": control.bounds,
                    "enabled": control.enabled,
                    "visible": control.visible,
                    "focused": control.focused,
                    "source": control.source,
                    "score": control.confidence,
                    "identity_confidence": control.identity_confidence,
                    "bounds_confidence": control.bounds_confidence,
                    "state_confidence": control.state_confidence,
                    "sources": control.sources,
                }))
                .collect::<Vec<_>>()
        }))
    }
}

#[async_trait]
impl GuiPerceptionProvider for FakePerception {
    async fn get_active_window(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "title": self.active_title }))
    }

    async fn get_desktop_state(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": self.active_title,
            "accessibility_operational": true,
            "applications": ["Test App", "Browser"],
        }))
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": true }))
    }

    async fn find_ui_elements(&self, role: &str) -> GuiProbeResult {
        match role {
            "text" => Self::element_probe(&self.text_fields),
            "push button" => Self::element_probe(&self.buttons),
            "dialog" => Self::element_probe(&self.dialogs),
            _ => GuiProbeResult::ok(serde_json::json!({ "elements": [] })),
        }
    }

    async fn focused_window_title(&self) -> Option<String> {
        Some(self.active_title.clone())
    }
}

struct FakeExecutor {
    success: bool,
    backend_status: GuiActionBackendStatus,
}

impl FakeExecutor {
    fn available(success: bool) -> Self {
        Self {
            success,
            backend_status: GuiActionBackendStatus::available("fake_backend"),
        }
    }

    fn blocked(reason: &str) -> Self {
        Self {
            success: true,
            backend_status: GuiActionBackendStatus::blocked("fake_blocked", reason, "wayland"),
        }
    }
}

#[async_trait]
impl GuiActionExecutor for FakeExecutor {
    async fn action_backend_status(&self) -> GuiActionBackendStatus {
        self.backend_status.clone()
    }

    async fn execute(&self, request: GuiActionRequest) -> GuiActionExecution {
        if self.success {
            GuiActionExecution::ok(
                request.execution_hint,
                serde_json::json!({ "evidence": "completed" }),
            )
        } else {
            GuiActionExecution::err(request.execution_hint, "fake execution failure")
        }
    }
}

#[test]
fn gui_cognition_backend_route_prompt_matrix_classifies_current_intents() {
    let cases = [
        (
            "Observe my current screen and tell me active window, visible controls, OCR/accessibility availability.",
            GuiCognitionIntentKind::Observe,
            false,
        ),
        (
            "Analyze the current GUI and create a plan for what can be done safely. Do not do risky actions.",
            GuiCognitionIntentKind::AnalyzePlan,
            false,
        ),
        (
            "Focus the visible search/input field on the current screen and verify it is focused.",
            GuiCognitionIntentKind::FocusInput,
            false,
        ),
        (
            "Type \"KRIA GUI cognition test\" into the visible text field and verify the text is entered.",
            GuiCognitionIntentKind::TypeText,
            false,
        ),
        (
            "Click the visible safe button named Search and verify the screen changed.",
            GuiCognitionIntentKind::ClickControl,
            false,
        ),
        (
            "Plan how to open a browser and search for \"KRIA test\", but do not perform any risky external submit.",
            GuiCognitionIntentKind::BrowserSearchPlan,
            false,
        ),
        (
            "Create a GUI plan to fill the visible form fields, then validate whether it is safe.",
            GuiCognitionIntentKind::FillFormPlan,
            false,
        ),
        (
            "If multiple buttons look similar, ask me for clarification instead of guessing.",
            GuiCognitionIntentKind::AmbiguityCheck,
            false,
        ),
        (
            "Perform one safe GUI action, observe again, and verify whether it succeeded.",
            GuiCognitionIntentKind::SafeAction,
            false,
        ),
        (
            "If the target is missing or ambiguous, stop safely and explain the blocker.",
            GuiCognitionIntentKind::TargetAvailabilityCheck,
            false,
        ),
        (
            "If focus is lost during the task, recover focus and continue only if safe.",
            GuiCognitionIntentKind::FocusRecovery,
            false,
        ),
        (
            "Prepare to click a Submit button, but ask for approval before executing.",
            GuiCognitionIntentKind::RiskApproval,
            true,
        ),
        (
            "Prepare to send this form, but require my confirmation before any external submission.",
            GuiCognitionIntentKind::RiskApproval,
            true,
        ),
        (
            "If this action can delete, send, pay, or submit anything, pause and ask me first.",
            GuiCognitionIntentKind::RiskApproval,
            true,
        ),
        (
            "Before I continue, check the current desktop and tell me which app/window is active and whether OCR/accessibility are usable.",
            GuiCognitionIntentKind::Observe,
            false,
        ),
        (
            "I am on a browser page. Plan how to search for \"quarterly report template\" safely, but do not submit anything external.",
            GuiCognitionIntentKind::BrowserSearchPlan,
            false,
        ),
        (
            "In the current notes or editor window, type \"Follow up with design team at 4 PM\" into the visible text field and verify it.",
            GuiCognitionIntentKind::TypeText,
            false,
        ),
        (
            "I am updating a local settings page. Click the visible safe button named Save and verify the result.",
            GuiCognitionIntentKind::ClickControl,
            false,
        ),
        (
            "A form is open. Fill the visible form fields, validate the values, and do not press Submit or Send.",
            GuiCognitionIntentKind::FillFormPlan,
            false,
        ),
        (
            "If a confirmation dialog or permission popup is visible, stop and explain what it is asking before doing anything.",
            GuiCognitionIntentKind::Observe,
            false,
        ),
        (
            "Prepare to send the email draft, but require my confirmation before clicking Send.",
            GuiCognitionIntentKind::RiskApproval,
            true,
        ),
        (
            "Prepare to delete the selected file, but ask for approval and do not click Delete yet.",
            GuiCognitionIntentKind::RiskApproval,
            true,
        ),
        (
            "The browser is on the wrong tab. Observe the active window and plan how to switch to the correct browser tab safely.",
            GuiCognitionIntentKind::AnalyzePlan,
            false,
        ),
        (
            "If focus moves away from the form while typing, recover focus only if the same field is still visible.",
            GuiCognitionIntentKind::FocusRecovery,
            false,
        ),
    ];

    for (prompt, expected_kind, expected_approval) in cases {
        let intent = classify_gui_cognition_prompt(prompt);
        assert_eq!(intent.kind, expected_kind, "prompt: {prompt}");
        assert_eq!(
            intent.requires_approval, expected_approval,
            "prompt: {prompt}"
        );
    }
}

#[test]
fn gui_cognition_backend_route_extracts_payloads_and_truncates_text() {
    let long_text = format!("\"{}\"", "a".repeat(300));
    assert_eq!(extract_first_quoted_segment(&long_text).unwrap().len(), 240);

    let typing =
        classify_gui_cognition_prompt("Type 'KRIA GUI cognition test' into the visible field.");
    assert_eq!(
        typing.typed_text.as_deref(),
        Some("KRIA GUI cognition test")
    );

    let lower = "click the visible safe button named search and verify.".to_string();
    assert_eq!(
        extract_named_control(
            "Click the visible safe button named Search and verify.",
            &lower
        ),
        Some("Search".into())
    );
}

#[test]
fn gui_cognition_backend_route_converts_perception_and_builds_context() {
    let probe = GuiProbeResult::ok(serde_json::json!({
        "elements": [
            {"role": "text", "name": " Search ", "path": "/app/input"},
            {"role": "push button", "name": "", "path": "/app/empty"}
        ]
    }));
    let controls = controls_from_probe_result(&probe);
    assert_eq!(controls.len(), 2);
    assert_eq!(controls[0].name, "Search");

    let context = GuiContext::from_observation(observation(
        "Test App",
        vec![control("text", "Search")],
        vec![control("push button", "Search")],
    ));
    assert_eq!(context.context_id, "ctx-1");
    assert_eq!(context.text_field_count(), 1);
    assert!(!context.safety.raw_ocr_trusted);
    assert!(!context.safety.llm_native_tool_loop);
}

#[test]
fn gui_cognition_backend_route_planner_generates_steps_for_every_intent() {
    let obs = observation("Test App", vec![control("text", "Search")], Vec::new());
    for prompt in [
        "Observe the screen.",
        "Analyze the current GUI.",
        "Focus the visible input field.",
        "Type \"hello\" into the visible text field.",
        "Click the button named Search.",
        "Plan browser search.",
        "Create a GUI plan to fill the visible form fields.",
        "If multiple buttons look similar, ask me.",
        "If target is missing or ambiguous, stop.",
        "Perform one safe GUI action.",
        "If focus is lost, recover focus.",
        "Prepare to send this form, ask approval.",
    ] {
        let intent = classify_gui_cognition_prompt(prompt);
        assert!(
            !gui_plan_steps(&intent, &obs).is_empty(),
            "prompt: {prompt}"
        );
    }
}

#[test]
fn gui_cognition_backend_route_validator_blocks_and_gates_expected_cases() {
    let ctx = GuiContext::from_observation(observation("Terminal", Vec::new(), Vec::new()));

    let missing_text = GuiCognitionIntent {
        kind: GuiCognitionIntentKind::TypeText,
        typed_text: None,
        control_name: None,
        requires_approval: false,
        risk_level: "low".into(),
        risk_reasons: Vec::new(),
    };
    assert_eq!(
        validate_intent(&missing_text, &ctx).status,
        GuiValidationStatus::Blocked
    );

    let terminal_typing = classify_gui_cognition_prompt("Type \"rm -rf\" into the visible field.");
    let report = validate_intent(&terminal_typing, &ctx);
    assert_eq!(report.status, GuiValidationStatus::Blocked);
    assert!(report.reasons[0].contains("terminal"));

    let missing_click = classify_gui_cognition_prompt("Click the visible button.");
    let report = validate_intent(&missing_click, &ctx);
    assert_eq!(report.status, GuiValidationStatus::Blocked);
    assert!(report.reasons[0].contains("button/control name"));

    let risky = classify_gui_cognition_prompt("Click the Submit button.");
    let report = validate_intent(&risky, &ctx);
    assert_eq!(report.status, GuiValidationStatus::NeedsApproval);
    assert!(report
        .reasons
        .iter()
        .any(|reason| reason.contains("submit")));
}

#[test]
fn gui_cognition_backend_route_resolver_handles_unique_missing_and_ambiguous_targets() {
    let unique_ctx = GuiContext::from_observation(observation(
        "Test App",
        vec![control("text", "Search")],
        vec![control("push button", "Search")],
    ));
    assert!(matches!(
        resolve_unique_text_field(&unique_ctx),
        TargetResolution::Resolved(_)
    ));
    assert!(matches!(
        resolve_type_text_target(&unique_ctx),
        TargetResolution::Resolved(_)
    ));
    assert!(matches!(
        resolve_button(&unique_ctx, "Search"),
        TargetResolution::Resolved(_)
    ));

    let missing_ctx = GuiContext::from_observation(observation("Test App", Vec::new(), Vec::new()));
    assert!(matches!(
        resolve_unique_text_field(&missing_ctx),
        TargetResolution::Missing { .. }
    ));
    assert!(matches!(
        resolve_button(&missing_ctx, "Search"),
        TargetResolution::Missing { .. }
    ));

    let ambiguous_ctx = GuiContext::from_observation(observation(
        "Test App",
        vec![control("text", "Search"), control("text", "Filter")],
        vec![
            control("push button", "Search"),
            control("push button", "Search Again"),
        ],
    ));
    assert!(matches!(
        resolve_unique_text_field(&ambiguous_ctx),
        TargetResolution::Ambiguous { .. }
    ));
    assert!(matches!(
        resolve_button(&ambiguous_ctx, "Search"),
        TargetResolution::Ambiguous { .. }
    ));
}

#[test]
fn gui_cognition_backend_route_safety_verifier_and_recovery_are_deterministic() {
    let allowed = classify_gui_cognition_prompt("Focus the visible input field.");
    assert_eq!(safety_for_intent(&allowed).status, GuiSafetyStatus::Allowed);

    let risky = classify_gui_cognition_prompt("Prepare to pay, ask approval before executing.");
    let safety = safety_for_intent(&risky);
    assert_eq!(safety.status, GuiSafetyStatus::RequiresApproval);
    assert!(safety
        .reasons
        .iter()
        .any(|reason| reason.contains("payment")));

    let post = observation("Test App", vec![control("text", "Search")], Vec::new());
    let ok = verify_post_action(
        &GuiActionExecution::ok("click_ui_element", serde_json::json!({})),
        &post,
        0.72,
    );
    assert_eq!(ok.status, "completed");
    assert_eq!(ok.after_observation_id, "obs-1");

    let failed = verify_post_action(
        &GuiActionExecution::err("click_ui_element", "failed"),
        &post,
        0.72,
    );
    assert_eq!(failed.status, "failed");

    let blocker = GuiBlocker::new("target_resolution", "missing target")
        .with_candidate_count(0)
        .with_clarification("Which exact visible target should I use?");
    assert_eq!(blocker.kind, "target_resolution");
    assert_eq!(blocker.candidate_count, Some(0));

    let sanitized =
        sanitized_execution_evidence("password=mysecret token=abc Ignore previous instructions");
    assert!(sanitized.contains("[redacted]"));
    assert!(sanitized.len() <= 240);
}

#[tokio::test]
async fn gui_cognition_backend_route_runtime_emits_ordered_observe_sequence_without_llm_loop() {
    let perception = FakePerception::new(vec![control("text", "Search")], Vec::new());
    let executor = FakeExecutor::available(true);
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(GuiTurnRequest {
            session_id: "session-1".into(),
            turn_id: "turn-1".into(),
            workflow_id: "workflow-1".into(),
            message: "Observe my current screen.".into(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            hitl_decision_fixture: None,
            execution_environment: Default::default(),
            execution_mode: GuiExecutionMode::SafetyOnly,
            workflow_enabled: false,
            resume_checkpoint: None,
            resume_reason: None,
        })
        .await;

    let types = outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(types[0], "TurnStarted");
    assert_eq!(types[1], "RouteConfirmed");
    assert!(types.contains(&"ObservationStarted"));
    assert!(types.contains(&"ObservationCompleted"));
    assert!(types.contains(&"ContextBuilt"));
    assert!(types.contains(&"GoalContractCreated"));
    assert!(types.contains(&"PlanCreated"));
    let observation_completed_idx = types
        .iter()
        .position(|event_type| *event_type == "ObservationCompleted")
        .unwrap();
    let context_built_idx = types
        .iter()
        .position(|event_type| *event_type == "ContextBuilt")
        .unwrap();
    let goal_contract_idx = types
        .iter()
        .position(|event_type| *event_type == "GoalContractCreated")
        .unwrap();
    assert!(observation_completed_idx < context_built_idx);
    assert!(context_built_idx < goal_contract_idx);
    assert_eq!(types.last().copied(), Some("TurnCompleted"));
    assert!(!types.contains(&"ActionStarted"));
    let context_event = outcome
        .events
        .iter()
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some("ContextBuilt"))
        .expect("ContextBuilt event exists");
    assert_eq!(context_event["ocr_untrusted"], true);
    assert_eq!(context_event["freshness"], "fresh");
    assert!(context_event["trusted_control_count"].as_u64().is_some());
    assert!(context_event["executable_control_count"].as_u64().is_some());
    assert!(context_event["source_confidence"]["accessibility"]
        .as_f64()
        .is_some());
    let goal_event = outcome
        .events
        .iter()
        .find(|event| {
            event.get("type").and_then(serde_json::Value::as_str) == Some("GoalContractCreated")
        })
        .expect("GoalContractCreated event exists");
    assert!(goal_event["contract_id"].as_str().is_some());
    assert_eq!(goal_event["action_type"], "observe");
    assert_eq!(
        goal_event["desired_final_state"],
        "desktop state observed and summarized"
    );
    assert_eq!(goal_event["risk_level"], "low");
    assert_eq!(goal_event["requires_user_approval"], false);
    assert_eq!(goal_event["extractor_mode"], "deterministic");
    assert!(goal_event["extraction_confidence"].as_f64().is_some());
    assert_eq!(
        outcome.response["gui_cognition"]["path"],
        "send_manual_tool_message"
    );
    assert_eq!(outcome.response["gui_cognition"]["llm_tool_loop"], false);
    assert_eq!(
        outcome.response["gui_cognition"]["context"]["ocr_untrusted"],
        true
    );
    assert!(
        outcome.response["gui_cognition"]["context"]["trusted_control_count"]
            .as_u64()
            .is_some()
    );
    assert_eq!(
        outcome.response["gui_cognition"]["goal_contract"]["action_type"],
        "observe"
    );
    assert!(
        outcome.response["gui_cognition"]["goal_contract"]["contract_id"]
            .as_str()
            .is_some()
    );
}

#[tokio::test]
async fn gui_cognition_backend_route_runtime_resolves_safe_unique_target_without_execution() {
    let perception = FakePerception::new(vec![control("text", "Search")], Vec::new());
    let executor = FakeExecutor::available(true);
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(GuiTurnRequest {
            session_id: "session-1".into(),
            turn_id: "turn-2".into(),
            workflow_id: "workflow-2".into(),
            message: "Focus the visible search/input field on the current screen.".into(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            hitl_decision_fixture: None,
            execution_environment: Default::default(),
            execution_mode: GuiExecutionMode::SafetyOnly,
            workflow_enabled: false,
            resume_checkpoint: None,
            resume_reason: None,
        })
        .await;

    let types = outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert!(types.contains(&"TargetResolutionStarted"));
    assert!(types.contains(&"TargetResolutionCompleted"));
    assert!(!types.contains(&"TargetResolved"));
    assert!(types.contains(&"SafetyGateCompleted"));
    assert!(!types.contains(&"ActionStarted"));
    assert!(!types.contains(&"ActionCompleted"));
    assert!(!types.contains(&"VerificationCompleted"));
    assert_eq!(outcome.response["status"], "ok");
    assert_eq!(
        outcome.response["gui_cognition"]["target_resolution"]["status"],
        "resolved"
    );
    assert_eq!(
        outcome.response["gui_cognition"]["target_resolution"]["can_execute"],
        false
    );
    assert_eq!(
        outcome.response["gui_cognition"]["safety_gate"]["can_execute"],
        false
    );
    assert_eq!(
        outcome.response["gui_cognition"]["safety_gate"]["safety_status"],
        "safe_no_approval_required"
    );
}

#[tokio::test]
async fn gui_cognition_backend_route_observe_still_works_when_action_backend_blocked() {
    let perception = FakePerception::new(vec![control("text", "Search")], Vec::new());
    let executor = FakeExecutor::blocked("global safety halt is engaged");
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(GuiTurnRequest {
            session_id: "session-1".into(),
            turn_id: "turn-observe-blocked-backend".into(),
            workflow_id: "workflow-observe-blocked-backend".into(),
            message: "Observe my current screen.".into(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            hitl_decision_fixture: None,
            execution_environment: Default::default(),
            execution_mode: GuiExecutionMode::SafetyOnly,
            workflow_enabled: false,
            resume_checkpoint: None,
            resume_reason: None,
        })
        .await;

    let types = outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert!(types.contains(&"ActionBackendStatus"));
    assert!(!types.contains(&"ExecutionBlocked"));
    assert!(!types.contains(&"ActionStarted"));
    assert_eq!(outcome.response["status"], "ok");
    assert_eq!(
        outcome.response["gui_cognition"]["action_backend"]["can_execute_actions"],
        false
    );
    assert_eq!(
        outcome.response["gui_cognition"]["action_backend"]["halt_kind"],
        "service_not_ready"
    );
    assert_eq!(
        outcome.response["gui_cognition"]["action_backend"]["can_observe"],
        true
    );
}

#[tokio::test]
async fn gui_cognition_backend_route_resolves_safe_action_without_execution_when_backend_blocked() {
    let perception = FakePerception::new(vec![control("text", "Search")], Vec::new());
    let executor = FakeExecutor::blocked("global safety halt is engaged");
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(GuiTurnRequest {
            session_id: "session-1".into(),
            turn_id: "turn-action-blocked-backend".into(),
            workflow_id: "workflow-action-blocked-backend".into(),
            message: "Focus the visible search/input field on the current screen.".into(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            hitl_decision_fixture: None,
            execution_environment: Default::default(),
            execution_mode: GuiExecutionMode::SafetyOnly,
            workflow_enabled: false,
            resume_checkpoint: None,
            resume_reason: None,
        })
        .await;

    let types = outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert!(types.contains(&"ActionBackendStatus"));
    assert!(types.contains(&"TargetResolutionCompleted"));
    assert!(!types.contains(&"ExecutionBlocked"));
    assert!(!types.contains(&"VerificationCompleted"));
    assert!(!types.contains(&"RecoveryEvaluationStarted"));
    assert!(!types.contains(&"RecoveryProposed"));
    assert!(!types.contains(&"ActionStarted"));
    assert!(!types.contains(&"ActionCompleted"));
    assert_eq!(outcome.response["status"], "ok");
    assert_eq!(
        outcome.response["gui_cognition"]["target_resolution"]["can_execute"],
        false
    );
}

#[tokio::test]
async fn gui_cognition_backend_route_runtime_risky_prompt_pauses_without_action() {
    let perception = FakePerception::new(Vec::new(), vec![control("push button", "Submit")]);
    let executor = FakeExecutor::available(true);
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(GuiTurnRequest {
            session_id: "session-1".into(),
            turn_id: "turn-3".into(),
            workflow_id: "workflow-3".into(),
            message: "Prepare to click a Submit button, but ask for approval before executing."
                .into(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            hitl_decision_fixture: None,
            execution_environment: Default::default(),
            execution_mode: GuiExecutionMode::SafetyOnly,
            workflow_enabled: false,
            resume_checkpoint: None,
            resume_reason: None,
        })
        .await;

    let types = outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert!(types.contains(&"HitlRequired"));
    assert!(types.contains(&"SafetyGateCompleted"));
    assert!(!types.contains(&"ActionStarted"));
    assert_eq!(outcome.response["status"], "needs_approval");
    assert_eq!(
        outcome.response["gui_cognition"]["safety_gate"]["safety_status"],
        "approval_required"
    );
    assert_eq!(
        outcome.response["gui_cognition"]["safety_gate"]["can_execute"],
        false
    );
}

fn backend_probe(session_type: &str) -> GuiBackendProbeInput {
    GuiBackendProbeInput {
        global_halt_engaged: false,
        halt_reason: None,
        automation_enabled: true,
        orchestrator_available: true,
        session_type: session_type.into(),
        vision_sidecar: "running".into(),
        uinput_daemon: "stopped".into(),
        xdotool_available: false,
        xdotool_display_usable: false,
        ydotool_available: false,
        ydotool_permission_ok: false,
        uinput_available: false,
        uinput_socket_path: Some("/run/user/1000/kria-uinput.sock".into()),
        uinput_socket_accessible: false,
    }
}

#[test]
fn gui_backend_selector_blocks_wayland_xdotool_only() {
    let mut probe = backend_probe("wayland");
    probe.xdotool_available = true;

    let selection = select_gui_action_backend(&probe);

    assert_eq!(selection.selected_backend, "unavailable");
    assert_eq!(selection.backend_probe_status, "wayland_no_input_backend");
    assert!(!selection.can_execute_actions);
    assert!(!selection.xdotool_usable_for_actions);
    assert!(selection
        .backend_probe_errors
        .iter()
        .any(|error| error.contains("xdotool detected but not usable")));
}

#[test]
fn gui_backend_selector_allows_wayland_uinput_socket() {
    let mut probe = backend_probe("wayland");
    probe.xdotool_available = true;
    probe.uinput_daemon = "running".into();
    probe.uinput_available = true;
    probe.uinput_socket_accessible = true;

    let selection = select_gui_action_backend(&probe);

    assert_eq!(selection.selected_backend, "uinput_accessibility");
    assert_eq!(selection.backend_probe_status, "wayland_uinput_ready");
    assert_eq!(selection.input_backend_kind, "uinput");
    assert!(selection.can_execute_actions);
}

#[test]
fn gui_backend_selector_allows_wayland_ydotool_only_after_probe() {
    let mut denied = backend_probe("wayland");
    denied.ydotool_available = true;
    denied.ydotool_permission_ok = false;
    let denied_selection = select_gui_action_backend(&denied);
    assert_eq!(denied_selection.selected_backend, "unavailable");
    assert!(!denied_selection.can_execute_actions);

    let mut ready = denied;
    ready.ydotool_permission_ok = true;
    let ready_selection = select_gui_action_backend(&ready);
    assert_eq!(ready_selection.selected_backend, "ydotool_accessibility");
    assert_eq!(
        ready_selection.backend_probe_status,
        "wayland_ydotool_ready"
    );
    assert!(ready_selection.ydotool_usable_for_actions);
    assert!(ready_selection.can_execute_actions);
}

#[test]
fn gui_backend_selector_allows_x11_xdotool_only_after_display_probe() {
    let mut probe = backend_probe("x11");
    probe.xdotool_available = true;
    probe.xdotool_display_usable = false;
    let blocked = select_gui_action_backend(&probe);
    assert_eq!(blocked.selected_backend, "unavailable");
    assert_eq!(blocked.backend_probe_status, "x11_no_xdotool");

    probe.xdotool_display_usable = true;
    let ready = select_gui_action_backend(&probe);
    assert_eq!(ready.selected_backend, "xdotool_accessibility");
    assert_eq!(ready.backend_probe_status, "x11_xdotool_ready");
    assert!(ready.xdotool_usable_for_actions);
    assert!(ready.can_execute_actions);
}

fn gui_event<'a>(
    outcome: &'a kria_core::agent::gui_cognition::GuiTurnOutcome,
    event_type: &str,
) -> Option<&'a serde_json::Value> {
    outcome
        .events
        .iter()
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some(event_type))
}

#[tokio::test]
async fn gui_cognition_pipeline_executes_and_verifies_focus_field_fixture() {
    let mut field = control("text", "Search");
    field.focused = true;
    let perception = FakePerception::new(vec![field], Vec::new());
    let executor = FakeExecutor::available(true);
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(GuiTurnRequest {
            session_id: "session-1".into(),
            turn_id: "turn-step8-focus".into(),
            workflow_id: "workflow-1".into(),
            message: "Focus the search box".into(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            hitl_decision_fixture: None,
            execution_environment: Default::default(),
            execution_mode: GuiExecutionMode::ExecuteFixture,
            workflow_enabled: false,
            resume_checkpoint: None,
            resume_reason: None,
        })
        .await;

    let types = outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert!(types.contains(&"ActionStarted"), "events: {types:?}");
    assert!(types.contains(&"ActionCompleted"));
    assert!(types.contains(&"ExecutionVerificationCompleted"));
    assert!(!types.contains(&"ActionFailed"));
    assert!(!types.contains(&"ExecutionBlocked"));

    let verification =
        gui_event(&outcome, "ExecutionVerificationCompleted").expect("verification event exists");
    assert_eq!(verification["status"], "verified");
    assert!(verification["verification_strategy"].as_str().is_some());
    assert_eq!(verification["matched_expected_state"], true);
    assert_eq!(outcome.status, "completed");

    // ActionCompleted is backend success only; the verification event carries
    // the verified verdict.
    let completed = gui_event(&outcome, "ActionCompleted").expect("ActionCompleted exists");
    assert_eq!(completed["status"], "completed");
}

#[tokio::test]
async fn gui_cognition_pipeline_reports_verification_failed_without_blind_success() {
    // Static screen: a click cannot be verified as result_visible, so the turn
    // must report verification_failed instead of a blind success.
    let perception = FakePerception::new(Vec::new(), vec![control("push button", "Search")]);
    let executor = FakeExecutor::available(true);
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(GuiTurnRequest {
            session_id: "session-1".into(),
            turn_id: "turn-step8-click".into(),
            workflow_id: "workflow-1".into(),
            message: "Click the Search button".into(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            hitl_decision_fixture: None,
            execution_environment: Default::default(),
            execution_mode: GuiExecutionMode::ExecuteFixture,
            workflow_enabled: false,
            resume_checkpoint: None,
            resume_reason: None,
        })
        .await;

    let types = outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert!(types.contains(&"ActionStarted"), "events: {types:?}");
    assert!(types.contains(&"ActionCompleted"));
    assert!(types.contains(&"ExecutionVerificationCompleted"));
    assert!(!types.contains(&"ExecutionBlocked"));

    let verification =
        gui_event(&outcome, "ExecutionVerificationCompleted").expect("verification event exists");
    assert_eq!(verification["status"], "verification_failed");
    assert_eq!(verification["matched_expected_state"], false);
    assert!(verification["recovery_hint"].as_str().is_some());

    // Backend reported success, but the action is not treated as final success.
    let completed = gui_event(&outcome, "ActionCompleted").expect("ActionCompleted exists");
    assert_eq!(completed["status"], "completed");
    assert_eq!(outcome.status, "verification_failed");

    // Step 9: a non-idempotent click failure runs recovery assessment but never
    // starts a blind recovery action.
    let assessment = gui_event(&outcome, "RecoveryAssessmentCompleted")
        .expect("RecoveryAssessmentCompleted exists");
    assert_eq!(assessment["can_execute_recovery"], false);
    assert!(types.contains(&"RecoveryBlocked"), "events: {types:?}");
    assert!(!types.contains(&"RecoveryActionStarted"));
}

/// Perception provider where keyboard focus is on the wrong control for the
/// post-action observation and returns to the target on the post-recovery
/// observation. Only `get_cursor_focus_state` advances the sequence, so there
/// is no cross-probe ordering hazard within a single observation.
struct FocusRecoversPerception {
    field: GuiControlSummary,
    cursor_seq: std::sync::atomic::AtomicU64,
}

impl FocusRecoversPerception {
    fn new() -> Self {
        Self {
            field: control("text", "Search"),
            cursor_seq: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl GuiPerceptionProvider for FocusRecoversPerception {
    async fn get_active_window(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "title": "Test App", "app_name": "Test App" }))
    }

    async fn get_desktop_state(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": "Test App",
            "accessibility_operational": true,
            "applications": ["Test App"],
        }))
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": true }))
    }

    async fn find_ui_elements(&self, role: &str) -> GuiProbeResult {
        if role == "text" {
            GuiProbeResult::ok(serde_json::json!({
                "elements": [{
                    "role": "text",
                    "name": self.field.name,
                    "path": self.field.path,
                    "control_id": self.field.control_id,
                    "bounds": self.field.bounds,
                    "enabled": true,
                    "visible": true,
                    "focused": false,
                    "in_active_window": true,
                    "source": "accessibility",
                    "score": 0.9,
                    "identity_confidence": 0.9,
                    "bounds_confidence": 0.9,
                    "state_confidence": 0.9,
                }]
            }))
        } else {
            GuiProbeResult::ok(serde_json::json!({ "elements": [] }))
        }
    }

    async fn get_cursor_focus_state(&self) -> GuiProbeResult {
        let seq = self
            .cursor_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // Pre (0) and post-action (1): focus is on the wrong control.
        // Post-recovery (2+): focus has returned to the Search field.
        let on_target = seq >= 2;
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": "Test App",
            "focused_app": "Test App",
            "focused_control_id": "other-control",
            "focused_control_label": if on_target { "Search" } else { "Other Field" },
            "focused_control_role": "text",
            "keyboard_focus_known": true,
            "text_cursor_known": true,
            "editable_target_known": true,
            "terminal_like": false,
            "focus_confidence": 0.9,
        }))
    }

    async fn focused_window_title(&self) -> Option<String> {
        Some("Test App".into())
    }
}

#[tokio::test]
async fn gui_cognition_pipeline_recovers_focus_lost_with_refocus_same_target() {
    let perception = FocusRecoversPerception::new();
    let executor = FakeExecutor::available(true);
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(GuiTurnRequest {
            session_id: "session-1".into(),
            turn_id: "turn-step9-focus".into(),
            workflow_id: "workflow-1".into(),
            message: "Focus the visible search field and verify it is focused".into(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            hitl_decision_fixture: None,
            execution_environment: Default::default(),
            execution_mode: GuiExecutionMode::ExecuteFixture,
            workflow_enabled: false,
            resume_checkpoint: None,
            resume_reason: None,
        })
        .await;

    let types = outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();

    // The first verification did not confirm focus, so recovery runs.
    assert!(types.contains(&"ExecutionVerificationCompleted"), "events: {types:?}");
    assert!(types.contains(&"RecoveryAssessmentCompleted"), "events: {types:?}");
    assert!(types.contains(&"RecoveryActionStarted"), "events: {types:?}");
    assert!(types.contains(&"RecoveryActionCompleted"), "events: {types:?}");
    assert!(!types.contains(&"RecoveryBlocked"));

    let assessment =
        gui_event(&outcome, "RecoveryAssessmentCompleted").expect("assessment exists");
    assert_eq!(assessment["failure_kind"], "focus_lost");
    assert_eq!(assessment["recovery_action_kind"], "RefocusSameTarget");
    assert_eq!(assessment["can_execute_recovery"], true);

    let completed = gui_event(&outcome, "RecoveryActionCompleted").expect("completed exists");
    assert_eq!(completed["status"], "recovered");
    assert_eq!(completed["can_continue_workflow"], false);
    assert_eq!(outcome.status, "recovered");
}

/// Task 9.5 (Requirements 10, 14, 15): with the `gui_cog_safety_polish` flag ON,
/// the idempotent focus-loss recovery emits the additive `RecoveryDecision`
/// telemetry so the decision (idempotent-gated, bounded single retry) is
/// inspectable from the event stream — and still recovers via RefocusSameTarget.
#[tokio::test]
async fn safety_polish_emits_recovery_decision_for_idempotent_focus_loss() {
    let perception = FocusRecoversPerception::new();
    let executor = FakeExecutor::available(true);
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled());

    let outcome = runtime
        .run_turn(GuiTurnRequest {
            session_id: "session-1".into(),
            turn_id: "turn-step9-focus-polish".into(),
            workflow_id: "workflow-1".into(),
            message: "Focus the visible search field and verify it is focused".into(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            hitl_decision_fixture: None,
            execution_environment: Default::default(),
            execution_mode: GuiExecutionMode::ExecuteFixture,
            workflow_enabled: false,
            resume_checkpoint: None,
            resume_reason: None,
        })
        .await;

    let types = outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();

    // The additive RecoveryDecision telemetry is present and inspectable.
    assert!(types.contains(&"RecoveryDecision"), "events: {types:?}");
    let decision = gui_event(&outcome, "RecoveryDecision").expect("decision exists");
    assert_eq!(decision["recovery_action_kind"], "RefocusSameTarget");
    assert_eq!(decision["failure_kind"], "focus_lost");
    assert_eq!(decision["idempotent_gated"], true);
    assert_eq!(decision["single_retry_respected"], true);
    assert_eq!(decision["unexpected_dialog_stop"], false);
    assert_eq!(decision["load_failure_explain"], false);
    // Recovery still succeeds via the bounded refocus.
    assert_eq!(outcome.status, "recovered");
}

/// Task 9.5: flag OFF = byte-for-byte unchanged — no `RecoveryDecision`
/// telemetry is emitted (only the existing RecoveryAssessmentCompleted /
/// RecoveryActionCompleted events appear).
#[tokio::test]
async fn flag_off_emits_no_recovery_decision_telemetry() {
    let perception = FocusRecoversPerception::new();
    let executor = FakeExecutor::available(true);
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(GuiTurnRequest {
            session_id: "session-1".into(),
            turn_id: "turn-step9-focus-flagoff".into(),
            workflow_id: "workflow-1".into(),
            message: "Focus the visible search field and verify it is focused".into(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            hitl_decision_fixture: None,
            execution_environment: Default::default(),
            execution_mode: GuiExecutionMode::ExecuteFixture,
            workflow_enabled: false,
            resume_checkpoint: None,
            resume_reason: None,
        })
        .await;

    let types = outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();

    assert!(!types.contains(&"RecoveryDecision"), "events: {types:?}");
    // The legacy recovery path is unchanged: it still recovers via refocus.
    assert!(types.contains(&"RecoveryActionCompleted"), "events: {types:?}");
    assert_eq!(outcome.status, "recovered");
}

// ---------------------------------------------------------------------------
// Task 0.3 — TestSubstrate auto-approval gate (Requirement 20.3)
//
// An auto-approval HITL fixture must be REJECTED on the real session and only
// honored inside the test substrate. These two tests pin both sides of the gate
// through the same runtime path the live audit uses.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auto_approve_fixture_is_rejected_on_real_session() {
    let perception = FakePerception::new(Vec::new(), vec![control("push button", "Submit")]);
    let executor = FakeExecutor::available(true);
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(GuiTurnRequest {
            session_id: "session-1".into(),
            turn_id: "turn-substrate-real".into(),
            workflow_id: "workflow-substrate-real".into(),
            message: "Prepare to click a Submit button, but ask for approval before executing."
                .into(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            // An auto-approve fixture is supplied, but the environment is the
            // user's real session — the gate must refuse to honor it.
            hitl_decision_fixture: Some(GuiHitlDecisionFixture::Approve),
            execution_environment: GuiExecutionEnvironment::RealSession,
            execution_mode: GuiExecutionMode::SafetyOnly,
            workflow_enabled: false,
            resume_checkpoint: None,
            resume_reason: None,
        })
        .await;

    let types = outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    // The fixture is rejected; the action stays gated and never executes.
    assert!(
        types.contains(&"HitlFixtureRejected"),
        "expected HitlFixtureRejected on real session, events: {types:?}"
    );
    assert!(!types.contains(&"ActionStarted"));
    assert!(!types.contains(&"ActionCompleted"));
    assert_eq!(outcome.response["status"], "needs_approval");
    assert_eq!(
        outcome.response["gui_cognition"]["execution_environment"]["environment"],
        "real_session"
    );
    assert_eq!(
        outcome.response["gui_cognition"]["execution_environment"]["allows_auto_approval"],
        false
    );
    assert_eq!(
        outcome.response["gui_cognition"]["hitl_decision"]["reason"],
        "auto_approval_requires_test_substrate"
    );
}

#[tokio::test]
async fn auto_approve_fixture_is_honored_in_test_substrate() {
    let perception = FakePerception::new(Vec::new(), vec![control("push button", "Submit")]);
    let executor = FakeExecutor::available(true);
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(GuiTurnRequest {
            session_id: "session-1".into(),
            turn_id: "turn-substrate-isolated".into(),
            workflow_id: "workflow-substrate-isolated".into(),
            message: "Prepare to click a Submit button, but ask for approval before executing."
                .into(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            hitl_decision_fixture: Some(GuiHitlDecisionFixture::Approve),
            // Inside an isolated substrate the auto-approval is permitted.
            execution_environment: GuiExecutionEnvironment::TestSubstrate {
                scratch_dir: None,
                restore_clipboard: true,
            },
            // SafetyOnly keeps the executor from running, but the approval is
            // still honored (status advances past needs_approval).
            execution_mode: GuiExecutionMode::SafetyOnly,
            workflow_enabled: false,
            resume_checkpoint: None,
            resume_reason: None,
        })
        .await;

    let types = outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert!(
        !types.contains(&"HitlFixtureRejected"),
        "fixture must NOT be rejected in substrate, events: {types:?}"
    );
    assert_eq!(
        outcome.response["gui_cognition"]["execution_environment"]["environment"],
        "test_substrate"
    );
    assert_eq!(
        outcome.response["gui_cognition"]["execution_environment"]["allows_auto_approval"],
        true
    );
    assert_eq!(outcome.response["status"], "approved_for_step7");
}

// ── Task 2.1: strict-validate + one repair-retry (Requirement 1.2) ───────────
//
// These T1 flow tests drive the full `run_turn` planner-selection path with the
// `SequencedFixtureGuiLlmPlanner` (a different fixture per attempt) so we can
// assert the constrained-decode + strict-validate + exactly-ONE-repair-retry
// behavior and its `gui_cog_smart_planner` flag gate.

/// Standard perception for the repair-retry flow: a Search text field plus
/// Search/Submit buttons, matching the planner unit-test fixture context so a
/// `ValidPlan` fixture validates as Valid.
fn repair_flow_perception() -> FakePerception {
    FakePerception::new(
        vec![control("text", "Search")],
        vec![control("push button", "Search"), control("push button", "Submit")],
    )
}

fn repair_flow_request(turn_id: &str) -> GuiTurnRequest {
    GuiTurnRequest {
        session_id: "session-repair".into(),
        turn_id: turn_id.into(),
        workflow_id: "workflow-repair".into(),
        message: "Click the visible safe button named Search.".into(),
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

fn event_types(outcome: &kria_core::agent::gui_cognition::GuiTurnOutcome) -> Vec<String> {
    outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .map(|value| value.to_string())
        .collect()
}

#[tokio::test]
async fn smart_planner_accepts_schema_valid_plan_first_try_without_retry() {
    let perception = repair_flow_perception();
    let executor = FakeExecutor::available(true);
    let planner = SequencedFixtureGuiLlmPlanner::new(vec![GuiLlmPlannerFixture::ValidPlan]);
    let planner_ref: &dyn GuiLlmPlanner = &planner;
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_llm_planner(Some(planner_ref))
        .with_smart_planner(GuiSmartPlannerConfig::enabled());

    let outcome = runtime.run_turn(repair_flow_request("turn-valid-first")).await;
    let types = event_types(&outcome);

    assert!(types.contains(&"LlmPlanningStarted".to_string()));
    assert!(types.contains(&"LlmPlanningCompleted".to_string()));
    assert!(
        !types.contains(&"LlmPlanRepairRetry".to_string()),
        "no repair-retry when the first attempt is valid"
    );
    assert!(!types.contains(&"LlmPlanningFailed".to_string()));
    assert_eq!(planner.call_count(), 1);
    assert_eq!(planner.repair_call_count(), 0);
    assert_eq!(outcome.response["gui_cognition"]["planner"]["mode"], "llm_schema");
    assert_eq!(
        outcome.response["gui_cognition"]["planner"]["llm_status"],
        "completed"
    );
}

#[tokio::test]
async fn smart_planner_repairs_after_first_invalid_attempt() {
    let perception = repair_flow_perception();
    let executor = FakeExecutor::available(true);
    // First attempt: invalid JSON (parse failure). Repair attempt: valid plan.
    let planner = SequencedFixtureGuiLlmPlanner::new(vec![
        GuiLlmPlannerFixture::InvalidJson,
        GuiLlmPlannerFixture::ValidPlan,
    ]);
    let planner_ref: &dyn GuiLlmPlanner = &planner;
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_llm_planner(Some(planner_ref))
        .with_smart_planner(GuiSmartPlannerConfig::enabled());

    let outcome = runtime.run_turn(repair_flow_request("turn-repair-ok")).await;
    let types = event_types(&outcome);

    assert!(
        types.contains(&"LlmPlanRepairRetry".to_string()),
        "a repair-retry is emitted after the first strict-validation failure"
    );
    assert!(
        !types.contains(&"LlmPlanningFailed".to_string()),
        "the repaired plan succeeds, so no terminal failure"
    );
    // The repaired plan is an accepted LLM plan, not a deterministic fallback.
    assert_eq!(outcome.response["gui_cognition"]["planner"]["mode"], "llm_schema");
    assert_eq!(
        outcome.response["gui_cognition"]["planner"]["llm_status"],
        "repaired"
    );
    assert_eq!(planner.call_count(), 2);
    assert_eq!(planner.repair_call_count(), 1);
}

#[tokio::test]
async fn smart_planner_falls_back_when_repair_also_invalid() {
    let perception = repair_flow_perception();
    let executor = FakeExecutor::available(true);
    let planner = SequencedFixtureGuiLlmPlanner::new(vec![
        GuiLlmPlannerFixture::InvalidJson,
        GuiLlmPlannerFixture::InvalidJson,
    ]);
    let planner_ref: &dyn GuiLlmPlanner = &planner;
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_llm_planner(Some(planner_ref))
        .with_smart_planner(GuiSmartPlannerConfig::enabled());

    let outcome = runtime.run_turn(repair_flow_request("turn-repair-fail")).await;
    let types = event_types(&outcome);

    assert!(types.contains(&"LlmPlanRepairRetry".to_string()));
    assert!(types.contains(&"LlmPlanningFailed".to_string()));
    let failed = outcome
        .events
        .iter()
        .find(|event| {
            event.get("type").and_then(serde_json::Value::as_str) == Some("LlmPlanningFailed")
                && event.get("after_repair_retry").and_then(serde_json::Value::as_bool)
                    == Some(true)
        })
        .expect("terminal failure is tagged after_repair_retry");
    assert_eq!(failed["status"], "rejected");
    // Deterministic fallback is used when the single repair also fails.
    assert_eq!(
        outcome.response["gui_cognition"]["planner"]["mode"],
        "llm_rejected_fallback"
    );
    assert_eq!(planner.call_count(), 2);
    assert_eq!(planner.repair_call_count(), 1);
}

#[tokio::test]
async fn smart_planner_never_scrapes_prose_response() {
    let perception = repair_flow_perception();
    let executor = FakeExecutor::available(true);
    // Both attempts return JSON wrapped in prose; the parser must reject both
    // and the planner must NOT lenient-scrape the embedded object.
    let planner = SequencedFixtureGuiLlmPlanner::new(vec![
        GuiLlmPlannerFixture::ProseWrapper,
        GuiLlmPlannerFixture::ProseWrapper,
    ]);
    let planner_ref: &dyn GuiLlmPlanner = &planner;
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_llm_planner(Some(planner_ref))
        .with_smart_planner(GuiSmartPlannerConfig::enabled());

    let outcome = runtime.run_turn(repair_flow_request("turn-prose")).await;

    // Prose is never accepted: the final plan is the deterministic fallback,
    // not an llm_schema plan scraped from the prose.
    assert_eq!(
        outcome.response["gui_cognition"]["planner"]["mode"],
        "llm_rejected_fallback"
    );
    assert_ne!(
        outcome.response["gui_cognition"]["planner"]["llm_status"],
        "completed"
    );
    assert_eq!(planner.repair_call_count(), 1);
}

#[tokio::test]
async fn smart_planner_performs_at_most_one_repair_attempt() {
    let perception = repair_flow_perception();
    let executor = FakeExecutor::available(true);
    // A third (valid) response is queued but must NEVER be consumed: only one
    // repair-retry is allowed (Requirement 1.2 — "exactly ONE repair-retry").
    let planner = SequencedFixtureGuiLlmPlanner::new(vec![
        GuiLlmPlannerFixture::InvalidJson,
        GuiLlmPlannerFixture::InvalidJson,
        GuiLlmPlannerFixture::ValidPlan,
    ]);
    let planner_ref: &dyn GuiLlmPlanner = &planner;
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_llm_planner(Some(planner_ref))
        .with_smart_planner(GuiSmartPlannerConfig::enabled());

    let outcome = runtime.run_turn(repair_flow_request("turn-one-repair")).await;

    // Exactly two planner calls total (first + one repair); the queued valid
    // third response is never reached.
    assert_eq!(planner.call_count(), 2, "bounded to first attempt + one repair");
    assert_eq!(planner.repair_call_count(), 1);
    assert_eq!(
        outcome.response["gui_cognition"]["planner"]["mode"],
        "llm_rejected_fallback"
    );
}

#[tokio::test]
async fn smart_planner_off_preserves_single_attempt_no_repair() {
    let perception = repair_flow_perception();
    let executor = FakeExecutor::available(true);
    // First attempt invalid; a valid repair response is queued but the flag is
    // OFF so it must never be used (prior single-attempt behavior preserved).
    let planner = SequencedFixtureGuiLlmPlanner::new(vec![
        GuiLlmPlannerFixture::InvalidJson,
        GuiLlmPlannerFixture::ValidPlan,
    ]);
    let planner_ref: &dyn GuiLlmPlanner = &planner;
    // Default config is OFF; assert that explicitly via `disabled()`.
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_llm_planner(Some(planner_ref))
        .with_smart_planner(GuiSmartPlannerConfig::disabled());

    let outcome = runtime.run_turn(repair_flow_request("turn-flag-off")).await;
    let types = event_types(&outcome);

    assert!(
        !types.contains(&"LlmPlanRepairRetry".to_string()),
        "no repair-retry when the flag is OFF"
    );
    assert!(types.contains(&"LlmPlanningFailed".to_string()));
    assert_eq!(planner.call_count(), 1, "single attempt only when flag is OFF");
    assert_eq!(planner.repair_call_count(), 0);
    assert_eq!(
        outcome.response["gui_cognition"]["planner"]["mode"],
        "llm_rejected_fallback"
    );
}

// ---------------------------------------------------------------------------
// Task 9.3 (Requirements 10, 11, 12, 13, 14, 15, 22, 23) — approval-gated
// actions: pause → execute ONLY on a fresh authorizing decision → NEVER on
// deny / expired / hash-mismatch; auto-approve fixtures honored ONLY in the
// TestSubstrate. The `gui_cog_safety_polish` flag adds the additive
// `ApprovalLifecycle` telemetry (paused → decision → executed/blocked/gated
// with verdict + hash-match/freshness status + carried decision id). While the
// flag is OFF these events are absent and behavior is byte-for-byte unchanged.
//
// All CI-safe: deterministic fixtures, no live KRIA desktop / display / network.
// Destructive/live approval coverage runs only in the substrate at the Task 9.7
// live gate; here we prove the invariants with the HITL decision fixtures.
// ---------------------------------------------------------------------------

const APPROVAL_PROMPT: &str =
    "Prepare to click a Submit button, but ask for approval before executing.";

fn approval_request(
    turn: &str,
    fixture: Option<GuiHitlDecisionFixture>,
    environment: GuiExecutionEnvironment,
    mode: GuiExecutionMode,
) -> GuiTurnRequest {
    GuiTurnRequest {
        session_id: "session-9-3".into(),
        turn_id: turn.into(),
        workflow_id: format!("workflow-{turn}"),
        message: APPROVAL_PROMPT.into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: fixture,
        execution_environment: environment,
        execution_mode: mode,
        workflow_enabled: false,
        resume_checkpoint: None,
        resume_reason: None,
    }
}

fn event_type_list(outcome: &kria_core::agent::gui_cognition::GuiTurnOutcome) -> Vec<String> {
    outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
}

fn approval_lifecycle(
    outcome: &kria_core::agent::gui_cognition::GuiTurnOutcome,
) -> Option<serde_json::Value> {
    outcome
        .events
        .iter()
        .find(|event| {
            event.get("type").and_then(serde_json::Value::as_str) == Some("ApprovalLifecycle")
        })
        .cloned()
}

fn substrate() -> GuiExecutionEnvironment {
    GuiExecutionEnvironment::TestSubstrate {
        scratch_dir: None,
        restore_clipboard: true,
    }
}

/// Flag discipline: the additive `ApprovalLifecycle` telemetry is emitted ONLY
/// when `gui_cog_safety_polish` is ON. While OFF it is absent and the prior
/// approval-gate behavior is byte-for-byte unchanged (still pauses, still
/// emits the HITL events).
#[tokio::test]
async fn approval_lifecycle_telemetry_is_flag_gated() {
    let perception = FakePerception::new(Vec::new(), vec![control("push button", "Submit")]);
    let executor = FakeExecutor::available(true);

    // Flag OFF (default): no ApprovalLifecycle event; gate still pauses.
    let runtime_off = GuiCognitionRuntime::new(&perception, &executor);
    let off = runtime_off
        .run_turn(approval_request(
            "turn-flag-off",
            None,
            GuiExecutionEnvironment::RealSession,
            GuiExecutionMode::SafetyOnly,
        ))
        .await;
    assert!(
        approval_lifecycle(&off).is_none(),
        "flag OFF must not emit ApprovalLifecycle, events: {:?}",
        event_type_list(&off)
    );
    assert_eq!(off.response["status"], "needs_approval");

    // Flag ON: the ApprovalLifecycle telemetry appears, paused awaiting human.
    let runtime_on = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled());
    let on = runtime_on
        .run_turn(approval_request(
            "turn-flag-on",
            None,
            GuiExecutionEnvironment::RealSession,
            GuiExecutionMode::SafetyOnly,
        ))
        .await;
    let lifecycle = approval_lifecycle(&on).expect("flag ON must emit ApprovalLifecycle");
    assert_eq!(lifecycle["paused"], true);
    assert_eq!(lifecycle["executed"], false);
    assert_eq!(lifecycle["outcome"], "gated_awaiting_human");
    assert_eq!(lifecycle["can_execute"], false);
    assert_eq!(on.response["status"], "needs_approval");
}

/// Approve in the substrate: the action executes on a FRESH authorizing
/// decision whose proposal/target hashes match the bound proposal and whose
/// decision id is carried into execution. The lifecycle telemetry records the
/// authorizing, hash-matched, fresh decision and an executed outcome.
#[tokio::test]
async fn approval_gated_action_executes_on_fresh_approval_in_substrate() {
    let perception = FakePerception::new(Vec::new(), vec![control("push button", "Submit")]);
    let executor = FakeExecutor::available(true);
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled());

    let outcome = runtime
        .run_turn(GuiTurnRequest {
            session_id: "session-9-3".into(),
            turn_id: "turn-approve-exec".into(),
            workflow_id: "workflow-approve-exec".into(),
            // A real, resolvable control whose "Submit" identity risk-escalates
            // the action so it is approval-gated (RED). On a fresh approval it
            // must execute against that exact bound target.
            message: "Click the visible button named Submit and verify the screen changed".into(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            hitl_decision_fixture: Some(GuiHitlDecisionFixture::Approve),
            execution_environment: substrate(),
            execution_mode: GuiExecutionMode::ExecuteFixture,
            workflow_enabled: false,
            resume_checkpoint: None,
            resume_reason: None,
        })
        .await;

    let types = event_type_list(&outcome);
    assert!(
        !types.contains(&"HitlFixtureRejected".to_string()),
        "fresh approval in substrate must not be rejected, events: {types:?}"
    );
    // It paused for approval first (the gate is real, not bypassed).
    assert!(
        types.contains(&"HitlRequired".to_string()),
        "approval-gated action must pause for approval, events: {types:?}"
    );
    assert!(
        types.contains(&"HitlDecisionRecorded".to_string()),
        "a fresh approval decision must be recorded, events: {types:?}"
    );
    // The action proceeds past the gate: it started (executed on approval).
    assert!(
        types.contains(&"ActionStarted".to_string()),
        "approval-gated action must execute on a fresh approval, events: {types:?}"
    );

    let lifecycle = approval_lifecycle(&outcome).expect("ApprovalLifecycle present");
    assert_eq!(lifecycle["paused"], true);
    assert_eq!(lifecycle["decision_verdict"], "approved");
    assert_eq!(lifecycle["authorizing"], true);
    assert_eq!(lifecycle["hash_matched"], true);
    assert_eq!(lifecycle["fresh"], true);
    assert_eq!(lifecycle["executed"], true);
    assert_eq!(lifecycle["outcome"], "executed_on_fresh_approval");
    // The carried decision id matches the recorded decision (bound to execution).
    let recorded = outcome
        .events
        .iter()
        .find(|e| e.get("type").and_then(serde_json::Value::as_str) == Some("HitlDecisionRecorded"))
        .expect("recorded decision");
    assert_eq!(lifecycle["decision_id"], recorded["decision_id"]);
    // Task 9.2 integration: the executed approval-gated action is recorded in the
    // append-only audit ledger with its authorization source + the SAME HITL
    // decision id that authorized it.
    let ledger = outcome
        .events
        .iter()
        .find(|e| {
            e.get("type").and_then(serde_json::Value::as_str)
                == Some("GuiActionLedgerEntryRecorded")
        })
        .expect("ledger entry recorded for an executed approval-gated action");
    assert_eq!(ledger["entry"]["authorization_source"], "hitl_approved");
    assert_eq!(ledger["entry"]["hitl_decision_id"], recorded["decision_id"]);
}

/// Deny: the action NEVER executes. The gate blocks and the lifecycle records a
/// non-authorizing `denied` verdict with no execution.
#[tokio::test]
async fn approval_gated_action_never_executes_on_deny() {
    let perception = FakePerception::new(Vec::new(), vec![control("push button", "Submit")]);
    let executor = FakeExecutor::available(true);
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled());

    let outcome = runtime
        .run_turn(approval_request(
            "turn-deny",
            Some(GuiHitlDecisionFixture::Deny),
            substrate(),
            GuiExecutionMode::ExecuteFixture,
        ))
        .await;

    let types = event_type_list(&outcome);
    assert!(
        !types.contains(&"ActionStarted".to_string()),
        "deny must never execute, events: {types:?}"
    );
    assert_eq!(outcome.response["status"], "blocked");

    let lifecycle = approval_lifecycle(&outcome).expect("ApprovalLifecycle present");
    assert_eq!(lifecycle["decision_verdict"], "denied");
    assert_eq!(lifecycle["authorizing"], false);
    assert_eq!(lifecycle["executed"], false);
    assert_eq!(lifecycle["outcome"], "blocked_denied");
}

/// Expired / hash-mismatch decisions NEVER execute: each yields a truthful
/// invalidated outcome (no `ActionStarted`), even inside the substrate. The
/// lifecycle records the verdict with `fresh=false` (expired) or
/// `hash_matched=false` (mismatch).
#[tokio::test]
async fn approval_gated_action_never_executes_on_expired_or_mismatch() {
    let cases = [
        (
            "turn-expired",
            GuiHitlDecisionFixture::ApproveExpired,
            "expired",
        ),
        (
            "turn-target-mismatch",
            GuiHitlDecisionFixture::ApproveTargetMismatch,
            "hash_mismatch_rejected",
        ),
        (
            "turn-proposal-mismatch",
            GuiHitlDecisionFixture::ApproveProposalMismatch,
            "hash_mismatch_rejected",
        ),
    ];

    for (turn, fixture, expected_verdict) in cases {
        let perception = FakePerception::new(Vec::new(), vec![control("push button", "Submit")]);
        let executor = FakeExecutor::available(true);
        let runtime = GuiCognitionRuntime::new(&perception, &executor)
            .with_safety_polish(GuiSafetyPolishConfig::enabled());

        let outcome = runtime
            .run_turn(approval_request(
                turn,
                Some(fixture),
                // Even in the substrate (where auto-approve is allowed) an
                // expired/mismatched decision is invalidated and never executes.
                substrate(),
                GuiExecutionMode::ExecuteFixture,
            ))
            .await;

        let types = event_type_list(&outcome);
        assert!(
            types.contains(&"HitlDecisionInvalidated".to_string()),
            "{turn}: expired/mismatch must invalidate, events: {types:?}"
        );
        assert!(
            !types.contains(&"ActionStarted".to_string()),
            "{turn}: invalidated decision must never execute, events: {types:?}"
        );

        let lifecycle = approval_lifecycle(&outcome).expect("ApprovalLifecycle present");
        assert_eq!(lifecycle["decision_verdict"], expected_verdict, "{turn}");
        assert_eq!(lifecycle["authorizing"], false, "{turn}");
        assert_eq!(lifecycle["executed"], false, "{turn}");
        assert_eq!(lifecycle["outcome"], "invalidated", "{turn}");
        if expected_verdict == "expired" {
            assert_eq!(lifecycle["fresh"], false, "{turn}: expired is not fresh");
        } else {
            assert_eq!(
                lifecycle["hash_matched"], false,
                "{turn}: mismatch must report hash_matched=false"
            );
        }
    }
}

/// Auto-approval is honored ONLY in the TestSubstrate. On the real session a
/// (would-be authorizing) fixture is rejected (`HitlFixtureRejected`), the
/// action stays gated, and the lifecycle records the refusal — nothing executes.
#[tokio::test]
async fn auto_approve_fixture_rejected_on_real_session_lifecycle() {
    let perception = FakePerception::new(Vec::new(), vec![control("push button", "Submit")]);
    let executor = FakeExecutor::available(true);
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled());

    let outcome = runtime
        .run_turn(approval_request(
            "turn-real-reject",
            Some(GuiHitlDecisionFixture::Approve),
            GuiExecutionEnvironment::RealSession,
            GuiExecutionMode::ExecuteFixture,
        ))
        .await;

    let types = event_type_list(&outcome);
    assert!(
        types.contains(&"HitlFixtureRejected".to_string()),
        "auto-approval must be rejected on real session, events: {types:?}"
    );
    assert!(
        !types.contains(&"ActionStarted".to_string()),
        "rejected fixture must never execute, events: {types:?}"
    );
    assert_eq!(outcome.response["status"], "needs_approval");

    let lifecycle = approval_lifecycle(&outcome).expect("ApprovalLifecycle present");
    assert_eq!(lifecycle["executed"], false);
    assert_eq!(lifecycle["outcome"], "fixture_rejected_outside_substrate");
    assert_eq!(lifecycle["environment"], "real_session");
}

// ---------------------------------------------------------------------------
// Task 9.4 (Requirements 10, 11, 12, 13, 14, 15, 22, 23) — ambiguity → ask
// (never guess); boundaries strictly respected; verify-and-stop terminates
// after verification. The `gui_cog_safety_polish` flag adds the additive
// `AmbiguityNoGuess`, `BoundaryCheck`, and `VerifyAndStopTerminated` telemetry
// that make each invariant inspectable. While the flag is OFF those events are
// absent and the turn is byte-for-byte unchanged.
//
// All CI-safe: deterministic fixtures, no live KRIA desktop / display / network.
// ---------------------------------------------------------------------------

fn find_event(
    outcome: &kria_core::agent::gui_cognition::GuiTurnOutcome,
    event_type: &str,
) -> Option<serde_json::Value> {
    outcome
        .events
        .iter()
        .find(|event| {
            event.get("type").and_then(serde_json::Value::as_str) == Some(event_type)
        })
        .cloned()
}

fn workflow_request(turn: &str, message: &str, polish_on: bool) -> GuiTurnRequest {
    let _ = polish_on;
    GuiTurnRequest {
        session_id: "session-9-4".into(),
        turn_id: turn.into(),
        workflow_id: format!("workflow-{turn}"),
        message: message.into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: None,
        execution_environment: GuiExecutionEnvironment::TestSubstrate {
            scratch_dir: None,
            restore_clipboard: true,
        },
        execution_mode: GuiExecutionMode::ExecuteFixture,
        workflow_enabled: true,
        resume_checkpoint: None,
        resume_reason: None,
    }
}

/// AMBIGUITY → ASK, NEVER GUESS: when two controls match the named target, the
/// runtime pauses and asks — it NEVER starts an action on a guessed target. The
/// `AmbiguityNoGuess` telemetry (flag ON) records the no-guess decision; with the
/// flag OFF the pause behavior is unchanged but no telemetry is emitted.
#[tokio::test]
async fn ambiguity_pauses_and_asks_never_guesses_flag_gated() {
    // Two buttons share the same name → resolution is ambiguous (no unique
    // target). A guess would pick one; KRIA must refuse to guess.
    let perception = FakePerception::new(
        Vec::new(),
        vec![control("push button", "Save"), control("push button", "Save")],
    );
    let executor = FakeExecutor::available(true);

    // Flag OFF: the run still pauses/blocks (never guesses) but emits no
    // AmbiguityNoGuess telemetry.
    let runtime_off = GuiCognitionRuntime::new(&perception, &executor);
    let off = runtime_off
        .run_turn(workflow_request(
            "ambiguity-off",
            "Click the visible button named Save and verify the screen changed",
            false,
        ))
        .await;
    assert!(
        find_event(&off, "AmbiguityNoGuess").is_none(),
        "flag OFF must not emit AmbiguityNoGuess, events: {:?}",
        event_type_list(&off)
    );
    assert!(
        !event_type_list(&off).contains(&"ActionStarted".to_string()),
        "ambiguous target must NEVER start an action (no guessing), events: {:?}",
        event_type_list(&off)
    );

    // Flag ON: the AmbiguityNoGuess telemetry appears with the no-guess flag set,
    // and still no action is started on a guessed target.
    let runtime_on = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled());
    let on = runtime_on
        .run_turn(workflow_request(
            "ambiguity-on",
            "Click the visible button named Save and verify the screen changed",
            true,
        ))
        .await;
    let ambiguity = find_event(&on, "AmbiguityNoGuess")
        .expect("flag ON must emit AmbiguityNoGuess for an ambiguous target");
    assert_eq!(ambiguity["decision"], "ask");
    assert_eq!(ambiguity["no_guess"], true);
    assert_eq!(ambiguity["can_execute"], false);
    assert!(
        ambiguity["candidate_count"].as_u64().unwrap_or(0) >= 2,
        "ambiguity telemetry must record the multiple candidates: {ambiguity}"
    );
    assert!(
        !event_type_list(&on).contains(&"ActionStarted".to_string()),
        "ambiguous target must NEVER start an action (no guessing), events: {:?}",
        event_type_list(&on)
    );
}

/// BOUNDARIES STRICTLY RESPECTED: a normal, in-scope action stays within the
/// requested capability boundary. The `BoundaryCheck` telemetry (flag ON)
/// records the within-bounds decision; with the flag OFF no boundary telemetry
/// is emitted and the gate behavior is unchanged.
#[tokio::test]
async fn boundary_check_records_within_bounds_for_in_scope_action_flag_gated() {
    let perception = FakePerception::new(Vec::new(), vec![control("push button", "Save")]);
    let executor = FakeExecutor::available(true);

    let request = |turn: &str| GuiTurnRequest {
        session_id: "session-9-4".into(),
        turn_id: turn.into(),
        workflow_id: format!("workflow-{turn}"),
        message: "Click the visible button named Save and verify the screen changed".into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: None,
        execution_environment: GuiExecutionEnvironment::RealSession,
        execution_mode: GuiExecutionMode::SafetyOnly,
        workflow_enabled: false,
        resume_checkpoint: None,
        resume_reason: None,
    };

    // Flag OFF: no BoundaryCheck telemetry.
    let runtime_off = GuiCognitionRuntime::new(&perception, &executor);
    let off = runtime_off.run_turn(request("boundary-off")).await;
    assert!(
        find_event(&off, "BoundaryCheck").is_none(),
        "flag OFF must not emit BoundaryCheck, events: {:?}",
        event_type_list(&off)
    );

    // Flag ON: BoundaryCheck telemetry appears, action stays within bounds and is
    // not refused.
    let runtime_on = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled());
    let on = runtime_on.run_turn(request("boundary-on")).await;
    let boundary =
        find_event(&on, "BoundaryCheck").expect("flag ON must emit BoundaryCheck at the gate");
    assert_eq!(boundary["within_bounds"], true, "{boundary}");
    assert_eq!(boundary["refused"], false, "{boundary}");
    assert_eq!(boundary["crossing_kind"], serde_json::Value::Null);
    assert_eq!(boundary["can_execute"], false);
}

/// VERIFY-AND-STOP TERMINATES AFTER VERIFICATION: a verify-and-stop intent
/// observes → verifies → then STOPS with NO state-changing action. The
/// `VerifyAndStopTerminated` telemetry (flag ON) asserts zero state-changing
/// actions executed; with the flag OFF no such telemetry is emitted and the turn
/// is unchanged. In both cases NO action is started.
#[tokio::test]
async fn verify_and_stop_terminates_after_verification_flag_gated() {
    let perception = FakePerception::new(Vec::new(), vec![control("push button", "Save")]);
    let executor = FakeExecutor::available(true);
    let prompt =
        "Verify that the Save button is visible and then stop without any further action";

    // Flag OFF: no VerifyAndStopTerminated telemetry; still no action started.
    let runtime_off = GuiCognitionRuntime::new(&perception, &executor);
    let off = runtime_off
        .run_turn(workflow_request("verify-stop-off", prompt, false))
        .await;
    assert!(
        find_event(&off, "VerifyAndStopTerminated").is_none(),
        "flag OFF must not emit VerifyAndStopTerminated, events: {:?}",
        event_type_list(&off)
    );
    assert!(
        !event_type_list(&off).contains(&"ActionStarted".to_string()),
        "verify-and-stop must not start any state-changing action, events: {:?}",
        event_type_list(&off)
    );

    // Flag ON: the VerifyAndStopTerminated telemetry asserts the turn observed →
    // verified → stopped with zero state-changing actions executed.
    let runtime_on = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled());
    let on = runtime_on
        .run_turn(workflow_request("verify-stop-on", prompt, true))
        .await;
    let terminated = find_event(&on, "VerifyAndStopTerminated")
        .expect("flag ON must emit VerifyAndStopTerminated for a verify-and-stop plan");
    assert_eq!(terminated["verified_then_stopped"], true, "{terminated}");
    assert_eq!(terminated["state_changing_actions_executed"], 0, "{terminated}");
    assert_eq!(terminated["terminal_step_type"], "VerifyState");
    assert!(
        !event_type_list(&on).contains(&"ActionStarted".to_string()),
        "verify-and-stop must not start any state-changing action, events: {:?}",
        event_type_list(&on)
    );
}
