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
    GuiWorkflowRun, GuiWorkflowStepKind, GuiWorkflowStepState,
};
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
