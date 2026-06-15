//! Task 9.2 (Requirements 10, 11, 12, 13, 14, 15, 22, 23) T2 — CI-safe
//! deterministic fixture-tier integration tests for the append-only, sanitized
//! audit ledger of EXECUTED GUI actions.
//!
//! These exercise the real `execute_authorized_proposal` path in-process (no
//! display, no network, no `KRIA_*` env var) through `run_turn`, asserting:
//!   * flag ON  → an executed action is recorded in the ledger, a
//!     `GuiActionLedgerEntryRecorded` event is emitted, and the ledger is
//!     inspectable in the turn response (entries preserved + ordered).
//!   * flag OFF → no ledger entries and no ledger event (events unchanged).
//!   * entries are sanitized: no raw prompt, no coordinates, no secret/clipboard
//!     value — only the redacted descriptor + hashes produced upstream.
//!   * a secret-payload action records only the redacted summary (the flag),
//!     never the value.

use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};

use kria_core::agent::gui_cognition::execution_environment::GuiExecutionEnvironment;
use kria_core::agent::gui_cognition::executor::{
    GuiActionBackendStatus, GuiActionExecution, GuiActionExecutor, GuiActionRequest,
    GuiExecutionMode, GuiPrimitivesConfig,
};
use kria_core::agent::gui_cognition::perception::{GuiPerceptionProvider, GuiProbeResult};
use kria_core::agent::gui_cognition::verifier::GuiSafetyPolishConfig;
use kria_core::agent::gui_cognition::{GuiCognitionRuntime, GuiTurnOutcome, GuiTurnRequest};

const RAW_PROMPT_MARKER: &str = "ZZZRAWPROMPTMARKER";
const SECRET_MARKER: &str = "ZZZSECRETVALUE9999";
const LEDGER_EVENT: &str = "GuiActionLedgerEntryRecorded";

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

fn fixture_request(message: &str, workflow_enabled: bool) -> GuiTurnRequest {
    GuiTurnRequest {
        session_id: "ledger-session".into(),
        turn_id: "ledger-turn".into(),
        workflow_id: "ledger-workflow".into(),
        message: message.into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: None,
        execution_environment: GuiExecutionEnvironment::RealSession,
        execution_mode: GuiExecutionMode::ExecuteFixture,
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
        .map(str::to_string)
        .collect()
}

fn ledger_events(outcome: &GuiTurnOutcome) -> Vec<&serde_json::Value> {
    outcome
        .events
        .iter()
        .filter(|event| {
            event.get("type").and_then(serde_json::Value::as_str) == Some(LEDGER_EVENT)
        })
        .collect()
}

fn ledger_value(outcome: &GuiTurnOutcome) -> serde_json::Value {
    outcome.response["gui_cognition"]["ledger"].clone()
}

// ── T2 tests ─────────────────────────────────────────────────────────────────

/// Flag ON: an executed single action is recorded in the ledger, a
/// `GuiActionLedgerEntryRecorded` event is emitted, and the ledger is
/// inspectable in the turn response.
#[tokio::test]
async fn t2_ledger_records_executed_action_and_emits_event_when_flag_on() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled());

    let outcome = runtime
        .run_turn(fixture_request(
            "Focus the visible search field and verify it is focused",
            false,
        ))
        .await;

    let types = event_types(&outcome);
    assert!(
        types.contains(&"ActionCompleted".to_string()),
        "fixture action should execute, events: {types:?}"
    );
    let recorded = ledger_events(&outcome);
    assert_eq!(recorded.len(), 1, "one executed action → one ledger event");
    let entry = &recorded[0]["entry"];
    assert_eq!(entry["sequence"], 0);
    assert!(entry["execution_id"].as_str().is_some_and(|v| !v.is_empty()));
    assert!(entry["proposal_hash"].as_str().is_some_and(|v| !v.is_empty()));
    assert!(entry["prompt_hash"].as_str().is_some_and(|v| !v.is_empty()));
    assert!(entry["verification_verdict"].as_str().is_some_and(|v| !v.is_empty()));
    assert_eq!(entry["authorization_source"], "safe_no_approval_required");
    assert!(entry["entry_hash"].as_str().is_some_and(|v| !v.is_empty()));

    // Inspectable read API surfaced in the response.
    let ledger = ledger_value(&outcome);
    assert_eq!(ledger["entry_count"], 1);
    assert_eq!(ledger["entries"].as_array().map(Vec::len), Some(1));
    assert!(ledger["head_hash"].as_str().is_some_and(|v| !v.is_empty()));
}

/// Flag OFF: no ledger entries and no ledger event — events/response unchanged.
#[tokio::test]
async fn t2_ledger_empty_and_no_event_when_flag_off() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    // Default OFF (no with_safety_polish).
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime
        .run_turn(fixture_request(
            "Focus the visible search field and verify it is focused",
            false,
        ))
        .await;

    let types = event_types(&outcome);
    assert!(
        types.contains(&"ActionCompleted".to_string()),
        "fixture action should still execute with the flag OFF, events: {types:?}"
    );
    assert!(
        ledger_events(&outcome).is_empty(),
        "no ledger event must be emitted while the flag is OFF, events: {types:?}"
    );
    // The ledger key is absent (null) from the response while the flag is OFF.
    assert!(
        ledger_value(&outcome).is_null(),
        "ledger must be absent from the response while the flag is OFF"
    );
}

/// Append-only + ordered across a multi-step combo: every executed step adds an
/// entry, sequences are contiguous from 0, and the chain head links the last
/// entry.
#[tokio::test]
async fn t2_ledger_is_append_only_and_ordered_across_combo() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled());

    let outcome = runtime
        .run_turn(fixture_request(
            "Open KRIA Fixture App and focus the visible search field",
            true,
        ))
        .await;

    let recorded = ledger_events(&outcome);
    assert!(
        !recorded.is_empty(),
        "a multi-step combo must record at least one executed action"
    );

    // The response ledger is the authoritative, ordered, append-only record.
    let ledger = ledger_value(&outcome);
    let entries = ledger["entries"].as_array().expect("entries array");
    assert_eq!(
        entries.len() as u64,
        ledger["entry_count"].as_u64().unwrap(),
        "entry_count matches the entries array"
    );
    // Sequences are contiguous from 0 in append order (never reordered/mutated).
    for (index, entry) in entries.iter().enumerate() {
        assert_eq!(entry["sequence"].as_u64(), Some(index as u64));
    }
    // The per-action events recorded entries in the same growing order.
    for (index, event) in recorded.iter().enumerate() {
        assert_eq!(event["entry"]["sequence"].as_u64(), Some(index as u64));
        assert_eq!(event["entry_count"].as_u64(), Some((index + 1) as u64));
    }
    // The last recorded event's head_hash equals the final ledger head.
    assert_eq!(
        recorded.last().unwrap()["head_hash"],
        ledger["head_hash"],
    );
}

/// Sanitized: the serialized ledger carries no raw prompt and no coordinates.
#[tokio::test]
async fn t2_ledger_entry_carries_no_raw_prompt_or_coordinates() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled());

    let outcome = runtime
        .run_turn(fixture_request(
            &format!("{RAW_PROMPT_MARKER} focus the visible search field and verify it is focused"),
            false,
        ))
        .await;

    let ledger = ledger_value(&outcome);
    assert!(!ledger.is_null(), "ledger present when the flag is ON");
    let serialized = serde_json::to_string(&ledger).unwrap();
    assert!(
        !serialized.contains(RAW_PROMPT_MARKER),
        "ledger must not carry the raw prompt: {serialized}"
    );
    for coord_key in ["\"x\"", "\"y\"", "\"width\"", "\"height\"", "\"bounds\""] {
        assert!(
            !serialized.contains(coord_key),
            "ledger must not carry coordinates ({coord_key}): {serialized}"
        );
    }
}

/// A secret-payload action records only the redacted summary — the ledger never
/// contains the secret value, regardless of the executed action.
#[tokio::test]
async fn t2_secret_field_action_records_only_redacted_summary() {
    let perception = FixtureContextProvider::with_secure_field("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_safety_polish(GuiSafetyPolishConfig::enabled())
        // Primitives ON forces the secret treatment for a resolved secure field.
        .with_primitives(GuiPrimitivesConfig::enabled());

    let outcome = runtime
        .run_turn(fixture_request(
            &format!("Type the password \"{SECRET_MARKER}\" into the password field"),
            false,
        ))
        .await;

    // The secret value must never appear anywhere in the ledger (entries or the
    // emitted ledger events).
    let ledger_serialized = serde_json::to_string(&ledger_value(&outcome)).unwrap();
    assert!(
        !ledger_serialized.contains(SECRET_MARKER),
        "ledger must never contain the secret value: {ledger_serialized}"
    );
    for event in ledger_events(&outcome) {
        let serialized = serde_json::to_string(event).unwrap();
        assert!(
            !serialized.contains(SECRET_MARKER),
            "ledger event must never contain the secret value: {serialized}"
        );
        // When a secret-field action is recorded, the flag (not the value)
        // captures the secrecy.
        if event["entry"]["action_type"]
            .as_str()
            .map(|a| a.eq_ignore_ascii_case("TypeText") || a.eq_ignore_ascii_case("FillField"))
            .unwrap_or(false)
        {
            assert_eq!(
                event["entry"]["is_secret_payload"], true,
                "a secret payload action must mark is_secret_payload"
            );
        }
    }
}
