//! Task 9.6 (Task 9.1 — Requirements 10, 13, 15, 22, 23) T2 — CI-safe,
//! deterministic, no-display integration tests proving the per-action-type
//! verification CONTRACT is surfaced through the real `run_turn` path under the
//! `gui_cog_safety_polish` flag.
//!
//! The contract's pure builder/applier functions are unit-tested inline in
//! `verifier.rs`; these tests close the remaining gap by asserting the runtime
//! WIRING: when the flag is ON the verification telemetry carries the explicit
//! contract (predicate + evidence source + bounded wait + confidence bar), and
//! when the flag is OFF the contract is ABSENT and the verdict is byte-for-byte
//! unchanged (additive-only). A secret-payload action's contract uses the
//! non-revealing `state_changed` predicate and never carries the secret value.
//!
//! No live display, network, or KRIA desktop API is required — a fixture
//! perception provider + always-succeeding fixture executor drive the turn.

use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};

use kria_core::agent::gui_cognition::execution_environment::GuiExecutionEnvironment;
use kria_core::agent::gui_cognition::executor::{
    GuiActionBackendStatus, GuiActionExecution, GuiActionExecutor, GuiActionRequest,
    GuiExecutionMode, GuiPrimitivesConfig,
};
use kria_core::agent::gui_cognition::perception::{GuiPerceptionProvider, GuiProbeResult};
use kria_core::agent::gui_cognition::verifier::{
    GuiSafetyPolishConfig, VERIFICATION_CONTRACT_MIN_CONFIDENCE,
};
use kria_core::agent::gui_cognition::{GuiCognitionRuntime, GuiTurnOutcome, GuiTurnRequest};

const SECRET_MARKER: &str = "ZZZSECRETVALUE9999";
const VERIFICATION_EVENT: &str = "ExecutionVerificationCompleted";

// ── Fixture perception provider (no display / no network) ────────────────────

struct FixtureContextProvider {
    active_window: String,
    secure_field: bool,
    screen_seq: AtomicU64,
}

impl FixtureContextProvider {
    fn new(active_window: &str) -> Self {
        Self {
            active_window: active_window.into(),
            secure_field: false,
            screen_seq: AtomicU64::new(0),
        }
    }

    fn with_secure_field(active_window: &str) -> Self {
        Self {
            active_window: active_window.into(),
            secure_field: true,
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
        let (field_role, field_name, field_label, field_id) = if self.secure_field {
            ("password text", "Password", "Password field", "fixture-password-field")
        } else {
            ("text", "Search", "Search", "fixture-search-field")
        };
        let elements = match role {
            "text" => vec![serde_json::json!({
                "role": field_role,
                "name": field_name,
                "label": field_label,
                "path": "/fixture/text/field",
                "control_id": field_id,
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
        let (control_id, label, role) = if self.secure_field {
            ("fixture-password-field", "Password", "password text")
        } else {
            ("fixture-search-field", "Search", "text")
        };
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": self.active_window,
            "focused_app": self.active_window,
            "focused_control_id": control_id,
            "focused_control_label": label,
            "focused_control_role": role,
            "keyboard_focus_known": true,
            "text_cursor_known": true,
            "editable_target_known": true,
            "terminal_like": false,
            "focus_confidence": 0.9,
        }))
    }

    async fn capture_screenshot(&self) -> GuiProbeResult {
        // A distinct screen hash per capture so a state-change predicate has
        // observable evidence to confirm against.
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

// ── Fixture executor (backend always succeeds) ───────────────────────────────

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

// ── Helpers ──────────────────────────────────────────────────────────────────

fn fixture_request(message: &str) -> GuiTurnRequest {
    GuiTurnRequest {
        session_id: "contract-session".into(),
        turn_id: "contract-turn".into(),
        workflow_id: "contract-workflow".into(),
        message: message.into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: None,
        execution_environment: GuiExecutionEnvironment::RealSession,
        execution_mode: GuiExecutionMode::ExecuteFixture,
        workflow_enabled: false,
        resume_checkpoint: None,
        resume_reason: None,
    }
}

fn event(outcome: &GuiTurnOutcome, event_type: &str) -> Option<serde_json::Value> {
    outcome
        .events
        .iter()
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some(event_type))
        .cloned()
}

const FOCUS_PROMPT: &str = "Focus the visible search field and verify it is focused";

// ── T2 tests ─────────────────────────────────────────────────────────────────

/// Flag ON: the per-action-type verification CONTRACT is surfaced as additive
/// telemetry on the `ExecutionVerificationCompleted` event (and the
/// `ActionCompleted` event). It carries all four guarantees: the PREDICATE
/// (focused_control for a focus action), the EVIDENCE source (accessibility —
/// never OCR/coordinates), the BOUNDED WAIT (the turn's Task 1 caps, never an
/// unbounded poll), and the CONFIDENCE bar.
#[tokio::test]
async fn t2_verification_contract_surfaced_on_telemetry_when_flag_on() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled());

    let outcome = runtime.run_turn(fixture_request(FOCUS_PROMPT)).await;

    let verification = event(&outcome, VERIFICATION_EVENT)
        .expect("an executed action must emit ExecutionVerificationCompleted");
    let contract = &verification["verification_contract"];
    assert!(
        contract.is_object(),
        "flag ON must attach the verification contract: {verification}"
    );
    // PREDICATE + EVIDENCE for a focus action.
    assert_eq!(contract["action_type"], "FocusField");
    assert_eq!(contract["predicate"], "focused_control");
    assert_eq!(contract["evidence_source"], "accessibility");
    // BOUNDED WAIT: the contract threads the turn's Task 1 caps (never unbounded).
    assert!(
        contract["bounded_wait_ms"].as_u64().is_some_and(|ms| ms > 0),
        "contract must carry a bounded (non-zero, finite) wait: {contract}"
    );
    assert!(
        contract["max_reobserve"].as_u64().is_some_and(|n| n > 0),
        "contract must carry a bounded re-observe cap: {contract}"
    );
    // CONFIDENCE: the explicit bar below which the honest verdict is inconclusive.
    let min_conf = contract["min_confidence"].as_f64().expect("min_confidence");
    assert!((min_conf - VERIFICATION_CONTRACT_MIN_CONFIDENCE).abs() < f64::EPSILON);

    // The contract is also surfaced on the ActionCompleted event for inspection.
    let completed = event(&outcome, "ActionCompleted").expect("ActionCompleted present");
    assert!(
        completed["verification_contract"].is_object(),
        "the contract is surfaced on ActionCompleted too: {completed}"
    );
}

/// Flag OFF (default): the verification path still runs and reports its verdict,
/// but the additive `verification_contract` field is ABSENT from every event —
/// the contract telemetry is additive-only and never alters the OFF path.
#[tokio::test]
async fn t2_no_verification_contract_field_when_flag_off() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    // Default OFF (no with_safety_polish).
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime.run_turn(fixture_request(FOCUS_PROMPT)).await;

    let verification = event(&outcome, VERIFICATION_EVENT)
        .expect("the verification event is still emitted with the flag OFF");
    // The verdict + strategy are present (path unchanged) ...
    assert!(verification["status"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(verification["verification_strategy"]
        .as_str()
        .is_some_and(|s| !s.is_empty()));
    // ... but the additive contract field is absent.
    assert!(
        verification.get("verification_contract").is_none(),
        "flag OFF must not attach the verification contract: {verification}"
    );
    let completed = event(&outcome, "ActionCompleted").expect("ActionCompleted present");
    assert!(
        completed.get("verification_contract").is_none(),
        "flag OFF must not attach the contract to ActionCompleted: {completed}"
    );
}

/// Additive-only guarantee: for the SAME reliable, above-bar fixture evidence
/// the final verdict is identical with the flag ON and OFF — the contract is a
/// no-op for a confident, reliably-evidenced `verified` and never fabricates a
/// downgrade. (The downgrade-to-inconclusive direction is covered by the
/// verifier inline unit tests and the SwitchWindow integration tests.)
#[tokio::test]
async fn t2_contract_leaves_reliable_verdict_unchanged_on_vs_off() {
    let run = |polish: bool| async move {
        let perception = FixtureContextProvider::new("KRIA Fixture App");
        let executor = FixtureExecutor::new();
        let mut runtime = GuiCognitionRuntime::new(&perception, &executor);
        if polish {
            runtime = runtime.with_safety_polish(GuiSafetyPolishConfig::enabled());
        }
        let outcome = runtime.run_turn(fixture_request(FOCUS_PROMPT)).await;
        let verification =
            event(&outcome, VERIFICATION_EVENT).expect("verification event present");
        (
            verification["status"].as_str().unwrap_or_default().to_string(),
            verification["verification_strategy"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        )
    };

    let (off_status, off_strategy) = run(false).await;
    let (on_status, on_strategy) = run(true).await;

    assert_eq!(
        off_status, on_status,
        "reliable evidence ⇒ the verdict is unchanged by the contract (no fabricated downgrade)"
    );
    assert_eq!(off_strategy, on_strategy, "the strategy/predicate is unchanged");
}

/// A secret-payload action's contract uses the NON-REVEALING `state_changed`
/// predicate (never `text_present`, so a secret is never searched for) with
/// observation evidence — and the secret value never appears in the contract or
/// anywhere in the turn's event stream.
#[tokio::test]
async fn t2_secret_payload_contract_uses_state_changed_and_never_leaks_value() {
    let perception = FixtureContextProvider::with_secure_field("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled())
        // Primitives ON forces the secret treatment for a resolved secure field.
        .with_primitives(GuiPrimitivesConfig::enabled());

    let outcome = runtime
        .run_turn(fixture_request(&format!(
            "Type the password \"{SECRET_MARKER}\" into the password field"
        )))
        .await;

    if let Some(verification) = event(&outcome, VERIFICATION_EVENT) {
        if let Some(contract) = verification.get("verification_contract") {
            // A secret typing/fill action must verify by state change, never by
            // searching for the typed text (which would echo the secret).
            assert_eq!(
                contract["predicate"], "state_changed",
                "secret payload must use the non-revealing state_changed predicate: {contract}"
            );
            assert_eq!(contract["evidence_source"], "observation");
            assert_ne!(contract["predicate"], "text_present");
        }
    }

    // The secret value must never appear anywhere in the entire event stream.
    let events_serialized = serde_json::to_string(&outcome.events).unwrap();
    assert!(
        !events_serialized.contains(SECRET_MARKER),
        "the secret value must never appear in any telemetry event: {events_serialized}"
    );
    // Nor anywhere in the turn response surface.
    let response_serialized = serde_json::to_string(&outcome.response).unwrap();
    assert!(
        !response_serialized.contains(SECRET_MARKER),
        "the secret value must never appear in the turn response"
    );
}

/// Task 4 (Issue #10): the per-action-type contract carries the ORDERED
/// `evidence_sources` chain (primary first, then honest fallbacks) as additive
/// telemetry, and the ordered-evidence honesty step does not regress a reliably-
/// evidenced fixture verdict (a11y up + focus known ⇒ still `verified`).
#[tokio::test]
async fn t4_contract_carries_ordered_evidence_sources_and_reliable_verdict_unchanged() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled());

    let outcome = runtime.run_turn(fixture_request(FOCUS_PROMPT)).await;

    let verification = event(&outcome, VERIFICATION_EVENT)
        .expect("an executed action must emit ExecutionVerificationCompleted");
    // Reliable evidence (a11y up + focus known) ⇒ the verdict is verified; the
    // ordered-evidence step never touches a reliable positive.
    assert_eq!(verification["status"], "verified");

    let contract = &verification["verification_contract"];
    let sources = contract["evidence_sources"]
        .as_array()
        .expect("contract carries the ordered evidence_sources chain");
    // Primary first (accessibility for a focus action), then the honest fallback.
    assert_eq!(sources.first().and_then(|v| v.as_str()), Some("accessibility"));
    assert!(
        sources.iter().any(|s| s.as_str() == Some("observation")),
        "a screen-change observation is the honest secondary: {sources:?}"
    );
    // Requirement 10.2: never an OCR/coordinate source in the chain.
    for s in sources {
        let s = s.as_str().unwrap_or_default();
        assert!(
            matches!(
                s,
                "accessibility" | "observation" | "active_window_probe" | "process"
                    | "backend_receipt" | "none"
            ),
            "evidence source taxonomy excludes OCR/coordinates: {s}"
        );
    }
}
