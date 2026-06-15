use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};

use kria_core::agent::gui_cognition::checkpoint::GuiWorkflowCheckpoint;
use kria_core::agent::gui_cognition::executor::{
    GuiActionBackendStatus, GuiActionExecution, GuiActionExecutor, GuiActionRequest,
    GuiExecutionMode,
};
use kria_core::agent::gui_cognition::llm_planner::GuiTypedPlanStep;
use kria_core::agent::gui_cognition::perception::{GuiProbeResult, GuiPerceptionProvider};
use kria_core::agent::gui_cognition::workflow_runtime::{
    workflow_step_is_state_changing, workflow_step_kind, workflow_step_requires_target,
    readiness_wait_event, target_presence_event, GuiWorkflowRun, GuiWorkflowStepKind,
    GuiWorkflowStepState,
};
use kria_core::agent::gui_cognition::turn_budget::GuiReobserveConfig;
use kria_core::agent::gui_cognition::{GuiCognitionRuntime, GuiTurnRequest};

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

// ── Contract serialization & classification ────────────────────────────────

#[test]
fn workflow_step_state_serializes_safely() {
    let step = typed_step("step-1", "FocusField");
    let state = GuiWorkflowStepState::pending(&step, 0, "prompt-hash");
    let json = state.summary_json();
    assert_eq!(json["status"], "pending");
    assert_eq!(json["step_type"], "FocusField");
    let serialized = serde_json::to_string(&json).unwrap();
    assert!(!serialized.contains("raw_prompt"));
    assert!(!serialized.contains("password"));
}

#[test]
fn workflow_run_serializes_safely() {
    let steps = vec![
        typed_step("s0", "OpenApp"),
        typed_step("s1", "FocusField"),
        typed_step("s2", "TypeText"),
    ];
    let run = GuiWorkflowRun::new(
        "session-1",
        "workflow-1",
        "turn-1",
        "goal-1",
        "plan-1",
        "context-1",
        &steps,
        "low",
        false,
        "execute_fixture",
        "prompt-hash",
    );
    assert_eq!(run.step_count, 3);
    assert_eq!(run.status, "running");
    let started = run.run_started_event();
    assert_eq!(started["type"], "WorkflowRunStarted");
    assert_eq!(started["step_count"], 3);
    let serialized = serde_json::to_string(&run.summary_json()).unwrap();
    assert!(!serialized.contains("raw_prompt"));
    assert!(!serialized.contains("SECRET"));
}

#[test]
fn workflow_step_classification_is_correct() {
    assert_eq!(workflow_step_kind("OpenApp"), GuiWorkflowStepKind::Executable);
    assert_eq!(workflow_step_kind("TypeText"), GuiWorkflowStepKind::Executable);
    assert_eq!(
        workflow_step_kind("WaitForState"),
        GuiWorkflowStepKind::WaitOrVerify
    );
    assert_eq!(
        workflow_step_kind("AskClarification"),
        GuiWorkflowStepKind::AskClarification
    );
    assert_eq!(
        workflow_step_kind("RequireApproval"),
        GuiWorkflowStepKind::RequireApproval
    );
    assert_eq!(
        workflow_step_kind("SummarizeVisibleContent"),
        GuiWorkflowStepKind::Summarize
    );
    assert_eq!(workflow_step_kind("Observe"), GuiWorkflowStepKind::Observe);

    assert!(workflow_step_requires_target("FocusField"));
    assert!(workflow_step_requires_target("TypeText"));
    assert!(workflow_step_requires_target("ClickControl"));
    assert!(!workflow_step_requires_target("OpenApp"));

    assert!(workflow_step_is_state_changing("OpenApp"));
    assert!(workflow_step_is_state_changing("ClickControl"));
    assert!(!workflow_step_is_state_changing("FocusField"));
}

// ── Task 3.3: bounded readiness-wait event ──────────────────────────────────

#[test]
fn readiness_wait_event_ready_shape_surfaces_cap_binding() {
    // A target that is already observable resolves the wait with ready=true and
    // zero additional re-observes, surfacing the Task 1 cap binding so the wait
    // is provably bounded (Property 9). No `reason` on the ready path.
    let event = readiness_wait_event(1, "readiness_wait", Some("Google Chrome"), true, 0, 1, 16, None);
    assert_eq!(event["type"], "WorkflowReadinessWait");
    assert_eq!(event["step_index"], 1);
    assert_eq!(event["ready"], true);
    assert_eq!(event["attempts"], 0);
    assert_eq!(event["max_reobserve"], 16);
    assert_eq!(event["bounded_by_runaway_caps"], true);
    assert_eq!(event["expected_hint"], "Google Chrome");
    assert!(event["reason"].is_null());
}

#[test]
fn readiness_wait_event_not_ready_shape_carries_sanitized_reason() {
    let reason = "Expected window did not become ready within the bounded re-observe budget";
    let event = readiness_wait_event(2, "readiness_wait", Some("Firefox"), false, 4, 16, 16, Some(reason));
    assert_eq!(event["ready"], false);
    assert_eq!(event["attempts"], 4);
    assert_eq!(event["reobserve_count"], 16);
    assert_eq!(event["bounded_by_runaway_caps"], true);
    assert_eq!(event["reason"], reason);
    // Sanitized + no raw secret/prompt fields.
    let serialized = serde_json::to_string(&event).unwrap();
    assert!(!serialized.contains("raw_prompt"));
}

// ── Task 3.4: present-after-change vs genuinely-absent event shape ───────────

#[test]
fn target_presence_event_present_after_change_continues_resolved() {
    // Present after change AND re-resolved → the workflow CONTINUES; no false
    // "resolved target is no longer present" stop, no reason on the continue path.
    let event = target_presence_event(
        2,
        Some("Search"),
        "present_after_change",
        true,
        1,
        2,
        16,
        None,
    );
    assert_eq!(event["type"], "WorkflowTargetPresence");
    assert_eq!(event["step_index"], 2);
    assert_eq!(event["decision"], "present_after_change");
    assert_eq!(event["resolved"], true);
    assert_eq!(event["expected_hint"], "Search");
    assert_eq!(event["bounded_by_runaway_caps"], true);
    // The decision is driven by observation evidence, never the action kind.
    assert_eq!(event["decided_from_observation_evidence"], true);
    assert!(event["reason"].is_null());
}

#[test]
fn target_presence_event_genuinely_absent_carries_sanitized_reason() {
    let reason =
        "The expected target 'Search' is not present on the current screen after a bounded re-observe, so I stopped safely.";
    let event = target_presence_event(
        3,
        Some("Search"),
        "genuinely_absent",
        false,
        16,
        16,
        16,
        Some(reason),
    );
    assert_eq!(event["decision"], "genuinely_absent");
    assert_eq!(event["resolved"], false);
    assert_eq!(event["reobserve_count"], 16);
    assert_eq!(event["max_reobserve"], 16);
    assert_eq!(event["bounded_by_runaway_caps"], true);
    assert_eq!(event["reason"], reason);
    // A genuinely-absent stop must NOT masquerade as the present case.
    let serialized = serde_json::to_string(&event).unwrap();
    assert!(!serialized.contains("raw_prompt"));
}

// ── Integration provider/executor ──────────────────────────────────────────

struct WorkflowPerception {
    active_window: String,
    controls_present: bool,
    screen_seq: AtomicU64,
}

impl WorkflowPerception {
    fn new(active_window: &str) -> Self {
        Self {
            active_window: active_window.into(),
            controls_present: true,
            screen_seq: AtomicU64::new(0),
        }
    }

    fn without_controls(active_window: &str) -> Self {
        Self {
            active_window: active_window.into(),
            controls_present: false,
            screen_seq: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl GuiPerceptionProvider for WorkflowPerception {
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
        if !self.controls_present {
            return GuiProbeResult::ok(serde_json::json!({ "elements": [] }));
        }
        let elements = match role {
            "text" => vec![serde_json::json!({
                "role": "text",
                "name": "Search",
                "label": "Search",
                "path": "/workflow/text/Search",
                "control_id": "workflow-search-field",
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
                "path": "/workflow/button/Search",
                "control_id": "workflow-search-button",
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
            "focused_control_id": "workflow-search-field",
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
        // Screen hash changes every observation so screen_changed strategies pass.
        let seq = self.screen_seq.fetch_add(1, Ordering::SeqCst);
        GuiProbeResult::ok(serde_json::json!({
            "screen_hash": format!("workflow-screen-{seq}"),
            "byte_count": 16,
            "source": "fixture",
        }))
    }

    async fn focused_window_title(&self) -> Option<String> {
        Some(self.active_window.clone())
    }
}

struct WorkflowExecutor {
    backend: GuiActionBackendStatus,
}

impl WorkflowExecutor {
    fn new() -> Self {
        Self {
            backend: GuiActionBackendStatus::available("fixture_executor"),
        }
    }
}

#[async_trait]
impl GuiActionExecutor for WorkflowExecutor {
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

fn event_types(outcome: &kria_core::agent::gui_cognition::GuiTurnOutcome) -> Vec<String> {
    outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .map(|value| value.to_string())
        .collect()
}

fn workflow_request(message: &str, mode: GuiExecutionMode) -> GuiTurnRequest {
    GuiTurnRequest {
        session_id: "session-1".into(),
        turn_id: "turn-1".into(),
        workflow_id: "workflow-1".into(),
        message: message.into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: None,
        execution_environment: Default::default(),
        execution_mode: mode,
        workflow_enabled: true,
        resume_checkpoint: None,
        resume_reason: None,
    }
}

#[tokio::test]
async fn workflow_starts_from_typed_plan_and_emits_run_events() {
    let perception = WorkflowPerception::new("KRIA Workflow App");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(workflow_request(
            "Open KRIA Workflow App and focus the visible search field",
            GuiExecutionMode::ExecuteFixture,
        ))
        .await;

    let types = event_types(&outcome);
    assert!(types.contains(&"WorkflowRunStarted".to_string()), "events: {types:?}");
    assert!(types.contains(&"WorkflowStepStarted".to_string()));
    assert!(
        types.iter().any(|t| t == "WorkflowRunCompleted"
            || t == "WorkflowRunBlocked"
            || t == "WorkflowRunPaused"),
        "events: {types:?}"
    );
    // workflow_run must be present and contain no raw prompt.
    let workflow_run = outcome.response.pointer("/gui_cognition/workflow_run");
    assert!(workflow_run.is_some());
    assert!(!serde_json::to_string(&outcome.response).unwrap().contains("raw_prompt"));
}

#[tokio::test]
async fn workflow_safety_only_does_not_emit_action_started() {
    let perception = WorkflowPerception::new("KRIA Workflow App");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(workflow_request(
            "Open KRIA Workflow App and focus the visible search field",
            GuiExecutionMode::SafetyOnly,
        ))
        .await;

    let types = event_types(&outcome);
    assert!(types.contains(&"WorkflowRunStarted".to_string()));
    assert!(types.contains(&"WorkflowStepStarted".to_string()));
    assert!(!types.contains(&"ActionStarted".to_string()), "events: {types:?}");
}

#[tokio::test]
async fn workflow_blocks_when_first_step_verification_fails_before_next_step() {
    // No controls are present, so a ClickControl step cannot resolve its target;
    // the workflow must block before any ActionStarted and never complete.
    let perception = WorkflowPerception::without_controls("KRIA Workflow App");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(workflow_request(
            "Click the Search button and verify the screen changed",
            GuiExecutionMode::ExecuteFixture,
        ))
        .await;

    let types = event_types(&outcome);
    assert!(types.contains(&"WorkflowRunStarted".to_string()));
    assert!(
        types.iter().any(|t| t == "WorkflowStepBlocked" || t == "WorkflowRunBlocked"),
        "events: {types:?}"
    );
    assert!(!types.contains(&"ActionStarted".to_string()), "events: {types:?}");
    assert!(!types.contains(&"WorkflowRunCompleted".to_string()));
}

#[tokio::test]
async fn workflow_no_raw_secret_leakage() {
    let perception = WorkflowPerception::new("KRIA Workflow App");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    // ZZZRAWPROMPTMARKER appears only in the raw prompt (outside any quoted
    // payload), so it must never be echoed into events or the response.
    let outcome = runtime
        .run_turn(workflow_request(
            "ZZZRAWPROMPTMARKER open KRIA Workflow App and focus the visible search field",
            GuiExecutionMode::ExecuteFixture,
        ))
        .await;

    let serialized = serde_json::to_string(&outcome.response).unwrap();
    assert!(!serialized.contains("ZZZRAWPROMPTMARKER"), "raw prompt leaked in response");
    assert!(!serialized.contains("\"raw_prompt\""), "raw_prompt field exposed");
    let events = serde_json::to_string(&outcome.events).unwrap();
    assert!(!events.contains("ZZZRAWPROMPTMARKER"), "raw prompt leaked in events");
}

#[tokio::test]
async fn workflow_saves_checkpoint_after_each_step() {
    let perception = WorkflowPerception::new("KRIA Workflow App");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(workflow_request(
            "Open KRIA Workflow App and focus the visible search field",
            GuiExecutionMode::ExecuteFixture,
        ))
        .await;

    let types = event_types(&outcome);
    assert!(types.contains(&"WorkflowCheckpointSaved".to_string()), "events: {types:?}");
    let checkpoint = outcome
        .response
        .pointer("/gui_cognition/workflow_checkpoint")
        .filter(|value| !value.is_null());
    assert!(checkpoint.is_some(), "checkpoint must be present in response");
    let serialized = serde_json::to_string(&outcome.response).unwrap();
    assert!(!serialized.contains("\"raw_prompt\""));
}

#[tokio::test]
async fn resume_after_completed_run_does_not_replay_completed_steps() {
    let perception = WorkflowPerception::new("KRIA Workflow App");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    // First run completes the workflow and saves a checkpoint.
    let first = runtime
        .run_turn(workflow_request(
            "Open KRIA Workflow App and focus the visible search field",
            GuiExecutionMode::ExecuteFixture,
        ))
        .await;
    let checkpoint_value = first
        .response
        .pointer("/gui_cognition/workflow_checkpoint")
        .cloned()
        .expect("checkpoint present");
    let checkpoint: GuiWorkflowCheckpoint =
        serde_json::from_value(checkpoint_value).expect("deserialize checkpoint");

    // Resume from the checkpoint: completed steps must not be re-executed.
    let mut resume_request = workflow_request(
        "Open KRIA Workflow App and focus the visible search field",
        GuiExecutionMode::ExecuteFixture,
    );
    resume_request.resume_checkpoint = Some(checkpoint);
    resume_request.resume_reason = Some("user_resume".into());

    let resumed = runtime.run_turn(resume_request).await;
    let types = event_types(&resumed);
    assert!(types.contains(&"WorkflowResumeRequested".to_string()), "events: {types:?}");
    assert!(types.contains(&"WorkflowCheckpointLoaded".to_string()));
    assert!(types.contains(&"WorkflowResumeValidated".to_string()));
    // All steps were already completed, so no new action is started on resume.
    assert!(!types.contains(&"ActionStarted".to_string()), "events: {types:?}");
}

#[tokio::test]
async fn resume_rejects_when_screen_changed_blocks_before_action() {
    let perception = WorkflowPerception::new("KRIA Workflow App");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let first = runtime
        .run_turn(workflow_request(
            "Open KRIA Workflow App and focus the visible search field",
            GuiExecutionMode::ExecuteFixture,
        ))
        .await;
    let checkpoint_value = first
        .response
        .pointer("/gui_cognition/workflow_checkpoint")
        .cloned()
        .expect("checkpoint present");
    let mut checkpoint: GuiWorkflowCheckpoint =
        serde_json::from_value(checkpoint_value).expect("deserialize checkpoint");
    // Tamper the recorded screen prefix so the current screen no longer matches,
    // but pretend an incomplete pending step remains so resume must revalidate.
    checkpoint.last_screen_hash_prefix = Some("totally-different-screen".into());
    checkpoint.current_step_index = 0;
    checkpoint.pending_step_id = checkpoint
        .step_states
        .first()
        .and_then(|state| state.get("step_id"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    checkpoint.completed_step_receipts.clear();
    checkpoint.pending_target_hash = Some("target-hash".into());
    checkpoint.checkpoint_hash =
        kria_core::agent::gui_cognition::checkpoint::checkpoint_hash(&checkpoint);

    let mut resume_request = workflow_request(
        "Open KRIA Workflow App and focus the visible search field",
        GuiExecutionMode::ExecuteFixture,
    );
    resume_request.resume_checkpoint = Some(checkpoint);

    let resumed = runtime.run_turn(resume_request).await;
    let types = event_types(&resumed);
    assert!(types.contains(&"WorkflowResumeRequested".to_string()));
    assert!(types.contains(&"WorkflowCheckpointLoaded".to_string()));
    // Screen changed -> target identity can't be trusted -> reject before action.
    assert!(
        types.iter().any(|t| t == "WorkflowResumeRejected"),
        "events: {types:?}"
    );
    assert!(!types.contains(&"ActionStarted".to_string()));
}

// ── Task 3.4: present-after-change vs genuinely-absent (integration) ─────────

fn find_event<'a>(
    outcome: &'a kria_core::agent::gui_cognition::GuiTurnOutcome,
    event_type: &str,
) -> Option<&'a serde_json::Value> {
    outcome
        .events
        .iter()
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some(event_type))
}

#[tokio::test]
async fn workflow_reobserve_classifies_genuinely_absent_target_and_stops_without_false_no_longer_present() {
    // No controls are observable, so a ClickControl target cannot resolve. With
    // `gui_cog_reobserve` ON the runtime classifies present-after-change vs
    // genuinely-absent from REAL observation evidence: the target is genuinely
    // absent (no matching control on the fresh screen), so it stops safely with
    // a clear reason — and NEVER with a false "resolved target is no longer
    // present" (Requirement 2.4, Property 2/8).
    let perception = WorkflowPerception::without_controls("KRIA Workflow App");
    let executor = WorkflowExecutor::new();
    let runtime =
        GuiCognitionRuntime::new(&perception, &executor).with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(
            "Click the Search button and verify the screen changed",
            GuiExecutionMode::ExecuteFixture,
        ))
        .await;

    let types = event_types(&outcome);
    assert!(
        types.contains(&"WorkflowTargetPresence".to_string()),
        "expected a present/absent classification event: {types:?}"
    );
    let presence = find_event(&outcome, "WorkflowTargetPresence").unwrap();
    assert_eq!(presence["decision"], "genuinely_absent", "presence: {presence}");
    assert_eq!(presence["resolved"], false);
    assert_eq!(presence["bounded_by_runaway_caps"], true);
    assert_eq!(presence["decided_from_observation_evidence"], true);

    // The workflow stops safely, never executes, and the stop reason must NOT be
    // the false "no longer present" absence claim.
    assert!(
        types.iter().any(|t| t == "WorkflowStepBlocked" || t == "WorkflowRunBlocked"),
        "events: {types:?}"
    );
    assert!(!types.contains(&"ActionStarted".to_string()), "events: {types:?}");
    assert!(!types.contains(&"WorkflowRunCompleted".to_string()));
    let serialized = serde_json::to_string(&outcome.response).unwrap();
    assert!(
        !serialized.contains("no longer present"),
        "must not emit the false 'no longer present' stop: {serialized}"
    );
}

#[tokio::test]
async fn workflow_flag_off_preserves_behavior_and_emits_no_presence_classification() {
    // With `gui_cog_reobserve` OFF (default) the same unresolved-target case is
    // handled by the exact prior block-and-stop path: no present/absent
    // classification event is emitted and no action is started.
    let perception = WorkflowPerception::without_controls("KRIA Workflow App");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(workflow_request(
            "Click the Search button and verify the screen changed",
            GuiExecutionMode::ExecuteFixture,
        ))
        .await;

    let types = event_types(&outcome);
    assert!(
        !types.contains(&"WorkflowTargetPresence".to_string()),
        "flag OFF must not classify present/absent: {types:?}"
    );
    assert!(
        types.iter().any(|t| t == "WorkflowStepBlocked" || t == "WorkflowRunBlocked"),
        "events: {types:?}"
    );
    assert!(!types.contains(&"ActionStarted".to_string()), "events: {types:?}");
}

// ── Task 8.2: cross-app clipboard combo (copy → switch → paste) ──────────────

use kria_core::agent::gui_cognition::clipboard::GuiCrossAppConfig;

/// Perception fixture for the cross-app combo: a target app window that exposes
/// a single visible/focused text input (the paste target). Used so FocusField in
/// the combo resolves against the fresh post-switch context.
struct CrossAppPerception {
    active_window: String,
    screen_seq: AtomicU64,
}

impl CrossAppPerception {
    fn new(active_window: &str) -> Self {
        Self {
            active_window: active_window.into(),
            screen_seq: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl GuiPerceptionProvider for CrossAppPerception {
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
            "applications": [self.active_window, "Chrome", "VS Code"],
        }))
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": true }))
    }

    async fn find_ui_elements(&self, role: &str) -> GuiProbeResult {
        let elements = match role {
            "text" => vec![serde_json::json!({
                "role": "text",
                "name": "Editor input",
                "label": "Editor input",
                "path": "/crossapp/text/editor",
                "control_id": "crossapp-editor-field",
                "enabled": true,
                "visible": true,
                "focused": true,
                "in_active_window": true,
                "bounds": { "x": 10, "y": 20, "width": 320, "height": 32 },
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
            "focused_control_id": "crossapp-editor-field",
            "focused_control_label": "Editor input",
            "focused_control_role": "text",
            "keyboard_focus_known": true,
            "text_cursor_known": true,
            "editable_target_known": true,
            "terminal_like": false,
            "focus_confidence": 0.9,
        }))
    }

    async fn capture_screenshot(&self) -> GuiProbeResult {
        let seq = self.screen_seq.fetch_add(1, Ordering::SeqCst);
        GuiProbeResult::ok(serde_json::json!({
            "screen_hash": format!("crossapp-screen-{seq}"),
            "byte_count": 16,
            "source": "fixture",
        }))
    }

    async fn focused_window_title(&self) -> Option<String> {
        Some(self.active_window.clone())
    }
}

const CROSS_APP_PROMPT: &str = "Copy the selected text from Chrome and paste it into VS Code";

/// Collect the validated plan's step types from the `PlanValidationCompleted`
/// event's `step_results` — the authoritative plan structure (independent of how
/// far execution later progresses).
fn validated_step_types(
    outcome: &kria_core::agent::gui_cognition::GuiTurnOutcome,
) -> Vec<String> {
    let validation = find_event(outcome, "PlanValidationCompleted")
        .expect("a PlanValidationCompleted event must be emitted");
    validation
        .get("step_results")
        .and_then(serde_json::Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter_map(|s| s.get("step_type").and_then(serde_json::Value::as_str))
                .map(|v| v.to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn crossapp_combo_flag_on_produces_complete_copy_switch_paste_plan() {
    // Flag ON: the cross-app clipboard combo prompt produces the complete valid
    // typed sequence Copy → SwitchWindow → FocusField → Paste → VerifyState.
    let perception = CrossAppPerception::new("VS Code");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_crossapp(GuiCrossAppConfig::enabled())
        .with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(CROSS_APP_PROMPT, GuiExecutionMode::ExecuteFixture))
        .await;

    let step_types = validated_step_types(&outcome);
    assert_eq!(
        step_types,
        vec!["Copy", "SwitchWindow", "FocusField", "Paste", "VerifyState"],
        "flag ON must emit the full cross-app combo sequence: {step_types:?}"
    );
}

#[tokio::test]
async fn crossapp_combo_flag_on_reaches_valid_for_resolution() {
    // The combo plan must be execution-ready (valid_for_resolution), not blocked
    // by the single-app contradiction check.
    let perception = CrossAppPerception::new("VS Code");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_crossapp(GuiCrossAppConfig::enabled())
        .with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(CROSS_APP_PROMPT, GuiExecutionMode::ExecuteFixture))
        .await;

    let validation = find_event(&outcome, "PlanValidationCompleted")
        .expect("plan validation event must be emitted");
    assert_eq!(
        validation["readiness_status"], "valid_for_resolution",
        "combo plan must reach valid_for_resolution: {validation}"
    );
    // The single-app contradiction check must NOT fire for a legitimate combo.
    let serialized = serde_json::to_string(&outcome.response).unwrap();
    assert!(
        !serialized.contains("contradicts goal contract"),
        "combo plan must not be blocked as a single-app contradiction: {serialized}"
    );
}

#[tokio::test]
async fn crossapp_combo_flag_on_is_multistep_with_reobserved_state_changes() {
    // Flag ON: the combo is a MULTI-STEP plan whose SwitchWindow and Paste steps
    // are state-changing, so the runtime's per-step re-observe (Task 3)
    // re-observes after them and resolves the next target against the fresh
    // post-switch screen (Requirement 2) — i.e. the paste targets the target
    // app's REAL focused field, not the stale initial observation. (The
    // re-observe mechanism itself is proven by the Task 3 integration test
    // above; here we assert the combo is the multi-step shape that triggers it.)
    let perception = CrossAppPerception::new("VS Code");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_crossapp(GuiCrossAppConfig::enabled())
        .with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(CROSS_APP_PROMPT, GuiExecutionMode::ExecuteFixture))
        .await;

    // The combo is a complete multi-step plan (5 steps) — the precondition for
    // per-step re-observe between steps.
    let step_types = validated_step_types(&outcome);
    assert_eq!(step_types.len(), 5, "combo must be a 5-step plan: {step_types:?}");

    // The combo's SwitchWindow and Paste steps are state-changing, so the runtime
    // re-observes after them and re-resolves the next step against the fresh
    // screen (Task 3 / Requirement 2). FocusField re-resolves against that fresh
    // context, so the paste lands on the target app's real focused field.
    assert!(workflow_step_is_state_changing("SwitchWindow"));
    assert!(workflow_step_is_state_changing("Paste"));
    assert!(workflow_step_requires_target("FocusField"));

    // The combo drives the workflow runtime (not a one-shot validate).
    let types = event_types(&outcome);
    assert!(
        types.contains(&"WorkflowRunStarted".to_string())
            && types.contains(&"WorkflowStepStarted".to_string()),
        "combo must drive the workflow runtime: {types:?}"
    );
}

#[tokio::test]
async fn crossapp_combo_flag_off_preserves_single_copy_plan() {
    // Flag OFF (default): the SAME prompt keeps the existing single-copy
    // primitive plan (Focus → Copy → Verify) — no SwitchWindow / Paste combo.
    let perception = CrossAppPerception::new("VS Code");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(CROSS_APP_PROMPT, GuiExecutionMode::ExecuteFixture))
        .await;

    let step_types = validated_step_types(&outcome);
    assert!(
        !step_types.contains(&"SwitchWindow".to_string())
            && !step_types.contains(&"Paste".to_string()),
        "flag OFF must NOT emit the cross-app combo: {step_types:?}"
    );
    // The flag-OFF plan is the existing single copy primitive (copy_steps).
    assert!(
        step_types.first().map(String::as_str) == Some("FocusField")
            && step_types.contains(&"Copy".to_string()),
        "flag OFF must keep the single copy primitive plan: {step_types:?}"
    );
}

// ── Task 8.3: file-manager NON-DESTRUCTIVE select flow (navigate→select→name) ─

/// Perception fixture for the file-manager select flow: a file-manager window
/// ("Files") that exposes observable file entries (the list the selection is
/// driven by). Used so the FocusField select step resolves against the observed
/// file list rather than an invented filename.
struct FileManagerPerception {
    screen_seq: AtomicU64,
}

impl FileManagerPerception {
    fn new() -> Self {
        Self {
            screen_seq: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl GuiPerceptionProvider for FileManagerPerception {
    async fn get_active_window(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "title": "Files",
            "app_name": "Files",
        }))
    }

    async fn get_desktop_state(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": "Files",
            "accessibility_operational": true,
            "applications": ["Files", "file manager"],
        }))
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": true }))
    }

    async fn find_ui_elements(&self, role: &str) -> GuiProbeResult {
        // Observed file entries — the selection is driven by THESE, by observed
        // order/position, never an invented filename.
        let elements = match role {
            "text" | "list item" | "table cell" => vec![
                serde_json::json!({
                    "role": "list item",
                    "name": "report-2024-06-01.pdf",
                    "label": "report-2024-06-01.pdf",
                    "path": "/files/list/0",
                    "control_id": "fm-file-0",
                    "enabled": true,
                    "visible": true,
                    "focused": true,
                    "in_active_window": true,
                    "bounds": { "x": 10, "y": 20, "width": 320, "height": 24 },
                    "score": 0.9,
                    "identity_confidence": 0.9,
                    "bounds_confidence": 0.9,
                    "state_confidence": 0.9
                }),
                serde_json::json!({
                    "role": "list item",
                    "name": "notes.txt",
                    "label": "notes.txt",
                    "path": "/files/list/1",
                    "control_id": "fm-file-1",
                    "enabled": true,
                    "visible": true,
                    "focused": false,
                    "in_active_window": true,
                    "bounds": { "x": 10, "y": 48, "width": 320, "height": 24 },
                    "score": 0.85,
                    "identity_confidence": 0.85,
                    "bounds_confidence": 0.85,
                    "state_confidence": 0.85
                }),
            ],
            _ => Vec::new(),
        };
        GuiProbeResult::ok(serde_json::json!({ "elements": elements }))
    }

    async fn get_cursor_focus_state(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": "Files",
            "focused_app": "Files",
            "focused_control_id": "fm-file-0",
            "focused_control_label": "report-2024-06-01.pdf",
            "focused_control_role": "list item",
            "keyboard_focus_known": true,
            "text_cursor_known": false,
            "editable_target_known": false,
            "terminal_like": false,
            "focus_confidence": 0.9,
        }))
    }

    async fn capture_screenshot(&self) -> GuiProbeResult {
        let seq = self.screen_seq.fetch_add(1, Ordering::SeqCst);
        GuiProbeResult::ok(serde_json::json!({
            "screen_hash": format!("filemanager-screen-{seq}"),
            "byte_count": 16,
            "source": "fixture",
        }))
    }

    async fn focused_window_title(&self) -> Option<String> {
        Some("Files".into())
    }
}

const FILE_MANAGER_PROMPT: &str =
    "Open the file manager and select the newest file and tell me its name";

/// The validated plan's per-step risk levels, from the `PlanValidationCompleted`
/// event's `step_results`.
fn validated_step_risk_levels(
    outcome: &kria_core::agent::gui_cognition::GuiTurnOutcome,
) -> Vec<String> {
    let validation = find_event(outcome, "PlanValidationCompleted")
        .expect("a PlanValidationCompleted event must be emitted");
    validation
        .get("step_results")
        .and_then(serde_json::Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter_map(|s| s.get("risk_level").and_then(serde_json::Value::as_str))
                .map(|v| v.to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn file_manager_select_flag_on_produces_navigate_select_show_name_plan() {
    // Flag ON: the file-manager select prompt produces the complete valid typed
    // sequence OpenApp → Observe → FocusField(select) → SummarizeVisibleContent.
    let perception = FileManagerPerception::new();
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_crossapp(GuiCrossAppConfig::enabled())
        .with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(FILE_MANAGER_PROMPT, GuiExecutionMode::ExecuteFixture))
        .await;

    let step_types = validated_step_types(&outcome);
    assert_eq!(
        step_types,
        vec!["OpenApp", "Observe", "FocusField", "SummarizeVisibleContent"],
        "flag ON must emit navigate → observe → select → show-name: {step_types:?}"
    );
}

#[tokio::test]
async fn file_manager_select_flag_on_reaches_valid_for_resolution() {
    let perception = FileManagerPerception::new();
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_crossapp(GuiCrossAppConfig::enabled())
        .with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(FILE_MANAGER_PROMPT, GuiExecutionMode::ExecuteFixture))
        .await;

    let validation = find_event(&outcome, "PlanValidationCompleted")
        .expect("plan validation event must be emitted");
    assert_eq!(
        validation["readiness_status"], "valid_for_resolution",
        "file-manager select plan must reach valid_for_resolution: {validation}"
    );
}

#[tokio::test]
async fn file_manager_select_is_non_destructive_and_selects_by_observed_order() {
    // The flow is strictly NON-DESTRUCTIVE: every step is low-risk, there is no
    // delete/move/rename step, and the selection is expressed by observed
    // ORDER/POSITION ("newest file"), never an invented/fabricated filename.
    let perception = FileManagerPerception::new();
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_crossapp(GuiCrossAppConfig::enabled())
        .with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(FILE_MANAGER_PROMPT, GuiExecutionMode::ExecuteFixture))
        .await;

    // Non-destructive: no destructive step type, no approval gate, all low-risk.
    let step_types = validated_step_types(&outcome);
    for destructive in ["ClickControl", "RequireApproval", "TypeText", "Paste"] {
        assert!(
            !step_types.contains(&destructive.to_string()),
            "non-destructive select flow must not contain {destructive}: {step_types:?}"
        );
    }
    let risk_levels = validated_step_risk_levels(&outcome);
    assert!(
        risk_levels.iter().all(|r| r == "low"),
        "every step must be low-risk: {risk_levels:?}"
    );

    // Selection by observed order/position — the plan names the "newest file",
    // never a fabricated filename, and never a delete/move/rename verb.
    let events = serde_json::to_string(&outcome.events).unwrap();
    assert!(
        events.contains("Select the newest file"),
        "selection must be expressed by observed order/position"
    );
    let serialized = serde_json::to_string(&outcome.response).unwrap();
    for destructive in ["delete", "Delete", "rename", "Rename", " move ", "trash"] {
        assert!(
            !serialized.contains(destructive),
            "non-destructive flow must never reference {destructive:?}: {serialized}"
        );
    }
}

#[tokio::test]
async fn file_manager_select_flag_off_preserves_prior_plan() {
    // Flag OFF (default): the SAME prompt does NOT produce the file-manager
    // select flow (no Observe→FocusField→SummarizeVisibleContent select sequence
    // driven by the descriptor). The prior single-action path is preserved.
    let perception = FileManagerPerception::new();
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(FILE_MANAGER_PROMPT, GuiExecutionMode::ExecuteFixture))
        .await;

    let step_types = validated_step_types(&outcome);
    assert_ne!(
        step_types,
        vec!["OpenApp", "Observe", "FocusField", "SummarizeVisibleContent"],
        "flag OFF must NOT emit the file-manager select flow: {step_types:?}"
    );
    // And the plan must never contain a fabricated selection step expressed as a
    // file-manager select-by-order summary.
    let events = serde_json::to_string(&outcome.events).unwrap();
    assert!(
        !events.contains("Select the newest file from the observed file list"),
        "flag OFF must not emit the descriptor-driven select step"
    );
}
