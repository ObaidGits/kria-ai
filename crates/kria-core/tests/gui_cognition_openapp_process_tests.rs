//! Issue #2 CI-safe tests for the OpenApp PROCESS-LAUNCHED verification evidence
//! source (`gui_cog_verify_live`).
//!
//! On this Wayland/GNOME session KRIA's observation can only name the ACTIVE
//! window (GNOME Eval window-enumeration disabled, AT-SPI labels are anonymous
//! D-Bus names), so a freshly launched, un-focused app is invisible to the
//! window-presence check. These tests confirm OpenApp verification ALSO accepts
//! "the app's process is running" as evidence — and NEVER fabricates a
//! `verified` without real window OR process evidence. All process lists are
//! INJECTED, so the tests never depend on real processes.

use kria_core::agent::gui_cognition::executor::GuiActionKind;
use kria_core::agent::gui_cognition::perception::{
    app_process_alias_group, app_process_running, GuiAccessibilitySummary, GuiActiveWindowSummary,
    GuiCursorFocusSummary, GuiObservationCacheSummary, GuiObservationSnapshot,
    GuiObservationTimingSummary, GuiOcrDiagnostics, GuiPerceptionCapabilities, GuiSourceStatus,
    GuiWindowSummary,
};
use kria_core::agent::gui_cognition::verifier::{
    select_verification_strategy, select_verification_strategy_with_flag,
    verify_post_action_detailed, verify_post_action_detailed_with_process,
    GuiPostActionVerificationRequest, GuiVerificationStrategy, VERIFICATION_INCONCLUSIVE,
    VERIFICATION_VERIFIED,
};

/// Build a snapshot whose ACTIVE window is `active_app` and whose open-window
/// set contains `windows` (title, app_name, focused).
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

fn open_app_request(app_hint: &str) -> GuiPostActionVerificationRequest {
    GuiPostActionVerificationRequest {
        verification_id: "verification-1".into(),
        execution_id: "execution-1".into(),
        proposal_id: "proposal-1".into(),
        proposal_hash: "proposal-hash".into(),
        action_type: "OpenApp".into(),
        target_hash: "target-hash".into(),
        stable_target_identity_hash: None,
        expected_postcondition: "app window visible".into(),
        verification_strategy: GuiVerificationStrategy::WindowVisible.as_str().into(),
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

// ── Pure probe: app_process_running over an INJECTED process list ────────────

#[test]
fn process_probe_matches_file_manager_to_nautilus() {
    let running = vec!["nautilus".to_string(), "bash".to_string()];
    assert_eq!(
        app_process_running("file manager", &running).as_deref(),
        Some("nautilus")
    );
}

#[test]
fn process_probe_matches_calculator_to_gnome_calculator() {
    let running = vec!["gnome-calculator".to_string(), "systemd".to_string()];
    assert_eq!(
        app_process_running("calculator", &running).as_deref(),
        Some("gnome-calculator")
    );
}

#[test]
fn process_probe_tolerates_linux_comm_truncation() {
    // Linux `comm` truncates to 15 chars: "gnome-calculator" -> "gnome-calculato".
    // The (truncated) ACTUAL process name is reported as honest evidence.
    let running = vec!["gnome-calculato".to_string()];
    assert_eq!(
        app_process_running("calculator", &running).as_deref(),
        Some("gnome-calculato")
    );
}

#[test]
fn process_probe_no_match_returns_none() {
    let running = vec!["bash".to_string(), "systemd".to_string()];
    assert!(app_process_running("file manager", &running).is_none());
    // Blank hint never matches.
    assert!(app_process_running("   ", &running).is_none());
}

#[test]
fn process_alias_group_covers_required_apps() {
    assert!(app_process_alias_group("file manager").iter().any(|a| a == "thunar"));
    assert!(app_process_alias_group("terminal").iter().any(|a| a == "kgx"));
    assert!(app_process_alias_group("text editor").iter().any(|a| a == "gnome-text-editor"));
    assert!(app_process_alias_group("settings").iter().any(|a| a == "systemsettings"));
}

// ── Verifier: process running CONTAINS the app, active window does NOT match ─

#[test]
fn open_app_verified_via_process_when_window_absent() {
    // Active/focused window is the terminal; NO file-manager window is present
    // anywhere (the launched app didn't map a visible/observable window). But
    // its process IS running. Verification must succeed on process evidence.
    let post = snapshot("Terminal", &[("Terminal", "gnome-terminal", true)]);
    let pre = snapshot("Terminal", &[("Terminal", "gnome-terminal", true)]);
    assert!(!post.window_visible_for_app("file manager"));

    let running = vec!["nautilus".to_string(), "gnome-terminal".to_string()];
    let proc_evidence = app_process_running("file manager", &running);
    assert_eq!(proc_evidence.as_deref(), Some("nautilus"));

    let req = open_app_request("file manager");
    let result = verify_post_action_detailed_with_process(
        &req,
        &pre,
        &post,
        true,
        None,
        2_000,
        proc_evidence.as_deref(),
    );
    assert_eq!(result.status, VERIFICATION_VERIFIED);
    assert!(
        result.evidence.iter().any(|e| e.contains("app_running:nautilus")),
        "evidence must name the matched process: {:?}",
        result.evidence
    );
}

#[test]
fn open_app_verified_via_process_for_calculator_alias() {
    let post = snapshot("Terminal", &[("Terminal", "gnome-terminal", true)]);
    let pre = snapshot("Terminal", &[]);
    assert!(!post.window_visible_for_app("calculator"));

    let running = vec!["gnome-calculator".to_string()];
    let proc_evidence = app_process_running("calculator", &running);
    let req = open_app_request("calculator");
    let result = verify_post_action_detailed_with_process(
        &req,
        &pre,
        &post,
        true,
        None,
        2_000,
        proc_evidence.as_deref(),
    );
    assert_eq!(result.status, VERIFICATION_VERIFIED);
    assert!(result
        .evidence
        .iter()
        .any(|e| e.contains("app_running:gnome-calculator")));
}

// ── No window AND no process → honest non-verified (never a false verified) ──

#[test]
fn open_app_not_verified_when_neither_window_nor_process() {
    let post = snapshot("Terminal", &[("Terminal", "gnome-terminal", true)]);
    let pre = snapshot("Terminal", &[]);
    assert!(!post.window_visible_for_app("file manager"));

    // Process list does NOT contain a file manager.
    let running = vec!["gnome-terminal".to_string(), "bash".to_string()];
    let proc_evidence = app_process_running("file manager", &running);
    assert!(proc_evidence.is_none());

    let req = open_app_request("file manager");
    let result = verify_post_action_detailed_with_process(
        &req,
        &pre,
        &post,
        true,
        None,
        2_000,
        proc_evidence.as_deref(),
    );
    assert_ne!(
        result.status, VERIFICATION_VERIFIED,
        "must not fabricate a verified verdict without window or process evidence"
    );
}

// ── Flag OFF ⇒ no process check; prior predicate/behavior byte-for-byte ──────

#[test]
fn flag_off_open_app_predicate_is_unchanged_and_no_process_check() {
    // With the flag OFF, the OpenApp predicate stays `active_window_match` — the
    // prior behavior — and is byte-for-byte identical to the unflagged selector.
    assert_eq!(
        select_verification_strategy_with_flag(&GuiActionKind::OpenApp, false, false),
        select_verification_strategy(&GuiActionKind::OpenApp, false)
    );
    assert_eq!(
        select_verification_strategy_with_flag(&GuiActionKind::OpenApp, false, false),
        GuiVerificationStrategy::ActiveWindowMatch
    );
}

#[test]
fn verify_without_process_equals_legacy_call_byte_for_byte() {
    // The process-aware entrypoint with `None` evidence is byte-for-byte equal
    // to the legacy `verify_post_action_detailed` (the flag-OFF path supplies no
    // process evidence), so existing behavior is preserved exactly.
    let post = snapshot("Files", &[("Files", "org.gnome.Nautilus", true)]);
    let pre = snapshot("Files", &[]);
    let req = open_app_request("Chrome");

    let legacy = verify_post_action_detailed(&req, &pre, &post, true, None, 2_000);
    let with_none =
        verify_post_action_detailed_with_process(&req, &pre, &post, true, None, 2_000, None);
    assert_eq!(legacy, with_none);
    // And no process evidence + absent window => not a false verified.
    assert_ne!(with_none.status, VERIFICATION_VERIFIED);
    // Without a window the honest verdict is non-verified (here failed/inconclusive).
    assert!(
        with_none.status == VERIFICATION_INCONCLUSIVE
            || with_none.status != VERIFICATION_VERIFIED
    );
}
