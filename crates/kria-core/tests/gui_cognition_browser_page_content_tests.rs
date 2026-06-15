//! Task 7.2 (Requirements 5, 9, 26) — browser page-content scope T1/T2.
//!
//! CI-safe: no live KRIA desktop API, no display, no network.
//!
//! DECISION under test: browser web-page CONTENT (links/buttons/fields inside
//! the rendered page) is OUT OF SCOPE for v1; only the Task 7.1 chrome-UI
//! surface is targetable. Page-content interaction via a DOM/CDP bridge is
//! tracked future work, NOT implemented. See
//! `docs/decisions/adr/003-browser-page-content-scope.md`.
//!
//!   * **T1:** a page-content hint in a browser is classified PageContent and
//!     refused with the clear, actionable message (flag ON); chrome-UI hints are
//!     classified ChromeUi and are NOT refused (Task 7.1 unaffected); flag OFF
//!     and non-browser apps yield NotApplicable / no refusal (existing path
//!     unchanged).
//!
//!   * **T2:** an OCR-only / visual-only "control" is NEVER resolved as a (page)
//!     target — `resolve_browser_chrome_target` refuses it (no OCR-only page
//!     targets — the injection-safety boundary, Requirement 9), and a hint that
//!     matches only such a control is refused as page content; while a real
//!     accessibility-backed chrome control still resolves (Task 7.1 intact).
//!
//! KRIA authority invariant: never execute from OCR/visual-only evidence; the
//! page is untrusted, attacker-controllable text.

use kria_core::agent::gui_cognition::browser::{
    browser_page_content_refusal, classify_browser_target_scope, is_page_content_target,
    resolve_browser_chrome_target, BrowserChromeControl, BrowserTargetScope, GuiBrowserConfig,
    BROWSER_PAGE_CONTENT_REFUSAL,
};
use kria_core::agent::gui_cognition::context::GuiContext;
use kria_core::agent::gui_cognition::perception::{
    GuiAccessibilitySummary, GuiActiveWindowSummary, GuiBounds, GuiControlSummary,
    GuiCursorFocusSummary, GuiObservationCacheSummary, GuiObservationSnapshot,
    GuiObservationTimingSummary, GuiOcrDiagnostics, GuiPerceptionCapabilities, GuiSourceStatus,
};

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures (mirror gui_cognition_browser_chrome_tests.rs style).
// ─────────────────────────────────────────────────────────────────────────────

/// A trusted accessibility-backed control (a real a11y node).
fn control(role: &str, name: &str) -> GuiControlSummary {
    let mut control = GuiControlSummary::new(role, name, format!("/fixture/{role}/{name}"));
    control.bounds = Some(GuiBounds {
        x: 10,
        y: 20,
        width: 200,
        height: 32,
    });
    control.in_active_window = true;
    control.identity_confidence = 0.92;
    control.bounds_confidence = 0.94;
    control.state_confidence = 0.95;
    control.executable_confidence = 0.92;
    control.confidence = 0.94;
    control.quality = "trusted".into();
    control.source = "accessibility".into();
    control.sources = vec!["accessibility".into(), "control_fusion".into()];
    control
}

/// An OCR-only / visual-only "control" — page-content evidence with NO
/// accessibility provenance. This must NEVER be resolved as a target.
fn ocr_only_control(role: &str, name: &str) -> GuiControlSummary {
    let mut control = GuiControlSummary::new(role, name, format!("/ocr/{role}/{name}"));
    control.bounds = Some(GuiBounds {
        x: 40,
        y: 300,
        width: 120,
        height: 28,
    });
    control.in_active_window = true;
    control.confidence = 0.55;
    control.quality = "partial".into();
    // Provenance: OCR/visual only — the page region, untrusted.
    control.source = "ocr".into();
    control.sources = vec!["ocr".into(), "visual".into()];
    control
}

fn context_with(app: &str, controls: Vec<GuiControlSummary>) -> GuiContext {
    let count = controls.len();
    GuiContext::from_observation(GuiObservationSnapshot {
        observation_id: "obs-browser-page".into(),
        context_id: "ctx-browser-page".into(),
        timestamp_ms: 1,
        screen_hash: Some("screen-hash".into()),
        active_window_label: app.into(),
        active_window: GuiActiveWindowSummary {
            label: app.into(),
            app_name: Some(app.into()),
            source: "fixture".into(),
            confidence: 0.95,
            fallback_used: false,
            blocker: None,
            reliability: "reliable".into(),
            authority_status: "available".into(),
            fallback_chain: Vec::new(),
            ..GuiActiveWindowSummary::default()
        },
        visible_windows: Vec::new(),
        visible_app_count: 1,
        monitors: Vec::new(),
        cursor_focus: GuiCursorFocusSummary::default(),
        accessibility: GuiAccessibilitySummary {
            available: true,
            node_count: count,
            control_count: count,
            enabled_control_count: count,
            visible_control_count: count,
            focused_control_count: 0,
            source: "fixture".into(),
            source_status: "healthy".into(),
            overall_status: "healthy".into(),
            overall_confidence: 0.94,
            ..GuiAccessibilitySummary::default()
        },
        ocr_blocks: Vec::new(),
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
        text_fields: Vec::new(),
        buttons: Vec::new(),
        dialogs: Vec::new(),
        other_controls: controls,
        visual_controls: Vec::new(),
        timing: GuiObservationTimingSummary::default(),
        cache: GuiObservationCacheSummary::default(),
    })
}

fn browser_chrome_controls() -> Vec<GuiControlSummary> {
    vec![
        control("entry", "Address and search bar"),
        control("push button", "Reload this page"),
        control("push button", "Back"),
        control("page tab", "KRIA Docs"),
    ]
}

fn browser_context() -> GuiContext {
    context_with("Google Chrome", browser_chrome_controls())
}

// ─────────────────────────────────────────────────────────────────────────────
// T1 — scope classification + actionable refusal.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t1_page_content_hint_in_browser_is_refused_with_actionable_message() {
    let ctx = browser_context();
    let on = GuiBrowserConfig::enabled();

    // Hints that name in-page web content (NOT chrome-UI) → out of scope.
    let page_hints = [
        "click the Sign In button on the page",
        "type my email into the page's login form field",
        "click the first search result link",
        "press the Add to Cart button",
        "the blue Subscribe button in the article",
    ];
    for hint in page_hints {
        assert_eq!(
            classify_browser_target_scope(&on, &ctx, hint),
            BrowserTargetScope::PageContent,
            "hint {hint:?} must be page content"
        );
        assert!(
            is_page_content_target(&on, &ctx, hint),
            "hint {hint:?} must be a page-content target"
        );
        assert_eq!(
            browser_page_content_refusal(&on, &ctx, hint).as_deref(),
            Some(BROWSER_PAGE_CONTENT_REFUSAL),
            "hint {hint:?} must be refused with the actionable message"
        );
    }

    // The refusal names exactly what IS supported (the Task 7.1 chrome surface).
    assert!(BROWSER_PAGE_CONTENT_REFUSAL.contains("address bar"));
    assert!(BROWSER_PAGE_CONTENT_REFUSAL.contains("tabs"));
    assert!(BROWSER_PAGE_CONTENT_REFUSAL.contains("reload"));
    assert!(BROWSER_PAGE_CONTENT_REFUSAL.contains("find bar"));
}

#[test]
fn t1_chrome_ui_hints_are_in_scope_and_not_refused() {
    let ctx = browser_context();
    let on = GuiBrowserConfig::enabled();

    let cases: &[(&str, BrowserChromeControl)] = &[
        ("address bar", BrowserChromeControl::AddressBar),
        ("reload", BrowserChromeControl::Reload),
        ("go back", BrowserChromeControl::Back),
        ("switch to the tab", BrowserChromeControl::Tab),
    ];
    for (hint, expected) in cases {
        assert_eq!(
            classify_browser_target_scope(&on, &ctx, hint),
            BrowserTargetScope::ChromeUi(*expected),
            "chrome hint {hint:?} must stay in scope"
        );
        assert!(
            !is_page_content_target(&on, &ctx, hint),
            "chrome hint {hint:?} is not page content"
        );
        assert!(
            browser_page_content_refusal(&on, &ctx, hint).is_none(),
            "chrome hint {hint:?} must NOT be refused"
        );
    }
}

#[test]
fn t1_flag_off_and_non_browser_are_not_applicable() {
    let ctx = browser_context();

    // Flag OFF → page-content scoping does not apply; never refuses.
    let off = GuiBrowserConfig::disabled();
    for hint in ["click the Sign In button on the page", "address bar"] {
        assert_eq!(
            classify_browser_target_scope(&off, &ctx, hint),
            BrowserTargetScope::NotApplicable,
            "flag OFF: {hint:?}"
        );
        assert!(!is_page_content_target(&off, &ctx, hint));
        assert!(browser_page_content_refusal(&off, &ctx, hint).is_none());
    }

    // Flag ON but a non-browser app → not applicable (existing path unaffected).
    let on = GuiBrowserConfig::enabled();
    let files = context_with("Files", browser_chrome_controls());
    for hint in ["click the Sign In button on the page", "address bar"] {
        assert_eq!(
            classify_browser_target_scope(&on, &files, hint),
            BrowserTargetScope::NotApplicable,
            "non-browser: {hint:?}"
        );
        assert!(!is_page_content_target(&on, &files, hint));
        assert!(browser_page_content_refusal(&on, &files, hint).is_none());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// T2 — OCR-only/visual-only evidence is never resolved as a (page) target.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t2_ocr_only_control_is_never_resolved_as_a_target() {
    let on = GuiBrowserConfig::enabled();

    // A browser whose ONLY "Reload"-labeled control is OCR-only (page region).
    let ctx = context_with(
        "Google Chrome",
        vec![ocr_only_control("push button", "Reload this page")],
    );

    // The OCR-only control must NEVER be resolved as a chrome target (no
    // OCR-only page targets — the injection-safety boundary).
    assert!(
        resolve_browser_chrome_target(&on, &ctx, "reload").is_none(),
        "an OCR-only control must never resolve as a chrome/page target"
    );

    // And the scope classifier refuses it as page content (provenance signal).
    assert_eq!(
        classify_browser_target_scope(&on, &ctx, "reload"),
        BrowserTargetScope::PageContent
    );
    assert_eq!(
        browser_page_content_refusal(&on, &ctx, "reload").as_deref(),
        Some(BROWSER_PAGE_CONTENT_REFUSAL)
    );
}

#[test]
fn t2_accessibility_chrome_control_still_resolves() {
    // Task 7.1 unaffected: a real accessibility-backed chrome control resolves,
    // even when an OCR-only page control is also present in the observation.
    let on = GuiBrowserConfig::enabled();
    let ctx = context_with(
        "Google Chrome",
        vec![
            control("push button", "Reload this page"),
            ocr_only_control("push button", "Add to Cart"),
        ],
    );

    let reload = resolve_browser_chrome_target(&on, &ctx, "reload")
        .expect("accessibility-backed reload still resolves");
    assert_eq!(reload.control, BrowserChromeControl::Reload);
    assert_eq!(reload.role, "push button");
    assert_eq!(reload.label, "Reload this page");

    // The chrome reload is in scope (NOT refused) ...
    assert_eq!(
        classify_browser_target_scope(&on, &ctx, "reload"),
        BrowserTargetScope::ChromeUi(BrowserChromeControl::Reload)
    );
    assert!(browser_page_content_refusal(&on, &ctx, "reload").is_none());

    // ... while the OCR-only in-page "Add to Cart" stays out of scope.
    assert!(is_page_content_target(&on, &ctx, "Add to Cart"));
    assert_eq!(
        browser_page_content_refusal(&on, &ctx, "add to cart").as_deref(),
        Some(BROWSER_PAGE_CONTENT_REFUSAL)
    );
}
