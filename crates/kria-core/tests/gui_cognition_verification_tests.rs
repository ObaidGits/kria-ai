use kria_core::agent::gui_cognition::executor::{stable_target_identity_hash, GuiActionKind};
use kria_core::agent::gui_cognition::perception::{
    GuiAccessibilitySummary, GuiActiveWindowSummary, GuiBounds, GuiControlSummary,
    GuiCursorFocusSummary, GuiObservationCacheSummary, GuiObservationSnapshot,
    GuiObservationTimingSummary, GuiOcrBlock, GuiOcrDiagnostics, GuiPerceptionCapabilities,
    GuiSourceStatus,
};
use kria_core::agent::gui_cognition::verifier::{
    select_verification_strategy, verify_post_action_detailed, GuiPostActionVerificationRequest,
    GuiVerificationStrategy,
};

fn control(role: &str, name: &str, focused: bool) -> GuiControlSummary {
    let mut control = GuiControlSummary::new(role, name, format!("/root/{role}/{name}"));
    control.bounds = Some(GuiBounds {
        x: 10,
        y: 20,
        width: 120,
        height: 30,
    });
    control.focused = focused;
    control.identity_confidence = 0.9;
    control.bounds_confidence = 0.9;
    control.state_confidence = 0.9;
    control.executable_confidence = 0.9;
    control.quality = "trusted".into();
    control
}

struct Snap {
    app: String,
    screen_hash: Option<String>,
    text_fields: Vec<GuiControlSummary>,
    buttons: Vec<GuiControlSummary>,
    dialogs: Vec<GuiControlSummary>,
    focused_control_id: Option<String>,
    focused_control_label: Option<String>,
    focused_control_role: Option<String>,
    keyboard_focus_known: bool,
    ocr_text: Vec<String>,
}

impl Snap {
    fn new(app: &str) -> Self {
        Self {
            app: app.into(),
            screen_hash: Some("screen-a".into()),
            text_fields: Vec::new(),
            buttons: Vec::new(),
            dialogs: Vec::new(),
            focused_control_id: None,
            focused_control_label: None,
            focused_control_role: None,
            keyboard_focus_known: false,
            ocr_text: Vec::new(),
        }
    }

    fn build(self, observation_id: &str) -> GuiObservationSnapshot {
        let control_count = self.text_fields.len() + self.buttons.len() + self.dialogs.len();
        GuiObservationSnapshot {
            observation_id: observation_id.into(),
            context_id: format!("ctx-{observation_id}"),
            timestamp_ms: 1,
            screen_hash: self.screen_hash.clone(),
            active_window_label: self.app.clone(),
            active_window: GuiActiveWindowSummary {
                label: self.app.clone(),
                app_name: Some(self.app.clone()),
                source: "test".into(),
                confidence: 0.95,
                fallback_used: false,
                blocker: None,
                reliability: "reliable".into(),
                fallback_chain: Vec::new(),
                ..GuiActiveWindowSummary::default()
            },
            visible_windows: Vec::new(),
            visible_app_count: 2,
            monitors: Vec::new(),
            cursor_focus: GuiCursorFocusSummary {
                cursor_x: None,
                cursor_y: None,
                focused_control_id: self.focused_control_id.clone(),
                focused_window_label: Some(self.app.clone()),
                keyboard_focus_known: self.keyboard_focus_known,
                source: "test".into(),
                focused_app: Some(self.app.clone()),
                focused_control_label: self.focused_control_label.clone(),
                focused_control_role: self.focused_control_role.clone(),
                focused_control_bounds: None,
                text_cursor_known: false,
                editable_target_known: false,
                terminal_like: false,
                ..GuiCursorFocusSummary::default()
            },
            accessibility: GuiAccessibilitySummary {
                available: true,
                node_count: control_count,
                control_count,
                omitted_node_count: 0,
                enabled_control_count: control_count,
                disabled_control_count: 0,
                visible_control_count: control_count,
                focused_control_count: 0,
                source: "test".into(),
                source_status: "healthy".into(),
                snapshot_total_ms: Some(12),
                skipped_app_count: 0,
                remediation: Vec::new(),
                ..GuiAccessibilitySummary::default()
            },
            ocr_blocks: self
                .ocr_text
                .iter()
                .enumerate()
                .map(|(idx, text)| GuiOcrBlock {
                    block_id: format!("ocr-{idx}"),
                    safe_text_preview: text.clone(),
                    text_hash: format!("hash-{idx}"),
                    bounds: None,
                    confidence: 0.6,
                    untrusted: true,
                    injection_suspected: false,
                    redaction_applied: false,
                })
                .collect(),
            ocr_diagnostics: GuiOcrDiagnostics::default(),
            capabilities: GuiPerceptionCapabilities {
                active_window: GuiSourceStatus::available("test"),
                desktop_state: GuiSourceStatus::available("test"),
                accessibility: GuiSourceStatus::available("test"),
                screenshot: GuiSourceStatus::available("test"),
                ocr: GuiSourceStatus::available("test"),
                monitor: GuiSourceStatus::blocked("test", "monitor unavailable"),
                cursor_focus: GuiSourceStatus::available("test"),
            },
            accessibility_ok: true,
            ocr_available: true,
            screenshot_available: true,
            active_window_probe_ok: true,
            desktop_state_probe_ok: true,
            capabilities_probe_ok: true,
            text_fields: self.text_fields,
            buttons: self.buttons,
            dialogs: self.dialogs,
            other_controls: Vec::new(),
            visual_controls: Vec::new(),
            timing: GuiObservationTimingSummary::default(),
            cache: GuiObservationCacheSummary::default(),
        }
    }
}

fn request(
    action_type: &str,
    strategy: GuiVerificationStrategy,
    is_secret: bool,
) -> GuiPostActionVerificationRequest {
    GuiPostActionVerificationRequest {
        verification_id: "verification-1".into(),
        execution_id: "execution-1".into(),
        proposal_id: "proposal-1".into(),
        proposal_hash: "proposal-hash".into(),
        action_type: action_type.into(),
        target_hash: "target-hash".into(),
        stable_target_identity_hash: None,
        expected_postcondition: "expected result visible".into(),
        verification_strategy: strategy.as_str().into(),
        pre_action_context_id: "ctx-pre".into(),
        post_action_observation_id: "obs-post".into(),
        post_action_context_id: "ctx-post".into(),
        started_at_ms: 1_000,
        is_secret_payload: is_secret,
        prompt_hash: "prompt-hash".into(),
        target_label: None,
        target_role: None,
        target_control_id: None,
        expected_app_hint: None,
        expected_window_hint: None,
    }
}

#[test]
fn open_app_window_visible_passes_when_window_matches() {
    assert_eq!(
        select_verification_strategy(&GuiActionKind::OpenApp, false),
        GuiVerificationStrategy::ActiveWindowMatch
    );
    let pre = Snap::new("Desktop").build("obs-pre");
    let post = Snap::new("Google Search - Chrome").build("obs-post");
    let mut req = request("OpenApp", GuiVerificationStrategy::ActiveWindowMatch, false);
    req.expected_app_hint = Some("Chrome".into());

    let result = verify_post_action_detailed(&req, &pre, &post, true, None, 2_000);
    assert_eq!(result.status, "verified");
    assert!(result.matched_expected_state);
    assert!(result.confidence >= 0.85);
}

#[test]
fn open_app_window_visible_fails_when_window_missing() {
    let pre = Snap::new("Desktop").build("obs-pre");
    let post = Snap::new("Some Other App").build("obs-post");
    let mut req = request("OpenApp", GuiVerificationStrategy::ActiveWindowMatch, false);
    req.expected_app_hint = Some("Chrome".into());

    let result = verify_post_action_detailed(&req, &pre, &post, true, None, 2_000);
    assert_eq!(result.status, "verification_failed");
    assert!(!result.matched_expected_state);
    assert!(result.recovery_hint.is_some());
}

#[test]
fn focus_field_passes_when_focused_control_matches() {
    let pre = Snap::new("Editor").build("obs-pre");
    let mut post = Snap::new("Editor");
    post.text_fields = vec![control("text", "Search", true)];
    post.focused_control_label = Some("Search".into());
    post.keyboard_focus_known = true;
    let post = post.build("obs-post");

    let mut req = request("FocusField", GuiVerificationStrategy::FocusedControl, false);
    req.target_label = Some("Search".into());
    req.target_role = Some("text".into());

    let result = verify_post_action_detailed(&req, &pre, &post, true, None, 2_000);
    assert_eq!(result.status, "verified");
    assert!(result.target_still_present);
}

#[test]
fn focus_field_fails_when_focus_moves_elsewhere() {
    let pre = Snap::new("Editor").build("obs-pre");
    let mut post = Snap::new("Editor");
    post.text_fields = vec![control("text", "Search", false)];
    post.focused_control_label = Some("Other Field".into());
    post.keyboard_focus_known = true;
    let post = post.build("obs-post");

    let mut req = request("FocusField", GuiVerificationStrategy::FocusedControl, false);
    req.target_label = Some("Search".into());
    req.target_role = Some("text".into());

    let result = verify_post_action_detailed(&req, &pre, &post, true, None, 2_000);
    assert_eq!(result.status, "verification_failed");
}

#[test]
fn type_text_passes_when_expected_text_present() {
    assert_eq!(
        select_verification_strategy(&GuiActionKind::TypeText, false),
        GuiVerificationStrategy::TextPresent
    );
    let pre = Snap::new("Editor").build("obs-pre");
    let mut post = Snap::new("Editor");
    post.text_fields = vec![control("text", "hello world", true)];
    let post = post.build("obs-post");

    let req = request("TypeText", GuiVerificationStrategy::TextPresent, false);
    let result = verify_post_action_detailed(&req, &pre, &post, true, Some("hello"), 2_000);
    assert_eq!(result.status, "verified");
}

#[test]
fn type_text_fails_when_expected_text_absent() {
    let pre = Snap::new("Editor").build("obs-pre");
    let mut post = Snap::new("Editor");
    post.text_fields = vec![control("text", "nothing here", true)];
    let post = post.build("obs-post");

    let req = request("TypeText", GuiVerificationStrategy::TextPresent, false);
    let result = verify_post_action_detailed(&req, &pre, &post, true, Some("hello"), 2_000);
    assert_eq!(result.status, "verification_failed");
}

#[test]
fn secret_type_uses_state_changed_and_never_emits_text() {
    assert_eq!(
        select_verification_strategy(&GuiActionKind::TypeText, true),
        GuiVerificationStrategy::StateChanged
    );
    let pre = Snap::new("Editor").build("obs-pre");
    let mut post = Snap::new("Editor");
    post.screen_hash = Some("screen-b".into());
    let post = post.build("obs-post");

    let req = request("TypeText", GuiVerificationStrategy::StateChanged, true);
    // Secret callers pass expected_text = None.
    let result = verify_post_action_detailed(&req, &pre, &post, true, None, 2_000);
    assert_eq!(result.status, "verified");
    assert_eq!(result.verification_strategy, "state_changed");
    let serialized = serde_json::to_string(&result.summary_json()).unwrap();
    assert!(!serialized.contains("SECRET"));
    assert!(!serialized.to_lowercase().contains("password"));
}

#[test]
fn click_passes_on_screen_change_and_fails_on_unchanged_screen() {
    assert_eq!(
        select_verification_strategy(&GuiActionKind::ClickControl, false),
        GuiVerificationStrategy::ResultVisible
    );
    let pre = Snap::new("App").build("obs-pre");

    let mut changed = Snap::new("App");
    changed.screen_hash = Some("screen-b".into());
    let changed = changed.build("obs-post");
    let req = request("ClickControl", GuiVerificationStrategy::ResultVisible, false);
    let ok = verify_post_action_detailed(&req, &pre, &changed, true, None, 2_000);
    assert_eq!(ok.status, "verified");

    let unchanged = Snap::new("App").build("obs-post");
    let failed = verify_post_action_detailed(&req, &pre, &unchanged, true, None, 2_000);
    assert_eq!(failed.status, "verification_failed");
}

#[test]
fn copy_reports_clipboard_changed_without_clipboard_content() {
    assert_eq!(
        select_verification_strategy(&GuiActionKind::Copy, false),
        GuiVerificationStrategy::ClipboardChanged
    );
    let pre = Snap::new("App").build("obs-pre");
    let post = Snap::new("App").build("obs-post");
    let req = request("Copy", GuiVerificationStrategy::ClipboardChanged, false);
    let result = verify_post_action_detailed(&req, &pre, &post, true, None, 2_000);
    assert_eq!(result.status, "verified");
    assert_eq!(result.verification_strategy, "clipboard_changed");
    let serialized = serde_json::to_string(&result.summary_json()).unwrap();
    assert!(serialized.contains("clipboard"));
    // No clipboard value is ever echoed.
    assert!(!serialized.contains("clipboard_value"));
}

#[test]
fn backend_failure_blocks_verification_with_no_blind_success() {
    let pre = Snap::new("App").build("obs-pre");
    let post = Snap::new("App").build("obs-post");
    let req = request("ClickControl", GuiVerificationStrategy::ResultVisible, false);
    let result = verify_post_action_detailed(&req, &pre, &post, false, None, 2_000);
    assert_eq!(result.status, "blocked");
    assert!(!result.matched_expected_state);
    assert!(result.safe_error_summary.is_some());
}

#[test]
fn screen_change_unobservable_is_inconclusive_not_success() {
    let mut pre = Snap::new("App");
    pre.screen_hash = None;
    let pre = pre.build("obs-pre");
    let mut post = Snap::new("App");
    post.screen_hash = None;
    let post = post.build("obs-post");

    let req = request("Scroll", GuiVerificationStrategy::ScreenChanged, false);
    let result = verify_post_action_detailed(&req, &pre, &post, true, None, 2_000);
    assert_eq!(result.status, "inconclusive");
    assert!(!result.matched_expected_state);
}

#[test]
fn control_action_fails_when_bound_target_identity_changes() {
    let pre = Snap::new("App").build("obs-pre");
    let mut post = Snap::new("App");
    post.screen_hash = Some("screen-b".into());
    post.buttons = vec![control("push button", "Search", false)];
    let post = post.build("obs-post");

    let mut req = request("ClickControl", GuiVerificationStrategy::ResultVisible, false);
    req.target_control_id = Some("control-does-not-exist".into());
    req.target_label = Some("Missing".into());
    req.stable_target_identity_hash = Some(stable_target_identity_hash(
        Some("control-does-not-exist"),
        Some("push button"),
        Some("Missing"),
        None,
        None,
        None,
    ));

    let result = verify_post_action_detailed(&req, &pre, &post, true, None, 2_000);
    assert_eq!(result.status, "verification_failed");
    assert!(!result.target_still_present);
}
