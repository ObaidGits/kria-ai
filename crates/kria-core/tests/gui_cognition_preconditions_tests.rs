//! Task 1.4 — T2 integration tests for the preconditions health-gate in the
//! GUI Cognition runtime (Requirement 25, design Property "preconditions
//! health-gate (R25)").
//!
//! These exercise the full in-process pipeline (deterministic fixtures, no
//! display) through `run_turn` and assert:
//!   - ready preconditions + ExecuteLive + guards ON → live actions execute
//!   - a missing action backend (uinput) → degraded observe/plan-only with a
//!     clear reason and a `PreconditionsDegraded` event; no action executes
//!   - a missing AT-SPI bus → degraded observe/plan-only; no action executes
//!   - with the `gui_cog_runtime_guards` flag OFF, existing behavior is preserved
//!     (no degrade even when a precondition is missing)
//!   - the sanitized readiness summary is always present in the response.

use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use kria_core::agent::gui_cognition::executor::{
    GuiActionBackendStatus, GuiActionExecution, GuiActionExecutor, GuiActionRequest,
    GuiExecutionMode,
};
use kria_core::agent::gui_cognition::perception::{GuiPerceptionProvider, GuiProbeResult};
use kria_core::agent::gui_cognition::turn_budget::{GuiRuntimeGuardConfig, TurnBudget};
use kria_core::agent::gui_cognition::{GuiCognitionRuntime, GuiTurnOutcome, GuiTurnRequest};

// ── Fixtures ────────────────────────────────────────────────────────────────

/// Perception fixture whose AT-SPI / accessibility availability is configurable
/// so the precondition gate can be driven deterministically.
struct PreconditionPerception {
    active_window: String,
    accessibility: bool,
    screen_seq: AtomicU64,
}

impl PreconditionPerception {
    fn new(active_window: &str, accessibility: bool) -> Self {
        Self {
            active_window: active_window.into(),
            accessibility,
            screen_seq: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl GuiPerceptionProvider for PreconditionPerception {
    async fn get_active_window(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "title": self.active_window,
            "app_name": self.active_window,
        }))
    }

    async fn get_desktop_state(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": self.active_window,
            "accessibility_operational": self.accessibility,
            "applications": [self.active_window, "Browser"],
        }))
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": self.accessibility }))
    }

    async fn find_ui_elements(&self, role: &str) -> GuiProbeResult {
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
        let seq = self.screen_seq.fetch_add(1, Ordering::SeqCst);
        GuiProbeResult::ok(serde_json::json!({
            "screen_hash": format!("precond-screen-{seq}"),
            "byte_count": 16,
            "source": "fixture",
        }))
    }

    async fn focused_window_title(&self) -> Option<String> {
        Some(self.active_window.clone())
    }
}

/// Executor whose reported backend status is configurable (healthy vs blocked)
/// and which counts how many live actions actually execute.
struct PreconditionExecutor {
    backend: GuiActionBackendStatus,
    executions: Arc<AtomicU64>,
}

impl PreconditionExecutor {
    fn healthy() -> Self {
        Self {
            backend: GuiActionBackendStatus::available("uinput_accessibility"),
            executions: Arc::new(AtomicU64::new(0)),
        }
    }

    /// A backend that cannot execute actions (e.g. no usable uinput socket).
    fn no_action_backend() -> Self {
        Self {
            backend: GuiActionBackendStatus::blocked(
                "unavailable",
                "Wayland session has no usable uinput socket or validated ydotool backend.",
                "wayland",
            ),
            executions: Arc::new(AtomicU64::new(0)),
        }
    }

    /// A backend that can type/click but cannot focus windows/controls — the
    /// `focus_backend` precondition is missing (Requirement 25.1).
    fn focus_unsupported() -> Self {
        let mut backend = GuiActionBackendStatus::available("uinput_no_focus");
        backend.focus_supported = false;
        Self {
            backend,
            executions: Arc::new(AtomicU64::new(0)),
        }
    }

    /// A healthy backend whose session/DISPLAY type is unknown — the `display`
    /// precondition is missing (Requirement 25.1).
    fn unknown_display() -> Self {
        let mut backend = GuiActionBackendStatus::available("uinput_accessibility");
        backend.session_type = "unknown".into();
        Self {
            backend,
            executions: Arc::new(AtomicU64::new(0)),
        }
    }

    fn executions_handle(&self) -> Arc<AtomicU64> {
        self.executions.clone()
    }
}

#[async_trait]
impl GuiActionExecutor for PreconditionExecutor {
    async fn action_backend_status(&self) -> GuiActionBackendStatus {
        self.backend.clone()
    }

    async fn execute(&self, request: GuiActionRequest) -> GuiActionExecution {
        self.executions.fetch_add(1, Ordering::SeqCst);
        GuiActionExecution::ok(
            "fixture_executor",
            serde_json::json!({ "executed": request.kind.as_str() }),
        )
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

const MULTI_STEP_PROMPT: &str = "Open KRIA Workflow App and focus the visible search field";

fn live_request() -> GuiTurnRequest {
    GuiTurnRequest {
        session_id: "precond-session".into(),
        turn_id: "precond-turn".into(),
        workflow_id: "precond-workflow".into(),
        message: MULTI_STEP_PROMPT.into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: None,
        execution_environment: Default::default(),
        execution_mode: GuiExecutionMode::ExecuteLive,
        workflow_enabled: true,
        resume_checkpoint: None,
        resume_reason: None,
    }
}

fn guards_on() -> GuiRuntimeGuardConfig {
    GuiRuntimeGuardConfig::enabled(TurnBudget::default())
}

fn event_types(outcome: &GuiTurnOutcome) -> Vec<String> {
    outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
}

fn preconditions(outcome: &GuiTurnOutcome) -> &serde_json::Value {
    &outcome.response["gui_cognition"]["preconditions"]
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial_test::serial]
async fn ready_preconditions_execute_live_actions() {
    kria_core::safety::release_halt("test reset");
    let perception = PreconditionPerception::new("KRIA Workflow App", true);
    let executor = PreconditionExecutor::healthy();
    let executions = executor.executions_handle();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(live_request()).await;
    let types = event_types(&outcome);

    // Readiness was reported and all preconditions are satisfied.
    assert!(types.contains(&"PreconditionsChecked".to_string()), "events: {types:?}");
    assert!(
        !types.contains(&"PreconditionsDegraded".to_string()),
        "must not degrade when ready: {types:?}"
    );
    assert_eq!(preconditions(&outcome)["ready"], true);

    // Live actions actually executed (not downgraded to observe/plan-only).
    assert!(
        executions.load(Ordering::SeqCst) >= 1,
        "ready preconditions must allow live execution"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn missing_uinput_degrades_to_observe_plan_only() {
    kria_core::safety::release_halt("test reset");
    // No usable action backend → the `uinput` precondition is missing.
    let perception = PreconditionPerception::new("KRIA Workflow App", true);
    let executor = PreconditionExecutor::no_action_backend();
    let executions = executor.executions_handle();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(live_request()).await;
    let types = event_types(&outcome);

    let degraded = outcome
        .events
        .iter()
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some("PreconditionsDegraded"))
        .expect("PreconditionsDegraded event present");
    assert_eq!(degraded["degraded_mode"], "observe_plan_only");
    let missing = degraded["missing"].as_array().expect("missing list");
    assert!(
        missing.iter().any(|m| m == "uinput"),
        "uinput must be listed missing: {missing:?}"
    );

    // Degraded → no live action executes, and the turn is NOT a misleading "completed".
    assert_eq!(executions.load(Ordering::SeqCst), 0, "no live action may execute when degraded");
    assert_ne!(outcome.status, "completed", "must not report completed when degraded");
    assert_eq!(preconditions(&outcome)["ready"], false);
    assert!(
        preconditions(&outcome)["degraded_reason"].is_string(),
        "a clear degraded reason must be surfaced"
    );
    // Sanity: the readiness was reported before degrading.
    assert!(types.contains(&"PreconditionsChecked".to_string()), "events: {types:?}");
}

#[tokio::test]
#[serial_test::serial]
async fn missing_atspi_degrades_to_observe_plan_only() {
    kria_core::safety::release_halt("test reset");
    // Backend is healthy but the AT-SPI accessibility bus is unavailable.
    let perception = PreconditionPerception::new("KRIA Workflow App", false);
    let executor = PreconditionExecutor::healthy();
    let executions = executor.executions_handle();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(live_request()).await;

    let degraded = outcome
        .events
        .iter()
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some("PreconditionsDegraded"))
        .expect("PreconditionsDegraded event present");
    let missing = degraded["missing"].as_array().expect("missing list");
    assert!(
        missing.iter().any(|m| m == "atspi"),
        "atspi must be listed missing: {missing:?}"
    );

    assert_eq!(executions.load(Ordering::SeqCst), 0, "no live action may execute when degraded");
    assert_ne!(outcome.status, "completed");
    assert_eq!(preconditions(&outcome)["atspi"], false);
}

#[tokio::test]
#[serial_test::serial]
async fn missing_focus_backend_degrades_to_observe_plan_only() {
    kria_core::safety::release_halt("test reset");
    // Backend can type/click but cannot focus windows/controls → the
    // `focus_backend` precondition is missing (Requirement 25.1).
    let perception = PreconditionPerception::new("KRIA Workflow App", true);
    let executor = PreconditionExecutor::focus_unsupported();
    let executions = executor.executions_handle();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(live_request()).await;

    let degraded = outcome
        .events
        .iter()
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some("PreconditionsDegraded"))
        .expect("PreconditionsDegraded event present");
    assert_eq!(degraded["degraded_mode"], "observe_plan_only");
    let missing = degraded["missing"].as_array().expect("missing list");
    assert!(
        missing.iter().any(|m| m == "focus_backend"),
        "focus_backend must be listed missing: {missing:?}"
    );

    assert_eq!(executions.load(Ordering::SeqCst), 0, "no live action may execute when degraded");
    assert_ne!(outcome.status, "completed", "must not report completed when degraded");
    assert_eq!(preconditions(&outcome)["focus_backend"], false);
    // uinput stays ready (the backend can still execute non-focus actions).
    assert_eq!(preconditions(&outcome)["uinput"], true);
    assert!(
        preconditions(&outcome)["degraded_reason"].is_string(),
        "a clear degraded reason must be surfaced"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn unknown_display_degrades_to_observe_plan_only() {
    kria_core::safety::release_halt("test reset");
    // Backend is healthy but the session/DISPLAY type is unknown → the `display`
    // precondition is missing (Requirement 25.1).
    let perception = PreconditionPerception::new("KRIA Workflow App", true);
    let executor = PreconditionExecutor::unknown_display();
    let executions = executor.executions_handle();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(live_request()).await;

    let degraded = outcome
        .events
        .iter()
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some("PreconditionsDegraded"))
        .expect("PreconditionsDegraded event present");
    let missing = degraded["missing"].as_array().expect("missing list");
    assert!(
        missing.iter().any(|m| m == "display"),
        "display must be listed missing: {missing:?}"
    );

    assert_eq!(executions.load(Ordering::SeqCst), 0, "no live action may execute when degraded");
    assert_ne!(outcome.status, "completed");
    assert_eq!(preconditions(&outcome)["display"], false);
}

#[tokio::test]
#[serial_test::serial]
async fn flag_off_preserves_behavior_even_with_missing_precondition() {
    kria_core::safety::release_halt("test reset");
    // AT-SPI missing, but the flag is OFF (default). The gate must NOT fire:
    // no degrade event, and the turn proceeds exactly as before (live actions
    // run through the healthy backend).
    let perception = PreconditionPerception::new("KRIA Workflow App", false);
    let executor = PreconditionExecutor::healthy();
    let executions = executor.executions_handle();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(GuiRuntimeGuardConfig::default()) // OFF
        .with_cancel_token(None);

    let outcome = runtime.run_turn(live_request()).await;
    let types = event_types(&outcome);

    assert!(
        !types.contains(&"PreconditionsDegraded".to_string()),
        "flag OFF must not degrade: {types:?}"
    );
    assert!(
        !types.contains(&"PreconditionsChecked".to_string()),
        "flag OFF emits no preconditions events: {types:?}"
    );
    assert!(
        executions.load(Ordering::SeqCst) >= 1,
        "flag OFF preserves existing live-execution behavior"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn readiness_summary_present_in_response() {
    kria_core::safety::release_halt("test reset");
    // The sanitized readiness summary is surfaced in response.gui_cognition.*
    // regardless of the flag state (additive, non-breaking).
    let perception = PreconditionPerception::new("KRIA Workflow App", true);
    let executor = PreconditionExecutor::healthy();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(GuiRuntimeGuardConfig::default()) // OFF
        .with_cancel_token(None);

    let outcome = runtime.run_turn(live_request()).await;
    let summary = preconditions(&outcome);

    for key in ["uinput", "atspi", "focus_backend", "display", "ready"] {
        assert!(summary.get(key).is_some(), "summary missing `{key}`: {summary}");
    }
    assert!(summary["details"].is_object(), "summary carries per-precondition details");
}
