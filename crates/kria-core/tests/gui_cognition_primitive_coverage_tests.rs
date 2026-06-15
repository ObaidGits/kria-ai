//! Task 6.4 (Requirements 5, 15) — comprehensive **per-primitive** T1/T2
//! coverage for the visible single-action primitive band.
//!
//! CI-safe: no live KRIA desktop API, no display, no network. Two tiers per
//! primitive (focus / type / clear / select-all / copy / paste / key-press /
//! scroll / click / checkbox / dialog-close / in-app-search):
//!
//!   * **T1 (unit, no pipeline):** with the `gui_cog_primitives` flag ON the
//!     primitive's action verb resolves to the correct
//!     [`GuiActionKind`](kria_core::agent::gui_cognition::executor::GuiActionKind)
//!     via `resolve_action_kind`; the executor-level `verification_strategy`
//!     ([`select_verification_strategy`]) and the GREEN/YELLOW
//!     [`primitive_tier`] are the expected ones; the deterministic planner's
//!     per-step contract is complete (a non-empty, type-valid
//!     `verification_strategy` via `default_verification_strategy_for_step`) and
//!     the tier↔idempotent invariant (`GREEN ⇒ idempotent`) holds. Flag-OFF
//!     mapping is asserted byte-for-byte against the legacy
//!     `from_action_type`.
//!
//!   * **T2 (deterministic fixture pipeline, no display):** `run_turn` is driven
//!     with a fixture perception + a *recording* fixture executor (mirroring the
//!     `gui_cognition_t2_fixture_tier.rs` / `gui_cognition_primitive_tier_tests.rs`
//!     style) with the `gui_cog_primitives` flag ON. For each primitive prompt:
//!       - the PLAN → VALIDATE pipeline reaches a **non-blocking** terminal
//!         readiness status (never `blocked`/`rejected`) and the plan never
//!         claims it can execute at the validate stage (KRIA authority); and
//!       - where the primitive resolves against the fixture, the action that
//!         routes **through the executor** carries the **correct
//!         `GuiActionKind`** for that primitive (i.e. the flag-ON typed mapping,
//!         not the legacy `ClickControl` catch-all), and the turn is not wrongly
//!         blocked.
//!
//! KRIA authority invariants asserted throughout: a plan never auto-executes at
//! the plan/validate stage (`can_execute == false`); no action starts in a
//! non-executing mode (no Prompt→Tool shortcut); every turn terminates with a
//! defined status (boundedness).

use async_trait::async_trait;
use std::sync::{Arc, Mutex};

use kria_core::agent::gui_cognition::execution_environment::GuiExecutionEnvironment;
use kria_core::agent::gui_cognition::executor::{
    primitive_tier, GuiActionBackendStatus, GuiActionExecution, GuiActionExecutor,
    GuiActionKind, GuiActionRequest, GuiExecutionMode, GuiPrimitiveTier, GuiPrimitivesConfig,
};
use kria_core::agent::gui_cognition::llm_planner::{
    default_verification_strategy_for_step,
};
use kria_core::agent::gui_cognition::perception::{GuiPerceptionProvider, GuiProbeResult};
use kria_core::agent::gui_cognition::safety_hitl::GuiHitlDecisionFixture;
use kria_core::agent::gui_cognition::verifier::{
    select_verification_strategy, GuiVerificationStrategy,
};
use kria_core::agent::gui_cognition::{
    default_idempotent_for, GuiCognitionRuntime, GuiTurnOutcome, GuiTurnRequest,
};

// ─────────────────────────────────────────────────────────────────────────────
// The per-primitive coverage table — the single source of truth for both tiers.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct PrimitiveSpec {
    /// Human-readable primitive name (for assertion messages).
    name: &'static str,
    /// Action-type verb aliases that must resolve to `kind` when the flag is ON.
    verbs: &'static [&'static str],
    /// The typed executor action kind this primitive must resolve to (flag ON).
    kind: GuiActionKind,
    /// True when this primitive is a NEW Task 6.1 typed primitive (i.e. flag OFF
    /// must map every `verbs` entry to the legacy `ClickControl` catch-all).
    new_primitive: bool,
    /// The planner step-type token (tier + idempotent + plan-step verification).
    step_type: &'static str,
    /// The executor-level verification strategy `select_verification_strategy`
    /// must pick for this kind on a NON-secret payload.
    exec_strategy: GuiVerificationStrategy,
    /// The GREEN/YELLOW primitive tier.
    tier: GuiPrimitiveTier,
    /// A natural-language prompt that drives this primitive through `run_turn`.
    prompt: &'static str,
}

const PRIMITIVES: &[PrimitiveSpec] = &[
    PrimitiveSpec {
        name: "focus",
        verbs: &["focus_field", "focusfield", "focus_input", "focusinput"],
        kind: GuiActionKind::FocusField,
        new_primitive: false,
        step_type: "FocusField",
        exec_strategy: GuiVerificationStrategy::FocusedControl,
        tier: GuiPrimitiveTier::Green,
        prompt: "Focus the visible Search field and verify it is focused",
    },
    PrimitiveSpec {
        name: "type",
        verbs: &["type_text", "typetext"],
        kind: GuiActionKind::TypeText,
        new_primitive: false,
        step_type: "TypeText",
        exec_strategy: GuiVerificationStrategy::TextPresent,
        tier: GuiPrimitiveTier::Yellow,
        prompt: "Type \"KRIA coverage tier\" into the visible Search field and verify the text is entered",
    },
    PrimitiveSpec {
        name: "clear",
        verbs: &["clear_field", "clearfield", "clear", "clear_text", "cleartext"],
        kind: GuiActionKind::ClearField,
        new_primitive: true,
        step_type: "ClearField",
        exec_strategy: GuiVerificationStrategy::StateChanged,
        tier: GuiPrimitiveTier::Yellow,
        prompt: "Clear the visible Search field",
    },
    PrimitiveSpec {
        name: "select-all",
        verbs: &["select_all", "selectall", "select-all", "select"],
        kind: GuiActionKind::SelectAll,
        new_primitive: true,
        step_type: "SelectAll",
        exec_strategy: GuiVerificationStrategy::StateChanged,
        tier: GuiPrimitiveTier::Green,
        prompt: "Select all the text in the visible Search field",
    },
    PrimitiveSpec {
        name: "copy",
        verbs: &["copy", "copy_content"],
        kind: GuiActionKind::Copy,
        new_primitive: false,
        step_type: "Copy",
        exec_strategy: GuiVerificationStrategy::ClipboardChanged,
        tier: GuiPrimitiveTier::Yellow,
        prompt: "Copy the selected text",
    },
    PrimitiveSpec {
        name: "paste",
        verbs: &["paste", "paste_content"],
        kind: GuiActionKind::Paste,
        new_primitive: false,
        step_type: "Paste",
        exec_strategy: GuiVerificationStrategy::TextPresent,
        tier: GuiPrimitiveTier::Yellow,
        prompt: "Paste the clipboard contents into the visible Search field",
    },
    PrimitiveSpec {
        name: "key-press",
        verbs: &["press_key", "presskey"],
        kind: GuiActionKind::PressKey,
        new_primitive: false,
        step_type: "PressKey",
        exec_strategy: GuiVerificationStrategy::ScreenChanged,
        tier: GuiPrimitiveTier::Yellow,
        prompt: "Press the Enter key",
    },
    PrimitiveSpec {
        name: "scroll",
        verbs: &["scroll"],
        kind: GuiActionKind::Scroll,
        new_primitive: false,
        step_type: "Scroll",
        exec_strategy: GuiVerificationStrategy::ScreenChanged,
        tier: GuiPrimitiveTier::Green,
        prompt: "Scroll down the page",
    },
    PrimitiveSpec {
        name: "click",
        verbs: &["click_control", "clickcontrol"],
        kind: GuiActionKind::ClickControl,
        new_primitive: false,
        step_type: "ClickControl",
        exec_strategy: GuiVerificationStrategy::ResultVisible,
        tier: GuiPrimitiveTier::Yellow,
        prompt: "Click the visible safe button named Search and verify the screen changed",
    },
    PrimitiveSpec {
        name: "checkbox",
        verbs: &["set_checkbox", "setcheckbox", "checkbox", "check", "uncheck", "toggle_checkbox"],
        kind: GuiActionKind::SetCheckbox,
        new_primitive: true,
        step_type: "SetCheckbox",
        exec_strategy: GuiVerificationStrategy::ResultVisible,
        tier: GuiPrimitiveTier::Yellow,
        prompt: "Check the \"Remember me\" checkbox",
    },
    PrimitiveSpec {
        name: "dialog-close",
        verbs: &["close_dialog", "closedialog", "dialog_close", "dismiss_dialog", "dismiss"],
        kind: GuiActionKind::CloseDialog,
        new_primitive: true,
        step_type: "CloseDialog",
        exec_strategy: GuiVerificationStrategy::DialogVisible,
        tier: GuiPrimitiveTier::Yellow,
        prompt: "Close the active dialog",
    },
    PrimitiveSpec {
        name: "in-app-search",
        verbs: &["in_app_search", "inappsearch", "in-app-search", "app_search"],
        kind: GuiActionKind::InAppSearch,
        new_primitive: true,
        step_type: "InAppSearch",
        exec_strategy: GuiVerificationStrategy::ResultVisible,
        tier: GuiPrimitiveTier::Green,
        prompt: "Search for \"quarterly report\" in the file manager",
    },
];

// ─────────────────────────────────────────────────────────────────────────────
// T1 — per-primitive unit coverage (no pipeline, no display).
// ─────────────────────────────────────────────────────────────────────────────

/// Flag ON: every action verb for the primitive resolves to its correct typed
/// executor [`GuiActionKind`] (the new typed primitives no longer fall back to
/// the legacy `ClickControl` catch-all).
#[test]
fn t1_flag_on_resolves_each_primitive_verb_to_its_typed_kind() {
    let on = GuiPrimitivesConfig::enabled();
    for spec in PRIMITIVES {
        for verb in spec.verbs {
            assert_eq!(
                on.resolve_action_kind(verb),
                spec.kind.clone(),
                "[{}] flag ON: verb {verb:?} must resolve to {:?}",
                spec.name,
                spec.kind
            );
        }
    }
}

/// Flag OFF: the path is byte-for-byte the legacy mapping. New typed primitives
/// (clear/select-all/checkbox/dialog-close/in-app-search) collapse to the legacy
/// `ClickControl` catch-all; recognized legacy verbs resolve identically across
/// the flag.
#[test]
fn t1_flag_off_preserves_legacy_mapping_for_each_primitive_verb() {
    let off = GuiPrimitivesConfig::disabled();
    for spec in PRIMITIVES {
        for verb in spec.verbs {
            // OFF always equals the legacy classifier (the definition of "flag
            // OFF is unchanged").
            assert_eq!(
                off.resolve_action_kind(verb),
                GuiActionKind::from_action_type(verb),
                "[{}] flag OFF for {verb:?} must equal legacy from_action_type",
                spec.name
            );
            if spec.new_primitive {
                // A Task 6.1 primitive verb is unknown to the legacy mapping →
                // ClickControl catch-all while OFF.
                assert_eq!(
                    off.resolve_action_kind(verb),
                    GuiActionKind::ClickControl,
                    "[{}] new primitive verb {verb:?} must be legacy ClickControl while OFF",
                    spec.name
                );
            } else {
                // A recognized legacy verb resolves to the SAME kind across the
                // flag (stability).
                assert_eq!(
                    off.resolve_action_kind(verb),
                    spec.kind.clone(),
                    "[{}] legacy verb {verb:?} must be stable across the flag",
                    spec.name
                );
            }
        }
    }
}

/// The executor-level verification strategy for each primitive's kind (on a
/// non-secret payload) is the expected one — the verification contract per
/// action type (Requirement 23 / 5).
#[test]
fn t1_executor_verification_strategy_per_primitive() {
    for spec in PRIMITIVES {
        assert_eq!(
            select_verification_strategy(&spec.kind, false),
            spec.exec_strategy,
            "[{}] non-secret verification strategy mismatch",
            spec.name
        );
    }
}

/// The GREEN/YELLOW tier is consistent across BOTH classifiers — the executor
/// kind classifier (`GuiActionKind::primitive_tier`) and the step-type
/// classifier (`primitive_tier(step_type)`).
#[test]
fn t1_tier_per_primitive_is_consistent_across_classifiers() {
    for spec in PRIMITIVES {
        assert_eq!(
            spec.kind.primitive_tier(),
            spec.tier,
            "[{}] kind tier mismatch",
            spec.name
        );
        assert_eq!(
            primitive_tier(spec.step_type),
            Some(spec.tier),
            "[{}] step-type tier mismatch",
            spec.name
        );
    }
}

/// Tier↔idempotent invariant per primitive: `GREEN ⇒ idempotent`, and a
/// non-idempotent primitive can never be GREEN.
#[test]
fn t1_tier_idempotent_invariant_per_primitive() {
    for spec in PRIMITIVES {
        let idempotent = default_idempotent_for(spec.step_type);
        if spec.tier == GuiPrimitiveTier::Green {
            assert!(
                idempotent,
                "[{}] GREEN primitive must be idempotent",
                spec.name
            );
        }
        if !idempotent {
            assert_eq!(
                spec.tier,
                GuiPrimitiveTier::Yellow,
                "[{}] non-idempotent primitive must be YELLOW",
                spec.name
            );
        }
    }
}

/// The deterministic planner's per-step contract is COMPLETE for every
/// primitive: the type-correct default `verification_strategy` exists and is
/// non-empty (plan-step completeness, Property 3 / Requirement 4.2). This is the
/// unit counterpart to the T2 "no step is blocked for a missing
/// verification_strategy" assertion.
#[test]
fn t1_planner_step_contract_complete_per_primitive() {
    for spec in PRIMITIVES {
        let strategy = default_verification_strategy_for_step(spec.step_type);
        let strategy = strategy.unwrap_or_else(|| {
            panic!("[{}] step {} has no default verification strategy", spec.name, spec.step_type)
        });
        assert!(
            !strategy.trim().is_empty(),
            "[{}] step {} default verification strategy is empty",
            spec.name,
            spec.step_type
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// T2 — per-primitive deterministic fixture pipeline (no display, no network).
// ─────────────────────────────────────────────────────────────────────────────

/// A deterministic desktop fixture that exposes the controls every primitive
/// needs to resolve: a focused "Search" text field, a "Search" push button, and
/// a "Remember me" check box. `capture_screenshot` advances a sequence so a
/// state-changing step's `screen_changed`/`state_changed` verification can
/// succeed against a fresh observation.
struct PrimitiveFixturePerception {
    active_window: String,
    screen_seq: std::sync::atomic::AtomicU64,
}

impl PrimitiveFixturePerception {
    fn new(active_window: &str) -> Self {
        Self {
            active_window: active_window.into(),
            screen_seq: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl GuiPerceptionProvider for PrimitiveFixturePerception {
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
            "applications": [self.active_window, "Browser", "Files"],
        }))
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": true }))
    }

    async fn find_ui_elements(&self, role: &str) -> GuiProbeResult {
        let element = match role {
            "text" => Some(serde_json::json!({
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
            })),
            "push button" => Some(serde_json::json!({
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
            })),
            "check box" => Some(serde_json::json!({
                "role": "check box",
                "name": "Remember me",
                "label": "Remember me",
                "path": "/fixture/checkbox/Remember",
                "control_id": "fixture-remember-checkbox",
                "enabled": true,
                "visible": true,
                "focused": false,
                "checked": false,
                "in_active_window": true,
                "bounds": { "x": 10, "y": 70, "width": 160, "height": 24 },
                "score": 0.9,
                "identity_confidence": 0.9,
                "bounds_confidence": 0.9,
                "state_confidence": 0.9
            })),
            _ => None,
        };
        GuiProbeResult::ok(serde_json::json!({ "elements": element.map(|e| vec![e]).unwrap_or_default() }))
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
        let seq = self
            .screen_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

/// A fixture executor that records the [`GuiActionKind`] of every action that
/// routes through it. Backend success is intentionally distinct from the
/// verified verdict.
#[derive(Clone)]
struct RecordingExecutor {
    backend: GuiActionBackendStatus,
    kinds: Arc<Mutex<Vec<GuiActionKind>>>,
}

impl RecordingExecutor {
    fn new() -> Self {
        Self {
            backend: GuiActionBackendStatus::available("fixture_executor"),
            kinds: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn routed_kinds(&self) -> Vec<GuiActionKind> {
        self.kinds.lock().expect("kinds lock").clone()
    }
}

#[async_trait]
impl GuiActionExecutor for RecordingExecutor {
    async fn action_backend_status(&self) -> GuiActionBackendStatus {
        self.backend.clone()
    }

    async fn execute(&self, request: GuiActionRequest) -> GuiActionExecution {
        self.kinds
            .lock()
            .expect("kinds lock")
            .push(request.kind.clone());
        GuiActionExecution::ok(
            "fixture_executor",
            serde_json::json!({ "executed": request.kind.as_str() }),
        )
    }
}

fn primitive_request(prompt: &str, mode: GuiExecutionMode, env: GuiExecutionEnvironment) -> GuiTurnRequest {
    GuiTurnRequest {
        session_id: "p64-session".into(),
        turn_id: "p64-turn".into(),
        workflow_id: "p64-workflow".into(),
        message: prompt.into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: None,
        execution_environment: env,
        execution_mode: mode,
        workflow_enabled: false,
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

const NON_BLOCKING_TERMINAL_STATUSES: &[&str] =
    &["valid_for_resolution", "needs_clarification", "approval_required"];

/// T2-A — plan/validate readiness: with the `gui_cog_primitives` flag ON, EVERY
/// primitive prompt threads the pipeline to a NON-blocking terminal readiness
/// status (never `blocked`/`rejected`), the plan never claims it can execute at
/// the validate stage, and nothing executes in this non-executing mode.
#[tokio::test]
async fn t2_each_primitive_reaches_non_blocking_readiness_with_flag_on() {
    for spec in PRIMITIVES {
        let perception = PrimitiveFixturePerception::new("KRIA Fixture App");
        let executor = RecordingExecutor::new();
        let runtime = GuiCognitionRuntime::new(&perception, &executor)
            .with_primitives(GuiPrimitivesConfig::enabled());

        let outcome = runtime
            .run_turn(primitive_request(
                spec.prompt,
                GuiExecutionMode::SafetyOnly,
                GuiExecutionEnvironment::RealSession,
            ))
            .await;

        let event = plan_validation_event(&outcome);
        let status = event["readiness_status"].as_str().unwrap_or("");
        assert!(
            NON_BLOCKING_TERMINAL_STATUSES.contains(&status),
            "[{}] landed on terminal status {status:?}; event: {event}",
            spec.name
        );
        // KRIA authority: a plan never auto-executes at the plan/validate stage.
        assert_eq!(
            event["can_execute"], false,
            "[{}] must not be executable at the validate stage",
            spec.name
        );
        // No Prompt→Tool shortcut in the non-executing mode.
        let types = event_types(&outcome);
        assert!(
            !types.contains(&"ActionStarted".to_string()),
            "[{}] must not start an action at the validate stage, events: {types:?}",
            spec.name
        );
        assert!(
            executor.routed_kinds().is_empty(),
            "[{}] SafetyOnly must not route any action through the executor",
            spec.name
        );
        // Boundedness.
        assert!(
            !outcome.status.is_empty(),
            "[{}] must terminate with a defined status",
            spec.name
        );
    }
}

/// T2-B — execution routing integrity: with the flag ON and inside the
/// TestSubstrate (so the deterministic pipeline runs the whole plan offline
/// through the executor), every action that routes THROUGH the executor is a
/// real typed [`GuiActionKind`] (never a leaked/unknown value), and the
/// **directly-executable** primitives route their EXACT expected typed kind.
///
/// Planner boundary (honest scope of this TEST task): the deterministic planner
/// (Task 2) models a primitive intent as a complete sequence built from the
/// `OpenApp`/`FocusField`/`TypeText`/`ClickControl` actions it currently emits
/// (e.g. "clear the field" is planned as a focus + observe sequence; "search in
/// the file manager" as focus + type). It does NOT yet emit the new dedicated
/// typed primitive *actions* (`ClearField`/`SelectAll`/`Copy`/`Paste`/`PressKey`/
/// `Scroll`/`SetCheckbox`/`CloseDialog`/`InAppSearch`) into the executor stream.
/// The flag-ON executor mapping for those verbs (`resolve_action_kind`) is what
/// turns them into the correct typed kind, and that mapping is proven
/// exhaustively at the unit tier above
/// (`t1_flag_on_resolves_each_primitive_verb_to_its_typed_kind`). At the
/// integration tier we therefore assert the routing the planner actually
/// produces is correct and leak-free, and that the directly-executable
/// primitives carry their exact typed kind end-to-end.
#[tokio::test]
async fn t2_executed_primitive_routes_correct_action_kind_with_flag_on() {
    use std::collections::HashMap;

    // The typed primitive vocabulary an executor action may legitimately carry.
    let typed_vocabulary = [
        GuiActionKind::OpenApp,
        GuiActionKind::SwitchWindow,
        GuiActionKind::FocusField,
        GuiActionKind::FillField,
        GuiActionKind::TypeText,
        GuiActionKind::ClickControl,
        GuiActionKind::PressKey,
        GuiActionKind::Hotkey,
        GuiActionKind::Scroll,
        GuiActionKind::Copy,
        GuiActionKind::Paste,
        GuiActionKind::ClearField,
        GuiActionKind::SelectAll,
        GuiActionKind::SetCheckbox,
        GuiActionKind::CloseDialog,
        GuiActionKind::InAppSearch,
    ];

    // Directly-executable primitives: the planner emits these as a concrete
    // action that routes through the executor against the fixture. Each must
    // carry its EXACT typed kind end-to-end.
    let directly_executable: HashMap<&str, GuiActionKind> = HashMap::from([
        ("focus", GuiActionKind::FocusField),
        ("type", GuiActionKind::TypeText),
        ("click", GuiActionKind::ClickControl),
    ]);

    let mut covered_direct = 0usize;

    for spec in PRIMITIVES {
        let perception = PrimitiveFixturePerception::new("KRIA Fixture App");
        let executor = RecordingExecutor::new();
        let runtime = GuiCognitionRuntime::new(&perception, &executor)
            .with_primitives(GuiPrimitivesConfig::enabled());

        let mut request = primitive_request(
            spec.prompt,
            GuiExecutionMode::ExecuteFixture,
            GuiExecutionEnvironment::TestSubstrate {
                scratch_dir: None,
                restore_clipboard: true,
            },
        );
        // Run the WHOLE plan (a primitive intent may be planned as a complete
        // sequence, e.g. open→focus→type) so the primitive's own action routes
        // through the executor, not just the first proposal.
        request.workflow_enabled = true;
        // Auto-approval is honored ONLY inside the substrate; supply it so a
        // risk-gated YELLOW step can reach execution deterministically.
        request.hitl_decision_fixture = Some(GuiHitlDecisionFixture::Approve);

        let outcome = runtime.run_turn(request).await;
        let routed = executor.routed_kinds();
        let types = event_types(&outcome);

        // Routing integrity: every action that reached the executor is a real
        // typed kind (no action-kind leakage, no catch-all surprise).
        for kind in &routed {
            assert!(
                typed_vocabulary.contains(kind),
                "[{}] routed an unknown action kind {:?}; events: {types:?}",
                spec.name,
                kind
            );
        }

        // Directly-executable primitives must carry their exact typed kind, with
        // the action started + completed at the backend and no plan block.
        if let Some(expected) = directly_executable.get(spec.name) {
            assert!(
                routed.iter().any(|k| k == expected),
                "[{}] expected routed action kind {:?}, got {:?}; events: {types:?}",
                spec.name,
                expected,
                routed
            );
            assert!(
                types.contains(&"ActionStarted".to_string())
                    && types.contains(&"ActionCompleted".to_string()),
                "[{}] primitive action must start and complete at the backend; events: {types:?}",
                spec.name
            );
            assert!(
                !types.contains(&"PlanBlocked".to_string()),
                "[{}] primitive plan must not be blocked; events: {types:?}",
                spec.name
            );
            covered_direct += 1;
        }

        // Boundedness: every primitive turn terminates with a defined status.
        assert!(
            !outcome.status.is_empty(),
            "[{}] must terminate with a defined status",
            spec.name
        );
    }

    assert_eq!(
        covered_direct,
        directly_executable.len(),
        "every directly-executable primitive must have been exercised end-to-end"
    );
}

/// Flag-OFF control for the execution path: with the primitives flag OFF, a NEW
/// typed primitive verb prompt that DOES route an action through the executor
/// must use the legacy `ClickControl` catch-all (never the typed kind) — the
/// byte-for-byte legacy behavior. We assert the negative: the typed kind never
/// appears on the routed actions while OFF.
#[tokio::test]
async fn t2_flag_off_never_routes_new_typed_primitive_kinds() {
    for spec in PRIMITIVES.iter().filter(|s| s.new_primitive) {
        let perception = PrimitiveFixturePerception::new("KRIA Fixture App");
        let executor = RecordingExecutor::new();
        // No `.with_primitives(...)` → `gui_cog_primitives` stays OFF.
        let runtime = GuiCognitionRuntime::new(&perception, &executor);

        let mut request = primitive_request(
            spec.prompt,
            GuiExecutionMode::ExecuteFixture,
            GuiExecutionEnvironment::TestSubstrate {
                scratch_dir: None,
                restore_clipboard: true,
            },
        );
        request.workflow_enabled = true;
        request.hitl_decision_fixture = Some(GuiHitlDecisionFixture::Approve);

        let outcome = runtime.run_turn(request).await;
        let routed = executor.routed_kinds();

        assert!(
            !routed.contains(&spec.kind),
            "[{}] flag OFF must NEVER route the typed kind {:?}; routed: {:?}",
            spec.name,
            spec.kind,
            routed
        );
        assert!(
            !outcome.status.is_empty(),
            "[{}] flag OFF turn must terminate with a defined status",
            spec.name
        );
    }
}
