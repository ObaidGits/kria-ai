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
