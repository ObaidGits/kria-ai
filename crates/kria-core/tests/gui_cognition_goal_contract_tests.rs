use kria_core::agent::gui_cognition::context::{GuiContextBuildRequest, GuiContextBuilder};
use kria_core::agent::gui_cognition::goal_contract::{
    extract_gui_goal_contract, GuiActionType, GuiRiskLevel,
};
use kria_core::agent::gui_cognition::perception::{
    GuiAccessibilitySummary, GuiActiveWindowSummary, GuiBounds, GuiControlSummary,
    GuiCursorFocusSummary, GuiObservationCacheSummary, GuiObservationSnapshot,
    GuiObservationTimingSummary, GuiOcrBlock, GuiOcrDiagnostics, GuiPerceptionCapabilities,
    GuiSourceStatus,
};

fn control(role: &str, name: &str) -> GuiControlSummary {
    let mut control = GuiControlSummary::new(role, name, format!("/fixture/{role}/{name}"));
    control.bounds = Some(GuiBounds {
        x: 10,
        y: 20,
        width: 120,
        height: 30,
    });
    control.identity_confidence = 0.9;
    control.bounds_confidence = 0.9;
    control.state_confidence = 0.9;
    control.executable_confidence = 0.9;
    control.quality = "trusted".into();
    control
}

fn context_with_ocr(
    ocr_blocks: Vec<GuiOcrBlock>,
) -> kria_core::agent::gui_cognition::context::GuiContext {
    let text_fields = vec![control("text", "Search")];
    let buttons = vec![
        control("push button", "Search"),
        control("push button", "Submit"),
    ];
    let control_count = text_fields.len() + buttons.len();
    let observation = GuiObservationSnapshot {
        observation_id: "obs-goal".into(),
        context_id: "ctx-goal".into(),
        timestamp_ms: 1,
        screen_hash: Some("screen-hash".into()),
        active_window_label: "Kria Browser".into(),
        active_window: GuiActiveWindowSummary {
            label: "Kria Browser".into(),
            app_name: Some("Browser".into()),
            source: "fixture".into(),
            confidence: 0.93,
            fallback_used: false,
            blocker: None,
            reliability: "reliable".into(),
            fallback_chain: Vec::new(),
            ..GuiActiveWindowSummary::default()
        },
        visible_windows: Vec::new(),
        visible_app_count: 1,
        monitors: Vec::new(),
        cursor_focus: GuiCursorFocusSummary::default(),
        accessibility: GuiAccessibilitySummary {
            available: true,
            node_count: control_count,
            control_count,
            omitted_node_count: 0,
            enabled_control_count: control_count,
            disabled_control_count: 0,
            visible_control_count: control_count,
            focused_control_count: 0,
            source: "fixture".into(),
            source_status: "healthy".into(),
            snapshot_total_ms: Some(12),
            skipped_app_count: 0,
            remediation: Vec::new(),
            ..GuiAccessibilitySummary::default()
        },
        ocr_blocks,
        ocr_diagnostics: GuiOcrDiagnostics::default(),
        capabilities: GuiPerceptionCapabilities {
            active_window: GuiSourceStatus::available("fixture"),
            desktop_state: GuiSourceStatus::available("fixture"),
            accessibility: GuiSourceStatus::available("fixture"),
            screenshot: GuiSourceStatus::available("fixture"),
            ocr: GuiSourceStatus::available("fixture"),
            monitor: GuiSourceStatus::blocked("fixture", "monitor unavailable"),
            cursor_focus: GuiSourceStatus::blocked("fixture", "focus unavailable"),
        },
        accessibility_ok: true,
        ocr_available: true,
        screenshot_available: true,
        active_window_probe_ok: true,
        desktop_state_probe_ok: true,
        capabilities_probe_ok: true,
        text_fields,
        buttons,
        dialogs: Vec::new(),
        other_controls: Vec::new(),
        visual_controls: Vec::new(),
        timing: GuiObservationTimingSummary::default(),
        cache: GuiObservationCacheSummary::default(),
    };
    GuiContextBuilder::new().build(GuiContextBuildRequest::new(observation))
}

fn context() -> kria_core::agent::gui_cognition::context::GuiContext {
    context_with_ocr(Vec::new())
}

fn injection_block() -> GuiOcrBlock {
    GuiOcrBlock {
        block_id: "ocr-injection".into(),
        safe_text_preview: "[untrusted text redacted]".into(),
        text_hash: "ocr-hash".into(),
        bounds: None,
        confidence: 0.8,
        untrusted: true,
        injection_suspected: true,
        redaction_applied: true,
    }
}

#[test]
fn gui_goal_contract_serializes_and_deserializes() {
    let ctx = context();
    let contract = extract_gui_goal_contract("Observe the current screen.", Some(&ctx)).contract;
    let encoded = serde_json::to_string(&contract).expect("contract serializes");
    let decoded: kria_core::agent::gui_cognition::goal_contract::GuiGoalContract =
        serde_json::from_str(&encoded).expect("contract deserializes");

    assert_eq!(decoded.observation_id, "obs-goal");
    assert_eq!(decoded.context_id, "ctx-goal");
    assert_eq!(decoded.action_type, GuiActionType::Observe);
    assert!(!decoded.prompt_hash.is_empty());
    assert_eq!(decoded.ambiguities.len(), 0);

    let event = contract.event_payload();
    assert!(event["prompt_hash"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert_eq!(event["ambiguity_count"], 0);
    let response = contract.response_summary();
    assert_eq!(response["ambiguity_count"], 0);
}

#[test]
fn gui_goal_contract_extracts_observe_focus_type_and_click() {
    let ctx = context();

    let observe = extract_gui_goal_contract("Observe the current screen.", Some(&ctx)).contract;
    assert_eq!(observe.action_type, GuiActionType::Observe);
    assert_eq!(observe.intent_kind, "observe");
    assert!(observe.desired_final_state.contains("observed"));

    let focus =
        extract_gui_goal_contract("Focus the visible search/input field.", Some(&ctx)).contract;
    assert_eq!(focus.action_type, GuiActionType::FocusInput);
    assert_eq!(
        focus.target_control_hint.as_deref(),
        Some("visible text input")
    );
    assert!(focus.desired_final_state.contains("focused"));

    let typing = extract_gui_goal_contract(
        "Type \"KRIA GUI cognition test\" into the visible text field.",
        Some(&ctx),
    )
    .contract;
    assert_eq!(typing.action_type, GuiActionType::TypeText);
    assert!(typing
        .desired_final_state
        .contains("KRIA GUI cognition test"));
    assert!(typing.extraction_confidence > 0.7);

    let click =
        extract_gui_goal_contract("Click the visible button named Search.", Some(&ctx)).contract;
    assert_eq!(click.action_type, GuiActionType::ClickControl);
    assert_eq!(click.target_control_hint.as_deref(), Some("Search"));
    assert!(click.desired_final_state.contains("Search"));
}

#[test]
fn gui_goal_contract_extracts_browser_app_and_window_hints() {
    let ctx = context();

    let browser = extract_gui_goal_contract(
        "Plan how to open a browser and search for \"KRIA test\".",
        Some(&ctx),
    )
    .contract;
    assert_eq!(browser.action_type, GuiActionType::BrowserSearch);
    assert_eq!(browser.target_app_hint.as_deref(), Some("browser"));
    assert_eq!(browser.target_app_kind.as_deref(), Some("browser"));
    assert_eq!(browser.query_summary.as_deref(), Some("KRIA test"));
    assert!(browser
        .desired_final_state
        .contains("search results visible"));

    let app = extract_gui_goal_contract("Open app Firefox.", Some(&ctx)).contract;
    assert_eq!(app.action_type, GuiActionType::OpenApp);
    assert_eq!(app.target_app_kind.as_deref(), Some("browser"));
    assert_eq!(app.target_app_hint.as_deref(), Some("Firefox"));

    let window = extract_gui_goal_contract("Switch to current window.", Some(&ctx)).contract;
    assert_eq!(window.action_type, GuiActionType::SwitchWindow);
    assert_eq!(window.target_window_hint.as_deref(), Some("Kria Browser"));
}

/// Issue #3 / Task 2.3 (app inference from intent): an explicit single-window
/// utility named in the prompt ("calculator") must be extracted as the target
/// app and must WIN over the active-window context. The fixture's active window
/// is a browser ("Browser"); before the fix `resolve_app_hint` fell back to that
/// active window, so "Open the calculator" was poisoned into "observe the
/// browser is already open" (the live open-app miss). With the
/// `gui_cog_smart_planner` vocabulary (default-ON) the prompt's app wins.
#[test]
fn gui_goal_contract_calculator_intent_beats_active_window() {
    let ctx = context();

    let calc = extract_gui_goal_contract("Open the calculator", Some(&ctx)).contract;
    assert_eq!(calc.action_type, GuiActionType::OpenApp);
    assert_eq!(
        calc.target_app_kind.as_deref(),
        Some("calculator"),
        "the prompt's calculator app-kind must win over the active-window browser"
    );
    assert_eq!(
        calc.target_app_hint.as_deref(),
        Some("calculator"),
        "the prompt's app must be the target, never the active-window app"
    );
    // "calc" shorthand also resolves.
    let calc_short = extract_gui_goal_contract("Open calc", Some(&ctx)).contract;
    assert_eq!(calc_short.action_type, GuiActionType::OpenApp);
    assert_eq!(calc_short.target_app_hint.as_deref(), Some("calculator"));
}

#[test]
fn gui_goal_contract_normalizes_browser_search_variations() {
    let ctx = context();
    let cases = [
        ("Open Chrome and search for weather", "Chrome", "weather"),
        ("search weather in Chrome", "Chrome", "weather"),
        (
            "open browser and find today's weather",
            "browser",
            "today's weather",
        ),
        ("look up weather on Google", "Google", "weather"),
        ("Chrome me weather search karo", "Chrome", "weather"),
        ("Google pe weather dekho", "Google", "weather"),
        ("browser me KRIA find karo", "browser", "KRIA"),
    ];

    for (prompt, app, query) in cases {
        let contract = extract_gui_goal_contract(prompt, Some(&ctx)).contract;
        assert_eq!(
            contract.action_type,
            GuiActionType::BrowserSearch,
            "{prompt}"
        );
        assert_eq!(contract.intent_kind, "browser_search", "{prompt}");
        assert_eq!(
            contract.target_app_kind.as_deref(),
            Some("browser"),
            "{prompt}"
        );
        assert_eq!(contract.target_app_hint.as_deref(), Some(app), "{prompt}");
        assert_eq!(contract.query_summary.as_deref(), Some(query), "{prompt}");
        assert!(contract
            .query_hash
            .as_deref()
            .is_some_and(|value| !value.is_empty()));
        assert_eq!(contract.risk_level, GuiRiskLevel::Low, "{prompt}");
        assert!(!contract.requires_user_approval, "{prompt}");
        assert!(
            contract.extraction_confidence >= 0.90,
            "{prompt}: {}",
            contract.extraction_confidence
        );
    }
}

#[test]
fn gui_goal_contract_does_not_treat_negated_submit_as_risky_intent() {
    let ctx = context();
    let contract = extract_gui_goal_contract(
        "Plan how to open a browser and search for \"KRIA test\", but do not perform any risky external submit.",
        Some(&ctx),
    )
    .contract;

    assert_eq!(contract.action_type, GuiActionType::BrowserSearch);
    assert_eq!(contract.query_summary.as_deref(), Some("KRIA test"));
    assert_eq!(contract.risk_level, GuiRiskLevel::Low);
    assert!(!contract.requires_user_approval);
}

#[test]
fn gui_goal_contract_extracts_action_coverage_and_unknown() {
    let ctx = context();
    let cases = [
        (
            "Navigate to https://example.com in the browser.",
            GuiActionType::BrowserNavigate,
        ),
        ("Save this note in the current app.", GuiActionType::Save),
        ("Download the visible report.", GuiActionType::Download),
        ("Copy the selected text.", GuiActionType::CopyContent),
        ("Paste into the visible field.", GuiActionType::PasteContent),
        ("Do something with this page.", GuiActionType::Unknown),
    ];

    for (prompt, expected) in cases {
        let contract = extract_gui_goal_contract(prompt, Some(&ctx)).contract;
        assert_eq!(contract.action_type, expected, "{prompt}");
    }

    let unknown = extract_gui_goal_contract("Do something with this page.", Some(&ctx)).contract;
    assert_eq!(unknown.intent_kind, "unknown");
    assert!(unknown.extraction_confidence < 0.50);
    assert!(unknown
        .ambiguities
        .iter()
        .any(|item| item.kind == "unsupported_goal"));
}

#[test]
fn gui_goal_contract_reports_browser_search_ambiguity() {
    let ctx = context();

    let missing_query = extract_gui_goal_contract("Search in Chrome.", Some(&ctx)).contract;
    assert_eq!(missing_query.action_type, GuiActionType::BrowserSearch);
    assert!(missing_query.query_summary.is_none());
    assert!(missing_query
        .ambiguities
        .iter()
        .any(|item| item.kind == "missing_query"));

    let multiple_apps =
        extract_gui_goal_contract("Search weather in Chrome and Firefox.", Some(&ctx)).contract;
    assert_eq!(multiple_apps.action_type, GuiActionType::BrowserSearch);
    assert_eq!(multiple_apps.query_summary.as_deref(), Some("weather"));
    assert!(multiple_apps
        .ambiguities
        .iter()
        .any(|item| item.kind == "multiple_app_targets"));
    assert!(multiple_apps.extraction_confidence < 0.90);
}

#[test]
fn gui_goal_contract_marks_risk_and_ambiguity() {
    let ctx = context();

    let risky = extract_gui_goal_contract("Click the Submit button.", Some(&ctx)).contract;
    assert_eq!(risky.action_type, GuiActionType::ClickControl);
    assert_eq!(risky.risk_level, GuiRiskLevel::High);
    assert!(risky.requires_user_approval);
    assert!(risky
        .ambiguities
        .iter()
        .any(|item| item.kind == "risky_without_explicit_approval_language"));

    let payment =
        extract_gui_goal_contract("Prepare to pay for this booking.", Some(&ctx)).contract;
    assert_eq!(payment.risk_level, GuiRiskLevel::Critical);
    assert!(payment.requires_user_approval);

    for prompt in [
        "Install this extension.",
        "Change the system setting.",
        "Update the security setting.",
        "Run git push.",
        "Confirm order.",
    ] {
        let contract = extract_gui_goal_contract(prompt, Some(&ctx)).contract;
        assert_eq!(contract.risk_level, GuiRiskLevel::High, "{prompt}");
        assert!(contract.requires_user_approval, "{prompt}");
    }

    let missing_text =
        extract_gui_goal_contract("Type into the visible text field.", Some(&ctx)).contract;
    assert_eq!(missing_text.action_type, GuiActionType::TypeText);
    assert!(missing_text
        .ambiguities
        .iter()
        .any(|item| item.kind == "missing_text_payload"));

    let missing_target =
        extract_gui_goal_contract("Click the visible button.", Some(&ctx)).contract;
    assert_eq!(missing_target.action_type, GuiActionType::ClickControl);
    assert!(missing_target
        .ambiguities
        .iter()
        .any(|item| item.kind == "missing_target_control"));
    assert!(missing_target.extraction_confidence < risky.extraction_confidence);
}

#[test]
fn gui_goal_contract_redacts_inline_natural_language_credential() {
    // Natural-language "type the password <secret> into ..." (whitespace, not
    // key=value) must not echo the secret into the contract/events
    // (Requirement 5.10 / Property 7).
    const SECRET: &str = "hunter2-topsecret-token";
    let contract = extract_gui_goal_contract(
        &format!("Type the password {SECRET} into the search field"),
        None,
    )
    .contract;
    let serialized = serde_json::to_string(&contract).expect("contract serializes");
    assert!(!serialized.contains(SECRET), "secret leaked: {serialized}");
    assert!(
        contract
            .text_payload_summary
            .as_deref()
            .unwrap_or("")
            .contains("[redacted]"),
        "payload should be redacted: {:?}",
        contract.text_payload_summary
    );
    assert!(
        contract
            .source_evidence
            .iter()
            .all(|item| !item.summary.contains(SECRET)),
        "source evidence leaked the secret"
    );
}

#[test]
fn gui_goal_contract_redacts_secrets_and_ignores_ocr_injection() {
    let ctx = context_with_ocr(vec![injection_block()]);

    let secret = extract_gui_goal_contract(
        "Type \"password=abc123 token=secret\" into the visible text field.",
        Some(&ctx),
    )
    .contract;
    let serialized = serde_json::to_string(&secret).expect("contract serializes");
    assert!(!serialized.contains("abc123"));
    assert!(!serialized.contains("secret\""));
    assert!(serialized.contains("[redacted]"));
    assert!(secret
        .text_payload_summary
        .as_deref()
        .unwrap_or("")
        .contains("[redacted]"));
    assert!(secret
        .source_evidence
        .iter()
        .all(|item| !item.summary.contains("abc123")));

    let injection = extract_gui_goal_contract("Observe the current screen.", Some(&ctx)).contract;
    assert_eq!(injection.action_type, GuiActionType::Observe);
    assert!(injection
        .ambiguities
        .iter()
        .any(|item| item.kind == "untrusted_ocr_present"));
    assert!(!serde_json::to_string(&injection)
        .unwrap()
        .to_lowercase()
        .contains("click delete"));
}

#[test]
fn gui_goal_contract_never_serializes_raw_prompt() {
    let ctx = context();
    let prompt = "Do something with this page RAW_PROMPT_SHOULD_NOT_LEAK";
    let contract = extract_gui_goal_contract(prompt, Some(&ctx)).contract;
    let event = contract.event_payload();
    let response = contract.response_summary();
    let event_json = serde_json::to_string(&event).unwrap();
    let response_json = serde_json::to_string(&response).unwrap();

    assert!(event_json.contains("prompt_hash"));
    assert!(!event_json.contains("RAW_PROMPT_SHOULD_NOT_LEAK"));
    assert!(!response_json.contains("RAW_PROMPT_SHOULD_NOT_LEAK"));
}

#[test]
fn gui_goal_contract_routes_new_primitive_families() {
    // Task 2.4: extraction routes each new primitive prompt to its action type so
    // the deterministic planner can build a complete typed sequence. Data-driven
    // keyword routing — no per-app hardcoding.
    let ctx = context();
    let cases = [
        ("Clear the search field.", GuiActionType::ClearField),
        ("Select all the text in the editor.", GuiActionType::SelectAll),
        ("Scroll down the page.", GuiActionType::Scroll),
        (
            "Check the Remember me checkbox.",
            GuiActionType::SetCheckbox,
        ),
        ("Close the dialog.", GuiActionType::CloseDialog),
        ("Press Enter.", GuiActionType::PressKey),
        (
            "Search the settings for display options.",
            GuiActionType::InAppSearch,
        ),
        (
            "Verify that dark mode is enabled and then stop.",
            GuiActionType::VerifyAndStop,
        ),
    ];

    for (prompt, expected) in cases {
        let contract = extract_gui_goal_contract(prompt, Some(&ctx)).contract;
        assert_eq!(contract.action_type, expected, "{prompt}");
    }
}

#[test]
fn gui_goal_contract_does_not_misroute_existing_search_and_copy_prompts() {
    // Regression guard for Task 2.4 routing: the new clear/select detections must
    // not steal browser-search or copy prompts.
    let ctx = context();

    let browser = extract_gui_goal_contract("Search weather in Chrome.", Some(&ctx)).contract;
    assert_eq!(browser.action_type, GuiActionType::BrowserSearch);

    let copy = extract_gui_goal_contract("Copy the selected text.", Some(&ctx)).contract;
    assert_eq!(copy.action_type, GuiActionType::CopyContent);

    let focus =
        extract_gui_goal_contract("Focus the visible search/input field.", Some(&ctx)).contract;
    assert_eq!(focus.action_type, GuiActionType::FocusInput);
}

// ── Task 8.2: cross-app clipboard combo recognition (gui_cog_crossapp) ───────

#[test]
fn task82_enrich_recognizes_cross_app_copy_paste_combo() {
    // "copy X from A and paste into B" across two DISTINCT apps → combo with the
    // SOURCE app (first mention) and TARGET app (last mention) threaded.
    let mut contract = extract_gui_goal_contract(
        "Copy the selected text from Chrome and paste it into VS Code",
        None,
    )
    .contract;
    // Pre-enrich: the field is None (the flag-OFF path leaves the contract
    // unchanged), so the action_type stays the single CopyContent primitive.
    assert!(contract.cross_app_clipboard.is_none());
    assert_eq!(contract.action_type, GuiActionType::CopyContent);

    // The runtime calls this only when the gui_cog_crossapp flag is ON.
    contract.enrich_cross_app_clipboard("Copy the selected text from Chrome and paste it into VS Code");
    let combo = contract
        .cross_app_clipboard
        .as_ref()
        .expect("a cross-app combo must be recognized");
    assert_eq!(combo.source_app_kind.as_deref(), Some("browser"));
    assert_eq!(combo.source_app_hint.as_deref(), Some("Chrome"));
    assert_eq!(combo.target_app_kind.as_deref(), Some("editor"));
    assert_eq!(combo.target_app_hint.as_deref(), Some("VS Code"));
    // action_type is intentionally NOT changed by enrichment (the combo lives in
    // the planned steps), so no contract event shape changes.
    assert_eq!(contract.action_type, GuiActionType::CopyContent);
}

#[test]
fn task82_single_copy_and_same_app_are_not_combos() {
    // A single copy (no paste) is NOT a cross-app combo.
    let mut single = extract_gui_goal_contract("Copy the selected text.", None).contract;
    single.enrich_cross_app_clipboard("Copy the selected text.");
    assert!(single.cross_app_clipboard.is_none());

    // Copy and paste WITHIN one app (no two distinct app endpoints) is NOT a
    // cross-app combo.
    let mut same_app =
        extract_gui_goal_contract("Copy and paste inside the editor", None).contract;
    same_app.enrich_cross_app_clipboard("Copy and paste inside the editor");
    assert!(same_app.cross_app_clipboard.is_none());
}

// ── Task 8.3: file-manager select flow recognition (gui_cog_crossapp) ────────

#[test]
fn task83_enrich_recognizes_file_manager_select_newest_flow() {
    // "open the file manager and select the newest file and tell me its name" →
    // a NON-DESTRUCTIVE file-manager select flow with the file-manager app hint
    // and a "newest" order/position selection threaded.
    let prompt = "Open the file manager and select the newest file and tell me its name";
    let mut contract = extract_gui_goal_contract(prompt, None).contract;
    // Pre-enrich (flag OFF path): the field is None and the contract is unchanged.
    assert!(contract.file_manager_select.is_none());

    // The runtime calls this only when the gui_cog_crossapp flag is ON.
    contract.enrich_file_manager_select(prompt);
    let flow = contract
        .file_manager_select
        .as_ref()
        .expect("a file-manager select flow must be recognized");
    assert_eq!(flow.app_kind.as_deref(), Some("file_manager"));
    assert_eq!(flow.app_hint.as_deref(), Some("file manager"));
    assert_eq!(flow.selection, "newest");
    assert_eq!(flow.selection_control_hint.as_deref(), Some("newest file entry"));
    // NON-DESTRUCTIVE: the contract must not be approval-gated or risky.
    assert!(!contract.requires_user_approval);
}

#[test]
fn task83_recognizes_first_file_and_folder_hint() {
    // "first" ordering + an explicit folder are both data-driven from the prompt.
    let prompt = "Open the file manager in the Downloads folder and select the first file and show its name";
    let mut contract = extract_gui_goal_contract(prompt, None).contract;
    contract.enrich_file_manager_select(prompt);
    let flow = contract
        .file_manager_select
        .as_ref()
        .expect("first-file flow must be recognized");
    assert_eq!(flow.selection, "first");
    assert_eq!(flow.folder_hint.as_deref(), Some("Downloads"));
}

#[test]
fn task83_destructive_request_never_rides_the_select_flow() {
    // A destructive verb (delete/move/rename) must route through the safety gate,
    // NEVER this non-destructive select flow.
    for prompt in [
        "Open the file manager and delete the newest file",
        "Open the file manager and move the newest file to trash",
        "Open the file manager and rename the first file",
    ] {
        let mut contract = extract_gui_goal_contract(prompt, None).contract;
        contract.enrich_file_manager_select(prompt);
        assert!(
            contract.file_manager_select.is_none(),
            "destructive prompt must not produce a select flow: {prompt}"
        );
    }
}

#[test]
fn task83_non_file_manager_or_no_ordering_is_not_a_select_flow() {
    // No file-manager mention → no flow.
    let mut no_fm =
        extract_gui_goal_contract("Select the newest file and tell me its name", None).contract;
    no_fm.enrich_file_manager_select("Select the newest file and tell me its name");
    assert!(no_fm.file_manager_select.is_none());

    // File manager but no ordering + no select/show intent → plain open, no flow.
    let mut plain_open =
        extract_gui_goal_contract("Open the file manager", None).contract;
    plain_open.enrich_file_manager_select("Open the file manager");
    assert!(plain_open.file_manager_select.is_none());
}

// ── Fix 1: trailing "type/write/enter <text>" payload capture (gui_cog_smart_planner) ─

#[test]
fn gui_goal_contract_captures_trailing_type_payload_after_open_and() {
    // "Open X and type Y" — the text payload follows a mid-sentence type verb,
    // which the destination-anchored patterns historically missed.
    let ctx = context();
    let contract =
        extract_gui_goal_contract("Open the text editor and type hello world", Some(&ctx)).contract;
    assert_eq!(contract.action_type, GuiActionType::TypeText);
    assert!(
        contract.target_app_kind.as_deref() == Some("editor")
            || contract.target_app_hint.as_deref() == Some("editor"),
        "editor app should be threaded: kind={:?} hint={:?}",
        contract.target_app_kind,
        contract.target_app_hint
    );
    assert_eq!(contract.text_payload_summary.as_deref(), Some("hello world"));
    assert!(contract.text_payload_hash.is_some());
}

#[test]
fn gui_goal_contract_captures_trailing_type_payload_for_notes() {
    let ctx = context();
    let contract =
        extract_gui_goal_contract("Open notes and type meeting at 3 PM", Some(&ctx)).contract;
    assert_eq!(contract.action_type, GuiActionType::TypeText);
    assert_eq!(
        contract.text_payload_summary.as_deref(),
        Some("meeting at 3 PM")
    );
}

#[test]
fn gui_goal_contract_trailing_capture_does_not_regress_into_clause() {
    // Already-working phrasing must still strip the trailing destination clause
    // and yield the same payload (action_type is unchanged existing behavior).
    let ctx = context();
    let contract =
        extract_gui_goal_contract("Type quarterly report into the search box", Some(&ctx))
            .contract;
    assert_eq!(
        contract.text_payload_summary.as_deref(),
        Some("quarterly report")
    );
}

#[test]
fn gui_goal_contract_trailing_capture_none_when_no_text() {
    // "open ... and type" with nothing after must stay clarifiable (payload None).
    let ctx = context();
    let contract =
        extract_gui_goal_contract("Open the text editor and type", Some(&ctx)).contract;
    assert!(contract.text_payload_summary.is_none());
    assert!(contract.text_payload_hash.is_none());
}

#[test]
fn gui_goal_contract_trailing_capture_flag_off_is_byte_for_byte() {
    // Pure, explicitly flag-gated helper: flag-OFF returns None (prior behavior),
    // flag-ON captures the trailing payload. Tested directly to avoid env races.
    use kria_core::agent::gui_cognition::goal_contract::capture_trailing_typed_payload;

    assert_eq!(
        capture_trailing_typed_payload("Open the text editor and type hello world", false),
        None,
        "flag-OFF must not capture (byte-for-byte with prior behavior)"
    );
    assert_eq!(
        capture_trailing_typed_payload("Open the text editor and type hello world", true)
            .as_deref(),
        Some("hello world"),
        "flag-ON captures the trailing payload"
    );
    // No-text and destination-only forms stay None under either flag state.
    assert_eq!(
        capture_trailing_typed_payload("Open the text editor and type", true),
        None
    );
    assert_eq!(
        capture_trailing_typed_payload("Type into the visible text field", true),
        None
    );
}

// ── Fix 2: FocusInput for "focus the <control> in/on the <app>" (gui_cog_smart_planner) ─

#[test]
fn gui_goal_contract_classifies_focus_control_in_app() {
    let ctx = context();
    let contract =
        extract_gui_goal_contract("Focus the address bar in the browser", Some(&ctx)).contract;
    assert_eq!(contract.action_type, GuiActionType::FocusInput);
    assert_eq!(contract.target_control_hint.as_deref(), Some("address bar"));
    assert_eq!(contract.target_app_kind.as_deref(), Some("browser"));
    assert_eq!(contract.target_app_hint.as_deref(), Some("browser"));
}

#[test]
fn gui_goal_contract_focus_visible_field_no_regression() {
    let ctx = context();
    let contract =
        extract_gui_goal_contract("Focus the visible search/input field.", Some(&ctx)).contract;
    assert_eq!(contract.action_type, GuiActionType::FocusInput);
    assert_eq!(
        contract.target_control_hint.as_deref(),
        Some("visible text input")
    );
}

// ── Task 4 (Issue #5): scroll DIRECTION threading (gui_cog_primitives) ────────

#[test]
fn gui_goal_contract_scroll_down_threads_down_direction() {
    // "Scroll down the current page" → direction down (default-ON flag).
    let ctx = context();
    let contract =
        extract_gui_goal_contract("Scroll down the current page", Some(&ctx)).contract;
    assert_eq!(contract.action_type, GuiActionType::Scroll);
    assert_eq!(contract.target_control_hint.as_deref(), Some("scroll:down"));
}

#[test]
fn gui_goal_contract_scroll_up_to_top_threads_top_or_up_direction() {
    // "Scroll up to the top of the page" → up/top.
    let ctx = context();
    let contract =
        extract_gui_goal_contract("Scroll up to the top of the page", Some(&ctx)).contract;
    assert_eq!(contract.action_type, GuiActionType::Scroll);
    assert!(
        matches!(
            contract.target_control_hint.as_deref(),
            Some("scroll:top") | Some("scroll:up")
        ),
        "expected scroll:top or scroll:up, got {:?}",
        contract.target_control_hint
    );
}

#[test]
fn gui_goal_contract_scroll_down_to_bottom_threads_bottom_or_down_direction() {
    // "Scroll down to the bottom" → bottom/down.
    let ctx = context();
    let contract =
        extract_gui_goal_contract("Scroll down to the bottom", Some(&ctx)).contract;
    assert_eq!(contract.action_type, GuiActionType::Scroll);
    assert!(
        matches!(
            contract.target_control_hint.as_deref(),
            Some("scroll:bottom") | Some("scroll:down")
        ),
        "expected scroll:bottom or scroll:down, got {:?}",
        contract.target_control_hint
    );
}

#[test]
fn gui_goal_contract_scroll_direction_flag_off_is_byte_for_byte() {
    // Pure, explicitly flag-gated helper: flag-OFF yields None (no direction
    // marker), which is byte-for-byte with the prior behavior; flag-ON yields the
    // marker. Tested directly to avoid process-global env races.
    use kria_core::agent::gui_cognition::goal_contract::scroll_direction_marker_for;

    assert_eq!(
        scroll_direction_marker_for("Scroll down the current page", false),
        None,
        "flag-OFF must not produce a direction marker (byte-for-byte)"
    );
    assert_eq!(
        scroll_direction_marker_for("Scroll up to the top of the page", false),
        None
    );
    // Flag-ON produces the marker.
    assert_eq!(
        scroll_direction_marker_for("Scroll down the current page", true).as_deref(),
        Some("scroll:down")
    );
    assert_eq!(
        scroll_direction_marker_for("Scroll up to the top of the page", true).as_deref(),
        Some("scroll:top")
    );
    assert_eq!(
        scroll_direction_marker_for("Scroll down to the bottom", true).as_deref(),
        Some("scroll:bottom")
    );
    assert_eq!(
        scroll_direction_marker_for("Scroll up", true).as_deref(),
        Some("scroll:up")
    );
    // Unknown direction defaults to down.
    assert_eq!(
        scroll_direction_marker_for("Scroll the page", true).as_deref(),
        Some("scroll:down")
    );
}
