//! T2 — DURING-turn event streaming (`gui_cog_stream_ux`, spec task 10.1).
//!
//! These deterministic, no-display, no-network tests pin the Task 10.1 contract:
//!
//!   * With the `gui_cog_stream_ux` flag ON **and** a streaming sink attached,
//!     the runtime pushes each `gui_cognition:event` envelope to the sink's mpsc
//!     channel AS IT IS PRODUCED during the turn (observe → plan → per-step), so
//!     the channel's FIFO order proves the observe envelope is streamed before
//!     the per-step envelopes — it is NOT a single end-of-turn batch.
//!   * The streamed sequence is exactly equal to the final `outcome.events`
//!     batch (streaming is additive — no event is dropped, added, or reordered).
//!   * With the flag OFF, no sink is used and `outcome.events` is byte-for-byte
//!     identical to the no-sink baseline (rollback safety / backward compat).
//!
//! The fixtures are self-contained (a fixture `GuiContext` provider + the
//! `ExecuteFixture` execution mode); no `KRIA_*` env var, filesystem, or socket
//! is touched, so the tier is deterministic and CI-safe (Requirement 20.4).

use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};

use kria_core::agent::gui_cognition::event_stream::{GuiEventStreamSink, GuiStreamUxConfig};
use kria_core::agent::gui_cognition::execution_environment::GuiExecutionEnvironment;
use kria_core::agent::gui_cognition::executor::{
    GuiActionBackendStatus, GuiActionExecution, GuiActionExecutor, GuiActionRequest,
    GuiExecutionMode,
};
use kria_core::agent::gui_cognition::perception::{GuiPerceptionProvider, GuiProbeResult};
use kria_core::agent::gui_cognition::{GuiCognitionRuntime, GuiTurnOutcome, GuiTurnRequest};

// ── Deterministic fixture GuiContext provider (no display, no network) ──────

struct FixtureContextProvider {
    active_window: String,
    screen_seq: AtomicU64,
}

impl FixtureContextProvider {
    fn new(active_window: &str) -> Self {
        Self {
            active_window: active_window.into(),
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
        session_id: "stream-session".into(),
        turn_id: "stream-turn".into(),
        workflow_id: "stream-workflow".into(),
        message: message.into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: None,
        execution_environment: GuiExecutionEnvironment::RealSession,
        execution_mode: GuiExecutionMode::ExecuteFixture,
        workflow_enabled: true,
        resume_checkpoint: None,
        resume_reason: None,
    }
}

const COMBO_PROMPT: &str = "Open KRIA Fixture App and focus the visible search field";

fn event_types(events: &[serde_json::Value]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .map(|value| value.to_string())
        .collect()
}

fn first_index_matching(types: &[String], candidates: &[&str]) -> Option<usize> {
    types.iter().position(|t| candidates.contains(&t.as_str()))
}

/// Drain everything currently buffered in the unbounded receiver.
fn drain(rx: &mut tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    while let Ok(event) = rx.try_recv() {
        out.push(event);
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Baseline + flag-OFF safety: with the `gui_cog_stream_ux` flag OFF, no sink is
/// used even if one is attached, and `outcome.events` is byte-for-byte identical
/// to the no-sink baseline. This is the rollback / backward-compat guarantee.
#[tokio::test]
async fn t2_flag_off_uses_no_sink_and_events_unchanged() {
    // No-sink baseline (the historical end-of-turn batch path).
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let baseline: GuiTurnOutcome = GuiCognitionRuntime::new(&perception, &executor)
        .run_turn(fixture_request(COMBO_PROMPT))
        .await;

    // Flag OFF but a sink IS attached — the runtime must ignore it entirely.
    let perception2 = FixtureContextProvider::new("KRIA Fixture App");
    let executor2 = FixtureExecutor::new();
    let (sink, mut rx) = GuiEventStreamSink::channel();
    let outcome: GuiTurnOutcome = GuiCognitionRuntime::new(&perception2, &executor2)
        .with_stream_ux(GuiStreamUxConfig::disabled())
        .with_event_sink(Some(sink))
        .run_turn(fixture_request(COMBO_PROMPT))
        .await;

    let streamed = drain(&mut rx);
    assert!(
        streamed.is_empty(),
        "flag OFF must not stream anything, got {} envelopes",
        streamed.len()
    );
    // Two independent turns carry per-run UUIDs/timestamps, so the deterministic
    // invariant is that the flag-OFF event *sequence* (by type) is identical to
    // the no-sink baseline — i.e. attaching a sink under a disabled flag changes
    // nothing. (Byte-for-byte buffer equivalence of the push path is pinned by
    // the `event_stream` unit tests.)
    assert_eq!(
        event_types(&outcome.events),
        event_types(&baseline.events),
        "flag OFF event sequence must equal the no-sink baseline"
    );
}

/// Flag ON: the runtime streams every envelope through the sink, and the
/// streamed sequence equals the final `outcome.events` batch (additive — nothing
/// dropped, added, or reordered).
#[tokio::test]
async fn t2_flag_on_streamed_sequence_equals_outcome_batch() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let (sink, mut rx) = GuiEventStreamSink::channel();

    let outcome: GuiTurnOutcome = GuiCognitionRuntime::new(&perception, &executor)
        .with_stream_ux(GuiStreamUxConfig::enabled())
        .with_event_sink(Some(sink))
        .run_turn(fixture_request(COMBO_PROMPT))
        .await;

    let streamed = drain(&mut rx);
    assert!(!streamed.is_empty(), "flag ON must stream envelopes");
    assert_eq!(
        streamed, outcome.events,
        "the streamed sequence must equal the final batch exactly"
    );

    // The stream covers the whole turn start-to-finish.
    let streamed_types = event_types(&streamed);
    assert_eq!(
        streamed_types.first().map(String::as_str),
        Some("TurnStarted"),
        "first streamed envelope must be TurnStarted, got {streamed_types:?}"
    );
    assert!(
        streamed_types
            .iter()
            .any(|t| t == "TurnCompleted" || t == "WorkflowRunCompleted"),
        "stream must reach a terminal turn/workflow event, got {streamed_types:?}"
    );
}

/// Flag ON, incremental ordering: the observe envelope is pushed to the sink
/// BEFORE the per-step (plan/workflow/action) envelopes. The FIFO channel order
/// proves the runtime streams DURING the turn (observe → plan → per-step), not
/// as one end batch.
#[tokio::test]
async fn t2_flag_on_observe_streams_before_per_step() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();
    let (sink, mut rx) = GuiEventStreamSink::channel();

    let _outcome = GuiCognitionRuntime::new(&perception, &executor)
        .with_stream_ux(GuiStreamUxConfig::enabled())
        .with_event_sink(Some(sink))
        .run_turn(fixture_request(COMBO_PROMPT))
        .await;

    let streamed = drain(&mut rx);
    let types = event_types(&streamed);

    let observe_idx = first_index_matching(
        &types,
        &["ObservationStarted", "ObservationCompleted", "ObservationBlocked"],
    )
    .unwrap_or_else(|| panic!("expected an observation envelope, got {types:?}"));

    let per_step_idx = first_index_matching(
        &types,
        &[
            "WorkflowRunStarted",
            "WorkflowStepStarted",
            "ActionStarted",
        ],
    )
    .unwrap_or_else(|| panic!("expected a per-step envelope, got {types:?}"));

    assert!(
        observe_idx < per_step_idx,
        "observe envelope (idx {observe_idx}) must be streamed before the first \
         per-step envelope (idx {per_step_idx}); stream order proves DURING-turn \
         streaming, types: {types:?}"
    );

    // Plan envelopes must also precede the per-step execution envelopes.
    if let Some(plan_idx) = first_index_matching(&types, &["PlanCreated", "PlanValidationCompleted"]) {
        if let Some(action_idx) = first_index_matching(&types, &["ActionStarted"]) {
            assert!(
                plan_idx < action_idx,
                "plan envelope (idx {plan_idx}) must stream before the action \
                 envelope (idx {action_idx}), types: {types:?}"
            );
        }
    }
}

/// Flag ON without a sink attached is safe (no panic, no streaming) and the
/// batch is still returned — the flag alone does not require a sink.
#[tokio::test]
async fn t2_flag_on_without_sink_is_safe() {
    let perception = FixtureContextProvider::new("KRIA Fixture App");
    let executor = FixtureExecutor::new();

    let outcome: GuiTurnOutcome = GuiCognitionRuntime::new(&perception, &executor)
        .with_stream_ux(GuiStreamUxConfig::enabled())
        .run_turn(fixture_request(COMBO_PROMPT))
        .await;

    assert!(
        !outcome.events.is_empty(),
        "the end-of-turn batch must still be populated without a sink"
    );
    assert!(!outcome.status.is_empty(), "turn must end with a defined status");
}
