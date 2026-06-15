//! Task 6.2 (Requirements 5 / 15) — Password-field privacy.
//!
//! These are CI-safe T1 tests: the FULL in-process single-proposal pipeline runs
//! through `run_turn` with deterministic fixtures — no live KRIA desktop API, no
//! display, no network.
//!
//! The privacy guarantee under test (behind the `gui_cog_primitives` flag):
//! focusing a PASSWORD / secure-entry field, and any typed payload destined for
//! one, is treated as secret. The value is routed through the payload vault
//! (handle + hash only) and NEVER appears in the produced events JSON, reply,
//! action summary, or verification result. A redacted placeholder stands in
//! instead, and the verification strategy chosen for a secret field never reads
//! the field text back (`text_present`) — it uses focus/observation evidence.
//!
//! Flag OFF = unchanged: the secret-field annotations are not added and the
//! verification strategy is the legacy one.

use async_trait::async_trait;

use kria_core::agent::gui_cognition::execution_environment::GuiExecutionEnvironment;
use kria_core::agent::gui_cognition::executor::{
    is_password_or_secure_field, GuiActionBackendStatus, GuiActionExecution, GuiActionExecutor,
    GuiActionKind, GuiActionRequest, GuiExecutionMode, GuiPrimitivesConfig,
};
use kria_core::agent::gui_cognition::perception::{GuiPerceptionProvider, GuiProbeResult};
use kria_core::agent::gui_cognition::safety_hitl::GuiHitlDecisionFixture;
use kria_core::agent::gui_cognition::verifier::{
    select_verification_strategy, GuiVerificationStrategy,
};
use kria_core::agent::gui_cognition::{GuiCognitionRuntime, GuiTurnOutcome, GuiTurnRequest};

// A distinctive sentinel so any echo of the typed secret is unambiguous when we
// scan the serialized turn output.
const SECRET: &str = "Hunter2SuperSecretValue";

// ── Perception: a desktop with a single accessible PASSWORD text field ───────
//
// `find_ui_elements("text")` returns one control whose role is the AT-SPI secure
// entry role "password text" and whose accessible name is the benign label
// "Password" — so the field uniquely resolves and is detected as a secure entry
// by its role descriptor.

struct PasswordFieldPerception {
    active_window: String,
}

impl PasswordFieldPerception {
    fn new() -> Self {
        Self {
            active_window: "Login Window".into(),
        }
    }
}

#[async_trait]
impl GuiPerceptionProvider for PasswordFieldPerception {
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
            "applications": [self.active_window],
        }))
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": true }))
    }

    async fn find_ui_elements(&self, role: &str) -> GuiProbeResult {
        let elements = if role == "text" {
            vec![serde_json::json!({
                "role": "password text",
                "name": "Password",
                "label": "Password",
                "path": "/fixture/password/Password",
                "control_id": "fixture-password-field",
                "enabled": true,
                "visible": true,
                "focused": true,
                "in_active_window": true,
                "bounds": { "x": 10, "y": 20, "width": 240, "height": 32 },
                "score": 0.9,
                "identity_confidence": 0.9,
                "bounds_confidence": 0.9,
                "state_confidence": 0.9
            })]
        } else {
            Vec::new()
        };
        GuiProbeResult::ok(serde_json::json!({ "elements": elements }))
    }

    async fn get_cursor_focus_state(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": self.active_window,
            "focused_app": self.active_window,
            "focused_control_id": "fixture-password-field",
            "focused_control_label": "Password",
            "focused_control_role": "password text",
            "keyboard_focus_known": true,
            "text_cursor_known": true,
            "editable_target_known": true,
            "terminal_like": false,
            "focus_confidence": 0.9,
        }))
    }

    async fn capture_screenshot(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "screen_hash": "password-screen",
            "byte_count": 16,
            "source": "fixture",
        }))
    }

    async fn focused_window_title(&self) -> Option<String> {
        Some(self.active_window.clone())
    }
}

// ── Executor: succeeds at the backend layer; NEVER echoes the typed value ────

struct RecordingExecutor {
    backend: GuiActionBackendStatus,
}

impl RecordingExecutor {
    fn new() -> Self {
        Self {
            backend: GuiActionBackendStatus::available("fixture_executor"),
        }
    }
}

#[async_trait]
impl GuiActionExecutor for RecordingExecutor {
    async fn action_backend_status(&self) -> GuiActionBackendStatus {
        self.backend.clone()
    }

    async fn execute(&self, request: GuiActionRequest) -> GuiActionExecution {
        // A well-behaved backend reports only the action kind in its evidence —
        // never the (possibly secret) typed value.
        GuiActionExecution::ok(
            "fixture_executor",
            serde_json::json!({ "executed": request.kind.as_str() }),
        )
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn secret_request(message: &str) -> GuiTurnRequest {
    let mut request = GuiTurnRequest {
        session_id: "pw-session".into(),
        turn_id: "pw-turn".into(),
        workflow_id: "pw-workflow".into(),
        message: message.into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: Some(GuiHitlDecisionFixture::Approve),
        // Auto-approval is honored ONLY inside the TestSubstrate; a password
        // action is risk-gated, so the substrate + approve fixture lets the
        // deterministic pipeline reach execution offline.
        execution_environment: GuiExecutionEnvironment::TestSubstrate {
            scratch_dir: None,
            restore_clipboard: true,
        },
        execution_mode: GuiExecutionMode::ExecuteFixture,
        // Single-proposal path → execute_authorized_proposal.
        workflow_enabled: false,
        resume_checkpoint: None,
        resume_reason: None,
    };
    request.hitl_decision_fixture = Some(GuiHitlDecisionFixture::Approve);
    request
}

/// The full serialized turn surface a user/operator could ever see: events,
/// reply, and the structured response. If the secret is anywhere, it is here.
fn full_surface(outcome: &GuiTurnOutcome) -> String {
    let blob = serde_json::json!({
        "status": outcome.status,
        "reply": outcome.reply,
        "response": outcome.response,
        "events": outcome.events,
    });
    serde_json::to_string(&blob).expect("serialize turn surface")
}

fn event<'a>(outcome: &'a GuiTurnOutcome, event_type: &str) -> Option<&'a serde_json::Value> {
    outcome
        .events
        .iter()
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some(event_type))
}

fn verification_strategy(outcome: &GuiTurnOutcome) -> Option<String> {
    event(outcome, "ExecutionVerificationCompleted").and_then(|event| {
        event
            .get("verification_strategy")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    })
}

// ── Unit: secure-field detection from the descriptor ─────────────────────────

#[test]
fn detects_password_and_secure_fields_from_descriptor() {
    // AT-SPI secure entry role + toolkit variants.
    assert!(is_password_or_secure_field("password text", "Password", false));
    assert!(is_password_or_secure_field("PasswordText", "", false));
    assert!(is_password_or_secure_field("secure text field", "", false));
    assert!(is_password_or_secure_field("text", "password field", false));
    // A secure/protected state flag forces the secret treatment regardless.
    assert!(is_password_or_secure_field("text", "Username", true));

    // Benign fields are NEVER mis-flagged.
    assert!(!is_password_or_secure_field("text", "Username", false));
    assert!(!is_password_or_secure_field("push button", "Password Manager", false));
    assert!(!is_password_or_secure_field("entry", "Search", false));
}

// ── Unit: secret-field verification never reads the field text back ──────────

#[test]
fn secret_field_verification_strategy_never_reads_field_text() {
    // A secret payload destined for a password / secure field must NOT verify by
    // searching for / echoing the typed text (`text_present`). Typing/filling/
    // pasting fall back to `state_changed`; focusing uses `focused_control`.
    for kind in [
        GuiActionKind::TypeText,
        GuiActionKind::FillField,
        GuiActionKind::Paste,
    ] {
        let strategy = select_verification_strategy(&kind, true);
        assert_ne!(
            strategy,
            GuiVerificationStrategy::TextPresent,
            "{kind:?} with a secret payload must not echo field text"
        );
        assert_eq!(
            strategy,
            GuiVerificationStrategy::StateChanged,
            "{kind:?} secret payload should verify by state change"
        );
    }
    // Focusing a secure entry verifies by focused-control evidence.
    assert_eq!(
        select_verification_strategy(&GuiActionKind::FocusField, true),
        GuiVerificationStrategy::FocusedControl
    );

    // Without the secret flag, a normal typing action MAY use `text_present`
    // (proves the secret flag is what suppresses the read-back).
    assert_eq!(
        select_verification_strategy(&GuiActionKind::TypeText, false),
        GuiVerificationStrategy::TextPresent
    );
}

// ── T1: typing a secret into a password field never echoes the value ─────────

#[tokio::test]
async fn typing_secret_into_password_field_never_echoes_value() {
    let perception = PasswordFieldPerception::new();
    let executor = RecordingExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_primitives(GuiPrimitivesConfig::enabled());

    let outcome = runtime
        .run_turn(secret_request(&format!(
            "Type \"{SECRET}\" into the visible text field"
        )))
        .await;

    let surface = full_surface(&outcome);

    // 1) The secret appears NOWHERE in the produced events / reply / response.
    assert!(
        !surface.contains(SECRET),
        "secret value leaked into the turn surface: {surface}"
    );

    // 2) A redacted placeholder stands in for the secret instead.
    assert!(
        surface.contains("[secret]") || surface.contains("[redacted]"),
        "expected a redacted placeholder in the turn surface: {surface}"
    );

    // 3) The secret-field verification strategy never reads the field text back
    //    (`text_present`); it uses focus/observation evidence instead.
    let strategy = verification_strategy(&outcome)
        .expect("a secret-field turn reaches post-action verification");
    assert_ne!(
        strategy, "text_present",
        "secret field must NOT verify by reading field text back; got {strategy}"
    );
    assert!(
        matches!(strategy.as_str(), "focused_control" | "state_changed"),
        "secret field must use focus/state evidence, not field text; got {strategy}"
    );

    // 4) The ActionStarted event marks the field secret (observability without
    //    the value).
    let started = event(&outcome, "ActionStarted").expect("ActionStarted event exists");
    assert_eq!(started["secret_field"], serde_json::json!(true), "event: {started}");
}

// ── T1: focusing a password field selects the secret-safe strategy ───────────

#[tokio::test]
async fn focusing_password_field_uses_secret_safe_verification() {
    let perception = PasswordFieldPerception::new();
    let executor = RecordingExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_primitives(GuiPrimitivesConfig::enabled());

    let outcome = runtime
        .run_turn(secret_request("Focus the visible text field"))
        .await;

    let surface = full_surface(&outcome);
    assert!(
        !surface.contains(SECRET),
        "no secret should appear for a focus turn: {surface}"
    );

    // FocusField on a secure entry verifies by focused-control evidence — it
    // never reads the field's (secret) text back.
    let strategy = verification_strategy(&outcome)
        .expect("a password FocusField turn reaches post-action verification");
    assert_eq!(strategy, "focused_control", "got {strategy}");
    assert_ne!(strategy, "text_present");

    let started = event(&outcome, "ActionStarted").expect("ActionStarted event exists");
    assert_eq!(started["secret_field"], serde_json::json!(true), "event: {started}");
}

// ── T1: flag OFF leaves the path unchanged (no secret-field annotation) ──────

#[tokio::test]
async fn flag_off_does_not_add_secret_field_annotation() {
    let perception = PasswordFieldPerception::new();
    let executor = RecordingExecutor::new();
    // No `.with_primitives(...)` → `gui_cog_primitives` stays OFF (default).
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(secret_request("Focus the visible text field"))
        .await;

    if let Some(started) = event(&outcome, "ActionStarted") {
        assert!(
            started.get("secret_field").is_none(),
            "flag OFF must NOT add the secret_field annotation: {started}"
        );
    }
}
