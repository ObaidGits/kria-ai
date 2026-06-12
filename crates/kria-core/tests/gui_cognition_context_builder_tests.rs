use kria_core::agent::gui_cognition::context::{
    GuiContextBuildRequest, GuiContextBuilder, GuiContextFreshness,
};
use kria_core::agent::gui_cognition::perception::{
    GuiAccessibilitySummary, GuiActiveWindowSummary, GuiBounds, GuiControlSummary,
    GuiCursorFocusSummary, GuiMonitorSummary, GuiObservationCacheSummary, GuiObservationSnapshot,
    GuiObservationTimingSummary, GuiOcrBlock, GuiOcrDiagnostics, GuiPerceptionCapabilities,
    GuiSourceStatus,
};

fn control(
    role: &str,
    name: &str,
    enabled: bool,
    visible: bool,
    focused: bool,
    bounds: Option<GuiBounds>,
) -> GuiControlSummary {
    let mut control = GuiControlSummary::new(role, name, format!("/fixture/{role}/{name}"));
    control.enabled = enabled;
    control.visible = visible;
    control.focused = focused;
    control.bounds = bounds;
    control.in_active_window = true;
    control.source = "accessibility".into();
    control.confidence = 0.9;
    control.identity_confidence = if control.name.trim().is_empty() {
        0.35
    } else {
        0.9
    };
    control.bounds_confidence = if control.bounds.is_some() { 0.9 } else { 0.0 };
    control.state_confidence = 0.9;
    control.executable_confidence = if enabled && visible && control.bounds.is_some() {
        0.9
    } else {
        0.0
    };
    control.quality = if control.executable_confidence >= 0.75 {
        "trusted".into()
    } else {
        "not_executable".into()
    };
    control
}

fn bounds(x: i32, y: i32, width: i32, height: i32) -> GuiBounds {
    GuiBounds {
        x,
        y,
        width,
        height,
    }
}

fn monitor(id: &str, scale_factor: f64) -> GuiMonitorSummary {
    GuiMonitorSummary {
        id: id.into(),
        name: Some(format!("Monitor {id}")),
        bounds: bounds(0, 0, 1920, 1080),
        work_area: Some(bounds(0, 0, 1920, 1040)),
        scale_factor,
        primary: id == "0",
    }
}

fn ocr_block(
    preview: &str,
    injection_suspected: bool,
    redaction_applied: bool,
    bounds: Option<GuiBounds>,
) -> GuiOcrBlock {
    GuiOcrBlock {
        block_id: "ocr-1".into(),
        safe_text_preview: preview.into(),
        text_hash: "ocr-hash".into(),
        bounds,
        confidence: 0.88,
        untrusted: true,
        injection_suspected,
        redaction_applied,
    }
}

fn observation(
    context_id: &str,
    observation_id: &str,
    active_window: &str,
    screen_hash: &str,
    text_fields: Vec<GuiControlSummary>,
    buttons: Vec<GuiControlSummary>,
    ocr_blocks: Vec<GuiOcrBlock>,
    monitors: Vec<GuiMonitorSummary>,
    focused_control_id: Option<String>,
) -> GuiObservationSnapshot {
    let total_controls = text_fields.len() + buttons.len();
    let disabled = text_fields
        .iter()
        .chain(buttons.iter())
        .filter(|control| !control.enabled || !control.visible)
        .count();
    GuiObservationSnapshot {
        observation_id: observation_id.into(),
        context_id: context_id.into(),
        timestamp_ms: 10,
        screen_hash: Some(screen_hash.into()),
        active_window_label: active_window.into(),
        active_window: GuiActiveWindowSummary {
            label: active_window.into(),
            app_name: Some(active_window.into()),
            source: "fixture".into(),
            confidence: 0.94,
            fallback_used: false,
            blocker: None,
            reliability: "reliable".into(),
            fallback_chain: Vec::new(),
            ..GuiActiveWindowSummary::default()
        },
        visible_windows: Vec::new(),
        visible_app_count: 1,
        monitors,
        cursor_focus: GuiCursorFocusSummary {
            cursor_x: Some(20),
            cursor_y: Some(30),
            focused_control_id,
            focused_window_label: Some(active_window.into()),
            keyboard_focus_known: true,
            source: "fixture".into(),
            ..GuiCursorFocusSummary::default()
        },
        accessibility: GuiAccessibilitySummary {
            available: true,
            node_count: total_controls + 4,
            control_count: total_controls,
            omitted_node_count: 0,
            enabled_control_count: total_controls.saturating_sub(disabled),
            disabled_control_count: disabled,
            visible_control_count: total_controls.saturating_sub(disabled),
            focused_control_count: 1,
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
            active_window: GuiSourceStatus::available("fixture active window"),
            desktop_state: GuiSourceStatus::available("fixture desktop state"),
            accessibility: GuiSourceStatus::available("fixture accessibility"),
            screenshot: GuiSourceStatus::available("fixture screenshot"),
            ocr: GuiSourceStatus::available("fixture ocr"),
            monitor: GuiSourceStatus::available("fixture monitor"),
            cursor_focus: GuiSourceStatus::available("fixture cursor focus"),
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
    }
}

#[test]
fn gui_context_builder_serializes_and_preserves_rich_observation_state() {
    let context = GuiContextBuilder::new().build(GuiContextBuildRequest::new(observation(
        "ctx-1",
        "obs-1",
        "Kria",
        "abcdef0123456789",
        vec![control(
            "text",
            "Search",
            true,
            true,
            true,
            Some(bounds(10, 20, 120, 30)),
        )],
        vec![control(
            "push button",
            "Search",
            true,
            true,
            false,
            Some(bounds(140, 20, 80, 30)),
        )],
        vec![ocr_block(
            "Search",
            false,
            false,
            Some(bounds(10, 20, 120, 30)),
        )],
        vec![monitor("0", 1.25)],
        Some("field-1".into()),
    )));

    let encoded = serde_json::to_string(&context).expect("context serializes");
    let decoded: kria_core::agent::gui_cognition::context::GuiContext =
        serde_json::from_str(&encoded).expect("context deserializes");

    assert_eq!(decoded.context_id, "ctx-1");
    assert_eq!(decoded.observation_id, "obs-1");
    assert_eq!(decoded.active_window.label, "Kria");
    assert_eq!(
        decoded.visual_evidence.screen_hash_prefix.as_deref(),
        Some("abcdef0123456789")
    );
    assert_eq!(decoded.monitor_layout[0].scale_factor, 1.25);
    assert!(decoded.focus_state.keyboard_focus_known);
    assert_eq!(decoded.freshness, GuiContextFreshness::Fresh);
}

#[test]
fn gui_context_builder_only_makes_visible_enabled_accessibility_controls_executable() {
    let context = GuiContextBuilder::new().build(GuiContextBuildRequest::new(observation(
        "ctx-1",
        "obs-1",
        "Kria",
        "hash-1",
        vec![
            control(
                "text",
                "Search",
                true,
                true,
                false,
                Some(bounds(10, 20, 120, 30)),
            ),
            control("text", "Disabled", false, true, false, None),
            control("text", "Hidden", true, false, false, None),
        ],
        vec![control(
            "push button",
            "Search",
            true,
            true,
            false,
            Some(bounds(140, 20, 80, 30)),
        )],
        Vec::new(),
        vec![monitor("0", 1.0)],
        None,
    )));

    assert_eq!(context.text_field_count(), 3);
    assert_eq!(context.executable_text_fields().len(), 1);
    assert_eq!(context.executable_buttons().len(), 1);
    assert_eq!(context.accessibility_evidence.trusted_control_count, 2);
    assert_eq!(context.accessibility_evidence.executable_control_count, 2);
    assert_eq!(context.accessibility_evidence.disabled_or_hidden_count, 2);
}

#[test]
fn gui_context_builder_marks_ocr_untrusted_and_keeps_injection_non_executable() {
    let context = GuiContextBuilder::new().build(GuiContextBuildRequest::new(observation(
        "ctx-1",
        "obs-1",
        "Kria",
        "hash-1",
        Vec::new(),
        Vec::new(),
        vec![ocr_block(
            "[untrusted text redacted]",
            true,
            true,
            Some(bounds(0, 0, 10, 10)),
        )],
        vec![monitor("0", 1.0)],
        None,
    )));

    assert!(context.ocr_has_injection());
    assert_eq!(context.ocr_evidence.block_count, 1);
    assert_eq!(context.ocr_evidence.injection_count, 1);
    assert!(context.redaction_report.ocr_untrusted);
    assert_eq!(context.executable_control_count(), 0);
    let safe_summary = context.context_summary().to_string();
    assert!(!safe_summary
        .to_lowercase()
        .contains("ignore previous instructions"));
}

#[test]
fn gui_context_builder_detects_previous_context_delta_and_staleness() {
    let previous = GuiContextBuilder::new().build(GuiContextBuildRequest::new(observation(
        "ctx-prev",
        "obs-prev",
        "Kria",
        "hash-prev",
        vec![control("text", "Search", true, true, true, None)],
        Vec::new(),
        Vec::new(),
        vec![monitor("0", 1.0)],
        Some("field-prev".into()),
    )));
    let current_observation = observation(
        "ctx-current",
        "obs-current",
        "Browser",
        "hash-current",
        vec![
            control("text", "Search", true, true, false, None),
            control("text", "Filter", true, true, true, None),
        ],
        Vec::new(),
        Vec::new(),
        vec![monitor("1", 1.25)],
        Some("field-current".into()),
    );

    let current = GuiContextBuilder::new().build(GuiContextBuildRequest::with_previous(
        current_observation,
        previous,
    ));

    assert_eq!(current.freshness, GuiContextFreshness::Stale);
    assert_eq!(
        current.previous.previous_context_id.as_deref(),
        Some("ctx-prev")
    );
    assert!(current.previous.delta.active_window_changed);
    assert!(current.previous.delta.screen_hash_changed);
    assert!(current.previous.delta.monitor_layout_changed);
    assert!(current.previous.delta.control_count_changed);
    assert!(current.previous.delta.focused_control_changed);
    assert!(current.previous.delta.stale_action_risk);
}

#[test]
fn gui_context_builder_preserves_source_blockers_and_terminal_helper() {
    let mut obs = observation(
        "ctx-1",
        "obs-1",
        "Terminal",
        "hash-1",
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
    );
    obs.capabilities.ocr = GuiSourceStatus::blocked("fixture ocr", "ocr unavailable");
    obs.capabilities.screenshot =
        GuiSourceStatus::blocked("fixture screenshot", "screen capture denied");

    let context = GuiContextBuilder::new().build(GuiContextBuildRequest::new(obs));

    assert!(context.active_window_is_terminal_like());
    assert!(context
        .source_blockers()
        .iter()
        .any(|item| item.contains("ocr unavailable")));
    assert!(context
        .source_blockers()
        .iter()
        .any(|item| item.contains("screen capture denied")));
}
