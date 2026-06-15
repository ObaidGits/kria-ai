//! T2 — CI-safe deterministic fixture tier (spec task 0.4).
//!
//! This tier exercises the FULL GUI Cognition pipeline in-process —
//! `observe → goal contract → plan → validate → resolve → safety gate →
//! (HITL) → execute → verify` — using deterministic fixtures (a fixture
//! `GuiContext` provider + the `ExecuteFixture` execution mode). It needs **no
//! display and no network**, so it runs in CI on a headless seat where the live
//! T3 audit (which drives a real desktop session) cannot. It is the
//! "deterministic fixture tier for environments without a display" required by
//! Requirement 20.4, and the T2 row of the four-tier methodology (Requirement 17).
//!
//! These tests pin the design's correctness properties at the integration level:
//!   * Property 1  — no action-kind leakage into target labels.
//!   * Property 2  — fresh-context resolution between state-changing steps.
//!   * Property 3  — plan completeness (every step carries a verification strategy).
//!   * Property 4/11 — auto-approval honored only inside the TestSubstrate.
//!   * Property 6  — boundary/observe prompts never execute a state-changing action.
//!   * Property 7  — no raw prompt / secret leakage into events or response.
//!   * Property 8  — `ActionCompleted` (backend success) is distinct from `verified`.
//!   * Property 9  — every turn terminates with a defined status (boundedness).
//!
//! The fixtures are fully self-contained: no `KRIA_*` environment variable, no
//! filesystem, and no socket is touched. The execution environment is passed
//! explicitly in the request, so the tier is deterministic regardless of the
//! host (Requirement 20.4 reproducibility).

use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};

use kria_core::agent::gui_cognition::execution_environment::GuiExecutionEnvironment;
use kria_core::agent::gui_cognition::executor::{
    GuiActionBackendStatus, GuiActionExecution, GuiActionExecutor, GuiActionRequest,
    GuiExecutionMode,
};
use kria_core::agent::gui_cognition::perception::{GuiPerceptionProvider, GuiProbeResult};
use kria_core::agent::gui_cognition::safety_hitl::GuiHitlDecisionFixture;
use kria_core::agent::gui_cognition::{GuiCognitionRuntime, GuiTurnOutcome, GuiTurnRequest};

// ── Deterministic fixture GuiContext provider (no display, no network) ──────
//
// Returns a stable, healthy desktop with a focused "Search" text field and a
// "Search" push button. `capture_screenshot` advances a sequence each call so
// `screen_changed` verification strategies succeed after a state-changing step
// (this is how a fresh observation is evidenced between steps — Property 2).

struct FixtureContextProvider {
    active_window: String,
    controls_present: bool,
    screen_seq: AtomicU64,
}

impl FixtureContextProvider {
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
impl GuiPerceptionProvider for FixtureContextProvider {
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
                "path": "/fixture/text/Search",
                "control_id": "fixture-search-field",
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
                "path": "/fixture/button/Search",
                "control_id": "fixture-search-button",
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
            "focused_control_id": "fixture-search-field",
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
        // A fresh observation each call: the screen hash advances so a
        // state-changing step's `screen_changed` verification can succeed and the
        // NEXT step resolves against a fresh context (Property 2).
        let seq = self.screen_seq.fetch_add(1, Ordering::SeqCst);
        GuiProbeResult::ok(serde_json::json!({
            "screen_hash": format!("fixture-screen-{seq}"),
            "byte_count": 16,
            "source": "fixture",
        }))
    }

    async fn focused_window_title(&self) -> Option<String> {
        Some(self.active_window.clone())
    }
}

// ── Deterministic fixture executor (always succeeds at the backend layer) ───
//
// `ActionCompleted` (backend success) is intentionally distinct from `verified`
// (state confirmed by re-observation) — Property 8.

struct FixtureExecutor {
    backend: GuiActionBackendStatus,
}

impl FixtureExecutor {
    fn new() -> Self {
        Self {
            backend: GuiActionBackendStatus::available("fixture_executor"),
        }
    }
}

#[async_trait]
impl GuiActionExecutor for FixtureExecutor {
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

// ── Request builders ────────────────────────────────────────────────────────

fn fixture_request(message: &str, mode: GuiExecutionMode, workflow_enabled: bool) -> GuiTurnRequest {
    GuiTurnRequest {
        session_id: "t2-session".into(),
        turn_id: "t2-turn".into(),
        workflow_id: "t2-workflow".into(),
        message: message.into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: None,
        execution_environment: GuiExecutionEnvironment::RealSession,
        execution_mode: mode,
        workflow_enabled,
        resume_checkpoint: None,
        resume_reason: None,
    }
}

fn event_types(outcome: &GuiTurnOutcome) -> Vec<String> {
    outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .map(|value| value.to_string())
        .collect()
}

const ACTION_KINDS: &[&str] = &[
    "OpenApp",
    "SwitchWindow",
    "FocusField",
    "FillField",
    "TypeText",
    "ClickControl",
    "PressKey",
    "Hotkey",
    "Scroll",
    "Copy",
    "Paste",
];

// ── T2 tests ─────────────────────────────────────────────────────────────────

/// Full pipeline for a multi-step combo completes in fixture mode with no
/// display and no network. The execution environment is reported as the real
/// session (no substrate env var is read), and the turn terminates.
#[tokio::test]
async fn t2_open_and_focus_combo_runs_full_pipeline_offline() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(fixture_request(
            "Open KRIA Fixture App and focus the visible search field",
            GuiExecutionMode::ExecuteFixture,
            true,
        ))
        .await;

    let types = event_types(&outcome);
    assert!(types.contains(&"TurnStarted".to_string()), "events: {types:?}");
    assert!(types.contains(&"WorkflowRunStarted".to_string()), "events: {types:?}");
    assert!(types.contains(&"WorkflowStepStarted".to_string()), "events: {types:?}");
    // The turn reaches a terminal workflow status (Property 9 — boundedness).
    assert!(
        types.iter().any(|t| t == "WorkflowRunCompleted"
            || t == "WorkflowRunBlocked"
            || t == "WorkflowRunPaused"),
        "expected a terminal workflow event, events: {types:?}"
    );
    assert!(!outcome.status.is_empty(), "turn must end with a defined status");

    // Deterministic environment: real session, auto-approval forbidden.
    let env = &outcome.response["gui_cognition"]["execution_environment"];
    assert_eq!(env["environment"], "real_session");
    assert_eq!(env["allows_auto_approval"], false);
}

/// Single-proposal path: a focus action executes and is verified. Backend
/// success (`ActionCompleted`) is distinct from the verified verdict (Property 8).
#[tokio::test]
async fn t2_focus_field_executes_and_verifies() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(fixture_request(
            "Focus the visible search field and verify it is focused",
            GuiExecutionMode::ExecuteFixture,
            false,
        ))
        .await;

    let types = event_types(&outcome);
    assert!(types.contains(&"ActionStarted".to_string()), "events: {types:?}");
    assert!(types.contains(&"ActionCompleted".to_string()), "events: {types:?}");
    assert!(
        types.contains(&"ExecutionVerificationCompleted".to_string()),
        "events: {types:?}"
    );

    let verification = outcome
        .events
        .iter()
        .find(|e| e.get("type").and_then(serde_json::Value::as_str)
            == Some("ExecutionVerificationCompleted"))
        .expect("verification event present");
    assert_eq!(verification["status"], "verified");
}

/// Property 3 — plan completeness: a well-formed type prompt is NOT blocked for
/// "missing payload/verification"; every emitted step carries a verification
/// strategy in the plan summary.
#[tokio::test]
async fn t2_plan_steps_carry_verification_strategy() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(fixture_request(
            "Type \"KRIA fixture tier\" into the visible text field and verify the text is entered",
            GuiExecutionMode::ExecuteFixture,
            true,
        ))
        .await;

    // No PlanBlocked for a well-formed, payload-bearing prompt.
    let types = event_types(&outcome);
    assert!(
        !types.contains(&"PlanBlocked".to_string()),
        "well-formed type prompt must not be plan-blocked, events: {types:?}"
    );

    // Every typed plan step exposes a non-empty verification_strategy.
    let plan = &outcome.response["gui_cognition"]["plan"];
    let steps = plan["typed_steps"]
        .as_array()
        .or_else(|| plan["steps"].as_array());
    if let Some(steps) = steps {
        assert!(!steps.is_empty(), "plan should contain at least one step");
        for step in steps {
            let strategy = step
                .get("verification_strategy")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let step_type = step
                .get("step_type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            // AskClarification is the only step type allowed without verification.
            if step_type != "AskClarification" {
                assert!(
                    !strategy.is_empty(),
                    "step {step_type} missing verification_strategy: {step}"
                );
            }
        }
    }
}

/// Property 1 — no action-kind leakage: a resolved target label/identity is
/// never one of the raw action-kind names.
#[tokio::test]
async fn t2_no_action_kind_leaks_into_target_label() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(fixture_request(
            "Click the visible safe button named Search and verify the screen changed",
            GuiExecutionMode::ExecuteFixture,
            false,
        ))
        .await;

    let tr = &outcome.response["gui_cognition"]["target_resolution"];
    for key in ["label", "target_label", "matched_label"] {
        if let Some(label) = tr.get(key).and_then(serde_json::Value::as_str) {
            assert!(
                !ACTION_KINDS.contains(&label),
                "target {key} leaked an action kind: {label}"
            );
        }
    }
}

/// Property 4/11 — the test substrate is the ONLY place an auto-approval HITL
/// fixture is honored. On the real session it must be rejected and nothing
/// executes.
#[tokio::test]
async fn t2_real_session_rejects_auto_approval_fixture() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let mut request = fixture_request(
        "Prepare to click a Submit button, but ask for approval before executing.",
        GuiExecutionMode::SafetyOnly,
        false,
    );
    request.hitl_decision_fixture = Some(GuiHitlDecisionFixture::Approve);
    request.execution_environment = GuiExecutionEnvironment::RealSession;

    let outcome = runtime.run_turn(request).await;

    let types = event_types(&outcome);
    assert!(
        types.contains(&"HitlFixtureRejected".to_string()),
        "auto-approval must be rejected on real session, events: {types:?}"
    );
    assert!(!types.contains(&"ActionStarted".to_string()), "events: {types:?}");
    assert_eq!(outcome.response["status"], "needs_approval");
    assert_eq!(
        outcome.response["gui_cognition"]["execution_environment"]["allows_auto_approval"],
        false
    );
}

/// Property 11 — inside an isolated substrate the auto-approval IS honored and
/// the gate advances past `needs_approval`.
#[tokio::test]
async fn t2_test_substrate_honors_auto_approval_fixture() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let mut request = fixture_request(
        "Prepare to click a Submit button, but ask for approval before executing.",
        GuiExecutionMode::SafetyOnly,
        false,
    );
    request.hitl_decision_fixture = Some(GuiHitlDecisionFixture::Approve);
    request.execution_environment = GuiExecutionEnvironment::TestSubstrate {
        scratch_dir: None,
        restore_clipboard: true,
    };

    let outcome = runtime.run_turn(request).await;

    let types = event_types(&outcome);
    assert!(
        !types.contains(&"HitlFixtureRejected".to_string()),
        "fixture must NOT be rejected in substrate, events: {types:?}"
    );
    assert_eq!(
        outcome.response["gui_cognition"]["execution_environment"]["environment"],
        "test_substrate"
    );
    assert_eq!(outcome.response["status"], "approved_for_step7");
}

/// Property 6 — an observe/boundary-style prompt ("just look, do not change")
/// never starts a state-changing action.
#[tokio::test]
async fn t2_observe_boundary_prompt_runs_no_state_changing_action() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(fixture_request(
            "Observe my current screen and tell me the active window and visible controls. \
             Do not change, type, click, or delete anything.",
            GuiExecutionMode::ExecuteFixture,
            false,
        ))
        .await;

    let types = event_types(&outcome);
    assert!(
        !types.contains(&"ActionStarted".to_string()),
        "boundary/observe prompt must not start an action, events: {types:?}"
    );
    assert!(!outcome.status.is_empty());
}

/// Property 7 — no raw prompt or secret leaks into events or the response.
#[tokio::test]
async fn t2_no_raw_prompt_or_secret_leak() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(fixture_request(
            "ZZZT2RAWMARKER open KRIA Fixture App and focus the visible search field",
            GuiExecutionMode::ExecuteFixture,
            true,
        ))
        .await;

    let response = serde_json::to_string(&outcome.response).unwrap();
    assert!(!response.contains("ZZZT2RAWMARKER"), "raw prompt leaked in response");
    assert!(!response.contains("\"raw_prompt\""), "raw_prompt field exposed");
    let events = serde_json::to_string(&outcome.events).unwrap();
    assert!(!events.contains("ZZZT2RAWMARKER"), "raw prompt leaked in events");
}

/// Property 9 — a turn whose target is genuinely absent still terminates safely
/// (bounded), blocking before any action rather than looping.
#[tokio::test]
async fn t2_absent_target_blocks_safely_and_terminates() {
    let perception = FixtureContextProvider::without_controls("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(fixture_request(
            "Click the Search button and verify the screen changed",
            GuiExecutionMode::ExecuteFixture,
            true,
        ))
        .await;

    let types = event_types(&outcome);
    assert!(
        types.iter().any(|t| t == "WorkflowStepBlocked"
            || t == "WorkflowRunBlocked"
            || t == "ExecutionBlocked"
            || t == "PlanBlocked"),
        "absent target should block safely, events: {types:?}"
    );
    assert!(!types.contains(&"ActionStarted".to_string()), "events: {types:?}");
    assert!(!outcome.status.is_empty(), "turn must terminate with a status");
}

/// SafetyOnly mode never executes — the deterministic fixture tier can run the
/// whole pipeline up to (but not through) the executor for plan/validation
/// coverage without any side effect.
#[tokio::test]
async fn t2_safety_only_mode_never_executes() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(fixture_request(
            "Focus the visible search field and verify it is focused",
            GuiExecutionMode::SafetyOnly,
            false,
        ))
        .await;

    let types = event_types(&outcome);
    assert!(!types.contains(&"ActionStarted".to_string()), "events: {types:?}");
    assert!(!types.contains(&"ActionCompleted".to_string()), "events: {types:?}");
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2.8 — T2: the PLAN → VALIDATE pipeline reaches `valid_for_resolution`
// for the FULL supported primitive/combo matrix (Requirements 1, 4).
//
// These tests drive the *broader runtime pipeline* — observe → goal contract →
// (deterministic) plan → `validate_plan_for_resolution` — via `run_turn` against
// the deterministic fixture tier (no display, no network), and assert on the
// emitted `PlanValidationCompleted` readiness event. This is the integration
// counterpart to the T1 planner units (`gui_cognition_llm_planner_tests.rs`):
// T1 exercises the planner-selection + validator helpers directly on synthetic
// contracts; T2 here proves the SAME readiness contract holds when the contract
// is extracted from a real prompt and threaded through the runtime.
//
// The point of Task 2.8: NO supported primitive/combo may land on `blocked`
// or `rejected` when the goal contract is complete. Where payload/target is
// genuinely missing the terminal status is `needs_clarification`; risky actions
// reach `approval_required`. KRIA authority invariants are asserted throughout:
// the plan NEVER auto-executes at the plan/validate stage (`can_execute == false`)
// and no action starts in this non-executing mode (no Prompt→Tool shortcut).
// ─────────────────────────────────────────────────────────────────────────────

/// The `PlanValidationCompleted` readiness event emitted by the runtime for a
/// given prompt. Driven in `SafetyOnly` mode so the pipeline runs through
/// plan + validate WITHOUT executing anything (deterministic, side-effect free).
async fn readiness_for_prompt(prompt: &str) -> GuiTurnOutcome {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor);
    runtime
        .run_turn(fixture_request(prompt, GuiExecutionMode::SafetyOnly, false))
        .await
}

fn plan_validation_event(outcome: &GuiTurnOutcome) -> serde_json::Value {
    outcome
        .events
        .iter()
        .find(|event| {
            event.get("type").and_then(serde_json::Value::as_str)
                == Some("PlanValidationCompleted")
        })
        .cloned()
        .expect("runtime must emit a PlanValidationCompleted readiness event")
}

/// The complete supported primitive/combo matrix, expressed as natural-language
/// prompts the way the UI endpoint would receive them. Every entry must thread
/// the whole pipeline to a *non-blocking* terminal readiness status.
const PRIMITIVE_COMBO_MATRIX: &[&str] = &[
    // ── Combos (multi-step) ──
    "Open browser, search KRIA, and summarize page",
    "Open the browser and navigate to \"https://github.com\"",
    "Search for \"quarterly report\" in the file manager",
    // ── Single primitives ──
    "Open Google Chrome",
    "Switch to the Browser window",
    "Type \"KRIA fixture tier\" into the visible Search field and verify the text is entered",
    "Clear the visible Search field",
    "Select all the text in the visible Search field",
    "Copy the selected text",
    "Paste the clipboard contents into the visible Search field",
    "Press the Enter key",
    "Scroll down the page",
    "Check the \"Remember me\" checkbox",
    "Close the active dialog",
    "Open the visible search field and focus it, then verify it is focused",
    "Click the visible safe button named Search and verify the screen changed",
    "Verify the visible Search field is focused and then stop",
];

const NON_BLOCKING_TERMINAL_STATUSES: &[&str] =
    &["valid_for_resolution", "needs_clarification", "approval_required"];

/// Core Task 2.8 assertion: NO supported primitive/combo lands on `blocked` or
/// `rejected`. The deterministic planner always emits complete, verified,
/// non-executable steps (or an AskClarification step) — so every matrix entry
/// terminates on a non-blocking readiness status, and the plan never claims it
/// can execute at the validate stage (KRIA authority invariant).
#[tokio::test]
async fn t2_full_primitive_combo_matrix_never_blocks_or_rejects() {
    for prompt in PRIMITIVE_COMBO_MATRIX {
        let outcome = readiness_for_prompt(prompt).await;
        let event = plan_validation_event(&outcome);
        let status = event["readiness_status"].as_str().unwrap_or("");
        assert!(
            NON_BLOCKING_TERMINAL_STATUSES.contains(&status),
            "prompt {prompt:?} landed on terminal status {status:?} \
             (must be one of {NON_BLOCKING_TERMINAL_STATUSES:?}); event: {event}"
        );

        // KRIA authority: a plan NEVER auto-executes at the plan/validate stage.
        assert_eq!(
            event["can_execute"], false,
            "prompt {prompt:?} must not be executable at the validate stage"
        );

        // No Prompt→Tool shortcut: nothing executes in the non-executing tier.
        let types = event_types(&outcome);
        assert!(
            !types.contains(&"ActionStarted".to_string()),
            "prompt {prompt:?} must not start an action at the validate stage, events: {types:?}"
        );

        // Boundedness (Property 9): the turn always terminates with a status.
        assert!(
            !outcome.status.is_empty(),
            "prompt {prompt:?} must terminate with a defined status"
        );
    }
}

/// Complete, well-formed primitives/combos reach `valid_for_resolution`
/// specifically (not merely non-blocking): browser search + summarize, browser
/// navigate, in-app search, open-app, type-text, focus, click. Each is allowed
/// to proceed to target resolution but is still non-executable at this stage.
#[tokio::test]
async fn t2_complete_primitives_and_combos_reach_valid_for_resolution() {
    let complete = [
        "Open browser, search KRIA, and summarize page",
        "Open the browser and navigate to \"https://github.com\"",
        "Search for \"quarterly report\" in the file manager",
        "Open Google Chrome",
        "Type \"KRIA fixture tier\" into the visible Search field and verify the text is entered",
        "Open the visible search field and focus it, then verify it is focused",
        "Click the visible safe button named Search and verify the screen changed",
    ];

    for prompt in complete {
        let outcome = readiness_for_prompt(prompt).await;
        let event = plan_validation_event(&outcome);
        assert_eq!(
            event["readiness_status"], "valid_for_resolution",
            "prompt {prompt:?} should reach valid_for_resolution; event: {event}"
        );
        assert_eq!(
            event["can_proceed_to_target_resolution"], true,
            "prompt {prompt:?} should be allowed to resolve targets; event: {event}"
        );
        assert_eq!(
            event["requires_user_approval"], false,
            "prompt {prompt:?} is non-risky and must not require approval; event: {event}"
        );
        // Plan-stage non-execution invariant holds even when resolvable.
        assert_eq!(event["can_execute"], false, "prompt {prompt:?}");
    }
}

/// When the payload/target is genuinely missing, the terminal readiness status
/// is `needs_clarification` (never `blocked`) — the pipeline asks rather than
/// guesses (Requirement 4.1). The plan still never executes.
#[tokio::test]
async fn t2_missing_payload_reaches_needs_clarification_not_blocked() {
    let outcome = readiness_for_prompt(
        "A form is open. Fill the visible form fields, validate the values, and do not press Submit or Send.",
    )
    .await;
    let event = plan_validation_event(&outcome);
    assert_eq!(
        event["readiness_status"], "needs_clarification",
        "missing form values should ask for clarification; event: {event}"
    );
    assert_eq!(event["can_execute"], false);
    assert_eq!(event["requires_user_approval"], false);
    let types = event_types(&outcome);
    assert!(
        !types.contains(&"ActionStarted".to_string()),
        "clarification path must not execute, events: {types:?}"
    );
}

/// A risky action that the user asked to gate reaches `approval_required` —
/// it pauses before resolution, requires approval, and never executes at the
/// validate stage (Requirements 1, 10).
#[tokio::test]
async fn t2_risky_gated_action_reaches_approval_required() {
    let outcome = readiness_for_prompt(
        "Prepare to click the Submit button, but ask for my approval before executing.",
    )
    .await;
    let event = plan_validation_event(&outcome);
    assert_eq!(
        event["readiness_status"], "approval_required",
        "a risk-gated action must require approval; event: {event}"
    );
    assert_eq!(event["requires_user_approval"], true);
    assert_eq!(
        event["can_proceed_to_target_resolution"], false,
        "approval-gated plans must not proceed to resolution before approval"
    );
    assert_eq!(event["can_execute"], false);
}

/// KRIA authority (deterministic orchestration + verification): across the whole
/// matrix, every emitted plan step is non-executable at the plan stage and every
/// non-clarification step carries a verification strategy — no Prompt→Tool
/// shortcut and no unverifiable step can reach the executor.
#[tokio::test]
async fn t2_matrix_plans_are_non_executable_and_carry_verification() {
    for prompt in PRIMITIVE_COMBO_MATRIX {
        let outcome = readiness_for_prompt(prompt).await;
        let plan = &outcome.response["gui_cognition"]["plan"];
        let steps = plan["typed_steps"]
            .as_array()
            .or_else(|| plan["steps"].as_array());
        let steps = steps.unwrap_or_else(|| panic!("prompt {prompt:?} produced no plan steps"));
        assert!(!steps.is_empty(), "prompt {prompt:?} produced an empty plan");
        for step in steps {
            assert_eq!(
                step.get("allowed_to_execute"),
                Some(&serde_json::Value::Bool(false)),
                "prompt {prompt:?} step must not be pre-authorized to execute: {step}"
            );
            let step_type = step
                .get("step_type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if step_type != "AskClarification" {
                let strategy = step
                    .get("verification_strategy")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                assert!(
                    !strategy.trim().is_empty(),
                    "prompt {prompt:?} step {step_type} missing verification_strategy: {step}"
                );
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 3.5 — T2: re-observe-between-steps captures a FRESH context per step
// (Requirement 2.1/2.2, Requirement 6.1, Property 2).
//
// The full in-process pipeline runs a multi-step combo with the
// `gui_cog_reobserve` flag ON. We prove the per-step re-observe acts on the
// CURRENT screen — not the stale initial observation — by asserting a fresh
// observation is captured between steps (distinct ObservationCompleted ids and
// an advanced workflow_run context), while the combo still completes with each
// step verified. KRIA authority invariant: no Prompt→Tool shortcut — every step
// flows observe→resolve→execute→verify (the run only completes after a real
// verification), and the loop stays bounded.
// ─────────────────────────────────────────────────────────────────────────────

fn observation_completed_ids(outcome: &GuiTurnOutcome) -> Vec<String> {
    outcome
        .events
        .iter()
        .filter(|event| {
            event.get("type").and_then(serde_json::Value::as_str) == Some("ObservationCompleted")
        })
        .filter_map(|event| event.get("observation_id").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn t2_reobserve_between_steps_resolves_each_step_against_fresh_context() {
    use kria_core::agent::gui_cognition::turn_budget::GuiReobserveConfig;

    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime =
        GuiCognitionRuntime::new(&perception, &executor).with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(fixture_request(
            "Type \"KRIA fixture tier\" into the visible Search field and verify the text is entered",
            GuiExecutionMode::ExecuteFixture,
            true,
        ))
        .await;

    let types = event_types(&outcome);

    // A fresh observation is captured between steps: more than one observation,
    // every observation_id distinct (never the stale initial one — Property 2).
    let obs_ids = observation_completed_ids(&outcome);
    assert!(
        obs_ids.len() > 1,
        "the combo must re-observe between steps (>1 ObservationCompleted): {types:?}"
    );
    let mut unique = obs_ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        obs_ids.len(),
        "each step must resolve against a FRESH observation (distinct ids): {obs_ids:?}"
    );

    // The run advanced onto the re-observed context rather than the initial one.
    let run = outcome
        .response
        .pointer("/gui_cognition/workflow_run")
        .expect("workflow_run present");
    let initial = run["initial_context_id"].as_str().unwrap_or("");
    let current = run["current_context_id"].as_str().unwrap_or("");
    assert!(!initial.is_empty() && !current.is_empty(), "context ids present: {run}");
    assert_ne!(
        initial, current,
        "current_context_id must advance from initial after re-observe: {run}"
    );

    // KRIA authority: each step is genuinely executed AND verified (no
    // Prompt→Tool shortcut) and the bounded combo reaches a terminal status.
    assert!(types.contains(&"ActionStarted".to_string()), "events: {types:?}");
    assert!(
        types.contains(&"ExecutionVerificationCompleted".to_string()),
        "each executed step must be verified: {types:?}"
    );
    assert!(
        types.iter().any(|t| t == "WorkflowRunCompleted"
            || t == "WorkflowRunBlocked"
            || t == "WorkflowRunPaused"),
        "the turn must terminate (bounded): {types:?}"
    );
    assert!(!outcome.status.is_empty(), "turn must end with a defined status");
}
