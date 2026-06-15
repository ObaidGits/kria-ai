//! Task 6.3 (Requirements 5, 15) — Primitive GREEN/YELLOW tier classification +
//! tier↔idempotent consistency + additive tier telemetry.
//!
//! CI-safe T1 tests: no live KRIA desktop API, no display, no network. The pure
//! classification/consistency checks run against the public `primitive_tier`
//! classifier and `default_idempotent_for`; the telemetry checks run the full
//! in-process single-proposal pipeline through `run_turn` with a deterministic
//! fixture and assert the additive `primitive_tiers` field on the `PlanCreated`
//! event is present ONLY when the `gui_cog_primitives` flag is ON.

use async_trait::async_trait;

use kria_core::agent::gui_cognition::execution_environment::GuiExecutionEnvironment;
use kria_core::agent::gui_cognition::executor::{
    primitive_tier, GuiActionBackendStatus, GuiActionExecution, GuiActionExecutor,
    GuiActionRequest, GuiExecutionMode, GuiPrimitiveTier, GuiPrimitivesConfig,
};
use kria_core::agent::gui_cognition::perception::{GuiPerceptionProvider, GuiProbeResult};
use kria_core::agent::gui_cognition::{
    default_idempotent_for, GuiCognitionRuntime, GuiTurnOutcome, GuiTurnRequest,
};

// ── Pure classification / consistency (no pipeline) ──────────────────────────

const GREEN_PRIMITIVES: &[&str] = &[
    "Observe",
    "FocusField",
    "Scroll",
    "SelectAll",
    "InAppSearch",
    "SummarizeVisibleContent",
    "WaitForState",
    "VerifyState",
    "AskClarification",
    "SwitchWindow",
];

const YELLOW_PRIMITIVES: &[&str] = &[
    "TypeText",
    "ClearField",
    "Paste",
    "ClickControl",
    "SetCheckbox",
    "PressKey",
    "Copy",
    "CloseDialog",
    "OpenApp",
    "BrowserNavigate",
];

#[test]
fn every_supported_primitive_has_a_tier() {
    for st in GREEN_PRIMITIVES.iter().chain(YELLOW_PRIMITIVES) {
        assert!(primitive_tier(st).is_some(), "{st} must have a tier");
    }
}

#[test]
fn green_are_read_only_and_yellow_are_state_changing() {
    for st in GREEN_PRIMITIVES {
        assert_eq!(primitive_tier(st), Some(GuiPrimitiveTier::Green), "{st}");
    }
    for st in YELLOW_PRIMITIVES {
        assert_eq!(primitive_tier(st), Some(GuiPrimitiveTier::Yellow), "{st}");
    }
}

#[test]
fn tier_and_idempotent_are_consistent() {
    // GREEN ⇒ idempotent (read-only / converges, never an extra side effect).
    for st in GREEN_PRIMITIVES {
        assert!(default_idempotent_for(st), "GREEN {st} must be idempotent");
    }
    // Non-idempotent ⇒ YELLOW (a non-converging mutation can never be GREEN).
    for st in GREEN_PRIMITIVES.iter().chain(YELLOW_PRIMITIVES) {
        if !default_idempotent_for(st) {
            assert_eq!(
                primitive_tier(st),
                Some(GuiPrimitiveTier::Yellow),
                "non-idempotent {st} must be YELLOW"
            );
        }
    }
}

#[test]
fn approval_gated_steps_stay_out_of_the_primitive_band() {
    for st in ["RequireApproval", "Save", "Download", "Unknown"] {
        assert!(primitive_tier(st).is_none(), "{st} must not be GREEN/YELLOW");
    }
}

// ── Deterministic fixture: a desktop with one focusable search field ─────────

struct SearchFieldPerception;

#[async_trait]
impl GuiPerceptionProvider for SearchFieldPerception {
    async fn get_active_window(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "title": "Fixture Window",
            "app_name": "Fixture Window",
        }))
    }

    async fn get_desktop_state(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": "Fixture Window",
            "accessibility_operational": true,
            "applications": ["Fixture Window"],
        }))
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": true }))
    }

    async fn find_ui_elements(&self, role: &str) -> GuiProbeResult {
        let elements = if role == "text" {
            vec![serde_json::json!({
                "role": "text",
                "name": "Search",
                "label": "Search",
                "path": "/fixture/search/Search",
                "control_id": "fixture-search-field",
                "enabled": true,
                "visible": true,
                "focused": false,
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
            "focused_window": "Fixture Window",
            "focused_app": "Fixture Window",
            "keyboard_focus_known": true,
            "text_cursor_known": true,
            "editable_target_known": true,
            "terminal_like": false,
            "focus_confidence": 0.9,
        }))
    }

    async fn capture_screenshot(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "screen_hash": "fixture-screen",
            "byte_count": 16,
            "source": "fixture",
        }))
    }

    async fn focused_window_title(&self) -> Option<String> {
        Some("Fixture Window".into())
    }
}

struct OkExecutor {
    backend: GuiActionBackendStatus,
}

impl OkExecutor {
    fn new() -> Self {
        Self {
            backend: GuiActionBackendStatus::available("fixture_executor"),
        }
    }
}

#[async_trait]
impl GuiActionExecutor for OkExecutor {
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

fn focus_request() -> GuiTurnRequest {
    GuiTurnRequest {
        session_id: "tier-session".into(),
        turn_id: "tier-turn".into(),
        workflow_id: "tier-workflow".into(),
        message: "Focus the visible Search field and verify it is focused".into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: None,
        execution_environment: GuiExecutionEnvironment::RealSession,
        execution_mode: GuiExecutionMode::SafetyOnly,
        workflow_enabled: false,
        resume_checkpoint: None,
        resume_reason: None,
    }
}

fn plan_created<'a>(outcome: &'a GuiTurnOutcome) -> &'a serde_json::Value {
    outcome
        .events
        .iter()
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some("PlanCreated"))
        .expect("a PlanCreated event is emitted")
}

// ── Telemetry: flag ON adds `primitive_tiers`; flag OFF does not ─────────────

#[tokio::test]
async fn flag_on_surfaces_primitive_tiers_on_plan_created() {
    let perception = SearchFieldPerception;
    let executor = OkExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_primitives(GuiPrimitivesConfig::enabled());

    let outcome = runtime.run_turn(focus_request()).await;
    let plan = plan_created(&outcome);

    let tiers = plan
        .get("primitive_tiers")
        .and_then(serde_json::Value::as_array)
        .expect("flag ON must add the additive primitive_tiers field");
    assert!(!tiers.is_empty(), "plan should classify at least one primitive");

    // Every annotated entry carries a GREEN/YELLOW tier + its idempotent flag,
    // and the tier is consistent with the step's idempotent classification.
    for entry in tiers {
        let tier = entry["tier"].as_str().expect("tier token");
        assert!(matches!(tier, "GREEN" | "YELLOW"), "unexpected tier: {tier}");
        let idempotent = entry["idempotent"].as_bool().expect("idempotent flag");
        if tier == "GREEN" {
            assert!(idempotent, "GREEN entry must be idempotent: {entry}");
        }
    }
}

#[tokio::test]
async fn flag_off_does_not_add_primitive_tiers() {
    let perception = SearchFieldPerception;
    let executor = OkExecutor::new();
    // No `.with_primitives(...)` → `gui_cog_primitives` stays OFF (default).
    let runtime = GuiCognitionRuntime::new(&perception, &executor);

    let outcome = runtime.run_turn(focus_request()).await;
    let plan = plan_created(&outcome);

    assert!(
        plan.get("primitive_tiers").is_none(),
        "flag OFF must NOT add the primitive_tiers field: {plan}"
    );
}
