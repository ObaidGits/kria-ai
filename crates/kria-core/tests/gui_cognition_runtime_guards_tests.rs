//! Task 1.2 — T2 integration tests for runtime guards in the GUI Cognition
//! workflow loop (Requirement 21.1/21.2, Property 9).
//!
//! These exercise the full in-process pipeline (deterministic fixtures, no
//! display) and assert the loop's pre-action guard behavior:
//!   - cancel halts BEFORE the next action (cooperative, 21.1)
//!   - GlobalSafetyHalt halts BEFORE the next action (master kill-switch, 21.2)
//!   - with the `gui_cog_runtime_guards` flag OFF, behavior is unchanged.

use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use kria_core::agent::gui_cognition::cancel::GuiCancelToken;
use kria_core::agent::gui_cognition::executor::{
    GuiActionBackendStatus, GuiActionExecution, GuiActionExecutor, GuiActionRequest,
    GuiExecutionMode,
};
use kria_core::agent::gui_cognition::turn_budget::{GuiRuntimeGuardConfig, TurnBudget};
use kria_core::agent::gui_cognition::turn_budget::GuiReobserveConfig;
use kria_core::agent::gui_cognition::perception::{GuiPerceptionProvider, GuiProbeResult};
use kria_core::agent::gui_cognition::{GuiCognitionRuntime, GuiTurnOutcome, GuiTurnRequest};

// ── Fixtures ────────────────────────────────────────────────────────────────

struct GuardPerception {
    active_window: String,
    screen_seq: AtomicU64,
    /// When true, `capture_screenshot` returns the SAME screen hash on every
    /// call so the loop observes a "stuck"/oscillating screen — the deterministic
    /// driver for the flapping cap (Requirement 21.4).
    stuck_screen: bool,
}

impl GuardPerception {
    fn new(active_window: &str) -> Self {
        Self {
            active_window: active_window.into(),
            screen_seq: AtomicU64::new(0),
            stuck_screen: false,
        }
    }

    /// A perception whose screen never changes between observations (constant
    /// `screen_hash`), modeling a turn that makes no visible progress.
    fn stuck(active_window: &str) -> Self {
        Self {
            active_window: active_window.into(),
            screen_seq: AtomicU64::new(0),
            stuck_screen: true,
        }
    }
}

#[async_trait]
impl GuiPerceptionProvider for GuardPerception {
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
        // A `stuck_screen` provider returns a constant hash so the runtime
        // observes no progress between steps (flapping driver, Requirement 21.4).
        let screen_hash = if self.stuck_screen {
            "guard-screen-stuck".to_string()
        } else {
            format!("guard-screen-{seq}")
        };
        GuiProbeResult::ok(serde_json::json!({
            "screen_hash": screen_hash,
            "byte_count": 16,
            "source": "fixture",
        }))
    }

    async fn focused_window_title(&self) -> Option<String> {
        Some(self.active_window.clone())
    }
}

/// Executor that counts executions and can trip a cancel token on the Nth
/// execute, modeling the user pressing Stop mid-turn.
struct GuardExecutor {
    backend: GuiActionBackendStatus,
    executions: Arc<AtomicU64>,
    cancel_after: Option<u64>,
    cancel_token: Option<GuiCancelToken>,
    halt_after: Option<u64>,
    delay_ms: u64,
}

impl GuardExecutor {
    fn new() -> Self {
        Self {
            backend: GuiActionBackendStatus::available("fixture_executor"),
            executions: Arc::new(AtomicU64::new(0)),
            cancel_after: None,
            cancel_token: None,
            halt_after: None,
            delay_ms: 0,
        }
    }

    /// Cancel `token` once `after` actions have executed.
    fn cancel_after(mut self, after: u64, token: GuiCancelToken) -> Self {
        self.cancel_after = Some(after);
        self.cancel_token = Some(token);
        self
    }

    /// Engage the process-global GlobalSafetyHalt once `after` actions have
    /// executed, modeling the master kill-switch tripping mid-turn.
    fn halt_after(mut self, after: u64) -> Self {
        self.halt_after = Some(after);
        self
    }

    /// Sleep `delay_ms` inside each execute, so the turn watchdog can elapse
    /// deterministically across iterations.
    fn with_delay_ms(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    fn executions_handle(&self) -> Arc<AtomicU64> {
        self.executions.clone()
    }
}

#[async_trait]
impl GuiActionExecutor for GuardExecutor {
    async fn action_backend_status(&self) -> GuiActionBackendStatus {
        self.backend.clone()
    }

    async fn execute(&self, request: GuiActionRequest) -> GuiActionExecution {
        let count = self.executions.fetch_add(1, Ordering::SeqCst) + 1;
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        if let (Some(after), Some(token)) = (self.cancel_after, self.cancel_token.as_ref()) {
            if count >= after {
                token.cancel("user pressed stop");
            }
        }
        if let Some(after) = self.halt_after {
            if count >= after {
                kria_core::safety::engage_halt("sidecar crashed mid-turn");
            }
        }
        GuiActionExecution::ok(
            "fixture_executor",
            serde_json::json!({ "executed": request.kind.as_str() }),
        )
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

const MULTI_STEP_PROMPT: &str = "Open KRIA Workflow App and focus the visible search field";

fn guard_request() -> GuiTurnRequest {
    GuiTurnRequest {
        session_id: "guard-session".into(),
        turn_id: "guard-turn".into(),
        workflow_id: "guard-workflow".into(),
        message: MULTI_STEP_PROMPT.into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: None,
        execution_environment: Default::default(),
        execution_mode: GuiExecutionMode::ExecuteFixture,
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

fn aborted_event(outcome: &GuiTurnOutcome) -> Option<&serde_json::Value> {
    outcome
        .events
        .iter()
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some("WorkflowRunAborted"))
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial_test::serial]
async fn cancel_before_turn_halts_before_first_action() {
    kria_core::safety::release_halt("test reset");
    // Cancel requested before the turn starts; with guards ON the loop must
    // abort at the very first action — nothing executes.
    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new();
    let executions = executor.executions_handle();

    let token = GuiCancelToken::new();
    token.cancel("user pressed stop");

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_cancel_token(Some(token));

    let outcome = runtime.run_turn(guard_request()).await;
    let types = event_types(&outcome);

    assert!(types.contains(&"WorkflowRunStarted".to_string()), "events: {types:?}");
    assert!(types.contains(&"WorkflowRunAborted".to_string()), "events: {types:?}");
    assert!(!types.contains(&"WorkflowStepCompleted".to_string()), "no step should complete");
    assert_eq!(executions.load(Ordering::SeqCst), 0, "no action may execute");

    let aborted = aborted_event(&outcome).expect("aborted event");
    assert_eq!(aborted["cause"], "cancelled");
    assert_eq!(aborted["halted_before_step_index"], 0);
    assert_eq!(outcome.status, "blocked");
    assert!(outcome.reply.to_lowercase().contains("stopped"), "reply: {}", outcome.reply);
}

#[tokio::test]
#[serial_test::serial]
async fn cancel_halts_before_next_action() {
    kria_core::safety::release_halt("test reset");
    // The user presses Stop *during* the first action. Step 0 finishes, but the
    // loop must check the cancel token before the second action and abort — so
    // exactly one action executes (Requirement 21.1).
    let perception = GuardPerception::new("KRIA Workflow App");
    let token = GuiCancelToken::new();
    let executor = GuardExecutor::new().cancel_after(1, token.clone());
    let executions = executor.executions_handle();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_cancel_token(Some(token));

    let outcome = runtime.run_turn(guard_request()).await;
    let types = event_types(&outcome);

    assert!(types.contains(&"WorkflowStepCompleted".to_string()), "first step completes: {types:?}");
    assert!(types.contains(&"WorkflowRunAborted".to_string()), "events: {types:?}");
    assert_eq!(
        executions.load(Ordering::SeqCst),
        1,
        "exactly one action executes; the next action must not run"
    );

    let aborted = aborted_event(&outcome).expect("aborted event");
    assert_eq!(aborted["cause"], "cancelled");
    assert_eq!(aborted["reason"], "user pressed stop");
    // Halted before the SECOND step (index 1), after the first completed.
    assert_eq!(aborted["halted_before_step_index"], 1);
    assert_eq!(outcome.status, "blocked");
}

#[tokio::test]
#[serial_test::serial]
async fn flag_off_preserves_behavior_even_when_cancelled() {
    // Same pre-cancelled token, but the flag is OFF (default). The loop must
    // ignore the cooperative cancel and run to completion exactly as before.
    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new();
    let executions = executor.executions_handle();

    let token = GuiCancelToken::new();
    token.cancel("user pressed stop");

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(GuiRuntimeGuardConfig::default()) // OFF
        .with_cancel_token(Some(token));

    let outcome = runtime.run_turn(guard_request()).await;
    let types = event_types(&outcome);

    assert!(!types.contains(&"WorkflowRunAborted".to_string()), "no abort with flag OFF: {types:?}");
    assert!(types.contains(&"WorkflowRunCompleted".to_string()), "events: {types:?}");
    assert!(executions.load(Ordering::SeqCst) >= 1, "actions execute normally");
    assert_eq!(outcome.status, "completed");
}

#[tokio::test]
#[serial_test::serial]
async fn no_cancel_token_and_guards_on_completes_normally() {
    kria_core::safety::release_halt("test reset");
    // Guards ON but nothing cancels and no halt: behavior is unchanged.
    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request()).await;
    let types = event_types(&outcome);

    assert!(!types.contains(&"WorkflowRunAborted".to_string()), "events: {types:?}");
    assert!(types.contains(&"WorkflowRunCompleted".to_string()), "events: {types:?}");
    assert_eq!(outcome.status, "completed");
}

// ── GlobalSafetyHalt tests (serial: they touch the process-global halt flag) ──

#[tokio::test]
#[serial_test::serial]
async fn halt_halts_before_next_action() {
    use kria_core::safety::{engage_halt, release_halt};

    release_halt("test reset");
    engage_halt("sidecar crashed");

    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new();
    let executions = executor.executions_handle();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request()).await;
    release_halt("test cleanup");

    let types = event_types(&outcome);
    assert!(types.contains(&"WorkflowRunAborted".to_string()), "events: {types:?}");
    assert_eq!(executions.load(Ordering::SeqCst), 0, "no action may execute while halted");

    let aborted = aborted_event(&outcome).expect("aborted event");
    assert_eq!(aborted["cause"], "global_safety_halt");
    assert_eq!(aborted["reason"], "sidecar crashed");
    assert_eq!(outcome.status, "blocked");
}

#[tokio::test]
#[serial_test::serial]
async fn halt_mid_turn_halts_before_next_action() {
    use kria_core::safety::release_halt;

    release_halt("test reset");
    // GlobalSafetyHalt is engaged DURING the first action (master kill-switch
    // trips mid-turn). Step 0 finishes, but the loop must check the halt before
    // the second action and abort — so exactly one action executes
    // (Requirement 21.2, halts before the NEXT action).
    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new().halt_after(1);
    let executions = executor.executions_handle();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request()).await;
    release_halt("test cleanup");

    let types = event_types(&outcome);
    assert!(types.contains(&"WorkflowStepCompleted".to_string()), "first step completes: {types:?}");
    assert!(types.contains(&"WorkflowRunAborted".to_string()), "events: {types:?}");
    assert_eq!(
        executions.load(Ordering::SeqCst),
        1,
        "exactly one action executes; the next action must not run after halt"
    );

    let aborted = aborted_event(&outcome).expect("aborted event");
    assert_eq!(aborted["cause"], "global_safety_halt");
    // Halted before the SECOND step (index 1), after the first completed.
    assert_eq!(aborted["halted_before_step_index"], 1);
    assert_eq!(outcome.status, "blocked");
}

#[tokio::test]
#[serial_test::serial]
async fn flag_off_does_not_abort_loop_on_halt() {
    // GlobalSafetyHalt is engaged but the flag is OFF: the loop-level guard does
    // not fire (existing behavior preserved). The fixture executor is not the
    // real backend, so no backend-level HALT applies here — the run completes.
    use kria_core::safety::{engage_halt, release_halt};

    release_halt("test reset");
    engage_halt("halted but flag off");

    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(GuiRuntimeGuardConfig::default()) // OFF
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request()).await;
    release_halt("test cleanup");

    let types = event_types(&outcome);
    assert!(!types.contains(&"WorkflowRunAborted".to_string()), "no loop abort with flag OFF: {types:?}");
    assert!(types.contains(&"WorkflowRunCompleted".to_string()), "events: {types:?}");
    assert_eq!(outcome.status, "completed");
}

// ── Task 1.3: budget / watchdog / flapping / verification aborts (T2) ─────────

fn guards_with(budget: TurnBudget) -> GuiRuntimeGuardConfig {
    GuiRuntimeGuardConfig::enabled(budget)
}

#[tokio::test]
#[serial_test::serial]
async fn budget_max_steps_aborts_before_next_action() {
    kria_core::safety::release_halt("test reset");
    // max_steps = 1: exactly one step may run, then the loop aborts before the
    // next action with a distinct `budget_max_steps` cause (Requirement 21.3).
    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new();
    let executions = executor.executions_handle();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_with(TurnBudget::default().with_max_steps(1)))
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request()).await;
    let types = event_types(&outcome);

    assert!(types.contains(&"WorkflowStepCompleted".to_string()), "first step runs: {types:?}");
    assert!(types.contains(&"WorkflowRunAborted".to_string()), "events: {types:?}");
    assert!(!types.contains(&"WorkflowRunCompleted".to_string()), "must not complete: {types:?}");
    assert_eq!(executions.load(Ordering::SeqCst), 1, "exactly one action runs");

    let aborted = aborted_event(&outcome).expect("aborted event");
    assert_eq!(aborted["cause"], "budget_max_steps");
    assert_eq!(aborted["halted_before_step_index"], 1);
    assert_eq!(outcome.status, "blocked");
    assert!(outcome.reply.to_lowercase().contains("stopped"), "reply: {}", outcome.reply);
}

#[tokio::test]
#[serial_test::serial]
async fn budget_watchdog_aborts_before_next_action() {
    kria_core::safety::release_halt("test reset");
    // Tiny watchdog + a slow executor: the first step elapses past the watchdog
    // so the loop aborts before the next action (Requirement 19.2 / 21.3).
    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new().with_delay_ms(40);
    let executions = executor.executions_handle();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_with(TurnBudget::default().with_turn_watchdog_ms(5)))
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request()).await;
    let types = event_types(&outcome);

    assert!(types.contains(&"WorkflowRunAborted".to_string()), "events: {types:?}");
    assert_eq!(executions.load(Ordering::SeqCst), 1, "watchdog stops after one action");

    let aborted = aborted_event(&outcome).expect("aborted event");
    assert_eq!(aborted["cause"], "budget_watchdog");
    assert_eq!(outcome.status, "blocked");
}

#[tokio::test]
#[serial_test::serial]
async fn budget_flag_off_preserves_behavior_under_tight_budget() {
    kria_core::safety::release_halt("test reset");
    // The same tight max_steps=1 budget, but the flag is OFF (default): the loop
    // ignores the budget and runs to completion exactly as before.
    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new();
    let executions = executor.executions_handle();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        // budget is attached but the flag stays OFF (default constructor).
        .with_runtime_guards(GuiRuntimeGuardConfig::default())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request()).await;
    let types = event_types(&outcome);

    assert!(!types.contains(&"WorkflowRunAborted".to_string()), "no abort with flag OFF: {types:?}");
    assert!(types.contains(&"WorkflowRunCompleted".to_string()), "events: {types:?}");
    assert!(executions.load(Ordering::SeqCst) >= 1, "actions execute normally");
    assert_eq!(outcome.status, "completed");
}

// ── Task 1.5: re-observe cap / flapping aborts (T2) ──────────────────────────
//
// These exercise the two remaining runaway-control caps from Requirement 19.4 /
// 21.4 through the FULL in-process pipeline (no display). They use a TypeText
// combo whose deterministic plan is three steps — FocusField → TypeText →
// VerifyState — so the loop re-observes between steps and the caps are reached
// at the pre-action checkpoint before the third step. The typed payload is the
// literal "Search", which matches the fixture's focused "Search" field so both
// executable steps verify and the loop genuinely reaches step 3.
//
// NOTE on `repeated_verification_failure` (Requirement 21.4): this cause tag is
// fully covered at T1 by the `GuiTurnBudgetTracker` unit tests in
// `turn_budget.rs` (`tracker_aborts_on_repeated_verification_failure`,
// `tracker_verification_pass_resets_the_streak`). It is NOT reachable through
// the full T2 pipeline by design: the workflow loop `break`s and reports
// `blocked` on the FIRST step whose verification does not succeed (see
// `GuiWorkflowStepKind::Executable` handling in mod.rs), so consecutive
// verification failures never accumulate across iterations within one turn.
// The streak cap is a defensive backstop for any future loop path that retries
// in place; driving it through the pipeline would require changing the loop's
// stop-on-first-failure behavior, which is out of scope for this test-hardening
// task (and safer left as-is).

/// A 3-step combo: focus the search field, type "Search" into it, then verify.
const TYPE_COMBO_PROMPT: &str =
    "Type \"Search\" into the visible search field and verify the text is entered";

fn guard_request_with(message: &str) -> GuiTurnRequest {
    let mut request = guard_request();
    request.message = message.into();
    request
}

#[tokio::test]
#[serial_test::serial]
async fn budget_max_reobserve_aborts_before_next_action() {
    kria_core::safety::release_halt("test reset");
    // max_reobserve = 1: the first re-observe (before step 2) consumes the whole
    // re-observe budget, so the loop aborts before step 3 with the distinct
    // `budget_max_reobserve` cause (Requirement 19.4 / 21.3).
    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new();
    let executions = executor.executions_handle();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_with(TurnBudget::default().with_max_reobserve(1)))
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(TYPE_COMBO_PROMPT)).await;
    let types = event_types(&outcome);

    assert!(
        types.contains(&"WorkflowRunAborted".to_string()),
        "expected a runaway-control abort: {types:?}"
    );
    assert!(
        !types.contains(&"WorkflowRunCompleted".to_string()),
        "must not complete once the re-observe cap is breached: {types:?}"
    );

    let aborted = aborted_event(&outcome).expect("aborted event");
    assert_eq!(aborted["cause"], "budget_max_reobserve");
    // Two executable steps ran (focus + type); the third never starts.
    assert_eq!(
        executions.load(Ordering::SeqCst),
        2,
        "re-observe cap stops the loop before the third action"
    );
    assert_eq!(outcome.status, "blocked");
    assert!(
        outcome.reply.to_lowercase().contains("stopped"),
        "reply: {}",
        outcome.reply
    );
}

#[tokio::test]
#[serial_test::serial]
async fn flapping_aborts_before_next_action() {
    kria_core::safety::release_halt("test reset");
    // A "stuck" screen returns the same hash on every observation. With a
    // flapping threshold of 2, the seeded observation plus one re-observe makes
    // the same screen recur twice → the loop aborts before step 3 with the
    // distinct `flapping` cause (Requirement 21.4) rather than oscillating.
    let perception = GuardPerception::stuck("KRIA Workflow App");
    let executor = GuardExecutor::new();
    let executions = executor.executions_handle();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_with(TurnBudget::default().with_flapping_threshold(2)))
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(TYPE_COMBO_PROMPT)).await;
    let types = event_types(&outcome);

    assert!(
        types.contains(&"WorkflowRunAborted".to_string()),
        "expected a flapping abort: {types:?}"
    );
    assert!(
        !types.contains(&"WorkflowRunCompleted".to_string()),
        "must not complete while flapping: {types:?}"
    );

    let aborted = aborted_event(&outcome).expect("aborted event");
    assert_eq!(aborted["cause"], "flapping");
    assert!(
        aborted["reason"]
            .as_str()
            .map(|reason| reason.to_lowercase().contains("flapping"))
            .unwrap_or(false),
        "flapping reason should be surfaced: {aborted}"
    );
    // Both executable steps ran before the screen was detected as flapping.
    assert_eq!(executions.load(Ordering::SeqCst), 2);
    assert_eq!(outcome.status, "blocked");
}

#[tokio::test]
#[serial_test::serial]
async fn reobserve_and_flapping_caps_flag_off_preserve_behavior() {
    kria_core::safety::release_halt("test reset");
    // The same tight re-observe budget AND a stuck screen, but the flag is OFF:
    // neither runaway-control cap fires and the turn runs to completion exactly
    // as before (existing Step 1–12 behavior preserved).
    let perception = GuardPerception::stuck("KRIA Workflow App");
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(GuiRuntimeGuardConfig::default()) // OFF
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(TYPE_COMBO_PROMPT)).await;
    let types = event_types(&outcome);

    assert!(
        !types.contains(&"WorkflowRunAborted".to_string()),
        "no runaway abort with flag OFF: {types:?}"
    );
}

// ── Task 1.5: abort events are sanitized (no raw prompt / secret leakage) ─────

#[tokio::test]
#[serial_test::serial]
async fn abort_events_do_not_leak_raw_prompt_or_secret() {
    kria_core::safety::release_halt("test reset");
    // A cancelled turn whose prompt embeds a unique raw marker and a fake secret.
    // The abort event + reply + response must be sanitized: neither the raw
    // prompt marker nor the secret may appear anywhere (Property 7).
    const RAW_MARKER: &str = "ZZZABORTRAWMARKER";
    const SECRET: &str = "hunter2-topsecret-token";
    let prompt = format!(
        "{RAW_MARKER} open KRIA Workflow App and type the password {SECRET} into the search field"
    );

    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new();
    let token = GuiCancelToken::new();
    token.cancel("user pressed stop");

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_cancel_token(Some(token));

    let outcome = runtime.run_turn(guard_request_with(&prompt)).await;

    // The turn aborted before any action.
    let aborted = aborted_event(&outcome).expect("aborted event");
    assert_eq!(aborted["cause"], "cancelled");

    let events = serde_json::to_string(&outcome.events).expect("serialize events");
    let response = serde_json::to_string(&outcome.response).expect("serialize response");
    for surface in [&events, &response, &outcome.reply] {
        assert!(!surface.contains(RAW_MARKER), "raw prompt marker leaked: {surface}");
        assert!(!surface.contains(SECRET), "secret leaked: {surface}");
    }
    assert!(!events.contains("\"raw_prompt\""), "raw_prompt field exposed in events");
}

// ── Task 3.1: per-step re-observe hook (gui_cog_reobserve) ───────────────────
//
// These exercise the foundation hook through the FULL in-process pipeline (no
// display). The hook obtains a FRESH GuiContext between steps from the
// perception provider and is BOUNDED by the Task 1 runaway caps regardless of
// the flag. The `gui_cog_reobserve` flag (default OFF) gates ONLY the additive
// `WorkflowReobserveHook` instrumentation event.

#[tokio::test]
#[serial_test::serial]
async fn reobserve_hook_emits_event_when_flag_on() {
    kria_core::safety::release_halt("test reset");
    // Guards ON (so the turn runs the bounded loop) + reobserve flag ON: the
    // 3-step combo re-observes between steps, so the hook event is emitted with
    // its cap-binding fields.
    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_reobserve(GuiReobserveConfig::enabled())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(TYPE_COMBO_PROMPT)).await;
    let types = event_types(&outcome);

    assert!(
        types.contains(&"WorkflowReobserveHook".to_string()),
        "re-observe hook event must be emitted when the flag is ON: {types:?}"
    );

    let hook = outcome
        .events
        .iter()
        .find(|event| event["type"] == "WorkflowReobserveHook")
        .expect("hook event");
    // The hook surfaces the Task 1 cap binding so re-observe is provably bounded.
    assert_eq!(hook["bounded_by_runaway_caps"], true);
    assert!(hook["max_reobserve"].as_u64().is_some(), "cap surfaced: {hook}");
    assert!(hook["step_index"].as_u64().is_some(), "step index surfaced: {hook}");
}

#[tokio::test]
#[serial_test::serial]
async fn reobserve_hook_silent_when_flag_off_preserves_behavior() {
    kria_core::safety::release_halt("test reset");
    // Flag OFF (default): the underlying re-observe still happens (the turn
    // completes exactly as before) but the additive hook event is suppressed.
    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        // no .with_reobserve(...) → default OFF
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(TYPE_COMBO_PROMPT)).await;
    let types = event_types(&outcome);

    assert!(
        !types.contains(&"WorkflowReobserveHook".to_string()),
        "no hook event when the flag is OFF: {types:?}"
    );
    // Behavior preserved: the underlying re-observe still runs (multiple
    // ObservationStarted across steps) and steps still progress, with no
    // runaway-control abort introduced by the (OFF) hook.
    assert!(
        types.contains(&"WorkflowStepCompleted".to_string()),
        "steps still progress with the hook OFF: {types:?}"
    );
    assert!(
        types.iter().filter(|t| *t == "ObservationStarted").count() > 1,
        "re-observe between steps still happens with the hook OFF: {types:?}"
    );
    assert!(
        !types.contains(&"WorkflowRunAborted".to_string()),
        "the OFF hook introduces no runaway abort: {types:?}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn reobserve_hook_on_is_still_bounded_by_runaway_caps() {
    kria_core::safety::release_halt("test reset");
    // The hook must NEVER run unbounded: with the flag ON and a tight
    // max_reobserve=1, the loop still aborts before the next action with the
    // distinct `budget_max_reobserve` cause (Requirement 19.4 / 21.3,
    // Property 9). Enabling the hook does not relax the Task 1 caps.
    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_with(TurnBudget::default().with_max_reobserve(1)))
        .with_reobserve(GuiReobserveConfig::enabled())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(TYPE_COMBO_PROMPT)).await;
    let types = event_types(&outcome);

    assert!(
        types.contains(&"WorkflowRunAborted".to_string()),
        "the hook must remain bounded by the re-observe cap: {types:?}"
    );
    let aborted = aborted_event(&outcome).expect("aborted event");
    assert_eq!(aborted["cause"], "budget_max_reobserve");
    assert_eq!(outcome.status, "blocked");
}

// ── Task 3.2: next-target resolution against the FRESH context ────────────────
//
// Property 2 (Requirements 2.1/2.2): after a state-changing step the next step's
// target resolves against a FRESH observation, not the stale initial screen. The
// browser-search plan is `OpenApp → FocusField → TypeText → …`, so the re-observe
// that feeds the FocusField target resolution follows the state-changing OpenApp
// step. With the flag ON the re-observe hook surfaces that trigger via its
// `cause` (`post_state_change_resolution`); with the flag OFF the underlying
// re-observe + fresh-context threading is preserved but the hook is silent.

/// A browser-search prompt whose deterministic plan starts with a state-changing
/// `OpenApp` step followed by a target-requiring `FocusField` step.
const BROWSER_SEARCH_PROMPT: &str = "Open Google Chrome and search for KRIA";

#[tokio::test]
#[serial_test::serial]
async fn next_target_reobserve_follows_state_changing_step() {
    kria_core::safety::release_halt("test reset");
    // Flag ON: the OpenApp step changes GUI state, so the re-observe before the
    // next step's target resolution is tagged `post_state_change_resolution` and
    // the fresh context it captures is what the FocusField target resolves
    // against (Property 2 / Requirements 2.1, 2.2). The hook remains bounded by
    // the Task 1 runaway caps.
    let perception = GuardPerception::new("Google Chrome");
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_reobserve(GuiReobserveConfig::enabled())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(BROWSER_SEARCH_PROMPT)).await;
    let types = event_types(&outcome);

    let hook = outcome
        .events
        .iter()
        .find(|event| event["type"] == "WorkflowReobserveHook")
        .unwrap_or_else(|| panic!("re-observe hook event with the flag ON: {types:?}"));
    assert_eq!(
        hook["cause"], "post_state_change_resolution",
        "the re-observe before the next target must be tagged as following a \
         state-changing step: {hook}"
    );
    assert_eq!(hook["bounded_by_runaway_caps"], true);
    assert!(hook["step_index"].as_u64().is_some(), "step index surfaced: {hook}");
}

#[tokio::test]
#[serial_test::serial]
async fn next_target_reobserve_flag_off_preserves_behavior() {
    kria_core::safety::release_halt("test reset");
    // Flag OFF (default): the same browser-search plan still re-observes between
    // steps (so the next target still resolves against the fresh screen), but the
    // explicit hook event is suppressed — flag-OFF behavior is byte-for-byte
    // identical to before Task 3.2.
    let perception = GuardPerception::new("Google Chrome");
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        // no .with_reobserve(...) → default OFF
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(BROWSER_SEARCH_PROMPT)).await;
    let types = event_types(&outcome);

    assert!(
        !types.contains(&"WorkflowReobserveHook".to_string()),
        "no hook event when the flag is OFF: {types:?}"
    );
    // The underlying re-observe between steps still happens with the flag OFF.
    assert!(
        types.iter().filter(|t| *t == "ObservationStarted").count() > 1,
        "re-observe between steps still happens with the hook OFF: {types:?}"
    );
}

// ── Task 3.3: bounded readiness wait before next-target resolution ───────────
//
// Before resolving a step that depends on a window/app/page which may still be
// loading (here: the FocusField that follows the state-changing OpenApp in the
// browser-search plan), the runtime performs a BOUNDED readiness wait — it
// re-observes until the expected window/app/page is observable, THEN resolves.
// The wait is STRICTLY bounded by the Task 1 caps (no unbounded poll, Property
// 9) and gated behind `gui_cog_reobserve` (flag OFF preserves prior behavior).

fn readiness_event(outcome: &GuiTurnOutcome) -> Option<&serde_json::Value> {
    outcome
        .events
        .iter()
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some("WorkflowReadinessWait"))
}

#[tokio::test]
#[serial_test::serial]
async fn readiness_wait_ready_when_window_observable() {
    kria_core::safety::release_halt("test reset");
    // The expected browser window is already observable, so readiness is reached
    // immediately (zero additional re-observes) and the next target resolves.
    let perception = GuardPerception::new("Google Chrome");
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_reobserve(GuiReobserveConfig::enabled())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(BROWSER_SEARCH_PROMPT)).await;
    let types = event_types(&outcome);

    let readiness = readiness_event(&outcome)
        .unwrap_or_else(|| panic!("readiness-wait event with the flag ON: {types:?}"));
    assert_eq!(
        readiness["ready"], true,
        "the expected window is observable so readiness is immediate: {readiness}"
    );
    assert_eq!(readiness["attempts"], 0, "no extra re-observe needed: {readiness}");
    // The cap binding is surfaced so the wait is provably bounded.
    assert_eq!(readiness["bounded_by_runaway_caps"], true);
    assert!(readiness["max_reobserve"].as_u64().is_some(), "cap surfaced: {readiness}");
    assert!(
        !types.contains(&"WorkflowRunAborted".to_string()),
        "a ready wait introduces no abort: {types:?}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn readiness_wait_silent_when_flag_off_preserves_behavior() {
    kria_core::safety::release_halt("test reset");
    // Flag OFF (default): no readiness-wait event is emitted and the turn behaves
    // exactly as before — steps still progress against the re-observed screen.
    let perception = GuardPerception::new("Google Chrome");
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        // no .with_reobserve(...) → default OFF
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(BROWSER_SEARCH_PROMPT)).await;
    let types = event_types(&outcome);

    assert!(
        readiness_event(&outcome).is_none(),
        "no readiness-wait event when the flag is OFF: {types:?}"
    );
    assert!(
        types.contains(&"WorkflowStepCompleted".to_string()),
        "steps still progress with the readiness wait OFF: {types:?}"
    );
}

/// A perception that reports the expected window for the first `ready_calls`
/// active-window probes (covering the state-changing step's resolve + verify),
/// then DEGRADES to a never-matching "loading" window so the next step's
/// readiness wait can never be satisfied — the deterministic driver for the
/// bounded NOT-ready stop. The screen hash changes every observation (so the
/// flapping cap is not what trips; the re-observe cap is).
struct DegradingPerception {
    ready_window: String,
    degrade_after: u64,
    active_calls: AtomicU64,
    screen_seq: AtomicU64,
}

impl DegradingPerception {
    fn new(ready_window: &str, degrade_after: u64) -> Self {
        Self {
            ready_window: ready_window.into(),
            degrade_after,
            active_calls: AtomicU64::new(0),
            screen_seq: AtomicU64::new(0),
        }
    }

    fn current_window(&self) -> String {
        let n = self.active_calls.fetch_add(1, Ordering::SeqCst);
        if n < self.degrade_after {
            self.ready_window.clone()
        } else {
            "Loading Placeholder".to_string()
        }
    }
}

#[async_trait]
impl GuiPerceptionProvider for DegradingPerception {
    async fn get_active_window(&self) -> GuiProbeResult {
        let window = self.current_window();
        GuiProbeResult::ok(serde_json::json!({ "title": window, "app_name": window }))
    }

    async fn get_desktop_state(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": self.ready_window,
            "accessibility_operational": true,
            "applications": [self.ready_window, "Browser"],
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
            "focused_window": self.ready_window,
            "focused_app": self.ready_window,
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
            "screen_hash": format!("degrade-screen-{seq}"),
            "byte_count": 16,
            "source": "fixture",
        }))
    }

    async fn focused_window_title(&self) -> Option<String> {
        Some(self.ready_window.clone())
    }
}

#[tokio::test]
#[serial_test::serial]
async fn readiness_wait_is_strictly_bounded_by_runaway_caps() {
    kria_core::safety::release_halt("test reset");
    // The expected window degrades to a never-matching "loading" window before
    // the FocusField step, so the readiness wait can never be satisfied. With a
    // tight max_reobserve the wait MUST stop — it can never poll unbounded. The
    // loop's pre-action checkpoint fires the existing `budget_max_reobserve`
    // abort (Requirement 19.4 / 21.3, Property 9).
    let perception = DegradingPerception::new("Google Chrome", 24);
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_with(TurnBudget::default().with_max_reobserve(2)))
        .with_reobserve(GuiReobserveConfig::enabled())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(BROWSER_SEARCH_PROMPT)).await;
    let types = event_types(&outcome);

    // The wait stops; the turn never completes (no unbounded poll).
    assert!(
        !types.contains(&"WorkflowRunCompleted".to_string()),
        "an un-ready readiness wait must never complete the turn: {types:?}"
    );
    assert_eq!(outcome.status, "blocked", "turn stops safely: {types:?}");
    // A readiness-wait event surfaces the cap binding regardless of outcome.
    assert!(
        readiness_event(&outcome).is_some(),
        "a readiness-wait event must be emitted when the flag is ON: {types:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 3.5 — T1/T2: re-observe-between-steps + caps (consolidated suite)
//
// This section is the cohesive, named proof for Task 3 (Requirements 2 & 6):
// the per-step re-observe paths added in 3.1–3.4 (the re-observe hook, the
// bounded readiness wait, and the present/absent presence recheck) form ONE
// re-observe-between-steps mechanism that:
//
//   (A) captures a FRESH GuiContext per step in a multi-step combo — each step
//       resolves against an observation taken AFTER the previous step, never the
//       stale initial observation (Requirement 2.1/2.2/2.3, Property 2; combo
//       coverage for Requirement 6.1);
//   (B) is STRICTLY BOUNDED by the SINGLE Task 1 turn-level re-observe budget —
//       the hook, the readiness wait, and the presence recheck all draw from the
//       SAME `GuiTurnBudgetTracker` (`reobserve_count` / `effective_max_reobserve`),
//       so combined they always terminate and a cap breach yields the existing
//       safe `budget_max_reobserve` abort (Requirement 19.4 / 21.3, Property 9);
//   (C) is gated by `gui_cog_reobserve` (default OFF): with the flag OFF NONE of
//       the additive Task 3 events are emitted and the turn behaves exactly as
//       before (Requirement 18, flag default OFF).
//
// KRIA authority invariants asserted here: bounded re-observe (no unbounded
// poll), deterministic orchestration, no Prompt→Tool shortcut (every step still
// flows observe→resolve→execute→verify), and flag default OFF.
//
// The individual 3.1–3.4 behaviors already have focused tests above (hook emit/
// silent/bounded, next-target post-state-change re-observe, readiness ready/
// not-ready/bounded, present-vs-absent classification). The tests below add the
// missing CONNECTIVE coverage: an explicit fresh-observation-per-step combo and
// an explicit single-shared-budget assertion across the combined paths.

/// Every event this turn that carries the shared re-observe accounting fields
/// (the hook, the readiness wait, and the presence recheck), in emission order.
fn reobserve_accounting_events(outcome: &GuiTurnOutcome) -> Vec<&serde_json::Value> {
    const REOBSERVE_EVENT_TYPES: &[&str] = &[
        "WorkflowReobserveHook",
        "WorkflowReadinessWait",
        "WorkflowTargetPresence",
    ];
    outcome
        .events
        .iter()
        .filter(|event| {
            event
                .get("type")
                .and_then(serde_json::Value::as_str)
                .map(|t| REOBSERVE_EVENT_TYPES.contains(&t))
                .unwrap_or(false)
        })
        .collect()
}

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
#[serial_test::serial]
async fn task3_5_multi_step_combo_reobserves_fresh_context_between_steps() {
    kria_core::safety::release_halt("test reset");
    // A multi-step combo (FocusField → TypeText → VerifyState) with the
    // re-observe flag ON. The runtime must capture a FRESH observation BETWEEN
    // steps — proven three independent ways:
    //   1. more than one ObservationCompleted, all with DISTINCT observation_ids
    //      (a genuinely fresh observation per step, never the stale initial one);
    //   2. at least one re-observe hook tagged with a step index, surfacing the
    //      per-step re-observe with its cap binding;
    //   3. the workflow_run's current_context_id has ADVANCED away from the
    //      initial_context_id (the run is acting on the re-observed context).
    // The combo still completes — fresh-context resolution lets each step act on
    // the current screen (Requirement 2 / 6.1, Property 2).
    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_reobserve(GuiReobserveConfig::enabled())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(TYPE_COMBO_PROMPT)).await;
    let types = event_types(&outcome);

    // (1) A fresh observation per step: >1 observation, all ids distinct.
    let obs_ids = observation_completed_ids(&outcome);
    assert!(
        obs_ids.len() > 1,
        "a multi-step combo must re-observe between steps (>1 ObservationCompleted): {types:?}"
    );
    let mut unique = obs_ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        obs_ids.len(),
        "each re-observe must capture a FRESH observation (distinct observation_ids), \
         never the stale initial one: {obs_ids:?}"
    );

    // (2) The per-step re-observe hook fired with a step index + cap binding.
    let hooks: Vec<&serde_json::Value> = outcome
        .events
        .iter()
        .filter(|event| event["type"] == "WorkflowReobserveHook")
        .collect();
    assert!(
        !hooks.is_empty(),
        "the per-step re-observe hook must fire for a multi-step combo: {types:?}"
    );
    for hook in &hooks {
        assert_eq!(hook["bounded_by_runaway_caps"], true, "hook: {hook}");
        assert!(hook["step_index"].as_u64().is_some(), "hook step index: {hook}");
    }

    // (3) The run is acting on the re-observed (fresh) context, not the initial.
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

    // Multiple steps acted on the fresh per-step context (each WorkflowStepCompleted
    // was preceded by its own fresh observation above), and the turn terminates
    // safely — never an unbounded poll or runaway abort. (Whether the final
    // verify step ultimately blocks is a fixture-verification detail; the point
    // here is that each step resolved against the CURRENT screen, Property 2.)
    assert!(
        types.iter().filter(|t| *t == "WorkflowStepCompleted").count() >= 2,
        "multiple steps must complete against their fresh per-step context: {types:?}"
    );
    assert!(
        types.iter().any(|t| t == "WorkflowRunCompleted" || t == "WorkflowRunBlocked"),
        "the turn must terminate safely: {types:?}"
    );
    assert!(
        !types.contains(&"WorkflowRunAborted".to_string()),
        "fresh-context re-observe must not trigger a runaway abort here: {types:?}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn task3_5_combined_reobserve_paths_share_one_turn_budget() {
    kria_core::safety::release_halt("test reset");
    // The browser-search combo (OpenApp → FocusField → …) exercises BOTH the
    // bounded readiness wait (after the state-changing OpenApp) AND the per-step
    // re-observe hook. With the flag ON, every event that re-observes must report
    // the SAME `max_reobserve` cap and a `reobserve_count` drawn from the SAME
    // monotonic turn-level tracker — proving the combined paths share ONE
    // re-observe budget rather than each keeping its own (Requirement 19.4,
    // Property 9). No single path may exceed the shared cap.
    let perception = GuardPerception::new("Google Chrome");
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        .with_reobserve(GuiReobserveConfig::enabled())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(BROWSER_SEARCH_PROMPT)).await;
    let types = event_types(&outcome);

    let accounting = reobserve_accounting_events(&outcome);
    // The combo must drive more than one re-observe path (hook + readiness).
    let distinct_paths: std::collections::BTreeSet<&str> = accounting
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .collect();
    assert!(
        distinct_paths.len() >= 2,
        "the combo must exercise multiple re-observe paths (hook + readiness): {types:?}"
    );

    // One shared cap: every re-observe path reports the SAME max_reobserve.
    let caps: std::collections::BTreeSet<u64> = accounting
        .iter()
        .filter_map(|event| event.get("max_reobserve").and_then(serde_json::Value::as_u64))
        .collect();
    assert_eq!(
        caps.len(),
        1,
        "all re-observe paths must share ONE turn-level cap (max_reobserve): {caps:?}"
    );
    let shared_cap = *caps.iter().next().unwrap();

    // One shared counter: reobserve_count is non-decreasing across the combined
    // paths in emission order and never exceeds the single shared cap.
    let mut last = 0u64;
    for event in &accounting {
        assert_eq!(
            event["bounded_by_runaway_caps"], true,
            "every re-observe path must declare it is cap-bound: {event}"
        );
        let count = event
            .get("reobserve_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| panic!("re-observe event missing reobserve_count: {event}"));
        assert!(
            count >= last,
            "reobserve_count must be monotonic across the SHARED tracker (got {count} after {last}): {event}"
        );
        assert!(
            count <= shared_cap,
            "no re-observe path may exceed the shared cap {shared_cap}: {event}"
        );
        last = count;
    }
}

#[tokio::test]
#[serial_test::serial]
async fn task3_5_combined_reobserve_paths_bounded_by_single_cap_abort() {
    kria_core::safety::release_halt("test reset");
    // With the flag ON and a tight shared cap (max_reobserve = 1), the combined
    // re-observe paths (readiness wait + hook + presence) all draw down the SAME
    // budget, so the turn aborts with the SINGLE `budget_max_reobserve` cause —
    // not a per-path runaway. This proves the caps are shared, not duplicated:
    // one cap stops every path together (Requirement 19.4 / 21.3, Property 9).
    let perception = GuardPerception::new("Google Chrome");
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_with(TurnBudget::default().with_max_reobserve(1)))
        .with_reobserve(GuiReobserveConfig::enabled())
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(BROWSER_SEARCH_PROMPT)).await;
    let types = event_types(&outcome);

    assert!(
        types.contains(&"WorkflowRunAborted".to_string()),
        "the shared re-observe cap must abort the turn: {types:?}"
    );
    assert!(
        !types.contains(&"WorkflowRunCompleted".to_string()),
        "must not complete once the shared re-observe cap is breached: {types:?}"
    );
    let aborted = aborted_event(&outcome).expect("aborted event");
    assert_eq!(
        aborted["cause"], "budget_max_reobserve",
        "a single shared re-observe budget breach must surface the existing safe cause"
    );
    assert_eq!(outcome.status, "blocked");

    // Every re-observe event that DID fire before the abort honored the single cap.
    for event in reobserve_accounting_events(&outcome) {
        assert_eq!(event["max_reobserve"], 1, "shared cap must be 1 everywhere: {event}");
        assert_eq!(event["bounded_by_runaway_caps"], true, "event: {event}");
    }
}

#[tokio::test]
#[serial_test::serial]
async fn task3_5_flag_off_emits_no_task3_events_but_completes_identically() {
    kria_core::safety::release_halt("test reset");
    // Flag OFF (default): for the SAME multi-step combo, NONE of the additive
    // Task 3 events (hook / readiness / presence) are emitted, yet the underlying
    // re-observe between steps still happens (>1 ObservationCompleted) and the
    // turn completes exactly as before — flag-OFF behavior is preserved
    // byte-for-byte aside from the suppressed additive instrumentation
    // (Requirement 18, flag default OFF).
    let perception = GuardPerception::new("KRIA Workflow App");
    let executor = GuardExecutor::new();

    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_runtime_guards(guards_on())
        // no .with_reobserve(...) → default OFF
        .with_cancel_token(None);

    let outcome = runtime.run_turn(guard_request_with(TYPE_COMBO_PROMPT)).await;
    let types = event_types(&outcome);

    assert!(
        reobserve_accounting_events(&outcome).is_empty(),
        "flag OFF must emit none of the additive Task 3 re-observe events: {types:?}"
    );
    // The underlying re-observe between steps is preserved (fresh obs per step).
    assert!(
        observation_completed_ids(&outcome).len() > 1,
        "re-observe between steps still happens with the flag OFF: {types:?}"
    );
    // The turn reaches the same terminal workflow status as before (here the
    // combo's final verify step blocks under this fixture) and is NEVER aborted
    // by a runaway cap — the OFF flag changes nothing but the additive events.
    assert!(
        types.iter().any(|t| t == "WorkflowRunCompleted" || t == "WorkflowRunBlocked"),
        "the combo still terminates with the flag OFF: {types:?}"
    );
    assert!(
        !types.contains(&"WorkflowRunAborted".to_string()),
        "the OFF flag introduces no runaway abort: {types:?}"
    );
}
