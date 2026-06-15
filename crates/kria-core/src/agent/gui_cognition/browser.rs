//! Task 7.1 (Requirements 5, 9, 26) — browser **chrome-UI** targeting.
//!
//! This module makes a browser window's *chrome* controls — the address/URL
//! bar, the tab strip / individual tabs, back/forward, reload/stop, and the
//! in-page Find bar — targetable through the existing accessibility (AT-SPI)
//! resolver. These are REAL a11y controls in the browser window's accessibility
//! tree, NOT web-page DOM content (page-content targeting is Task 7.2 and stays
//! out of scope here).
//!
//! The whole module is gated behind the `gui_cog_browser` feature flag
//! ([`GuiBrowserConfig`], default OFF until the Task 7.5 gate). While the flag
//! is OFF none of this code runs, so the executor/resolver path is byte-for-byte
//! unchanged.
//!
//! ## Design — intelligence first, no invention
//!
//! [`resolve_browser_chrome_target`] is the data-driven bridge: given the
//! observed [`GuiContext`] and a natural browser-chrome hint (e.g. "address
//! bar", "reload", "new tab", "find"), it
//!   1. confirms the active app is a recognized browser (by app name / window
//!      identity already in the observation — never assumed), then
//!   2. classifies the hint into a [`BrowserChromeControl`], then
//!   3. selects the matching REAL observed control by role + label,
//! and returns that control's role/label so the existing resolver can resolve
//! it the same way it resolves any other a11y control. It NEVER invents a
//! control and NEVER uses coordinates: if no observed control matches it returns
//! `None`, and resolution stays the resolver's job.
//!
//! ## Task 7.2 (Requirements 5, 9, 26) — page-content scope DECISION
//!
//! **Decision:** browser **web-page CONTENT** interaction — clicking links /
//! buttons *inside the rendered page*, typing into in-page form fields — is
//! **OUT OF SCOPE for v1**. Only browser **chrome-UI** (Task 7.1: address/URL
//! bar, tabs, back/forward, reload/stop, find bar) is targetable. Page-content
//! interaction via a browser **DOM/CDP bridge** is **tracked as future work**
//! and is intentionally NOT implemented now. See the ADR at
//! `docs/decisions/adr/003-browser-page-content-scope.md`.
//!
//! **Why (Requirement 9 — injection-safe / Requirement 26):** chrome controls
//! are REAL accessibility (AT-SPI) controls in the browser window's a11y tree —
//! a trusted execution authority. In-page content, by contrast, is only
//! reachable through OCR / visual-only evidence or a page-DOM bridge. KRIA
//! **never executes from OCR / visual-only evidence** (the injection-safety
//! boundary), so resolving a page target from OCR-only evidence is forbidden:
//! the page is untrusted, attacker-controllable text. There are therefore **no
//! OCR-only page targets**.
//!
//! **Enforcement (this task):** when the `gui_cog_browser` flag is ON and the
//! active app is a recognized browser, [`classify_browser_target_scope`] /
//! [`is_page_content_target`] classify a target hint into chrome-UI (in scope)
//! vs page-content (out of scope) by **observed control provenance** — a chrome
//! control resolves via accessibility; a hint that names in-page content (not a
//! chrome control per [`classify_browser_chrome_hint`]) or that matches only an
//! OCR/visual-only control in the page region is page content and is **REFUSED**
//! ([`BROWSER_PAGE_CONTENT_REFUSAL`]) rather than guessed at or acted on from
//! OCR-only evidence. While the flag is OFF, none of this runs (Step 1–12 path
//! is byte-for-byte unchanged).

use super::context::GuiContext;
use super::perception::{GuiActiveWindowSummary, GuiBounds, GuiControlSummary};

/// Environment variable that enables the `gui_cog_browser` flag (Task 7).
///
/// Truthy (`1`/`true`/`yes`/`on`) turns browser chrome-UI targeting ON. Default
/// (unset or any other value) keeps it OFF, preserving the existing executor /
/// resolver path byte-for-byte. The wave gate (Task 7.5) flips the live/desktop
/// path to default ON.
pub const BROWSER_ENV_FLAG: &str = "KRIA_GUI_COG_BROWSER";

/// Parse a `gui_cog_browser` env value as truthy (`1`/`true`/`yes`/`on`).
fn browser_flag_truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// Parse a `gui_cog_browser` env value as an explicit falsy opt-out
/// (`0`/`false`/`no`/`off`/empty) — the documented rollback switch. An absent
/// value (`None`) is NOT falsy: the default stays ON for the default-on path.
fn browser_flag_falsy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0") | Some("false") | Some("no") | Some("off") | Some("")
    )
}

/// The `gui_cog_browser` feature-flag bundle (default OFF) — Task 7.1.
///
/// When enabled, [`resolve_browser_chrome_target`] maps a browser-chrome target
/// hint to the matching observed a11y control so the existing resolver can find
/// it. When disabled (the default), the helper short-circuits to `None` and no
/// browser-specific recognition runs — the prior Step 1–12 behavior. The wave
/// gate (Task 7.5) flips this flag ON for the live/desktop path.
///
/// Mirrors the established `GuiPrimitivesConfig` flag pattern exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GuiBrowserConfig {
    /// Whether browser chrome-UI targeting is active.
    pub enabled: bool,
}

impl Default for GuiBrowserConfig {
    fn default() -> Self {
        // Task 7: flag default OFF until the wave gate (Task 7.5) flips it.
        Self { enabled: false }
    }
}

impl GuiBrowserConfig {
    /// Construct an explicitly-enabled browser config.
    pub fn enabled() -> Self {
        Self { enabled: true }
    }

    /// Construct an explicitly-disabled browser config.
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Derive the config from the process environment. The flag is OFF unless
    /// [`BROWSER_ENV_FLAG`] is truthy.
    pub fn from_env() -> Self {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env`](Self::from_env) with an injectable lookup.
    pub fn from_env_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        Self {
            enabled: browser_flag_truthy(lookup(BROWSER_ENV_FLAG).as_deref()),
        }
    }

    /// Derive the config from the process environment with the flag defaulting
    /// **ON** (wave gate flip, Task 7.5). Browser chrome-UI targeting is active
    /// unless [`BROWSER_ENV_FLAG`] is explicitly falsy
    /// (`0`/`false`/`no`/`off`/empty), which is the documented rollback switch.
    /// An absent env value keeps the flag ON.
    pub fn from_env_default_on() -> Self {
        Self::from_env_lookup_default_on(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env_default_on`](Self::from_env_default_on) with
    /// an injectable lookup.
    pub fn from_env_lookup_default_on<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        // Default ON: enabled unless the operator explicitly opts out via a
        // falsy env value (the rollback switch). Absent (None) is NOT falsy.
        Self {
            enabled: !browser_flag_falsy(lookup(BROWSER_ENV_FLAG).as_deref()),
        }
    }

    /// Whether browser chrome-UI targeting should run.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// App-name / window-identity needles that identify a recognized browser
/// (Chrome / Chromium / Firefox / Edge / Brave). Matched case-insensitively
/// against the observed active-window app name, app id, and window label — the
/// observation's own identity, never an assumption.
const BROWSER_APP_NEEDLES: &[&str] = &[
    "chrome",
    "chromium",
    "google-chrome",
    "firefox",
    "mozilla",
    "msedge",
    "microsoft edge",
    "edge",
    "brave",
    "vivaldi",
    "opera",
];

/// Detect whether the observed active window belongs to a recognized browser.
///
/// Reads ONLY the observed active-window identity (label / `app_name` /
/// `app_id`) — it never assumes a browser. Used to scope chrome-UI recognition
/// so non-browser apps are completely unaffected (Requirement 5/9/26).
pub fn is_browser_app(active: &GuiActiveWindowSummary) -> bool {
    let mut haystacks = vec![active.label.to_ascii_lowercase()];
    if let Some(app) = &active.app_name {
        haystacks.push(app.to_ascii_lowercase());
    }
    if let Some(app_id) = &active.app_id {
        haystacks.push(app_id.to_ascii_lowercase());
    }
    haystacks
        .iter()
        .any(|hay| BROWSER_APP_NEEDLES.iter().any(|needle| hay.contains(needle)))
}

/// The browser **chrome-UI** controls this task makes targetable. These are
/// real a11y controls in the browser window's accessibility tree (NOT web-page
/// DOM content, which is Task 7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BrowserChromeControl {
    /// The address / URL / search bar (the "omnibox").
    AddressBar,
    /// The "new tab" button on the tab strip.
    NewTab,
    /// An individual tab in the tab strip.
    Tab,
    /// Navigate back.
    Back,
    /// Navigate forward.
    Forward,
    /// Reload the current page.
    Reload,
    /// Stop loading the current page.
    Stop,
    /// The in-page Find bar.
    Find,
}

impl BrowserChromeControl {
    /// Stable token for events / telemetry / test assertions.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AddressBar => "address_bar",
            Self::NewTab => "new_tab",
            Self::Tab => "tab",
            Self::Back => "back",
            Self::Forward => "forward",
            Self::Reload => "reload",
            Self::Stop => "stop",
            Self::Find => "find",
        }
    }
}

/// Classify a natural browser-chrome target hint into a [`BrowserChromeControl`].
///
/// Returns `None` for a hint that does not name a chrome control (e.g. page
/// content) so the caller never forces a browser interpretation onto an
/// unrelated target. Ordering is deliberate: more specific phrases ("new tab",
/// "go back") are tested before their generic substrings ("tab", "back").
pub fn classify_browser_chrome_hint(hint: &str) -> Option<BrowserChromeControl> {
    let h = hint.trim().to_ascii_lowercase();
    if h.is_empty() {
        return None;
    }

    // Address / URL / omnibox.
    if h.contains("address bar")
        || h.contains("url bar")
        || h.contains("location bar")
        || h.contains("omnibox")
        || h.contains("address and search")
        || h.contains("search or enter address")
        || h == "address"
        || h == "url"
        || (h.contains("address") && h.contains("bar"))
    {
        return Some(BrowserChromeControl::AddressBar);
    }

    // Find bar (in-page find). Test before generic words.
    if h.contains("find bar") || h.contains("find in page") || h == "find" || h.contains("find ") {
        return Some(BrowserChromeControl::Find);
    }

    // Tab strip: "new tab" must be tested before the generic "tab".
    if h.contains("new tab") {
        return Some(BrowserChromeControl::NewTab);
    }
    if h.contains("tab") {
        return Some(BrowserChromeControl::Tab);
    }

    // Navigation: more specific phrasings first.
    if h.contains("go back") || h == "back" || h.contains("back button") || h.contains("navigate back")
    {
        return Some(BrowserChromeControl::Back);
    }
    if h.contains("go forward")
        || h == "forward"
        || h.contains("forward button")
        || h.contains("navigate forward")
    {
        return Some(BrowserChromeControl::Forward);
    }
    if h.contains("reload") || h.contains("refresh") {
        return Some(BrowserChromeControl::Reload);
    }
    if h.contains("stop") {
        return Some(BrowserChromeControl::Stop);
    }
    None
}

/// A recognized browser-chrome control matched to a REAL observed a11y control.
/// Carries the observed control's role/label so the existing resolver can
/// resolve it by role+label — the helper never invents a target.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BrowserChromeMatch {
    /// Which chrome control this is.
    pub control: BrowserChromeControl,
    /// The matched observed control's stable id.
    pub control_id: String,
    /// The matched observed control's accessible role (as observed).
    pub role: String,
    /// The matched observed control's accessible label/name (as observed).
    pub label: String,
    /// The matched observed control's bounds, when observed.
    pub bounds: Option<GuiBounds>,
}

impl BrowserChromeMatch {
    /// The resolver-friendly target hint (the observed accessible label). Feed
    /// this as a typed step's `target_control_hint` so the existing resolver
    /// resolves the same control by role+label.
    pub fn target_hint(&self) -> &str {
        &self.label
    }
}

// ── role classification (local, browser-scoped — no coupling to the resolver) ──

fn role_is_editable(role: &str) -> bool {
    let r = role.to_ascii_lowercase();
    ["entry", "text", "searchbox", "textbox", "input", "editor", "combo box", "combobox"]
        .iter()
        .any(|needle| r.contains(needle))
}

fn role_is_button(role: &str) -> bool {
    role.to_ascii_lowercase().contains("button")
}

fn role_is_tab(role: &str) -> bool {
    let r = role.to_ascii_lowercase();
    // Precise: a real "table" role must never be treated as a tab.
    r == "tab" || r.contains("page tab") || r.contains("tab list") || r.contains("tab item")
}

/// Whether an observed control's role is acceptable for the given chrome control.
fn role_matches(control: BrowserChromeControl, role: &str) -> bool {
    match control {
        BrowserChromeControl::AddressBar => role_is_editable(role),
        // Find can be exposed as the find entry OR a Find toolbar button.
        BrowserChromeControl::Find => role_is_editable(role) || role_is_button(role),
        // The new-tab affordance is usually a button, sometimes a tab item.
        BrowserChromeControl::NewTab => role_is_button(role) || role_is_tab(role),
        BrowserChromeControl::Tab => role_is_tab(role),
        BrowserChromeControl::Back
        | BrowserChromeControl::Forward
        | BrowserChromeControl::Reload
        | BrowserChromeControl::Stop => role_is_button(role),
    }
}

/// Label keywords that identify the given chrome control. An empty set means the
/// control is identified by role alone (e.g. an individual tab whose label is
/// the page title).
fn label_keywords(control: BrowserChromeControl) -> &'static [&'static str] {
    match control {
        BrowserChromeControl::AddressBar => &[
            "address",
            "url",
            "location",
            "search or enter address",
            "search with google or enter address",
            "omnibox",
        ],
        BrowserChromeControl::NewTab => &["new tab"],
        // An individual tab is identified by role; its label is the page title.
        BrowserChromeControl::Tab => &[],
        BrowserChromeControl::Back => &["back"],
        BrowserChromeControl::Forward => &["forward"],
        BrowserChromeControl::Reload => &["reload", "refresh"],
        BrowserChromeControl::Stop => &["stop"],
        BrowserChromeControl::Find => &["find"],
    }
}

/// Whether an observed control's label satisfies the chrome control's keywords.
fn label_matches(control: BrowserChromeControl, label: &str) -> bool {
    let keywords = label_keywords(control);
    if keywords.is_empty() {
        // Role-only identification (individual tab): any non-empty label is a
        // legitimate tab title; an empty label is still a tab by role.
        return true;
    }
    let l = label.to_ascii_lowercase();
    keywords.iter().any(|keyword| l.contains(keyword))
}

/// Score an observed control as a candidate for the chrome control. Higher is
/// better; `None` means it is not a candidate at all. Scoring prefers an
/// enabled+visible control and an exact label-keyword hit so the best real
/// control is chosen deterministically — the resolver still performs the final
/// safe resolution.
fn candidate_score(control: BrowserChromeControl, c: &GuiControlSummary) -> Option<u32> {
    // Task 7.2 (Requirement 9): a chrome-UI control MUST be backed by the
    // accessibility tree. An OCR-only / visual-only control is page-content
    // evidence and is NEVER an execution authority — never resolve a chrome (or
    // any) target from OCR-only evidence (the injection-safety boundary). Such a
    // control is refused as page content, not matched here.
    if !control_is_accessibility_backed(c) {
        return None;
    }
    if !role_matches(control, &c.role) {
        return None;
    }
    if !label_matches(control, &c.name) {
        return None;
    }
    let mut score = 1u32;
    if c.visible {
        score += 2;
    }
    if c.enabled {
        score += 2;
    }
    if c.in_active_window {
        score += 2;
    }
    // Exact keyword hit (vs. role-only) ranks higher.
    let keywords = label_keywords(control);
    if !keywords.is_empty() {
        let l = c.name.to_ascii_lowercase();
        if keywords.iter().any(|k| l == *k) {
            score += 3;
        }
    }
    Some(score)
}

/// Map a browser-chrome target hint to the matching observed a11y control so the
/// existing resolver can resolve it by role+label (Task 7.1).
///
/// Returns `None` — never a fabricated control — when:
///   * the `gui_cog_browser` flag is OFF, or
///   * the active app is not a recognized browser (non-browser apps unaffected),
///     or
///   * the hint does not name a chrome control, or
///   * no observed control matches the chrome control's role+label.
///
/// The match is derived ENTIRELY from observed controls
/// ([`GuiContext::fused_controls`]); it never invents a control and never uses
/// coordinates.
pub fn resolve_browser_chrome_target(
    config: &GuiBrowserConfig,
    context: &GuiContext,
    hint: &str,
) -> Option<BrowserChromeMatch> {
    if !config.enabled {
        return None;
    }
    if !is_browser_app(&context.active_window) {
        return None;
    }
    let control = classify_browser_chrome_hint(hint)?;

    context
        .fused_controls
        .iter()
        .filter_map(|c| candidate_score(control, c).map(|score| (score, c)))
        .max_by_key(|(score, _)| *score)
        .map(|(_, c)| BrowserChromeMatch {
            control,
            control_id: c.control_id.clone(),
            role: c.role.clone(),
            label: c.name.clone(),
            bounds: c.bounds.clone(),
        })
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 7.2 (Requirements 5, 9, 26) — page-content scope classification.
//
// Browser web-page CONTENT (links/buttons/fields inside the rendered page) is
// OUT OF SCOPE for v1; only chrome-UI (Task 7.1) is targetable. Page-content
// interaction via a DOM/CDP bridge is tracked future work (see the ADR). These
// helpers classify a target hint into chrome-UI (in scope) vs page-content (out
// of scope) by observed control provenance, and surface the refusal message so
// a page target is never guessed at or resolved from OCR/visual-only evidence.
// ─────────────────────────────────────────────────────────────────────────────

/// The clear, actionable refusal returned when a browser target hint refers to
/// in-page web content (Task 7.2). It names exactly what IS supported (the
/// chrome-UI surface from Task 7.1) so the caller can redirect the user instead
/// of guessing or acting on an OCR-only page target.
pub const BROWSER_PAGE_CONTENT_REFUSAL: &str =
    "Web page content targeting isn't supported yet; I can act on the browser's address bar, \
     tabs, back/forward, reload, and find bar.";

/// The v1 scope of a browser target hint (Task 7.2).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BrowserTargetScope {
    /// Page-content scoping does not apply: either the `gui_cog_browser` flag is
    /// OFF, or the active app is not a recognized browser. The caller's normal
    /// (non-browser) path is unaffected.
    NotApplicable,
    /// **In scope (v1):** a browser **chrome-UI** control (address/URL bar,
    /// tabs, back/forward, reload/stop, find bar) — a REAL accessibility control
    /// in the browser window's a11y tree (Task 7.1).
    ChromeUi(BrowserChromeControl),
    /// **Out of scope (v1):** browser **web-page CONTENT** — a link/button/field
    /// inside the rendered page, reachable only via a page-DOM/CDP bridge
    /// (tracked future work) or OCR/visual-only evidence (never an execution
    /// authority). This is REFUSED with [`BROWSER_PAGE_CONTENT_REFUSAL`]; it is
    /// never guessed at or resolved from OCR-only evidence (Requirement 9).
    PageContent,
}

impl BrowserTargetScope {
    /// Stable token for events / telemetry / test assertions.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotApplicable => "not_applicable",
            Self::ChromeUi(_) => "chrome_ui",
            Self::PageContent => "page_content",
        }
    }

    /// Whether this scope is the out-of-scope page-content case (i.e. must be
    /// refused).
    pub fn is_page_content(&self) -> bool {
        matches!(self, Self::PageContent)
    }
}

/// Whether an observed control is backed by the accessibility (AT-SPI) tree.
///
/// A chrome-UI control is a real a11y control (trusted execution authority). A
/// control whose only provenance is OCR / visual-only / screenshot is
/// page-content evidence and is NEVER an execution authority (Requirement 9 —
/// the injection-safety boundary: KRIA never executes from OCR/visual-only
/// evidence).
fn control_is_accessibility_backed(c: &GuiControlSummary) -> bool {
    const ACCESSIBILITY: &str = "accessibility";
    if c.source.eq_ignore_ascii_case(ACCESSIBILITY) {
        return true;
    }
    c.sources
        .iter()
        .any(|source| source.eq_ignore_ascii_case(ACCESSIBILITY))
}

/// Whether an observed control is OCR-only / visual-only (no accessibility
/// provenance) — i.e. page-content evidence that must never be resolved as a
/// target (Task 7.2 / Requirement 9). The inverse of
/// [`control_is_accessibility_backed`].
fn control_is_ocr_or_visual_only(c: &GuiControlSummary) -> bool {
    !control_is_accessibility_backed(c)
}

/// Whether any observed control whose label matches the hint is OCR-only /
/// visual-only with NO accessibility-backed match — the provenance signal that
/// the hint refers to in-page content drawn on the page (Task 7.2).
fn hint_matches_only_ocr_visual_controls(context: &GuiContext, hint: &str) -> bool {
    let needle = hint.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return false;
    }
    let label_matches = |c: &GuiControlSummary| {
        let name = c.name.trim().to_ascii_lowercase();
        !name.is_empty() && (name.contains(&needle) || needle.contains(&name))
    };
    let matching: Vec<&GuiControlSummary> = context
        .fused_controls
        .iter()
        .filter(|c| label_matches(c))
        .collect();
    if matching.is_empty() {
        return false;
    }
    // Page content iff at least one OCR/visual-only match exists AND none of the
    // matches is accessibility-backed (so there is no trusted chrome control to
    // target — only untrusted page evidence).
    matching.iter().any(|c| control_is_ocr_or_visual_only(c))
        && !matching.iter().any(|c| control_is_accessibility_backed(c))
}

/// Classify a browser target hint into its v1 scope (Task 7.2).
///
/// Returns:
///   * [`BrowserTargetScope::NotApplicable`] when the `gui_cog_browser` flag is
///     OFF or the active app is not a recognized browser (the normal path is
///     unaffected),
///   * [`BrowserTargetScope::ChromeUi`] when the hint names a chrome-UI control
///     ([`classify_browser_chrome_hint`]) — the in-scope Task 7.1 surface,
///   * [`BrowserTargetScope::PageContent`] when the hint names in-page web
///     content (not a chrome control) OR matches only OCR/visual-only controls
///     in the page region — the OUT-OF-SCOPE case that must be refused. A page
///     target is NEVER resolved from OCR-only evidence (Requirement 9).
///
/// The classification reads ONLY observed evidence (the active-window identity
/// and observed control provenance); it never invents a target and never uses
/// coordinates.
pub fn classify_browser_target_scope(
    config: &GuiBrowserConfig,
    context: &GuiContext,
    hint: &str,
) -> BrowserTargetScope {
    if !config.enabled {
        return BrowserTargetScope::NotApplicable;
    }
    if !is_browser_app(&context.active_window) {
        return BrowserTargetScope::NotApplicable;
    }
    match classify_browser_chrome_hint(hint) {
        // The hint names a chrome control. Confirm it is not in fact only
        // backed by OCR/visual evidence in the page region: if the only matching
        // observed controls are OCR/visual-only, treat it as page content and
        // refuse (never resolve from OCR-only). Otherwise it is the in-scope
        // chrome surface — even when no control is observed yet (the resolver
        // then reports it unresolved; that is NOT a page-content refusal).
        Some(control) => {
            if hint_matches_only_ocr_visual_controls(context, hint) {
                BrowserTargetScope::PageContent
            } else {
                BrowserTargetScope::ChromeUi(control)
            }
        }
        // The hint does not name a chrome control → it refers to in-page web
        // content (a link/button/field inside the rendered page). Out of scope.
        None => BrowserTargetScope::PageContent,
    }
}

/// Whether a browser target hint refers to out-of-scope web-page CONTENT
/// (Task 7.2). `true` only when the `gui_cog_browser` flag is ON, the active app
/// is a recognized browser, and the hint is page content (not chrome-UI). When
/// `true`, the caller MUST refuse with [`BROWSER_PAGE_CONTENT_REFUSAL`] rather
/// than guess or act on an OCR-only page target (Requirement 9).
pub fn is_page_content_target(config: &GuiBrowserConfig, context: &GuiContext, hint: &str) -> bool {
    classify_browser_target_scope(config, context, hint).is_page_content()
}

/// The clear, actionable refusal for a browser target hint, when that hint is
/// out-of-scope web-page content (Task 7.2). Returns `Some(message)` only when
/// [`is_page_content_target`] holds (flag ON + browser + page-content hint), and
/// `None` otherwise (chrome-UI, non-browser, or flag OFF) so the existing path
/// is unaffected. The message names exactly what IS supported (the Task 7.1
/// chrome-UI surface) so the caller redirects the user instead of acting on an
/// OCR-only page target.
pub fn browser_page_content_refusal(
    config: &GuiBrowserConfig,
    context: &GuiContext,
    hint: &str,
) -> Option<String> {
    is_page_content_target(config, context, hint).then(|| BROWSER_PAGE_CONTENT_REFUSAL.to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 7.3 (Requirements 5, 9, 26) — read/summarize uses OCR/page text as DATA
// only; it NEVER influences the planner or executor (injection defense), and the
// surfaced text is explicitly marked untrusted.
//
// The read/summarize path produces a summary STRICTLY from already-sanitized
// observed OCR/page text. The produced [`VisibleContentSummary`]:
//   * carries an explicit `untrusted` provenance marker
//     ([`UNTRUSTED_VISIBLE_CONTENT_PROVENANCE`]) so the UI and any downstream
//     consumer know it is DATA, not instruction;
//   * reuses the existing OCR injection markers (`injection_suspected` /
//     `ocr_injection_count`) so injection-suspected blocks are flagged in the
//     summary;
//   * exposes NO path to plan steps, targets, or actions — by construction the
//     summary cannot introduce a step, change a target, or trigger an action.
//
// The planner request construction already EXCLUDES raw OCR/page text from the
// instructions it sends to the model — `GuiLlmPlannerRequest::safe_json` carries
// only the `ocr_block_count` / `ocr_injection_count` *counts*, never the text —
// so an injection phrasing observed on screen cannot reach the planner. This
// module's read/summarize helper preserves that boundary: it returns the text as
// tagged data for display, and never as planner/executor input.
//
// Like the rest of this module, the helper is gated behind `gui_cog_browser`:
// while the flag is OFF it returns `None` and the existing summarize path is
// byte-for-byte unchanged.
// ─────────────────────────────────────────────────────────────────────────────

/// Stable provenance marker stamped on every read/summarize result so the UI and
/// any downstream consumer know the text is UNTRUSTED observed DATA, never an
/// instruction (Task 7.3, Requirements 5/9/26).
pub const UNTRUSTED_VISIBLE_CONTENT_PROVENANCE: &str = "untrusted_observed_content";

/// A read/summarize result produced STRICTLY as data (Task 7.3).
///
/// Built only from already-sanitized observed OCR/page text (the perception
/// layer's `safe_previews`, which redact secrets and replace injection-suspected
/// blocks with `[untrusted text redacted]`). It is tagged untrusted and carries
/// the existing OCR injection markers so the UI and any downstream consumer treat
/// it as DATA, never instructions.
///
/// **Injection defense (Requirement 9):** this type intentionally exposes NO API
/// that yields plan steps, target hints, or actions. It cannot introduce a step,
/// change a target, or trigger an action — the observed text reaches only the
/// summary's data fields, never the planner or executor. Injection-suspected
/// text is already redacted upstream, so the summary references only safe
/// observed content and never reproduces an attacker instruction.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VisibleContentSummary {
    /// Always [`UNTRUSTED_VISIBLE_CONTENT_PROVENANCE`].
    pub provenance: String,
    /// Always `true` — observed OCR/page text is untrusted.
    pub untrusted: bool,
    /// Always `true` — this is DATA surfaced for display, never instruction.
    pub data_only: bool,
    /// Sanitized observed text lines (the OCR/page text), redacted upstream.
    /// References only observed content; never reproduces injection instructions.
    pub observed_text: Vec<String>,
    /// Whether any observed block was injection-suspected (reuses the existing
    /// OCR injection markers). Flagged here, but never acted on.
    pub injection_suspected: bool,
    /// Count of injection-suspected OCR blocks (mirrors `ocr_injection_count`).
    pub injection_block_count: usize,
    /// Count of redactions applied to observed text before surfacing.
    pub redaction_count: usize,
}

impl VisibleContentSummary {
    /// Whether this summary is untrusted observed data (always `true`).
    pub fn is_untrusted(&self) -> bool {
        self.untrusted
    }

    /// Whether any observed block was injection-suspected.
    pub fn has_injection(&self) -> bool {
        self.injection_suspected
    }

    /// Sanitized JSON for events / telemetry / UI. Explicitly stamps the
    /// untrusted/data-only provenance and `is_instruction: false` so no consumer
    /// can mistake the observed text for an instruction (Task 7.3).
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "provenance": self.provenance,
            "untrusted": self.untrusted,
            "data_only": self.data_only,
            "is_instruction": false,
            "observed_text": self.observed_text,
            "injection_suspected": self.injection_suspected,
            "injection_block_count": self.injection_block_count,
            "redaction_count": self.redaction_count,
        })
    }
}

/// Summarize the visible OCR/page content as UNTRUSTED data (Task 7.3).
///
/// Reads ONLY the already-sanitized observed OCR/page text from the context
/// ([`GuiContext::ocr_evidence`]'s `safe_previews`) and returns a
/// [`VisibleContentSummary`] tagged untrusted, with the existing OCR injection
/// markers preserved. The observed text reaches the summary's data fields ONLY —
/// it is never returned as a plan step, target hint, or action, so it cannot
/// influence the planner or executor (injection defense, Requirement 9).
///
/// Returns `None` when the `gui_cog_browser` flag is OFF, so the existing
/// summarize path stays byte-for-byte unchanged.
pub fn summarize_visible_content_as_data(
    config: &GuiBrowserConfig,
    context: &GuiContext,
) -> Option<VisibleContentSummary> {
    if !config.enabled {
        return None;
    }
    let observed_text = context
        .ocr_evidence
        .safe_previews
        .iter()
        .map(|preview| preview.trim())
        .filter(|preview| !preview.is_empty())
        .map(|preview| preview.to_string())
        .collect::<Vec<_>>();

    Some(VisibleContentSummary {
        provenance: UNTRUSTED_VISIBLE_CONTENT_PROVENANCE.to_string(),
        untrusted: true,
        data_only: true,
        observed_text,
        injection_suspected: context.ocr_has_injection(),
        injection_block_count: context.ocr_evidence.injection_count,
        redaction_count: context.ocr_evidence.redaction_count,
    })
}
