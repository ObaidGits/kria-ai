//! Task 8.4 (Requirements 6, 7, 8) — T2 integration for cross-app combo +
//! file-manager select + clipboard SAVE→USE→RESTORE.
//!
//! These are CI-safe T2 integration tests: NO real display, network, or system
//! clipboard. Perception/execution are driven by in-memory fixtures and the
//! clipboard sits behind the in-memory [`ClipboardBackend`] fake from Task 8.1,
//! so the whole suite runs headless.
//!
//! ## Clipboard semantics this task locks in (Requirement 8)
//!
//! There are two DISTINCT cross-app clipboard usages, and they have OPPOSITE
//! restore semantics by design:
//!
//! 1. **Genuine copy→paste combo** (Task 8.2): the copied content IS the
//!    intended deliverable — the user asked to move that content from app A to
//!    app B. The combo therefore legitimately LEAVES the copied content on the
//!    clipboard afterward; restoring would defeat the user's request. No restore.
//!
//! 2. **Transient borrow**: an operation that uses the clipboard only as scratch
//!    (e.g. to read a value out of an app and hand it back) MUST restore the
//!    user's pre-existing clipboard so it is never clobbered. This goes through
//!    the Task 8.1 guard [`with_clipboard`] / [`ClipboardSession`] (SAVE → USE →
//!    RESTORE, serialized, RAII restore even on error/panic).
//!
//! Wiring note (8.4): the current cross-app plan vocabulary contains exactly ONE
//! clipboard usage — the genuine combo (Copy/Paste as the deliverable) — and the
//! file-manager select flow reads the filename from OBSERVATION
//! (`SummarizeVisibleContent`), never via a clipboard borrow. So there is no
//! transient-scratch clipboard operation in the runtime path to wire, and the
//! genuine combo must NOT restore. The Task 8.1 guard remains the mechanism any
//! future transient-borrow op routes through; these tests prove its
//! SAVE→USE→RESTORE contract end-to-end with the fake backend, prove the genuine
//! combo retains its content, and prove the fm flow never touches the clipboard.

use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use kria_core::agent::gui_cognition::clipboard::{
    clipboard_lock_available, clipboard_value_summary, with_clipboard, ClipboardBackend,
    ClipboardError, ClipboardSession, GuiCrossAppConfig,
};
use kria_core::agent::gui_cognition::executor::{
    GuiActionBackendStatus, GuiActionExecution, GuiActionExecutor, GuiActionRequest,
    GuiExecutionMode,
};
use kria_core::agent::gui_cognition::perception::{GuiPerceptionProvider, GuiProbeResult};
use kria_core::agent::gui_cognition::turn_budget::GuiReobserveConfig;
use kria_core::agent::gui_cognition::workflow_runtime::{
    workflow_step_is_state_changing, workflow_step_requires_target,
};
use kria_core::agent::gui_cognition::{GuiCognitionRuntime, GuiTurnRequest};

// ── Shared fixtures (mirror gui_cognition_workflow_runtime_tests.rs) ─────────

struct WorkflowExecutor {
    backend: GuiActionBackendStatus,
}

impl WorkflowExecutor {
    fn new() -> Self {
        Self {
            backend: GuiActionBackendStatus::available("fixture_executor"),
        }
    }
}

#[async_trait]
impl GuiActionExecutor for WorkflowExecutor {
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

/// Cross-app combo fixture: the target app exposes one visible/focused text
/// input (the paste target) so FocusField resolves against the fresh post-switch
/// context.
struct CrossAppPerception {
    active_window: String,
    screen_seq: AtomicU64,
}

impl CrossAppPerception {
    fn new(active_window: &str) -> Self {
        Self {
            active_window: active_window.into(),
            screen_seq: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl GuiPerceptionProvider for CrossAppPerception {
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
            "applications": [self.active_window, "Chrome", "VS Code"],
        }))
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": true }))
    }

    async fn find_ui_elements(&self, role: &str) -> GuiProbeResult {
        let elements = match role {
            "text" => vec![serde_json::json!({
                "role": "text",
                "name": "Editor input",
                "label": "Editor input",
                "path": "/crossapp/text/editor",
                "control_id": "crossapp-editor-field",
                "enabled": true,
                "visible": true,
                "focused": true,
                "in_active_window": true,
                "bounds": { "x": 10, "y": 20, "width": 320, "height": 32 },
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
            "focused_control_id": "crossapp-editor-field",
            "focused_control_label": "Editor input",
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
            "screen_hash": format!("crossapp-screen-{seq}"),
            "byte_count": 16,
            "source": "fixture",
        }))
    }

    async fn focused_window_title(&self) -> Option<String> {
        Some(self.active_window.clone())
    }
}

/// File-manager fixture: a "Files" window exposing observable file entries so
/// the select step resolves against the observed list, never an invented name.
struct FileManagerPerception {
    screen_seq: AtomicU64,
}

impl FileManagerPerception {
    fn new() -> Self {
        Self {
            screen_seq: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl GuiPerceptionProvider for FileManagerPerception {
    async fn get_active_window(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "title": "Files",
            "app_name": "Files",
        }))
    }

    async fn get_desktop_state(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": "Files",
            "accessibility_operational": true,
            "applications": ["Files", "file manager"],
        }))
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": true }))
    }

    async fn find_ui_elements(&self, role: &str) -> GuiProbeResult {
        let elements = match role {
            "text" | "list item" | "table cell" => vec![
                serde_json::json!({
                    "role": "list item",
                    "name": "report-2024-06-01.pdf",
                    "label": "report-2024-06-01.pdf",
                    "path": "/files/list/0",
                    "control_id": "fm-file-0",
                    "enabled": true,
                    "visible": true,
                    "focused": true,
                    "in_active_window": true,
                    "bounds": { "x": 10, "y": 20, "width": 320, "height": 24 },
                    "score": 0.9,
                    "identity_confidence": 0.9,
                    "bounds_confidence": 0.9,
                    "state_confidence": 0.9
                }),
                serde_json::json!({
                    "role": "list item",
                    "name": "notes.txt",
                    "label": "notes.txt",
                    "path": "/files/list/1",
                    "control_id": "fm-file-1",
                    "enabled": true,
                    "visible": true,
                    "focused": false,
                    "in_active_window": true,
                    "bounds": { "x": 10, "y": 48, "width": 320, "height": 24 },
                    "score": 0.85,
                    "identity_confidence": 0.85,
                    "bounds_confidence": 0.85,
                    "state_confidence": 0.85
                }),
            ],
            _ => Vec::new(),
        };
        GuiProbeResult::ok(serde_json::json!({ "elements": elements }))
    }

    async fn get_cursor_focus_state(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": "Files",
            "focused_app": "Files",
            "focused_control_id": "fm-file-0",
            "focused_control_label": "report-2024-06-01.pdf",
            "focused_control_role": "list item",
            "keyboard_focus_known": true,
            "text_cursor_known": false,
            "editable_target_known": false,
            "terminal_like": false,
            "focus_confidence": 0.9,
        }))
    }

    async fn capture_screenshot(&self) -> GuiProbeResult {
        let seq = self.screen_seq.fetch_add(1, Ordering::SeqCst);
        GuiProbeResult::ok(serde_json::json!({
            "screen_hash": format!("filemanager-screen-{seq}"),
            "byte_count": 16,
            "source": "fixture",
        }))
    }

    async fn focused_window_title(&self) -> Option<String> {
        Some("Files".into())
    }
}

const CROSS_APP_PROMPT: &str = "Copy the selected text from Chrome and paste it into VS Code";
const FILE_MANAGER_PROMPT: &str =
    "Open the file manager and select the newest file and tell me its name";

fn workflow_request(message: &str, mode: GuiExecutionMode) -> GuiTurnRequest {
    GuiTurnRequest {
        session_id: "session-1".into(),
        turn_id: "turn-1".into(),
        workflow_id: "workflow-1".into(),
        message: message.into(),
        route_path: "send_manual_tool_message".into(),
        llm_tool_loop: false,
        hitl_decision_fixture: None,
        execution_environment: Default::default(),
        execution_mode: mode,
        workflow_enabled: true,
        resume_checkpoint: None,
        resume_reason: None,
    }
}

fn event_types(outcome: &kria_core::agent::gui_cognition::GuiTurnOutcome) -> Vec<String> {
    outcome
        .events
        .iter()
        .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
        .map(|value| value.to_string())
        .collect()
}

fn find_event<'a>(
    outcome: &'a kria_core::agent::gui_cognition::GuiTurnOutcome,
    event_type: &str,
) -> Option<&'a serde_json::Value> {
    outcome
        .events
        .iter()
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some(event_type))
}

/// The validated plan's step types from `PlanValidationCompleted.step_results` —
/// the authoritative plan structure independent of execution progress.
fn validated_step_types(
    outcome: &kria_core::agent::gui_cognition::GuiTurnOutcome,
) -> Vec<String> {
    let validation = find_event(outcome, "PlanValidationCompleted")
        .expect("a PlanValidationCompleted event must be emitted");
    validation
        .get("step_results")
        .and_then(serde_json::Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter_map(|s| s.get("step_type").and_then(serde_json::Value::as_str))
                .map(|v| v.to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn validated_step_risk_levels(
    outcome: &kria_core::agent::gui_cognition::GuiTurnOutcome,
) -> Vec<String> {
    let validation = find_event(outcome, "PlanValidationCompleted")
        .expect("a PlanValidationCompleted event must be emitted");
    validation
        .get("step_results")
        .and_then(serde_json::Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter_map(|s| s.get("risk_level").and_then(serde_json::Value::as_str))
                .map(|v| v.to_string())
                .collect()
        })
        .unwrap_or_default()
}

// ── In-memory fake clipboard backend (CI-safe — no real clipboard/display) ───

/// Records every write so a test can assert the exact SAVE→USE→RESTORE sequence
/// and that nothing leaked. Mirrors the Task 8.1 unit-test fake.
#[derive(Default)]
struct FakeClipboard {
    value: StdMutex<Option<String>>,
    write_log: StdMutex<Vec<Option<String>>>,
}

impl FakeClipboard {
    fn with_contents(initial: Option<&str>) -> Self {
        Self {
            value: StdMutex::new(initial.map(|s| s.to_string())),
            write_log: StdMutex::new(Vec::new()),
        }
    }

    fn current(&self) -> Option<String> {
        self.value.lock().unwrap().clone()
    }

    fn writes(&self) -> Vec<Option<String>> {
        self.write_log.lock().unwrap().clone()
    }
}

impl ClipboardBackend for FakeClipboard {
    fn read(&self) -> Result<Option<String>, ClipboardError> {
        Ok(self.value.lock().unwrap().clone())
    }

    fn write(&self, value: Option<&str>) -> Result<(), ClipboardError> {
        self.write_log
            .lock()
            .unwrap()
            .push(value.map(|s| s.to_string()));
        *self.value.lock().unwrap() = value.map(|s| s.to_string());
        Ok(())
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 1. Cross-app clipboard combo end-to-end through the workflow runtime (flag ON)
// ═════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn crossapp_combo_runs_end_to_end_through_runtime_with_full_sequence() {
    // Flag ON: the cross-app combo prompt drives the workflow runtime and the
    // validated plan is the full Copy → SwitchWindow → FocusField → Paste →
    // VerifyState sequence.
    let perception = CrossAppPerception::new("VS Code");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_crossapp(GuiCrossAppConfig::enabled())
        .with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(CROSS_APP_PROMPT, GuiExecutionMode::ExecuteFixture))
        .await;

    let step_types = validated_step_types(&outcome);
    assert_eq!(
        step_types,
        vec!["Copy", "SwitchWindow", "FocusField", "Paste", "VerifyState"],
        "flag ON must emit the full cross-app combo sequence: {step_types:?}"
    );

    // The combo drives the workflow runtime (observe → plan → per-step).
    let types = event_types(&outcome);
    assert!(
        types.contains(&"WorkflowRunStarted".to_string())
            && types.contains(&"WorkflowStepStarted".to_string()),
        "combo must drive the workflow runtime: {types:?}"
    );
    // The legitimate combo must NOT be rejected as a single-app contradiction.
    let serialized = serde_json::to_string(&outcome.response).unwrap();
    assert!(
        !serialized.contains("contradicts goal contract"),
        "combo plan must not be blocked as a single-app contradiction: {serialized}"
    );
}

#[tokio::test]
async fn crossapp_combo_reobserves_between_state_changing_steps() {
    // The combo's SwitchWindow and Paste steps are state-changing, so the
    // runtime's per-step re-observe (Task 3) re-observes after them and
    // re-resolves the next step's target against the FRESH post-switch screen —
    // so the paste lands on the target app's real focused field (Requirement 2).
    let perception = CrossAppPerception::new("VS Code");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_crossapp(GuiCrossAppConfig::enabled())
        .with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(CROSS_APP_PROMPT, GuiExecutionMode::ExecuteFixture))
        .await;

    let step_types = validated_step_types(&outcome);
    assert_eq!(step_types.len(), 5, "combo must be a 5-step plan: {step_types:?}");

    // The state-changing steps are exactly the ones that trigger re-observe, and
    // the next target-bearing step re-resolves against the fresh context.
    assert!(workflow_step_is_state_changing("SwitchWindow"));
    assert!(workflow_step_is_state_changing("Paste"));
    assert!(workflow_step_requires_target("FocusField"));

    let validation = find_event(&outcome, "PlanValidationCompleted")
        .expect("plan validation event must be emitted");
    assert_eq!(
        validation["readiness_status"], "valid_for_resolution",
        "combo plan must reach valid_for_resolution: {validation}"
    );
}

#[tokio::test]
async fn crossapp_combo_emits_no_raw_prompt_or_secret_leak() {
    // ZZZRAWPROMPTMARKER appears only in the raw prompt prefix, so it must never
    // be echoed into events or the response (privacy / Requirement 8).
    let perception = CrossAppPerception::new("VS Code");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_crossapp(GuiCrossAppConfig::enabled())
        .with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(
            &format!("ZZZRAWPROMPTMARKER {CROSS_APP_PROMPT}"),
            GuiExecutionMode::ExecuteFixture,
        ))
        .await;

    let response = serde_json::to_string(&outcome.response).unwrap();
    let events = serde_json::to_string(&outcome.events).unwrap();
    assert!(!response.contains("ZZZRAWPROMPTMARKER"), "raw prompt leaked in response");
    assert!(!response.contains("\"raw_prompt\""), "raw_prompt field exposed");
    assert!(!events.contains("ZZZRAWPROMPTMARKER"), "raw prompt leaked in events");
}

#[tokio::test]
async fn crossapp_combo_flag_off_preserves_single_copy_plan() {
    // Flag OFF (default): the SAME prompt keeps the existing single-copy
    // primitive plan — no SwitchWindow / Paste combo.
    let perception = CrossAppPerception::new("VS Code");
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(CROSS_APP_PROMPT, GuiExecutionMode::ExecuteFixture))
        .await;

    let step_types = validated_step_types(&outcome);
    assert!(
        !step_types.contains(&"SwitchWindow".to_string())
            && !step_types.contains(&"Paste".to_string()),
        "flag OFF must NOT emit the cross-app combo: {step_types:?}"
    );
    assert!(
        step_types.first().map(String::as_str) == Some("FocusField")
            && step_types.contains(&"Copy".to_string()),
        "flag OFF must keep the single copy primitive plan: {step_types:?}"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. File-manager NON-DESTRUCTIVE select flow end-to-end (flag ON)
// ═════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn file_manager_select_runs_end_to_end_non_destructive() {
    // Flag ON: navigate → observe → select → show-name, strictly
    // non-destructive (no delete/move/rename, no approval gate, all low-risk),
    // and the selection is by observed ORDER/POSITION, never an invented name.
    let perception = FileManagerPerception::new();
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_crossapp(GuiCrossAppConfig::enabled())
        .with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(FILE_MANAGER_PROMPT, GuiExecutionMode::ExecuteFixture))
        .await;

    let step_types = validated_step_types(&outcome);
    assert_eq!(
        step_types,
        vec!["OpenApp", "Observe", "FocusField", "SummarizeVisibleContent"],
        "flag ON must emit navigate → observe → select → show-name: {step_types:?}"
    );

    let validation = find_event(&outcome, "PlanValidationCompleted")
        .expect("plan validation event must be emitted");
    assert_eq!(
        validation["readiness_status"], "valid_for_resolution",
        "file-manager select plan must reach valid_for_resolution: {validation}"
    );

    // Non-destructive: no destructive step type, all low-risk.
    for destructive in ["RequireApproval", "TypeText", "Paste"] {
        assert!(
            !step_types.contains(&destructive.to_string()),
            "non-destructive select flow must not contain {destructive}: {step_types:?}"
        );
    }
    let risk_levels = validated_step_risk_levels(&outcome);
    assert!(
        risk_levels.iter().all(|r| r == "low"),
        "every step must be low-risk: {risk_levels:?}"
    );

    // Selection expressed by observed order/position, never a destructive verb.
    let serialized = serde_json::to_string(&outcome.response).unwrap();
    for destructive in ["delete", "Delete", "rename", "Rename", " move ", "trash"] {
        assert!(
            !serialized.contains(destructive),
            "non-destructive flow must never reference {destructive:?}: {serialized}"
        );
    }

    // The flow reads the name from OBSERVATION (SummarizeVisibleContent), never
    // by borrowing the clipboard.
    let types = event_types(&outcome);
    assert!(
        types.contains(&"WorkflowRunStarted".to_string()),
        "fm select must drive the workflow runtime: {types:?}"
    );
}

#[tokio::test]
async fn file_manager_select_flag_off_preserves_prior_plan() {
    let perception = FileManagerPerception::new();
    let executor = WorkflowExecutor::new();
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_reobserve(GuiReobserveConfig::enabled());

    let outcome = runtime
        .run_turn(workflow_request(FILE_MANAGER_PROMPT, GuiExecutionMode::ExecuteFixture))
        .await;

    let step_types = validated_step_types(&outcome);
    assert_ne!(
        step_types,
        vec!["OpenApp", "Observe", "FocusField", "SummarizeVisibleContent"],
        "flag OFF must NOT emit the file-manager select flow: {step_types:?}"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 3. Clipboard SAVE→USE→RESTORE (Task 8.1 guard) — transient borrow restores,
//    genuine combo retains, user clipboard never clobbered (Requirement 8)
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn transient_borrow_restores_preexisting_text_clipboard() {
    // A transient borrow uses the clipboard as scratch and MUST hand back the
    // user's pre-existing contents afterward.
    let clip = FakeClipboard::with_contents(Some("USER ORIGINAL"));
    let read_back = with_clipboard(&clip, |backend| {
        // Scratch: stash a value to read something out of an app, then we're done.
        backend.write(Some("transient scratch value"))?;
        let observed = backend.read()?;
        Ok(observed)
    })
    .expect("transient borrow should succeed");

    assert_eq!(read_back.as_deref(), Some("transient scratch value"));
    // The user's original clipboard is restored, not clobbered.
    assert_eq!(clip.current().as_deref(), Some("USER ORIGINAL"));
    // The write log proves SAVE→USE→RESTORE: scratch write then restore write.
    assert_eq!(
        clip.writes(),
        vec![
            Some("transient scratch value".to_string()),
            Some("USER ORIGINAL".to_string()),
        ],
        "must write scratch then restore the original"
    );
}

#[test]
fn transient_borrow_restores_empty_clipboard_as_empty() {
    // A previously-empty clipboard is restored to empty (cleared), not left
    // holding the transient scratch value.
    let clip = FakeClipboard::with_contents(None);
    with_clipboard(&clip, |backend| backend.write(Some("transient scratch")))
        .expect("transient borrow should succeed");
    assert_eq!(clip.current(), None, "empty clipboard must be restored to empty");
    assert_eq!(
        clip.writes(),
        vec![Some("transient scratch".to_string()), None],
        "must restore the empty (cleared) state"
    );
}

#[test]
fn transient_borrow_restores_even_when_operation_errors() {
    // Even when the borrowing op fails mid-way, the user's clipboard is restored.
    let clip = FakeClipboard::with_contents(Some("USER ORIGINAL"));
    let result: Result<(), ClipboardError> = with_clipboard(&clip, |backend| {
        backend.write(Some("half-done"))?;
        Err(ClipboardError::backend("op blew up"))
    });
    assert!(result.is_err(), "op error must propagate");
    assert_eq!(
        clip.current().as_deref(),
        Some("USER ORIGINAL"),
        "original must be restored despite the op error"
    );
}

#[test]
fn transient_borrow_restores_even_when_operation_panics() {
    // RAII restore on Drop covers a panicking borrow too.
    let clip = FakeClipboard::with_contents(Some("USER ORIGINAL"));
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut session = ClipboardSession::acquire(&clip).unwrap();
        clip.write(Some("transient")).unwrap();
        let _ = &mut session;
        panic!("transient borrow exploded");
    }))
    .is_err();
    assert!(panicked, "the closure should have panicked");
    assert_eq!(
        clip.current().as_deref(),
        Some("USER ORIGINAL"),
        "Drop must restore the original despite the panic"
    );
}

#[test]
fn genuine_copy_paste_combo_retains_copied_content_no_restore() {
    // The genuine copy→paste combo's deliverable IS the copied content, so it
    // must NOT go through the restore guard — the copied value legitimately
    // remains on the clipboard after the combo (contrast with a transient
    // borrow). This documents the deliberate OPPOSITE semantics.
    let clip = FakeClipboard::with_contents(Some("USER ORIGINAL"));

    // The combo path copies the requested content (the deliverable) and does NOT
    // wrap the copy in with_clipboard, so no restore happens.
    clip.write(Some("content copied from Chrome")).unwrap();

    assert_eq!(
        clip.current().as_deref(),
        Some("content copied from Chrome"),
        "the genuine combo must LEAVE the copied content (no restore)"
    );
    assert_ne!(
        clip.current().as_deref(),
        Some("USER ORIGINAL"),
        "the combo deliverable must not be reverted to the prior value"
    );
}

#[test]
fn concurrent_transient_borrows_are_serialized_and_never_interleave() {
    // Two turns borrowing the clipboard must serialize: the second observes the
    // restored original, never the first turn's transient scratch value.
    let clip = Arc::new(FakeClipboard::with_contents(Some("ORIGINAL")));
    let started = Arc::new(AtomicBool::new(false));

    let clip_a = Arc::clone(&clip);
    let started_a = Arc::clone(&started);
    let handle = std::thread::spawn(move || {
        with_clipboard(clip_a.as_ref(), |backend| {
            started_a.store(true, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(50));
            backend.write(Some("thread-A scratch"))
        })
    });

    while !started.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }

    with_clipboard(clip.as_ref(), |backend| {
        assert_eq!(
            backend.read().unwrap().as_deref(),
            Some("ORIGINAL"),
            "second borrow must observe the restored original, not A's scratch"
        );
        backend.write(Some("thread-B scratch"))
    })
    .unwrap();

    handle.join().unwrap().unwrap();
    assert_eq!(
        clip.current().as_deref(),
        Some("ORIGINAL"),
        "the user's original survives both serialized borrows"
    );
}

#[test]
fn transient_borrow_never_leaks_secret_contents_in_summary() {
    // Clipboard contents may be a secret; the surfaced summary reveals only a
    // non-revealing shape (length), never the bytes (Requirement 8 / privacy).
    const SECRET: &str = "hunter2-super-secret-password";
    let clip = FakeClipboard::with_contents(Some(SECRET));
    let session = ClipboardSession::acquire(&clip).unwrap();

    let summary = session.saved_summary();
    assert!(!summary.contains(SECRET), "summary must not leak the secret");
    assert!(summary.contains("chars"), "summary should report a shape");
    assert!(!session.saved_was_empty());

    assert!(!clipboard_value_summary(Some(SECRET)).contains(SECRET));
    assert_eq!(clipboard_value_summary(None), "<empty>");
}

#[test]
fn clipboard_lock_frees_after_transient_borrow_completes() {
    // After a transient borrow completes (restore done, session dropped), the
    // serialized lock is free again for the next turn.
    let clip = FakeClipboard::with_contents(Some("ORIGINAL"));
    {
        let _session = ClipboardSession::acquire(&clip).unwrap();
        assert!(
            !clipboard_lock_available(),
            "lock must be held during a borrow"
        );
    }
    assert!(
        clipboard_lock_available(),
        "lock must free once the borrow's session is released"
    );
}
