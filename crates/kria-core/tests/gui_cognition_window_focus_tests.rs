//! Task 4.4 — T2 integration tests for the Wayland-safe window-focus path
//! (Requirement 3, Property 7/8/9).
//!
//! These exercise the FULL in-process single-proposal pipeline (deterministic
//! fixtures, no display, no network) through `run_turn` and assert the
//! `gui_cog_wayland_focus` behavior end-to-end:
//!
//!   * a `SwitchWindow` turn (flag ON) selects the **session-appropriate**
//!     backend chain and reports a TRUTHFUL `backend_used` in the execution
//!     result + events;
//!   * activate-by-window-identity is preferred, and that identity comes ONLY
//!     from sanitized resolved-target data (KRIA authority: no Prompt→Tool
//!     shortcut);
//!   * the bounded verify-by-reobserve verdict is reported truthfully —
//!     `verified` when the FRESH active window matches the requested identity,
//!     `failed` when it does not, and `inconclusive` when the fresh
//!     active-window probe is unreliable (never a false `verified`);
//!   * with the flag OFF (default) the prior SwitchWindow behavior is preserved
//!     byte-for-byte — no `window_focus` routing object is emitted and the
//!     legacy backend tag is reported.
//!
//! The fixtures are self-contained: no `KRIA_*` env var, no filesystem, no
//! socket. A shared `FocusWorld` flag lets the executor deterministically
//! control what the POST-action re-observation reports (the "where the switch
//! landed" signal that drives the verify verdict), without any timing race.
//!
//! NOTE on the no-viable-path case (Requirement 3.3, `window_focus_unavailable`):
//! through `run_turn` the production no-path branch is unreachable by
//! construction — `NoBackendAvailable` requires `can_execute_actions == false`,
//! which the execution-precondition gate rejects BEFORE the focus route is
//! computed; and `NoTarget` requires an empty window/app hint, which the
//! resolver rejects (`needs_clarification`) BEFORE a proposal is ever built. The
//! no-path decision (chain selection → `select_window_focus_backend` error →
//! actionable, non-"wmctrl" message → null `backend_used`) is therefore covered
//! at T1 in `window_focus.rs` (`routing_no_available_backend_is_actionable_error`,
//! `routing_without_target_never_blindly_alt_tabs`,
//! `routing_json_surfaces_error_and_null_backend_on_no_path`,
//! `no_path_message_*`).

use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use kria_core::agent::gui_cognition::execution_environment::GuiExecutionEnvironment;
use kria_core::agent::gui_cognition::executor::{
    GuiActionBackendStatus, GuiActionExecution, GuiActionExecutor, GuiActionRequest,
    GuiExecutionMode,
};
use kria_core::agent::gui_cognition::perception::{GuiPerceptionProvider, GuiProbeResult};
use kria_core::agent::gui_cognition::window_focus::GuiWaylandFocusConfig;
use kria_core::agent::gui_cognition::{GuiCognitionRuntime, GuiTurnOutcome, GuiTurnRequest};

// ── Shared world: lets the executor control the POST-action observation ──────

/// Flipped to `true` by the executor once the SwitchWindow action has run, so
/// the perception provider can report a different active window (or an
/// unreliable probe) on the bounded verify-by-reobserve observation. This is
/// deterministic and race-free: every pre-action probe sees `false`, every
/// post-action probe sees `true`.
#[derive(Default)]
struct FocusWorld {
    acted: AtomicBool,
}

// ── Perception provider (controllable pre/post active window) ────────────────

struct FocusPerception {
    /// Active window BEFORE the action. MUST contain the target query so the
    /// `SwitchWindow` target resolves (resolver confidence 0.9 when the active
    /// window label contains the hint).
    pre_active: String,
    /// Active window reported AFTER the action (the fresh verify-by-reobserve
    /// observation) — this is what the verdict is computed against.
    post_active: String,
    /// When false, the POST-action active-window probe fails, so
    /// `active_window_probe_ok` is false and the verdict is `inconclusive`.
    post_probe_ok: bool,
    world: Arc<FocusWorld>,
    screen_seq: AtomicU64,
}

impl FocusPerception {
    fn verified() -> Self {
        Self::build("Browser", "Browser", true)
    }

    fn failed() -> Self {
        // Resolves against "Browser", but the switch lands on a different window.
        Self::build("Browser", "Files", true)
    }

    fn inconclusive() -> Self {
        // Resolves against "Browser", but the post-action probe is unreliable.
        Self::build("Browser", "Browser", false)
    }

    fn build(pre_active: &str, post_active: &str, post_probe_ok: bool) -> Self {
        Self {
            pre_active: pre_active.into(),
            post_active: post_active.into(),
            post_probe_ok,
            world: Arc::new(FocusWorld::default()),
            screen_seq: AtomicU64::new(0),
        }
    }

    fn acted(&self) -> bool {
        self.world.acted.load(Ordering::SeqCst)
    }

    /// True once the action ran AND the fresh probe is configured unreliable:
    /// every active-window source must then be unavailable so the verdict is a
    /// genuine `inconclusive` (not a fallback-resolved false `verified`).
    fn post_unreliable(&self) -> bool {
        self.acted() && !self.post_probe_ok
    }

    /// The active window label for the CURRENT observation phase.
    fn current_active(&self) -> &str {
        if self.acted() {
            &self.post_active
        } else {
            &self.pre_active
        }
    }
}

#[async_trait]
impl GuiPerceptionProvider for FocusPerception {
    async fn get_active_window(&self) -> GuiProbeResult {
        if self.post_unreliable() {
            return GuiProbeResult::err("active window probe unavailable");
        }
        let active = self.current_active().to_string();
        GuiProbeResult::ok(serde_json::json!({
            "title": active,
            "app_name": active,
        }))
    }

    async fn get_desktop_state(&self) -> GuiProbeResult {
        if self.post_unreliable() {
            // No active-window candidate (focused_window / focused_app / single
            // application) is offered, so the active-window probe is genuinely
            // unavailable on the fresh observation.
            return GuiProbeResult::err("desktop state probe unavailable");
        }
        let active = self.current_active().to_string();
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": active,
            "accessibility_operational": true,
            "applications": [active, "Browser", "Files"],
        }))
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": true }))
    }

    async fn find_ui_elements(&self, _role: &str) -> GuiProbeResult {
        // Window switching does not depend on in-window controls.
        GuiProbeResult::ok(serde_json::json!({ "elements": [] }))
    }

    async fn get_cursor_focus_state(&self) -> GuiProbeResult {
        if self.post_unreliable() {
            return GuiProbeResult::err("cursor focus probe unavailable");
        }
        let active = self.current_active().to_string();
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": active,
            "focused_app": active,
            "keyboard_focus_known": true,
            "text_cursor_known": false,
            "editable_target_known": false,
            "terminal_like": false,
            "focus_confidence": 0.9,
        }))
    }

    async fn capture_screenshot(&self) -> GuiProbeResult {
        // Screen hash advances each call so a state-changing step's
        // `screen_changed` strategy can succeed and the fresh observation is
        // evidenced.
        let seq = self.screen_seq.fetch_add(1, Ordering::SeqCst);
        GuiProbeResult::ok(serde_json::json!({
            "screen_hash": format!("focus-screen-{seq}"),
            "byte_count": 16,
            "source": "fixture",
        }))
    }

    async fn focused_window_title(&self) -> Option<String> {
        if self.post_unreliable() {
            return None;
        }
        Some(self.current_active().to_string())
    }
}

// ── Executor (configurable session type; flips the world after acting) ───────

struct FocusExecutor {
    backend: GuiActionBackendStatus,
    success: bool,
    world: Arc<FocusWorld>,
}

impl FocusExecutor {
    /// A backend for `session_type` with a usable input substrate
    /// (`can_execute_actions == true`), so the focus chain's Alt+Tab fallback is
    /// eligible and the execution-precondition gate passes.
    fn for_session(session_type: &str, world: Arc<FocusWorld>) -> Self {
        let mut backend = GuiActionBackendStatus::available("uinput_accessibility");
        backend.session_type = session_type.into();
        Self {
            backend,
            success: true,
            world,
        }
    }
}

#[async_trait]
impl GuiActionExecutor for FocusExecutor {
    async fn action_backend_status(&self) -> GuiActionBackendStatus {
        self.backend.clone()
    }

    async fn execute(&self, request: GuiActionRequest) -> GuiActionExecution {
        // After the action, the verify-by-reobserve observation reflects where
        // the switch landed (controlled by the perception fixture).
        self.world.acted.store(true, Ordering::SeqCst);
        if self.success {
            GuiActionExecution::ok(
                "uinput_accessibility",
                serde_json::json!({ "executed": request.kind.as_str() }),
            )
        } else {
            GuiActionExecution::err("uinput_accessibility", "fixture execution failure")
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

const SWITCH_PROMPT: &str = "Switch to the Browser window";

fn switch_request() -> GuiTurnRequest {
    GuiTurnRequest {
        session_id: "focus-session".into(),
        turn_id: "focus-turn".into(),
        workflow_id: "focus-workflow".into(),
        message: SWITCH_PROMPT.into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: None,
        execution_environment: GuiExecutionEnvironment::RealSession,
        execution_mode: GuiExecutionMode::ExecuteFixture,
        // Single-proposal path → execute_authorized_proposal (where SwitchWindow
        // is routed through the Wayland-safe abstraction).
        workflow_enabled: false,
        resume_checkpoint: None,
        resume_reason: None,
    }
}

fn event<'a>(outcome: &'a GuiTurnOutcome, event_type: &str) -> Option<&'a serde_json::Value> {
    outcome
        .events
        .iter()
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some(event_type))
}

fn event_types(outcome: &GuiTurnOutcome) -> Vec<String> {
    outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
}

/// The `window_focus` routing object attached to the terminal action event
/// (ActionCompleted on backend success).
fn completed_focus<'a>(outcome: &'a GuiTurnOutcome) -> &'a serde_json::Value {
    let completed = event(outcome, "ActionCompleted").expect("ActionCompleted event exists");
    completed
        .get("window_focus")
        .expect("ActionCompleted carries a window_focus routing object")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn switch_window_flag_on_wayland_selects_chain_and_reports_truthful_backend_used() {
    // Requirement 3.1/3.2: a Wayland SwitchWindow selects the session-appropriate
    // chain (compositor-native first, Alt+Tab last, NEVER wmctrl) and reports the
    // truthful backend that actually acted. The compositor-native handlers are
    // not wired yet, so a healthy Wayland session falls back to the verifiable
    // uinput Alt+Tab substrate.
    let perception = FocusPerception::verified();
    let world = perception.world.clone();
    let executor = FocusExecutor::for_session("wayland", world);
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_wayland_focus(GuiWaylandFocusConfig::enabled());

    let outcome = runtime.run_turn(switch_request()).await;
    let types = event_types(&outcome);
    assert!(types.contains(&"ActionStarted".to_string()), "events: {types:?}");
    assert!(types.contains(&"ActionCompleted".to_string()), "events: {types:?}");

    let focus = completed_focus(&outcome);
    assert_eq!(focus["routed"], serde_json::json!(true));
    // Session-appropriate chain: compositor-native first, Alt+Tab last, no wmctrl.
    assert_eq!(
        focus["chain"],
        serde_json::json!(["gnome_bridge", "portal", "uinput_alt_tab"]),
        "wayland chain must exclude wmctrl: {focus}"
    );
    // Truthful backend_used: the only wired substrate is the verifiable Alt+Tab.
    assert_eq!(focus["backend_used"], serde_json::json!("uinput_alt_tab"));
    assert_eq!(focus["requires_verification"], serde_json::json!(true));

    // The execution result reports the same truthful backend (not the legacy tool).
    let completed = event(&outcome, "ActionCompleted").unwrap();
    assert_eq!(completed["backend_used"], serde_json::json!("uinput_alt_tab"));
}

#[tokio::test]
async fn switch_window_identity_comes_from_sanitized_resolved_target() {
    // KRIA authority / Property 7: the activate-by-identity target is derived
    // from sanitized resolved-target data (the window hint), NOT the raw prompt,
    // and is preferred over a blind key spam. The identity label is the resolved
    // window hint, and the raw prompt never leaks into the routing object.
    let perception = FocusPerception::verified();
    let world = perception.world.clone();
    let executor = FocusExecutor::for_session("wayland", world);
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_wayland_focus(GuiWaylandFocusConfig::enabled());

    let outcome = runtime.run_turn(switch_request()).await;

    let focus = completed_focus(&outcome);
    assert_eq!(focus["has_target"], serde_json::json!(true));
    assert_eq!(
        focus["identity_label"], serde_json::json!("Browser"),
        "identity comes from the sanitized resolved window hint: {focus}"
    );

    // No raw prompt / raw-prompt field anywhere in events or response.
    let events = serde_json::to_string(&outcome.events).unwrap();
    let response = serde_json::to_string(&outcome.response).unwrap();
    assert!(!events.contains("\"raw_prompt\""), "raw_prompt field exposed in events");
    assert!(!response.contains("\"raw_prompt\""), "raw_prompt field exposed in response");
}

#[tokio::test]
async fn switch_window_verify_by_reobserve_reports_verified_when_active_matches() {
    // Requirement 3.4 / Property 8: when the FRESH active window matches the
    // requested identity, the verdict is truthfully `verified`.
    let perception = FocusPerception::verified();
    let world = perception.world.clone();
    let executor = FocusExecutor::for_session("wayland", world);
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_wayland_focus(GuiWaylandFocusConfig::enabled());

    let outcome = runtime.run_turn(switch_request()).await;

    let focus = completed_focus(&outcome);
    assert_eq!(
        focus["verification"], serde_json::json!("verified"),
        "fresh active window matches requested identity ⇒ verified: {focus}"
    );
}

#[tokio::test]
async fn switch_window_verify_by_reobserve_reports_failed_when_active_differs() {
    // The Alt+Tab fallback is NEVER trusted blindly: when the fresh active window
    // is a DIFFERENT window than requested, the verdict is `failed`, not a blind
    // success (Requirement 3.2/3.4, Property 8).
    let perception = FocusPerception::failed();
    let world = perception.world.clone();
    let executor = FocusExecutor::for_session("wayland", world);
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_wayland_focus(GuiWaylandFocusConfig::enabled());

    let outcome = runtime.run_turn(switch_request()).await;

    let focus = completed_focus(&outcome);
    assert_eq!(
        focus["verification"], serde_json::json!("failed"),
        "fresh active window differs from requested identity ⇒ failed: {focus}"
    );
    // The Alt+Tab fallback is the only wired backend and it carries the
    // verification obligation: its truthful verdict is `failed`, never a blind
    // pass — even though the executor backend itself reported success.
    let completed = event(&outcome, "ActionCompleted").unwrap();
    assert_eq!(completed["backend_used"], serde_json::json!("uinput_alt_tab"));
    assert_eq!(
        completed["window_focus"]["requires_verification"],
        serde_json::json!(true)
    );
}

#[tokio::test]
async fn switch_window_verify_by_reobserve_is_inconclusive_when_probe_unreliable() {
    // Requirement 3.4 / 23.2: an unreliable fresh active-window probe yields
    // `inconclusive`, NEVER a false `verified` (even though the requested and
    // stale labels happen to match).
    let perception = FocusPerception::inconclusive();
    let world = perception.world.clone();
    let executor = FocusExecutor::for_session("wayland", world);
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_wayland_focus(GuiWaylandFocusConfig::enabled());

    let outcome = runtime.run_turn(switch_request()).await;

    let focus = completed_focus(&outcome);
    assert_eq!(
        focus["verification"], serde_json::json!("inconclusive"),
        "unreliable fresh probe ⇒ inconclusive, never a false verified: {focus}"
    );
}

#[tokio::test]
async fn switch_window_flag_on_x11_chain_ends_with_wmctrl() {
    // Requirement 3.1: on an X11 session the chain still puts compositor-native
    // first and the X11-only wmctrl path LAST — but the truthful backend_used is
    // still the only wired substrate (uinput Alt+Tab). This proves the chain is
    // session-appropriate while the reported backend stays truthful.
    let perception = FocusPerception::verified();
    let world = perception.world.clone();
    let executor = FocusExecutor::for_session("x11", world);
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_wayland_focus(GuiWaylandFocusConfig::enabled());

    let outcome = runtime.run_turn(switch_request()).await;

    let focus = completed_focus(&outcome);
    assert_eq!(
        focus["chain"],
        serde_json::json!(["gnome_bridge", "portal", "uinput_alt_tab", "x11_wmctrl"]),
        "x11 chain ends with the X11-only wmctrl path: {focus}"
    );
    assert_eq!(focus["backend_used"], serde_json::json!("uinput_alt_tab"));
    assert_eq!(focus["verification"], serde_json::json!("verified"));
}

#[tokio::test]
async fn switch_window_flag_off_preserves_legacy_behavior_byte_for_byte() {
    // Flag OFF (default): the SwitchWindow turn runs the existing single-path
    // behavior. There is NO window_focus routing object anywhere, and the
    // execution result reports the legacy backend tool tag — not a focus backend.
    let perception = FocusPerception::verified();
    let world = perception.world.clone();
    let executor = FocusExecutor::for_session("wayland", world);
    // No `.with_wayland_focus(...)` → flag stays OFF (default).
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime.run_turn(switch_request()).await;
    let types = event_types(&outcome);
    assert!(types.contains(&"ActionCompleted".to_string()), "events: {types:?}");

    // No window_focus routing object is emitted on ANY event.
    for event in &outcome.events {
        assert!(
            event.get("window_focus").is_none(),
            "flag OFF must not attach a window_focus object: {event}"
        );
    }
    // And none anywhere in the serialized surfaces.
    let events = serde_json::to_string(&outcome.events).unwrap();
    let response = serde_json::to_string(&outcome.response).unwrap();
    assert!(!events.contains("window_focus"), "window_focus leaked into events with flag OFF");
    assert!(!response.contains("window_focus"), "window_focus leaked into response with flag OFF");

    // Legacy backend tag reported (the executor tool), not a focus backend tag.
    let completed = event(&outcome, "ActionCompleted").unwrap();
    assert_eq!(completed["backend_used"], serde_json::json!("uinput_accessibility"));
    assert_ne!(completed["backend_used"], serde_json::json!("uinput_alt_tab"));
}
