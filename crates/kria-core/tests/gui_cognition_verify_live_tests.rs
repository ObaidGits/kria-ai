//! Phase 1 (Requirement 1) CI-safe T2 tests for the `gui_cog_verify_live`
//! verification predicate: OpenApp verifies `window_visible` against the desktop
//! open-window set (alias-tolerant), succeeds when the launched app's window is
//! PRESENT but NOT focused, performs a bounded readiness wait, and is
//! byte-for-byte the prior `active_window_match` verdict when the flag is OFF.
//! No live desktop, display, or backend is required.

use kria_core::agent::gui_cognition::executor::GuiActionKind;
use kria_core::agent::gui_cognition::perception::{
    GuiAccessibilitySummary, GuiActiveWindowSummary, GuiCursorFocusSummary,
    GuiObservationCacheSummary, GuiObservationSnapshot, GuiObservationTimingSummary,
    GuiOcrDiagnostics, GuiPerceptionCapabilities, GuiSourceStatus, GuiWindowSummary,
};
use kria_core::agent::gui_cognition::verifier::{
    evidence_source_for_strategy, select_verification_strategy,
    select_verification_strategy_with_flag, verification_contract_for,
    verification_contract_for_with_flag, verify_post_action_detailed,
    GuiPostActionVerificationRequest, GuiVerificationEvidenceSource, GuiVerificationStrategy,
    VERIFICATION_INCONCLUSIVE, VERIFICATION_VERIFIED,
};

/// Build a snapshot whose ACTIVE window is `active_app` and whose desktop
/// open-window set contains `windows` (title, app_name, focused). This lets a
/// test model "the launched app's window is present but is NOT the focused
/// active window" — the core Wayland case.
fn snapshot(active_app: &str, windows: &[(&str, &str, bool)]) -> GuiObservationSnapshot {
    let visible_windows = windows
        .iter()
        .map(|(title, app, focused)| GuiWindowSummary {
            title: (*title).into(),
            app_name: Some((*app).into()),
            bounds: None,
            focused: *focused,
            visible: true,
            monitor_id: None,
            source: "test".into(),
        })
        .collect();

    let active_window = if active_app == "unknown" {
        GuiActiveWindowSummary::default()
    } else {
        GuiActiveWindowSummary {
            label: active_app.into(),
            app_name: Some(active_app.into()),
            source: "test".into(),
            confidence: 0.95,
            fallback_used: false,
            blocker: None,
            reliability: "reliable".into(),
            ..GuiActiveWindowSummary::default()
        }
    };

    GuiObservationSnapshot {
        observation_id: "obs".into(),
        context_id: "ctx".into(),
        timestamp_ms: 1,
        screen_hash: Some("screen-a".into()),
        active_window_label: active_window.label.clone(),
        active_window,
        visible_windows,
        visible_app_count: windows.len(),
        monitors: Vec::new(),
        cursor_focus: GuiCursorFocusSummary::default(),
        accessibility: GuiAccessibilitySummary::default(),
        ocr_blocks: Vec::new(),
        ocr_diagnostics: GuiOcrDiagnostics::default(),
        capabilities: GuiPerceptionCapabilities {
            active_window: GuiSourceStatus::available("test"),
            desktop_state: GuiSourceStatus::available("test"),
            accessibility: GuiSourceStatus::available("test"),
            screenshot: GuiSourceStatus::available("test"),
            ocr: GuiSourceStatus::available("test"),
            monitor: GuiSourceStatus::available("test"),
            cursor_focus: GuiSourceStatus::available("test"),
        },
        accessibility_ok: true,
        ocr_available: false,
        screenshot_available: true,
        active_window_probe_ok: true,
        desktop_state_probe_ok: true,
        capabilities_probe_ok: true,
        text_fields: Vec::new(),
        buttons: Vec::new(),
        dialogs: Vec::new(),
        other_controls: Vec::new(),
        visual_controls: Vec::new(),
        timing: GuiObservationTimingSummary::default(),
        cache: GuiObservationCacheSummary::default(),
    }
}

fn open_app_request(strategy: GuiVerificationStrategy, app_hint: &str) -> GuiPostActionVerificationRequest {
    GuiPostActionVerificationRequest {
        verification_id: "verification-1".into(),
        execution_id: "execution-1".into(),
        proposal_id: "proposal-1".into(),
        proposal_hash: "proposal-hash".into(),
        action_type: "OpenApp".into(),
        target_hash: "target-hash".into(),
        stable_target_identity_hash: None,
        expected_postcondition: "app window visible".into(),
        verification_strategy: strategy.as_str().into(),
        pre_action_context_id: "ctx-pre".into(),
        post_action_observation_id: "obs-post".into(),
        post_action_context_id: "ctx-post".into(),
        started_at_ms: 1_000,
        is_secret_payload: false,
        prompt_hash: "prompt-hash".into(),
        target_label: None,
        target_role: None,
        target_control_id: None,
        expected_app_hint: Some(app_hint.into()),
        expected_window_hint: None,
    }
}

// ── Contract: OpenApp predicate == window_visible + evidence == observation ──

#[test]
fn open_app_contract_predicate_is_window_visible_and_evidence_observation_when_flag_on() {
    assert_eq!(
        select_verification_strategy_with_flag(&GuiActionKind::OpenApp, false, true),
        GuiVerificationStrategy::WindowVisible
    );
    let contract =
        verification_contract_for_with_flag(&GuiActionKind::OpenApp, false, 4_000, 12, true);
    assert_eq!(contract.predicate, "window_visible");
    assert_eq!(contract.evidence_source, "observation");
    assert_eq!(
        evidence_source_for_strategy(GuiVerificationStrategy::WindowVisible),
        GuiVerificationEvidenceSource::Observation
    );
}

// ── Alias match: chrome / chromium / google-chrome all verify ────────────────

#[test]
fn open_app_window_visible_matches_chrome_aliases() {
    // The desktop window set reports the app as "google-chrome"; the user asked
    // for "Chrome". Alias-tolerant matching must verify it.
    for requested in ["Chrome", "chromium", "google-chrome"] {
        let post = snapshot("Desktop", &[("Google Search - Chromium", "google-chrome", false)]);
        let pre = snapshot("Desktop", &[]);
        let req = open_app_request(GuiVerificationStrategy::WindowVisible, requested);
        let result = verify_post_action_detailed(&req, &pre, &post, true, None, 2_000);
        assert_eq!(
            result.status, VERIFICATION_VERIFIED,
            "requested {requested:?} should verify against a google-chrome window"
        );
    }
}

// ── Window present but NOT focused still verifies ────────────────────────────

#[test]
fn open_app_window_present_but_not_focused_still_verifies() {
    // Active/focused window is the file manager; the launched Chrome window is
    // present in the open-window set but NOT focused (Wayland focus-stealing
    // prevention). window_visible must still verify.
    let post = snapshot("Files", &[
        ("Files", "org.gnome.Nautilus", true),
        ("New Tab - Google Chrome", "google-chrome", false),
    ]);
    let pre = snapshot("Files", &[("Files", "org.gnome.Nautilus", true)]);

    // The presence predicate itself sees the non-focused window.
    assert!(post.window_visible_for_app("Chrome"));

    let req = open_app_request(GuiVerificationStrategy::WindowVisible, "Chrome");
    let result = verify_post_action_detailed(&req, &pre, &post, true, None, 2_000);
    assert_eq!(result.status, VERIFICATION_VERIFIED);
}

#[test]
fn open_app_window_absent_does_not_falsely_verify() {
    // No chrome window anywhere → honest non-verified (failed), never a false
    // verified.
    let post = snapshot("Files", &[("Files", "org.gnome.Nautilus", true)]);
    let pre = snapshot("Files", &[]);
    assert!(!post.window_visible_for_app("Chrome"));
    let req = open_app_request(GuiVerificationStrategy::WindowVisible, "Chrome");
    let result = verify_post_action_detailed(&req, &pre, &post, true, None, 2_000);
    assert_ne!(result.status, VERIFICATION_VERIFIED);
}

#[test]
fn open_app_window_visible_inconclusive_without_hint() {
    // No app/window hint → presence cannot be confirmed for a specific app →
    // honest inconclusive (never a false verified).
    let post = snapshot("Files", &[("Files", "org.gnome.Nautilus", true)]);
    let pre = snapshot("Files", &[]);
    let mut req = open_app_request(GuiVerificationStrategy::WindowVisible, "Chrome");
    req.expected_app_hint = None;
    req.expected_window_hint = None;
    req.target_label = None;
    let result = verify_post_action_detailed(&req, &pre, &post, true, None, 2_000);
    assert_eq!(result.status, VERIFICATION_INCONCLUSIVE);
}

// ── Flag-OFF byte-for-byte: prior active_window_match preserved ──────────────

#[test]
fn flag_off_open_app_is_byte_for_byte_prior_active_window_match() {
    // Flag OFF: selection + contract identical to the prior behavior.
    assert_eq!(
        select_verification_strategy_with_flag(&GuiActionKind::OpenApp, false, false),
        select_verification_strategy(&GuiActionKind::OpenApp, false)
    );
    assert_eq!(
        select_verification_strategy(&GuiActionKind::OpenApp, false),
        GuiVerificationStrategy::ActiveWindowMatch
    );
    let off =
        verification_contract_for_with_flag(&GuiActionKind::OpenApp, false, 4_000, 12, false);
    let prior = verification_contract_for(&GuiActionKind::OpenApp, false, 4_000, 12);
    assert_eq!(off, prior);
    assert_eq!(off.predicate, "active_window_match");
    assert_eq!(off.evidence_source, "active_window_probe");
}

#[test]
fn switch_window_predicate_unchanged_regardless_of_flag() {
    for flag in [false, true] {
        assert_eq!(
            select_verification_strategy_with_flag(&GuiActionKind::SwitchWindow, false, flag),
            GuiVerificationStrategy::ActiveWindowMatch
        );
    }
}

// ── Bounded readiness wait (no unbounded poll) ───────────────────────────────
//
// When the launched app's window NEVER appears, the OpenApp readiness wait must
// re-observe a BOUNDED number of times (capped by this turn's Task 1 re-observe
// cap) and then conclude — never poll forever. Driven by the full in-process
// runtime with a perception that never shows the target window.

mod bounded_readiness {
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicU64, Ordering};

    use kria_core::agent::gui_cognition::executor::{
        GuiActionBackendStatus, GuiActionExecution, GuiActionExecutor, GuiActionRequest,
        GuiExecutionMode,
    };
    use kria_core::agent::gui_cognition::perception::{GuiPerceptionProvider, GuiProbeResult};
    use kria_core::agent::gui_cognition::turn_budget::{GuiRuntimeGuardConfig, TurnBudget};
    use kria_core::agent::gui_cognition::verifier::GuiVerifyLiveConfig;
    use kria_core::agent::gui_cognition::{
        GuiCognitionRuntime, GuiTurnOutcome, GuiTurnRequest,
    };

    /// A perception whose active window is always "Desktop" and whose open-window
    /// set NEVER contains the launched app — so an OpenApp readiness wait can
    /// never be satisfied (the deterministic driver for the bounded NOT-ready
    /// path). The screen hash changes each observation so the flapping cap is not
    /// what trips; the re-observe cap is.
    struct NeverVisiblePerception {
        screen_seq: AtomicU64,
    }

    impl NeverVisiblePerception {
        fn new() -> Self {
            Self {
                screen_seq: AtomicU64::new(0),
            }
        }
    }

    #[async_trait]
    impl GuiPerceptionProvider for NeverVisiblePerception {
        async fn get_active_window(&self) -> GuiProbeResult {
            GuiProbeResult::ok(serde_json::json!({
                "title": "Desktop",
                "app_name": "Desktop",
            }))
        }

        async fn get_desktop_state(&self) -> GuiProbeResult {
            GuiProbeResult::ok(serde_json::json!({
                "focused_window": "Desktop",
                "accessibility_operational": true,
                "applications": ["Desktop"],
            }))
        }

        async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
            GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": true }))
        }

        async fn find_ui_elements(&self, _role: &str) -> GuiProbeResult {
            GuiProbeResult::ok(serde_json::json!({ "elements": [] }))
        }

        async fn capture_screenshot(&self) -> GuiProbeResult {
            let seq = self.screen_seq.fetch_add(1, Ordering::SeqCst);
            GuiProbeResult::ok(serde_json::json!({
                "screen_hash": format!("verify-live-screen-{seq}"),
                "byte_count": 16,
                "source": "fixture",
            }))
        }

        async fn focused_window_title(&self) -> Option<String> {
            Some("Desktop".into())
        }
    }

    struct OkExecutor;

    #[async_trait]
    impl GuiActionExecutor for OkExecutor {
        async fn action_backend_status(&self) -> GuiActionBackendStatus {
            GuiActionBackendStatus::available("fixture_executor")
        }

        async fn execute(&self, request: GuiActionRequest) -> GuiActionExecution {
            GuiActionExecution::ok(
                "fixture_executor",
                serde_json::json!({ "executed": request.kind.as_str() }),
            )
        }
    }

    fn open_app_request() -> GuiTurnRequest {
        GuiTurnRequest {
            session_id: "verify-live-session".into(),
            turn_id: "verify-live-turn".into(),
            workflow_id: "verify-live-workflow".into(),
            message: "Open Firefox".into(),
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

    fn open_app_readiness_events(outcome: &GuiTurnOutcome) -> Vec<&serde_json::Value> {
        outcome
            .events
            .iter()
            .filter(|event| {
                event.get("type").and_then(serde_json::Value::as_str)
                    == Some("WorkflowReadinessWait")
                    && event.get("cause").and_then(serde_json::Value::as_str)
                        == Some("open_app_readiness_wait")
            })
            .collect()
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn open_app_readiness_wait_is_bounded_when_window_never_appears() {
        kria_core::safety::release_halt("test reset");
        let perception = NeverVisiblePerception::new();
        let executor = OkExecutor;

        let runtime = GuiCognitionRuntime::new(&perception, &executor)
            .with_runtime_guards(GuiRuntimeGuardConfig::enabled(TurnBudget::default()))
            .with_verify_live(GuiVerifyLiveConfig::enabled())
            .with_cancel_token(None);

        // The turn RETURNS (no unbounded poll) — completing this await proves
        // termination.
        let outcome = runtime.run_turn(open_app_request()).await;

        let readiness = open_app_readiness_events(&outcome);
        assert!(
            !readiness.is_empty(),
            "an OpenApp readiness wait must run with verify_live ON when the window is absent"
        );
        for event in &readiness {
            let attempts = event["attempts"].as_u64().expect("attempts surfaced");
            let cap = event["max_reobserve"].as_u64().expect("cap surfaced");
            assert!(
                attempts <= cap,
                "readiness attempts must never exceed the re-observe cap (bounded): {event}"
            );
            assert_eq!(
                event["bounded_by_runaway_caps"], true,
                "the wait must report it is bounded by the runaway caps: {event}"
            );
        }
        // The final readiness verdict is NOT-ready (the window never appeared) and
        // the wait stopped at/under the cap — proof it did not poll forever.
        let last = readiness.last().expect("at least one readiness event");
        assert_eq!(last["ready"], false, "window never appears => not ready: {last}");
        let last_attempts = last["attempts"].as_u64().unwrap();
        let cap = last["max_reobserve"].as_u64().unwrap();
        assert!(last_attempts <= cap);
    }
}
