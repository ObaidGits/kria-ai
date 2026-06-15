use super::executor::{stable_target_identity_hash, GuiActionExecution, GuiActionKind};
use super::perception::{sanitize_gui_text, GuiControlSummary, GuiObservationSnapshot};

/// Legacy Step 7 verification report. Retained for backward compatibility with
/// the pre-Step-8 execution path and existing tests. Step 8 callers should use
/// [`GuiPostActionVerificationResult`] instead.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiVerificationReport {
    pub status: String,
    pub confidence: f64,
    pub after_observation_id: String,
}

pub fn verify_post_action(
    execution: &GuiActionExecution,
    post_observation: &GuiObservationSnapshot,
    success_confidence: f64,
) -> GuiVerificationReport {
    GuiVerificationReport {
        status: if execution.success {
            "completed".into()
        } else {
            "failed".into()
        },
        confidence: if execution.success {
            success_confidence
        } else {
            0.2
        },
        after_observation_id: post_observation.observation_id.clone(),
    }
}

// ---------------------------------------------------------------------------
// Step 8: Post-Action Verification
// ---------------------------------------------------------------------------

/// Deterministic verification strategies. Visual/OCR evidence may support a
/// strategy but can never invent an executable result on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiVerificationStrategy {
    WindowVisible,
    ActiveWindowMatch,
    FocusedControl,
    TextPresent,
    StateChanged,
    ScreenChanged,
    ResultVisible,
    DialogVisible,
    FileSaved,
    DownloadStartedOrCompleted,
    ClipboardChanged,
    TargetResolved,
    VisibleContentSummarized,
    Inconclusive,
}

impl GuiVerificationStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WindowVisible => "window_visible",
            Self::ActiveWindowMatch => "active_window_match",
            Self::FocusedControl => "focused_control",
            Self::TextPresent => "text_present",
            Self::StateChanged => "state_changed",
            Self::ScreenChanged => "screen_changed",
            Self::ResultVisible => "result_visible",
            Self::DialogVisible => "dialog_visible",
            Self::FileSaved => "file_saved",
            Self::DownloadStartedOrCompleted => "download_started_or_completed",
            Self::ClipboardChanged => "clipboard_changed",
            Self::TargetResolved => "target_resolved",
            Self::VisibleContentSummarized => "visible_content_summarized",
            Self::Inconclusive => "inconclusive",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "window_visible" => Self::WindowVisible,
            "active_window_match" => Self::ActiveWindowMatch,
            "focused_control" => Self::FocusedControl,
            "text_present" => Self::TextPresent,
            "state_changed" => Self::StateChanged,
            "screen_changed" => Self::ScreenChanged,
            "result_visible" => Self::ResultVisible,
            "dialog_visible" => Self::DialogVisible,
            "file_saved" => Self::FileSaved,
            "download_started_or_completed" => Self::DownloadStartedOrCompleted,
            "clipboard_changed" => Self::ClipboardChanged,
            "target_resolved" => Self::TargetResolved,
            "visible_content_summarized" => Self::VisibleContentSummarized,
            _ => Self::Inconclusive,
        }
    }
}

/// Choose the action-specific verification strategy. Secret payloads never use
/// `text_present` so raw secret text is never searched for or echoed; they use
/// `state_changed` evidence instead.
pub fn select_verification_strategy(
    action: &GuiActionKind,
    is_secret_payload: bool,
) -> GuiVerificationStrategy {
    match action {
        GuiActionKind::OpenApp => GuiVerificationStrategy::ActiveWindowMatch,
        GuiActionKind::SwitchWindow => GuiVerificationStrategy::ActiveWindowMatch,
        GuiActionKind::FocusField => GuiVerificationStrategy::FocusedControl,
        GuiActionKind::TypeText | GuiActionKind::FillField => {
            if is_secret_payload {
                GuiVerificationStrategy::StateChanged
            } else {
                GuiVerificationStrategy::TextPresent
            }
        }
        GuiActionKind::Paste => {
            if is_secret_payload {
                GuiVerificationStrategy::StateChanged
            } else {
                GuiVerificationStrategy::TextPresent
            }
        }
        GuiActionKind::ClickControl => GuiVerificationStrategy::ResultVisible,
        GuiActionKind::PressKey | GuiActionKind::Hotkey => GuiVerificationStrategy::ScreenChanged,
        GuiActionKind::Scroll => GuiVerificationStrategy::ScreenChanged,
        GuiActionKind::Copy => GuiVerificationStrategy::ClipboardChanged,
        // Task 6.1 typed primitives (Requirement 5 / 23): clear/select-all change
        // focused-field state; checkbox/in-app-search produce a visible result;
        // dialog-close changes the dialog-visible state.
        GuiActionKind::ClearField | GuiActionKind::SelectAll => {
            GuiVerificationStrategy::StateChanged
        }
        GuiActionKind::SetCheckbox | GuiActionKind::InAppSearch => {
            GuiVerificationStrategy::ResultVisible
        }
        GuiActionKind::CloseDialog => GuiVerificationStrategy::DialogVisible,
    }
}

// ---------------------------------------------------------------------------
// Phase 1 (Requirement 1): `gui_cog_verify_live` flag + flag-aware OpenApp
// predicate. The NEW predicate change (OpenApp `ActiveWindowMatch` →
// `WindowVisible`) is OFF-able so flag-OFF restores the prior
// `active_window_match` verdict byte-for-byte.
// ---------------------------------------------------------------------------

/// Environment variable that enables the `gui_cog_verify_live` flag (Phase 1).
///
/// Truthy (`1`/`true`/`yes`/`on`) turns the live-verification predicate path ON
/// (OpenApp verifies `window_visible` against desktop-state/observation evidence
/// with a bounded readiness wait). Default (unset or any other value) keeps it
/// OFF, preserving the prior `active_window_match` verdict byte-for-byte. The
/// desktop wires `from_env_default_on()` since the prior waves are ON.
pub const VERIFY_LIVE_ENV_FLAG: &str = "KRIA_GUI_COG_VERIFY_LIVE";

/// Parse a `gui_cog_verify_live` env value as truthy (`1`/`true`/`yes`/`on`).
fn verify_live_flag_truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// Parse a `gui_cog_verify_live` env value as an explicit falsy opt-out
/// (`0`/`false`/`no`/`off`/empty) — the documented rollback switch. An absent
/// value (`None`) is NOT falsy: the default stays ON for the default-on path.
fn verify_live_flag_falsy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0") | Some("false") | Some("no") | Some("off") | Some("")
    )
}

/// The `gui_cog_verify_live` feature-flag bundle (default OFF) — Phase 1.
///
/// When enabled, the OpenApp verification predicate is `WindowVisible` (a window
/// PRESENT/visible in the desktop open-window set, evidence
/// `observation`/desktop-state) instead of `ActiveWindowMatch`, and the runtime
/// performs a bounded readiness wait before concluding. When disabled (the
/// default) the prior `active_window_match` verdict runs byte-for-byte
/// unchanged. `SwitchWindow` is never affected (it stays `ActiveWindowMatch`).
///
/// Mirrors the established `GuiSafetyPolishConfig` / `GuiWaylandFocusConfig`
/// flag pattern exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GuiVerifyLiveConfig {
    /// Whether the live-verification predicate path is active.
    pub enabled: bool,
}

impl Default for GuiVerifyLiveConfig {
    fn default() -> Self {
        // Phase 1: flag default OFF until the live gate flips it.
        Self { enabled: false }
    }
}

impl GuiVerifyLiveConfig {
    /// Construct an explicitly-enabled verify-live config.
    pub fn enabled() -> Self {
        Self { enabled: true }
    }

    /// Construct an explicitly-disabled verify-live config.
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Derive the config from the process environment. The flag is OFF unless
    /// [`VERIFY_LIVE_ENV_FLAG`] is truthy.
    pub fn from_env() -> Self {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env`](Self::from_env) with an injectable lookup.
    pub fn from_env_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        Self {
            enabled: verify_live_flag_truthy(lookup(VERIFY_LIVE_ENV_FLAG).as_deref()),
        }
    }

    /// Derive the config from the process environment with the flag defaulting
    /// **ON** (live gate flip). Live verification is active unless
    /// [`VERIFY_LIVE_ENV_FLAG`] is explicitly falsy
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
            enabled: !verify_live_flag_falsy(lookup(VERIFY_LIVE_ENV_FLAG).as_deref()),
        }
    }

    /// Whether the live-verification predicate path should run.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Flag-aware verification strategy selection (Phase 1, Requirement 1.1).
///
/// When `verify_live_enabled` is true, an `OpenApp` action verifies
/// `WindowVisible` (window present in the desktop open-window set) instead of
/// `ActiveWindowMatch`. Every other action — including `SwitchWindow` — is
/// unchanged and delegates to [`select_verification_strategy`]. When the flag is
/// OFF this is byte-for-byte identical to [`select_verification_strategy`].
pub fn select_verification_strategy_with_flag(
    action: &GuiActionKind,
    is_secret_payload: bool,
    verify_live_enabled: bool,
) -> GuiVerificationStrategy {
    if verify_live_enabled && matches!(action, GuiActionKind::OpenApp) {
        GuiVerificationStrategy::WindowVisible
    } else {
        select_verification_strategy(action, is_secret_payload)
    }
}

// ---------------------------------------------------------------------------
// Task 9.1 (Requirements 10, 13, 15, 22, 23): verification CONTRACT per action
// type + `gui_cog_safety_polish` flag.
// ---------------------------------------------------------------------------

/// Environment variable that enables the `gui_cog_safety_polish` flag (Task 9).
///
/// Truthy (`1`/`true`/`yes`/`on`) turns the safety-polish path ON. Default
/// (unset or any other value) keeps it OFF, preserving the existing executor /
/// runtime verdict behavior byte-for-byte. The wave gate (Task 9.7) flips the
/// live/desktop path to default ON.
pub const SAFETY_POLISH_ENV_FLAG: &str = "KRIA_GUI_COG_SAFETY_POLISH";

/// Parse a `gui_cog_safety_polish` env value as truthy (`1`/`true`/`yes`/`on`).
fn safety_polish_flag_truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// Parse a `gui_cog_safety_polish` env value as an explicit falsy opt-out
/// (`0`/`false`/`no`/`off`/empty) — the documented rollback switch. An absent
/// value (`None`) is NOT falsy: the default stays ON for the default-on path.
fn safety_polish_flag_falsy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0") | Some("false") | Some("no") | Some("off") | Some("")
    )
}

/// The `gui_cog_safety_polish` feature-flag bundle (default OFF) — Task 9.1.
///
/// When enabled, the verification CONTRACT per action type is enforced
/// ([`verification_contract_for`] / [`apply_verification_contract`]): a weak or
/// unreliable-evidence outcome is reported as the honest `inconclusive` verdict
/// (never a false `verified`), and the contract (predicate + evidence source +
/// bounded wait + confidence) is surfaced as additive telemetry. When disabled
/// (the default) the prior verdict behavior runs byte-for-byte unchanged. The
/// wave gate (Task 9.7) flips this flag ON for the live/desktop path.
///
/// Mirrors the established `GuiCrossAppConfig` flag pattern exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GuiSafetyPolishConfig {
    /// Whether the safety-polish verification contract path is active.
    pub enabled: bool,
}

impl Default for GuiSafetyPolishConfig {
    fn default() -> Self {
        // Task 9: flag default OFF until the wave gate (Task 9.7) flips it.
        Self { enabled: false }
    }
}

impl GuiSafetyPolishConfig {
    /// Construct an explicitly-enabled safety-polish config.
    pub fn enabled() -> Self {
        Self { enabled: true }
    }

    /// Construct an explicitly-disabled safety-polish config.
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Derive the config from the process environment. The flag is OFF unless
    /// [`SAFETY_POLISH_ENV_FLAG`] is truthy.
    pub fn from_env() -> Self {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env`](Self::from_env) with an injectable lookup.
    pub fn from_env_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        Self {
            enabled: safety_polish_flag_truthy(lookup(SAFETY_POLISH_ENV_FLAG).as_deref()),
        }
    }

    /// Derive the config from the process environment with the flag defaulting
    /// **ON** (wave gate flip, Task 9.7). Safety polish is active unless
    /// [`SAFETY_POLISH_ENV_FLAG`] is explicitly falsy
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
            enabled: !safety_polish_flag_falsy(lookup(SAFETY_POLISH_ENV_FLAG).as_deref()),
        }
    }

    /// Whether the safety-polish verification contract should run.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// The EVIDENCE source a verification contract relies on to check its predicate.
///
/// KRIA is verifier-aware: a step is never marked verified without evidence, and
/// evidence is drawn from accessibility / observation / an active-window probe /
/// a backend receipt — never OCR-only or coordinate guesses (Requirement 23).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiVerificationEvidenceSource {
    /// The accessibility tree (focused control, control labels/roles).
    Accessibility,
    /// The fused observation snapshot (screen-hash / dialog / control state).
    Observation,
    /// The active-window probe (which window is frontmost).
    ActiveWindowProbe,
    /// A real running-process probe (Issue #2): the launched app's binary is
    /// running. Strong, honest evidence that "the app opened" even when no window
    /// is observable (Wayland focus-stealing prevention / no usable window list).
    /// NEVER an OCR/coordinate guess.
    Process,
    /// A backend receipt only (e.g. clipboard write); content never captured.
    BackendReceipt,
    /// No deterministic evidence source applies (=> inconclusive).
    None,
}

impl GuiVerificationEvidenceSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accessibility => "accessibility",
            Self::Observation => "observation",
            Self::ActiveWindowProbe => "active_window_probe",
            Self::Process => "process",
            Self::BackendReceipt => "backend_receipt",
            Self::None => "none",
        }
    }
}

/// Minimum confidence a `verified` verdict must carry under the safety-polish
/// contract; below this the honest verdict is `inconclusive` (Task 9.1).
pub const VERIFICATION_CONTRACT_MIN_CONFIDENCE: f64 = 0.7;

/// The EVIDENCE source used to check a given verification predicate (strategy).
pub fn evidence_source_for_strategy(
    strategy: GuiVerificationStrategy,
) -> GuiVerificationEvidenceSource {
    match strategy {
        // Phase 1 (`gui_cog_verify_live`, Requirement 1.1): a window being
        // PRESENT/visible in the desktop open-window set is desktop-state /
        // observation evidence — NOT the active-window probe. On Wayland a freshly
        // launched app is not guaranteed to become the focused/active window, so
        // verifying "the app opened" against the active-window probe falsely fails.
        GuiVerificationStrategy::WindowVisible => GuiVerificationEvidenceSource::Observation,
        // SwitchWindow keeps the active-window probe: a switch is only "done" when
        // the requested window is the active/frontmost one (Phase 3 lands the real
        // activation).
        GuiVerificationStrategy::ActiveWindowMatch => {
            GuiVerificationEvidenceSource::ActiveWindowProbe
        }
        GuiVerificationStrategy::FocusedControl
        | GuiVerificationStrategy::TextPresent
        | GuiVerificationStrategy::TargetResolved => GuiVerificationEvidenceSource::Accessibility,
        GuiVerificationStrategy::StateChanged
        | GuiVerificationStrategy::ScreenChanged
        | GuiVerificationStrategy::ResultVisible
        | GuiVerificationStrategy::DialogVisible
        | GuiVerificationStrategy::FileSaved
        | GuiVerificationStrategy::DownloadStartedOrCompleted
        | GuiVerificationStrategy::VisibleContentSummarized => {
            GuiVerificationEvidenceSource::Observation
        }
        GuiVerificationStrategy::ClipboardChanged => GuiVerificationEvidenceSource::BackendReceipt,
        GuiVerificationStrategy::Inconclusive => GuiVerificationEvidenceSource::None,
    }
}

/// Task 4 (Issue #10): the ORDERED evidence sources for a predicate — primary
/// first, then honest fallbacks. The verifier prefers the primary; if it is
/// unavailable/unreliable the honest verdict is `inconclusive` (never a false
/// `verification_failed`). `Observation` (a screen change) is the universal,
/// always-available secondary for accessibility / active-window predicates,
/// because a real on-screen effect is observable even when a11y is off (e.g.
/// Chrome) or the active-window probe is unreliable on Wayland. A backend-receipt
/// predicate (clipboard) has no visual fallback by design.
pub fn ordered_evidence_for_strategy(
    strategy: GuiVerificationStrategy,
) -> Vec<GuiVerificationEvidenceSource> {
    use GuiVerificationEvidenceSource as E;
    let primary = evidence_source_for_strategy(strategy);
    let mut sources = vec![primary];
    let mut push = |s: E| {
        if !sources.contains(&s) {
            sources.push(s);
        }
    };
    match strategy {
        // Window/app presence: prefer observation (desktop window set), then the
        // active-window probe (frontmost), then a running-process probe — the
        // Issue #2 honest fallback when no window is observable on Wayland.
        GuiVerificationStrategy::WindowVisible => {
            push(E::ActiveWindowProbe);
            push(E::Process);
        }
        GuiVerificationStrategy::ActiveWindowMatch => {
            push(E::Observation);
            push(E::Process);
        }
        // Accessibility predicates (focus / typed text): on Wayland with a11y off
        // the field is unreadable, so a screen change (observation) is the honest
        // secondary signal that the action had an effect.
        GuiVerificationStrategy::FocusedControl
        | GuiVerificationStrategy::TextPresent
        | GuiVerificationStrategy::TargetResolved => {
            push(E::Observation);
        }
        // Observation predicates: the active-window probe is a coarse secondary
        // (e.g. a navigation that changed the frontmost window/title).
        GuiVerificationStrategy::StateChanged
        | GuiVerificationStrategy::ScreenChanged
        | GuiVerificationStrategy::ResultVisible
        | GuiVerificationStrategy::DialogVisible
        | GuiVerificationStrategy::FileSaved
        | GuiVerificationStrategy::DownloadStartedOrCompleted
        | GuiVerificationStrategy::VisibleContentSummarized => {
            push(E::ActiveWindowProbe);
        }
        // Clipboard is a backend receipt only — NEVER an OCR/screenshot fallback
        // for a state-change verdict (Requirement 10.2).
        GuiVerificationStrategy::ClipboardChanged | GuiVerificationStrategy::Inconclusive => {}
    }
    sources
}

/// Task 4 (Issue #10): whether the PRIMARY evidence source for a predicate is
/// reliable given the post-action observation's capability signals. When it is
/// NOT reliable, a `verification_failed` verdict is downgraded to the honest
/// `inconclusive` (never a false `failed`) by [`apply_evidence_fallback`].
pub fn primary_evidence_reliable(
    strategy: GuiVerificationStrategy,
    accessibility_ok: bool,
    screenshot_available: bool,
    active_window_probe_ok: bool,
) -> bool {
    match evidence_source_for_strategy(strategy) {
        GuiVerificationEvidenceSource::Accessibility => accessibility_ok,
        GuiVerificationEvidenceSource::Observation => screenshot_available,
        GuiVerificationEvidenceSource::ActiveWindowProbe => active_window_probe_ok,
        // Process is never a PRIMARY source for any predicate (it is only ever a
        // fallback in the ordered chain), so it is not "primary-reliable".
        GuiVerificationEvidenceSource::Process => false,
        // A backend receipt is always available; "none" has no evidence.
        GuiVerificationEvidenceSource::BackendReceipt => true,
        GuiVerificationEvidenceSource::None => false,
    }
}
/// 1. PREDICATE — the post-state that proves success (the [`GuiVerificationStrategy`]).
/// 2. EVIDENCE — the source used to check the predicate ([`GuiVerificationEvidenceSource`]);
///    never OCR-only / coordinate guesses (Requirement 23).
/// 3. BOUNDED WAIT — a small, capped re-observe budget (Task 1 caps); never an
///    unbounded poll (`bounded_wait_ms` / `max_reobserve`).
/// 4. CONFIDENCE — the minimum confidence a `verified` verdict must carry, below
///    which the honest verdict is `inconclusive`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiVerificationContract {
    pub action_type: String,
    pub predicate: String,
    pub evidence_source: String,
    /// Task 4 (Issue #10): the ORDERED evidence sources for this action type —
    /// primary first, then honest fallbacks. The verifier tries them in order;
    /// if the primary is unavailable/unreliable the verdict is the honest
    /// `inconclusive` (never a false `verification_failed`). Additive + serde
    /// default so flag-OFF deserialization is byte-for-byte unchanged.
    #[serde(default)]
    pub evidence_sources: Vec<String>,
    pub bounded_wait_ms: u64,
    pub max_reobserve: u32,
    pub min_confidence: f64,
}

impl GuiVerificationContract {
    /// Non-revealing JSON summary for additive telemetry (no payload/secret).
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "action_type": self.action_type,
            "predicate": self.predicate,
            "evidence_source": self.evidence_source,
            "evidence_sources": self.evidence_sources,
            "bounded_wait_ms": self.bounded_wait_ms,
            "max_reobserve": self.max_reobserve,
            "min_confidence": self.min_confidence,
        })
    }
}

/// Derive the verification CONTRACT for an action type (Task 9.1).
///
/// The predicate is the action-specific [`GuiVerificationStrategy`]
/// ([`select_verification_strategy`]); the evidence source follows that
/// predicate ([`evidence_source_for_strategy`]); the bounded wait reuses the
/// caller's Task 1 caps (`bounded_wait_ms` = per-step verify budget,
/// `max_reobserve` = the effective re-observe cap) so verification can never
/// poll unbounded; and the minimum confidence is
/// [`VERIFICATION_CONTRACT_MIN_CONFIDENCE`].
pub fn verification_contract_for(
    action: &GuiActionKind,
    is_secret_payload: bool,
    bounded_wait_ms: u64,
    max_reobserve: u32,
) -> GuiVerificationContract {
    let predicate = select_verification_strategy(action, is_secret_payload);
    GuiVerificationContract {
        action_type: action.as_str().to_string(),
        predicate: predicate.as_str().to_string(),
        evidence_source: evidence_source_for_strategy(predicate).as_str().to_string(),
        evidence_sources: ordered_evidence_for_strategy(predicate)
            .into_iter()
            .map(|s| s.as_str().to_string())
            .collect(),
        bounded_wait_ms,
        max_reobserve,
        min_confidence: VERIFICATION_CONTRACT_MIN_CONFIDENCE,
    }
}

/// Flag-aware variant of [`verification_contract_for`] (Phase 1, Requirement
/// 1.1). When `verify_live_enabled` is true the OpenApp predicate is
/// `window_visible` (evidence `observation`); otherwise this is byte-for-byte
/// identical to [`verification_contract_for`].
pub fn verification_contract_for_with_flag(
    action: &GuiActionKind,
    is_secret_payload: bool,
    bounded_wait_ms: u64,
    max_reobserve: u32,
    verify_live_enabled: bool,
) -> GuiVerificationContract {
    let predicate =
        select_verification_strategy_with_flag(action, is_secret_payload, verify_live_enabled);
    GuiVerificationContract {
        action_type: action.as_str().to_string(),
        predicate: predicate.as_str().to_string(),
        evidence_source: evidence_source_for_strategy(predicate).as_str().to_string(),
        evidence_sources: ordered_evidence_for_strategy(predicate)
            .into_iter()
            .map(|s| s.as_str().to_string())
            .collect(),
        bounded_wait_ms,
        max_reobserve,
        min_confidence: VERIFICATION_CONTRACT_MIN_CONFIDENCE,
    }
}

/// Apply a verification CONTRACT to a verdict (Task 9.1, additive / flag-ON
/// only).
///
/// KRIA is verifier-aware: a `verified` verdict is honest only when the
/// evidence is reliable AND the confidence clears the contract's bar. This
/// downgrades a weak `verified` to the explicit `inconclusive` verdict when:
/// - the active-window probe is the evidence source but it is unreliable
///   (`active_window_probe_ok == false`), so "the window matched" cannot be
///   trusted, OR
/// - the confidence is below the contract's `min_confidence`.
///
/// It NEVER upgrades a verdict, NEVER turns a `failed`/`blocked` into something
/// softer, and NEVER fabricates a `verified`. A `verified` with reliable
/// evidence above the bar is returned unchanged. Callers run this ONLY when the
/// `gui_cog_safety_polish` flag is ON; while OFF the verdict is untouched.
pub fn apply_verification_contract(
    result: &GuiPostActionVerificationResult,
    contract: &GuiVerificationContract,
    active_window_probe_ok: bool,
) -> GuiPostActionVerificationResult {
    // Only a currently-`verified` verdict can be downgraded to `inconclusive`.
    if result.status != VERIFICATION_VERIFIED {
        return result.clone();
    }

    let predicate = GuiVerificationStrategy::from_str(&contract.predicate);
    let evidence_is_active_window = matches!(
        evidence_source_for_strategy(predicate),
        GuiVerificationEvidenceSource::ActiveWindowProbe
    );
    let unreliable_active_window = evidence_is_active_window && !active_window_probe_ok;
    let below_confidence_bar = result.confidence < contract.min_confidence;

    if !unreliable_active_window && !below_confidence_bar {
        return result.clone();
    }

    let mut downgraded = result.clone();
    downgraded.status = VERIFICATION_INCONCLUSIVE.into();
    downgraded.matched_expected_state = false;
    if unreliable_active_window {
        downgraded.evidence.push(safe_token(
            "verification contract: active-window evidence is unreliable; honest verdict is inconclusive (not a false verified)",
            200,
        ));
    }
    if below_confidence_bar {
        downgraded.evidence.push(safe_token(
            "verification contract: confidence below the contract bar; honest verdict is inconclusive (not a false verified)",
            200,
        ));
    }
    downgraded.safe_error_summary =
        Some("Post-action state could not be confirmed from available evidence.".into());
    downgraded.recovery_hint = Some(
        "Re-observe and confirm the expected state before retrying; do not blind-retry.".into(),
    );
    downgraded
}

/// Environment variable that enables the `gui_cog_verify_evidence` flag (Task 4).
pub const VERIFY_EVIDENCE_ENV_FLAG: &str = "KRIA_GUI_COG_VERIFY_EVIDENCE";

/// Task 4 (Issue #10): whether the ordered-evidence fallback is active. Default
/// ON; rollback via `KRIA_GUI_COG_VERIFY_EVIDENCE` set to a falsy value
/// (`0`/`false`/`no`/`off`/empty), which restores the prior single-strategy
/// verdict byte-for-byte. An absent env value keeps the flag ON.
pub fn verify_evidence_enabled() -> bool {
    verify_evidence_enabled_lookup(|key| std::env::var(key).ok())
}

/// Testable core of [`verify_evidence_enabled`] with an injectable lookup.
pub fn verify_evidence_enabled_lookup<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    match lookup(VERIFY_EVIDENCE_ENV_FLAG) {
        Some(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off" | ""
        ),
        None => true,
    }
}

/// Task 4 (Issue #10): apply the ordered-evidence honesty rule (additive,
/// flag-ON only).
///
/// When the flag is ON and the current verdict is `verification_failed` BUT the
/// predicate's PRIMARY evidence source was UNAVAILABLE/UNRELIABLE (a11y off for
/// an accessibility predicate, no screenshot for an observation predicate, an
/// unreliable active-window probe), the verdict is downgraded to the honest
/// `inconclusive` — KRIA could not confirm success OR failure, so it must not
/// report a FALSE `verification_failed`. It NEVER upgrades a verdict to
/// `verified` (that stays the job of a reliable positive predicate, so a false
/// `verified` is impossible here), and it leaves a genuine `failed` (primary
/// evidence WAS reliable and showed no change) untouched. While the flag is OFF
/// this is a no-op — byte-for-byte the prior verdict.
pub fn apply_evidence_fallback(
    result: &GuiPostActionVerificationResult,
    accessibility_ok: bool,
    screenshot_available: bool,
    active_window_probe_ok: bool,
    enabled: bool,
) -> GuiPostActionVerificationResult {
    if !enabled || result.status != VERIFICATION_FAILED {
        return result.clone();
    }
    let predicate = GuiVerificationStrategy::from_str(&result.verification_strategy);
    let primary_reliable = primary_evidence_reliable(
        predicate,
        accessibility_ok,
        screenshot_available,
        active_window_probe_ok,
    );
    // The primary evidence WAS reliable and still showed no change → a genuine
    // failure; do NOT soften it.
    if primary_reliable {
        return result.clone();
    }
    let mut downgraded = result.clone();
    downgraded.status = VERIFICATION_INCONCLUSIVE.into();
    downgraded.matched_expected_state = false;
    downgraded.evidence.push(safe_token(
        "verification evidence: the primary evidence source was unavailable/unreliable; \
         honest verdict is inconclusive (not a false verification_failed)",
        220,
    ));
    downgraded.safe_error_summary =
        Some("Post-action state could not be confirmed from available evidence.".into());
    downgraded.recovery_hint = Some(
        "Re-observe with a reliable evidence source before retrying; do not blind-retry.".into(),
    );
    downgraded
}

/// Task 4 (Issue #10): the SECONDARY (fallback) evidence signals, computed by the
/// caller from the pre/post observations. When the PRIMARY evidence source for a
/// predicate is unavailable/unreliable (so the core verdict could not be a
/// reliable `verified`), the ordered model tries these fallbacks IN ORDER; the
/// FIRST reliable source that positively confirms a real, observable effect
/// upgrades the honest `inconclusive` to `verified`.
///
/// Requirement 10.2 (Task 4.2): NONE of these is OCR- or coordinate-derived. A
/// screen change is a screen-HASH delta (not OCR text), the active-window signal
/// is the window probe, and `process_running` is a real process probe. OCR text
/// and coordinate guesses are structurally excluded from the evidence taxonomy
/// ([`GuiVerificationEvidenceSource`] has no OCR/coordinate variant), so they can
/// NEVER be the sole evidence for a state-change verdict.
#[derive(Debug, Clone, Copy, Default)]
pub struct GuiSecondaryEvidence {
    /// The screen HASH changed between pre and post (Observation evidence).
    pub screen_changed: bool,
    /// The active window changed/became known after the action (ActiveWindowProbe).
    pub active_window_changed: bool,
    /// A real running-process probe confirmed the launched app (Process evidence).
    pub process_running: bool,
    /// Whether the accessibility primary is reliable (a11y bus up).
    pub accessibility_ok: bool,
    /// Whether the observation/screenshot primary is reliable.
    pub screenshot_available: bool,
    /// Whether the active-window probe is reliable.
    pub active_window_probe_ok: bool,
}

/// Whether a given fallback evidence source POSITIVELY confirms a real effect,
/// given the secondary signals. Only the structured, non-OCR sources can confirm
/// (Requirement 10.2); `Accessibility`/`BackendReceipt`/`None` never act as a
/// soft secondary confirmation for a state-change/app-presence verdict.
fn secondary_source_confirms(
    source: GuiVerificationEvidenceSource,
    ev: &GuiSecondaryEvidence,
) -> bool {
    match source {
        GuiVerificationEvidenceSource::Observation => ev.screenshot_available && ev.screen_changed,
        GuiVerificationEvidenceSource::ActiveWindowProbe => {
            ev.active_window_probe_ok && ev.active_window_changed
        }
        GuiVerificationEvidenceSource::Process => ev.process_running,
        GuiVerificationEvidenceSource::Accessibility
        | GuiVerificationEvidenceSource::BackendReceipt
        | GuiVerificationEvidenceSource::None => false,
    }
}

/// The FIRST fallback source (after the primary) in the predicate's ordered
/// evidence chain that positively confirms a real effect, or `None` if no
/// reliable secondary confirms. Clipboard/backend-receipt predicates have no
/// visual fallback by design, so they never confirm via a secondary here.
fn first_confirming_secondary(
    strategy: GuiVerificationStrategy,
    ev: &GuiSecondaryEvidence,
) -> Option<GuiVerificationEvidenceSource> {
    let ordered = ordered_evidence_for_strategy(strategy);
    // Skip the primary (index 0); the primary was already evaluated by the core
    // verifier. Try the honest fallbacks in their documented order.
    ordered
        .into_iter()
        .skip(1)
        .find(|&source| secondary_source_confirms(source, ev))
}

/// Task 4 (Issue #10): apply the ORDERED-EVIDENCE honesty rule (additive,
/// flag-ON only) — the full ordered model that generalizes the Task-2 browser
/// `screen_changed` override into a per-predicate evidence chain.
///
/// The core verifier evaluates the PRIMARY evidence source. This step tries the
/// ordered FALLBACK sources when the primary was unavailable/unreliable:
///
/// - A `verification_failed` whose primary was UNRELIABLE (a11y off for an
///   accessibility predicate, no screenshot for an observation predicate, an
///   unreliable active-window probe) is NOT a real failure. If a reliable
///   secondary source positively confirms a real effect (a screen-hash change,
///   an active-window change, or a running process) the honest verdict is
///   `verified`; otherwise it is the honest `inconclusive` — never a false
///   `verification_failed`.
/// - An `inconclusive` (primary could not be evaluated) is upgraded to `verified`
///   when a reliable secondary confirms, else stays `inconclusive`.
/// - A `verification_failed` whose primary WAS reliable is a GENUINE failure and
///   is left untouched (a secondary never overrides a reliable negative).
/// - A `verified` or `blocked` verdict is never touched (no false `verified`,
///   never softens a `blocked`).
///
/// Requirement 10.2: the secondary confirmation is screen-hash / active-window /
/// process based — NEVER OCR text or a coordinate guess. While the flag is OFF
/// this is a byte-for-byte no-op.
pub fn apply_ordered_evidence(
    result: &GuiPostActionVerificationResult,
    secondary: &GuiSecondaryEvidence,
    enabled: bool,
) -> GuiPostActionVerificationResult {
    if !enabled {
        return result.clone();
    }
    let status = result.status.as_str();
    if status != VERIFICATION_FAILED && status != VERIFICATION_INCONCLUSIVE {
        // verified / blocked are never touched.
        return result.clone();
    }

    let strategy = GuiVerificationStrategy::from_str(&result.verification_strategy);
    let primary_reliable = primary_evidence_reliable(
        strategy,
        secondary.accessibility_ok,
        secondary.screenshot_available,
        secondary.active_window_probe_ok,
    );

    // A reliably-evidenced failure is a GENUINE failure — never softened or
    // overridden by a secondary source.
    if status == VERIFICATION_FAILED && primary_reliable {
        return result.clone();
    }

    // The primary was unavailable/unreliable. Try the ordered fallback sources.
    if let Some(source) = first_confirming_secondary(strategy, secondary) {
        // Requirement 10.2: the confirming source is structurally non-OCR.
        debug_assert!(
            matches!(
                source,
                GuiVerificationEvidenceSource::Observation
                    | GuiVerificationEvidenceSource::ActiveWindowProbe
                    | GuiVerificationEvidenceSource::Process
            ),
            "a state-change verdict must never be confirmed by OCR/coordinate evidence"
        );
        let mut upgraded = result.clone();
        upgraded.status = VERIFICATION_VERIFIED.into();
        upgraded.matched_expected_state = true;
        // A secondary-confirmed verdict is honest but slightly less certain than a
        // direct primary match; keep it above the contract bar so the safety
        // polish step does not re-downgrade an honest verified.
        upgraded.confidence = upgraded.confidence.max(0.8);
        upgraded.safe_error_summary = None;
        upgraded.recovery_hint = None;
        upgraded.can_retry = false;
        upgraded.evidence.push(safe_token(
            &format!(
                "ordered evidence: primary unavailable; confirmed by secondary source ({})",
                source.as_str()
            ),
            200,
        ));
        return upgraded;
    }

    // No reliable secondary confirmed a real effect.
    if status == VERIFICATION_INCONCLUSIVE {
        // Already honest inconclusive — leave it.
        return result.clone();
    }

    // A `verification_failed` with an unreliable primary and no confirming
    // secondary is really "could not tell" → the honest `inconclusive`.
    let mut downgraded = result.clone();
    downgraded.status = VERIFICATION_INCONCLUSIVE.into();
    downgraded.matched_expected_state = false;
    downgraded.evidence.push(safe_token(
        "ordered evidence: the primary evidence source was unavailable/unreliable and no \
         secondary source confirmed; honest verdict is inconclusive (not a false verification_failed)",
        240,
    ));
    downgraded.safe_error_summary =
        Some("Post-action state could not be confirmed from available evidence.".into());
    downgraded.recovery_hint = Some(
        "Re-observe with a reliable evidence source before retrying; do not blind-retry.".into(),
    );
    downgraded
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiPostActionVerificationRequest {
    pub verification_id: String,
    pub execution_id: String,
    pub proposal_id: String,
    pub proposal_hash: String,
    pub action_type: String,
    pub target_hash: String,
    pub stable_target_identity_hash: Option<String>,
    pub expected_postcondition: String,
    pub verification_strategy: String,
    pub pre_action_context_id: String,
    pub post_action_observation_id: String,
    pub post_action_context_id: String,
    pub started_at_ms: i64,
    pub is_secret_payload: bool,
    pub prompt_hash: String,
    pub target_label: Option<String>,
    pub target_role: Option<String>,
    pub target_control_id: Option<String>,
    pub expected_app_hint: Option<String>,
    pub expected_window_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiPostActionVerificationResult {
    pub verification_id: String,
    pub execution_id: String,
    pub proposal_id: String,
    pub status: String,
    pub verification_strategy: String,
    pub evidence: Vec<String>,
    pub pre_state_summary: String,
    pub post_state_summary: String,
    pub matched_expected_state: bool,
    pub target_still_present: bool,
    pub target_identity_matches: bool,
    pub confidence: f64,
    pub safe_error_summary: Option<String>,
    pub recovery_hint: Option<String>,
    pub can_retry: bool,
    pub prompt_hash: String,
}

pub const VERIFICATION_VERIFIED: &str = "verified";
pub const VERIFICATION_FAILED: &str = "verification_failed";
pub const VERIFICATION_INCONCLUSIVE: &str = "inconclusive";
pub const VERIFICATION_BLOCKED: &str = "blocked";

impl GuiPostActionVerificationResult {
    pub fn is_verified(&self) -> bool {
        self.status == VERIFICATION_VERIFIED
    }

    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "verification_id": self.verification_id,
            "execution_id": self.execution_id,
            "proposal_id": self.proposal_id,
            "status": self.status,
            "verification_strategy": self.verification_strategy,
            "evidence": self.evidence,
            "pre_state_summary": self.pre_state_summary,
            "post_state_summary": self.post_state_summary,
            "matched_expected_state": self.matched_expected_state,
            "target_still_present": self.target_still_present,
            "target_identity_matches": self.target_identity_matches,
            "confidence": self.confidence,
            "safe_error_summary": self.safe_error_summary,
            "recovery_hint": self.recovery_hint,
            "can_retry": self.can_retry,
            "prompt_hash": self.prompt_hash,
        })
    }

    pub fn event_payload(&self) -> serde_json::Value {
        let mut payload = self.summary_json();
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                "type".into(),
                serde_json::Value::String("ExecutionVerificationCompleted".into()),
            );
        }
        payload
    }
}

fn idempotent_retry(action: &GuiActionKind) -> bool {
    matches!(
        action,
        GuiActionKind::OpenApp
            | GuiActionKind::SwitchWindow
            | GuiActionKind::FocusField
            | GuiActionKind::Scroll
    )
}

fn safe_token(value: &str, limit: usize) -> String {
    sanitize_gui_text(value, limit).text
}

fn screen_hash_prefix(observation: &GuiObservationSnapshot) -> String {
    observation
        .screen_hash
        .as_deref()
        .map(|hash| hash.chars().take(8).collect::<String>())
        .unwrap_or_else(|| "unknown".into())
}

fn state_summary(observation: &GuiObservationSnapshot) -> String {
    let focus_role = observation
        .cursor_focus
        .focused_control_role
        .as_deref()
        .map(|role| safe_token(role, 40))
        .unwrap_or_else(|| "none".into());
    format!(
        "app={}; controls={}; dialogs={}; focus_role={}; screen={}",
        safe_token(
            observation
                .active_window
                .app_name
                .as_deref()
                .unwrap_or("unknown"),
            60
        ),
        observation.visible_control_count(),
        observation.dialogs.len(),
        focus_role,
        screen_hash_prefix(observation),
    )
}

fn text_contains(haystack: &str, needle: &str) -> bool {
    if needle.trim().is_empty() {
        return false;
    }
    haystack.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
}

fn find_target_control<'a>(
    observation: &'a GuiObservationSnapshot,
    control_id: Option<&str>,
    label: Option<&str>,
    role: Option<&str>,
) -> Option<GuiControlSummary> {
    let controls = observation.all_controls();
    if let Some(control_id) = control_id.filter(|value| !value.trim().is_empty()) {
        if let Some(found) = controls.iter().find(|control| control.control_id == control_id) {
            return Some(found.clone());
        }
    }
    if let Some(label) = label.filter(|value| !value.trim().is_empty()) {
        if let Some(found) = controls.iter().find(|control| {
            control.name.eq_ignore_ascii_case(label)
                && role
                    .map(|role| control.role.eq_ignore_ascii_case(role))
                    .unwrap_or(true)
        }) {
            return Some(found.clone());
        }
    }
    None
}

fn screen_changed(
    pre: &GuiObservationSnapshot,
    post: &GuiObservationSnapshot,
) -> Option<bool> {
    match (pre.screen_hash.as_deref(), post.screen_hash.as_deref()) {
        (Some(before), Some(after)) => Some(before != after),
        _ => None,
    }
}

fn focus_changed(pre: &GuiObservationSnapshot, post: &GuiObservationSnapshot) -> bool {
    pre.cursor_focus.focused_control_id != post.cursor_focus.focused_control_id
}

fn window_token_match(post: &GuiObservationSnapshot, hint: &str) -> bool {
    let hint = hint.trim();
    if hint.len() < 3 {
        return false;
    }
    let label = post.active_window.label.to_ascii_lowercase();
    let app = post
        .active_window
        .app_name
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let hint_lower = hint.to_ascii_lowercase();
    if label.contains(&hint_lower) || app.contains(&hint_lower) {
        return true;
    }
    // Token-wise match so "Google Search - Chrome" matches a "Chrome" hint.
    hint_lower
        .split_whitespace()
        .filter(|token| token.len() >= 3)
        .any(|token| label.contains(token) || app.contains(token))
        || post.visible_windows.iter().any(|window| {
            window.title.to_ascii_lowercase().contains(&hint_lower)
                || window
                    .app_name
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains(&hint_lower)
        })
}

fn text_present_in_observation(post: &GuiObservationSnapshot, needle: &str) -> bool {
    post.all_controls()
        .iter()
        .any(|control| text_contains(&control.name, needle))
        || post
            .ocr_blocks
            .iter()
            .any(|block| text_contains(&block.safe_text_preview, needle))
}

/// Run deterministic post-action verification. The verifier re-observes are
/// passed in by the caller (`pre_observation` = pre-action context observation,
/// `post_observation` = re-observed snapshot after the action attempt).
///
/// `expected_text` is the backend-only raw payload for non-secret typing/paste
/// actions. It is used to confirm presence but is never written into the
/// result. For secret payloads, callers must pass `None` and rely on
/// `state_changed`.
pub fn verify_post_action_detailed(
    request: &GuiPostActionVerificationRequest,
    pre_observation: &GuiObservationSnapshot,
    post_observation: &GuiObservationSnapshot,
    backend_success: bool,
    expected_text: Option<&str>,
    now_ms: i64,
) -> GuiPostActionVerificationResult {
    // Byte-for-byte prior behavior: no process-launched evidence source. The
    // process check is opt-in via [`verify_post_action_detailed_with_process`]
    // and is only ever supplied for OpenApp under the `gui_cog_verify_live` flag.
    verify_post_action_detailed_with_process(
        request,
        pre_observation,
        post_observation,
        backend_success,
        expected_text,
        now_ms,
        None,
    )
}

/// Issue #2: [`verify_post_action_detailed`] plus an OpenApp PROCESS-LAUNCHED
/// evidence source.
///
/// `app_process_evidence` is the matched binary name (e.g. `nautilus`) when the
/// launched app's process is RUNNING — computed by the caller via the mockable
/// [`crate::agent::gui_cognition::perception::app_process_running`] probe — or
/// `None` when no process evidence is available. It is consulted ONLY by the
/// `WindowVisible` predicate (OpenApp under `gui_cog_verify_live`): when no
/// window for the app is observable but its process is running, the verdict is
/// `verified` with evidence `app_running:<binary>`. When neither a window nor a
/// process matches, the honest non-verified verdict is kept — a `verified` is
/// NEVER fabricated without real evidence. Every other predicate ignores this
/// argument, so `SwitchWindow` and all other actions are unaffected.
pub fn verify_post_action_detailed_with_process(
    request: &GuiPostActionVerificationRequest,
    pre_observation: &GuiObservationSnapshot,
    post_observation: &GuiObservationSnapshot,
    backend_success: bool,
    expected_text: Option<&str>,
    _now_ms: i64,
    app_process_evidence: Option<&str>,
) -> GuiPostActionVerificationResult {
    let action_kind = GuiActionKind::from_action_type(&request.action_type);
    let strategy = GuiVerificationStrategy::from_str(&request.verification_strategy);
    let pre_state_summary = state_summary(pre_observation);
    let post_state_summary = state_summary(post_observation);

    let target_control = find_target_control(
        post_observation,
        request.target_control_id.as_deref(),
        request.target_label.as_deref(),
        request.target_role.as_deref(),
    );
    let target_still_present = target_control.is_some();
    let target_identity_matches = match (&target_control, request.stable_target_identity_hash.as_deref()) {
        (Some(control), Some(expected_hash)) => {
            let recomputed = stable_target_identity_hash(
                Some(&control.control_id),
                Some(&control.role),
                Some(&control.name),
                control.bounds.as_ref(),
                request.expected_app_hint.as_deref(),
                request.expected_window_hint.as_deref(),
            );
            recomputed == expected_hash
        }
        // No stable identity recorded (e.g. OpenApp) -> not a mismatch.
        (_, None) => true,
        (None, Some(_)) => false,
    };

    let mut evidence: Vec<String> = Vec::new();

    // Backend did not complete: there is nothing to verify. Fail closed.
    if !backend_success {
        evidence.push("backend action did not complete; no state to verify".into());
        return GuiPostActionVerificationResult {
            verification_id: request.verification_id.clone(),
            execution_id: request.execution_id.clone(),
            proposal_id: request.proposal_id.clone(),
            status: VERIFICATION_BLOCKED.into(),
            verification_strategy: strategy.as_str().into(),
            evidence,
            pre_state_summary,
            post_state_summary,
            matched_expected_state: false,
            target_still_present,
            target_identity_matches,
            confidence: 0.2,
            safe_error_summary: Some("Backend action failed before verification.".into()),
            recovery_hint: Some(
                "Re-observe the screen and resolve a fresh target before any retry.".into(),
            ),
            can_retry: false,
            prompt_hash: request.prompt_hash.clone(),
        };
    }

    let changed = screen_changed(pre_observation, post_observation);

    // matched: Some(true|false) when the strategy could be evaluated, None when
    // the available evidence is insufficient (=> inconclusive, never blind pass).
    let matched: Option<bool> = match strategy {
        GuiVerificationStrategy::WindowVisible => {
            // Phase 1 (Requirement 1.1/1.3): the app's window is PRESENT/visible
            // in the desktop open-window set (alias-tolerant by app_name/title) —
            // NOT necessarily the focused/active window. On Wayland a freshly
            // launched app may not steal focus, so presence (not active match) is
            // the correct, strong evidence that the app opened.
            let hint = request
                .expected_window_hint
                .as_deref()
                .filter(|value| value.trim().len() >= 2)
                .or(request.expected_app_hint.as_deref())
                .filter(|value| value.trim().len() >= 2)
                .or(request.target_label.as_deref())
                .filter(|value| value.trim().len() >= 2);
            match hint {
                Some(hint) => {
                    let window_ok = post_observation.window_visible_for_app(hint);
                    if window_ok {
                        evidence.push(format!(
                            "app window is present/visible in the desktop window set ({})",
                            safe_token(hint, 60)
                        ));
                        Some(true)
                    } else if let Some(proc_name) = app_process_evidence
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        // Issue #2: no observable window (Wayland focus-stealing
                        // prevention + GNOME Eval disabled => no usable window
                        // list), but the launched app's PROCESS is running — the
                        // strong, correct evidence that "the app opened".
                        evidence.push(format!(
                            "app process is running (app_running:{})",
                            safe_token(proc_name, 60)
                        ));
                        Some(true)
                    } else {
                        evidence.push(format!(
                            "app window is not yet present in the desktop window set ({})",
                            safe_token(hint, 60)
                        ));
                        Some(false)
                    }
                }
                None => {
                    if let Some(proc_name) = app_process_evidence
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        // Issue #2: no app/window hint, but the launched app's
                        // process is running — real evidence the app opened.
                        evidence.push(format!(
                            "app process is running (app_running:{})",
                            safe_token(proc_name, 60)
                        ));
                        Some(true)
                    } else {
                        // No app/window hint and no process evidence: presence
                        // cannot be confirmed for a specific app. Honest
                        // inconclusive rather than a false pass.
                        evidence.push(
                            "no expected app/window hint provided; window presence is inconclusive"
                                .into(),
                        );
                        None
                    }
                }
            }
        }
        GuiVerificationStrategy::ActiveWindowMatch => {
            let hint = request
                .expected_window_hint
                .as_deref()
                .filter(|value| value.trim().len() >= 3)
                .or(request.expected_app_hint.as_deref())
                .filter(|value| value.trim().len() >= 3)
                .or(request.target_label.as_deref())
                .filter(|value| value.trim().len() >= 3);
            match hint {
                Some(hint) => {
                    let ok = window_token_match(post_observation, hint);
                    if ok {
                        evidence.push(format!(
                            "active window matches expected app/window hint ({})",
                            safe_token(hint, 60)
                        ));
                    } else {
                        evidence.push(format!(
                            "active window did not match expected app/window hint ({})",
                            safe_token(hint, 60)
                        ));
                    }
                    Some(ok)
                }
                None => {
                    if post_observation.active_window_probe_ok
                        && post_observation.active_window.confidence > 0.0
                    {
                        evidence.push(
                            "active window is known but no expected app/window hint was provided"
                                .into(),
                        );
                        None
                    } else {
                        evidence.push("active window is unknown after the action".into());
                        Some(false)
                    }
                }
            }
        }
        GuiVerificationStrategy::FocusedControl => {
            let focus = &post_observation.cursor_focus;
            let id_match = match (
                focus.focused_control_id.as_deref(),
                request.target_control_id.as_deref(),
            ) {
                (Some(found), Some(expected)) if !expected.trim().is_empty() => found == expected,
                _ => false,
            };
            let label_match = match (
                focus.focused_control_label.as_deref(),
                request.target_label.as_deref(),
            ) {
                (Some(found), Some(expected)) if !expected.trim().is_empty() => {
                    found.eq_ignore_ascii_case(expected)
                }
                _ => false,
            };
            let control_focused = post_observation.all_controls().iter().any(|control| {
                control.focused
                    && request
                        .target_label
                        .as_deref()
                        .map(|label| control.name.eq_ignore_ascii_case(label))
                        .unwrap_or(false)
            });
            if focus.keyboard_focus_known || control_focused {
                let ok = id_match || label_match || control_focused;
                if ok {
                    evidence.push("expected control reports keyboard focus".into());
                } else {
                    evidence.push("focus moved to a different control after the action".into());
                }
                Some(ok)
            } else {
                evidence.push("keyboard focus is not observable after the action".into());
                None
            }
        }
        GuiVerificationStrategy::TextPresent => match expected_text {
            Some(text) if !text.trim().is_empty() => {
                let ok = text_present_in_observation(post_observation, text);
                if ok {
                    evidence.push("expected text is present in the post-action GUI state".into());
                } else {
                    evidence
                        .push("expected text was not found in the post-action GUI state".into());
                }
                Some(ok)
            }
            _ => {
                evidence.push("no observable text payload to verify".into());
                None
            }
        },
        GuiVerificationStrategy::StateChanged => {
            let focus_moved = focus_changed(pre_observation, post_observation);
            match changed {
                Some(true) => {
                    evidence.push("screen state changed after the action".into());
                    Some(true)
                }
                Some(false) if focus_moved => {
                    evidence.push("focused control changed after the action".into());
                    Some(true)
                }
                Some(false) => {
                    evidence.push("no observable state change after the action".into());
                    Some(false)
                }
                None if focus_moved => {
                    evidence.push("focused control changed after the action".into());
                    Some(true)
                }
                None => {
                    evidence.push("screen state change could not be observed".into());
                    None
                }
            }
        }
        GuiVerificationStrategy::ScreenChanged => match changed {
            Some(value) => {
                if value {
                    evidence.push("screen content changed after the action".into());
                } else {
                    evidence.push("screen content did not change after the action".into());
                }
                Some(value)
            }
            None => {
                evidence.push("screen hash unavailable; screen change not observable".into());
                None
            }
        },
        GuiVerificationStrategy::ResultVisible => {
            let dialog = !post_observation.dialogs.is_empty();
            let postcondition_visible = !request.expected_postcondition.trim().is_empty()
                && post_observation.all_controls().iter().any(|control| {
                    request
                        .expected_postcondition
                        .to_ascii_lowercase()
                        .split_whitespace()
                        .filter(|token| token.len() >= 4)
                        .any(|token| text_contains(&control.name, token))
                });
            match changed {
                Some(true) => {
                    evidence.push("screen changed and a result is visible after the action".into());
                    Some(true)
                }
                _ if dialog => {
                    evidence.push("a dialog became visible after the action".into());
                    Some(true)
                }
                _ if postcondition_visible => {
                    evidence.push("expected result content is visible after the action".into());
                    Some(true)
                }
                Some(false) => {
                    evidence.push("screen did not change and no result became visible".into());
                    Some(false)
                }
                None => {
                    evidence.push("result visibility could not be observed".into());
                    None
                }
            }
        }
        GuiVerificationStrategy::DialogVisible => {
            let dialog = !post_observation.dialogs.is_empty();
            if dialog {
                evidence.push("expected dialog is visible after the action".into());
            } else {
                evidence.push("expected dialog was not visible after the action".into());
            }
            Some(dialog)
        }
        GuiVerificationStrategy::ClipboardChanged => {
            // Clipboard contents are never read into the observation pipeline, so
            // verification relies on the backend receipt only. Never echo content.
            evidence.push("clipboard change reported by backend; content not captured".into());
            Some(true)
        }
        GuiVerificationStrategy::FileSaved
        | GuiVerificationStrategy::DownloadStartedOrCompleted => match changed {
            Some(true) => {
                evidence.push("observable state changed consistent with the expected result".into());
                Some(true)
            }
            Some(false) => {
                evidence.push("no observable change for file/download verification".into());
                Some(false)
            }
            None => {
                evidence.push("file/download result is not observable from the GUI state".into());
                None
            }
        },
        GuiVerificationStrategy::TargetResolved => {
            if target_still_present {
                evidence.push("target remains resolved after the action".into());
                Some(true)
            } else {
                evidence.push("target is no longer present after the action".into());
                Some(false)
            }
        }
        GuiVerificationStrategy::VisibleContentSummarized => {
            evidence.push("visible content summary verification is not observable here".into());
            None
        }
        GuiVerificationStrategy::Inconclusive => {
            evidence.push("no deterministic verification strategy applied".into());
            None
        }
    };

    let (status, confidence) = match matched {
        Some(true) => {
            let conf = match strategy {
                GuiVerificationStrategy::ClipboardChanged
                | GuiVerificationStrategy::StateChanged => 0.86,
                _ => 0.9,
            };
            (VERIFICATION_VERIFIED, conf)
        }
        Some(false) => (VERIFICATION_FAILED, 0.2),
        None => (VERIFICATION_INCONCLUSIVE, 0.5),
    };

    // A resolved-but-missing or identity-mismatched target downgrades a control
    // action to failure; we never claim success when the bound target moved.
    let control_action = matches!(
        action_kind,
        GuiActionKind::FocusField
            | GuiActionKind::ClickControl
            | GuiActionKind::TypeText
            | GuiActionKind::FillField
            | GuiActionKind::Paste
    );
    let (status, confidence) = if status == VERIFICATION_VERIFIED
        && control_action
        && request.target_control_id.is_some()
        && (!target_still_present || !target_identity_matches)
    {
        evidence.push("bound target identity is no longer stable after the action".into());
        (VERIFICATION_FAILED, 0.2)
    } else {
        (status, confidence)
    };

    let safe_error_summary = match status {
        VERIFICATION_FAILED => Some(format!(
            "Expected post-action state was not verified for {}.",
            safe_token(&request.action_type, 40)
        )),
        VERIFICATION_INCONCLUSIVE => Some(
            "Post-action state could not be confirmed from available evidence.".into(),
        ),
        _ => None,
    };
    let recovery_hint = match status {
        VERIFICATION_FAILED | VERIFICATION_INCONCLUSIVE => Some(
            "Re-observe and confirm the expected state before retrying; do not blind-retry."
                .into(),
        ),
        _ => None,
    };
    let can_retry = matches!(status, VERIFICATION_FAILED | VERIFICATION_INCONCLUSIVE)
        && idempotent_retry(&action_kind);

    GuiPostActionVerificationResult {
        verification_id: request.verification_id.clone(),
        execution_id: request.execution_id.clone(),
        proposal_id: request.proposal_id.clone(),
        status: status.into(),
        verification_strategy: strategy.as_str().into(),
        evidence: evidence
            .into_iter()
            .map(|value| safe_token(&value, 200))
            .collect(),
        pre_state_summary,
        post_state_summary,
        matched_expected_state: status == VERIFICATION_VERIFIED,
        target_still_present,
        target_identity_matches,
        confidence,
        safe_error_summary,
        recovery_hint,
        can_retry,
        prompt_hash: request.prompt_hash.clone(),
    }
}

#[cfg(test)]
mod primitive_strategy_tests {
    //! Task 6.1 (Requirement 5 / 23) T1: each typed primitive maps to its
    //! correct verification strategy.
    use super::*;

    #[test]
    fn new_primitives_select_correct_verification_strategy() {
        assert_eq!(
            select_verification_strategy(&GuiActionKind::ClearField, false),
            GuiVerificationStrategy::StateChanged
        );
        assert_eq!(
            select_verification_strategy(&GuiActionKind::SelectAll, false),
            GuiVerificationStrategy::StateChanged
        );
        assert_eq!(
            select_verification_strategy(&GuiActionKind::SetCheckbox, false),
            GuiVerificationStrategy::ResultVisible
        );
        assert_eq!(
            select_verification_strategy(&GuiActionKind::InAppSearch, false),
            GuiVerificationStrategy::ResultVisible
        );
        assert_eq!(
            select_verification_strategy(&GuiActionKind::CloseDialog, false),
            GuiVerificationStrategy::DialogVisible
        );
    }
}

#[cfg(test)]
mod safety_polish_contract_tests {
    //! Task 9.1 (Requirements 10, 13, 15, 22, 23) T1/T2: the per-action-type
    //! verification CONTRACT (predicate + evidence + bounded wait + confidence),
    //! the honest `inconclusive` verdict for low-confidence / unreliable-evidence
    //! outcomes (never a false `verified`), and the `gui_cog_safety_polish` flag
    //! plumbing (default OFF + rollback). All CI-safe: no live desktop, display,
    //! or backend required.
    use super::*;

    fn verified_result(strategy: GuiVerificationStrategy, confidence: f64) -> GuiPostActionVerificationResult {
        GuiPostActionVerificationResult {
            verification_id: "v-1".into(),
            execution_id: "e-1".into(),
            proposal_id: "p-1".into(),
            status: VERIFICATION_VERIFIED.into(),
            verification_strategy: strategy.as_str().into(),
            evidence: vec!["base evidence".into()],
            pre_state_summary: "pre".into(),
            post_state_summary: "post".into(),
            matched_expected_state: true,
            target_still_present: true,
            target_identity_matches: true,
            confidence,
            safe_error_summary: None,
            recovery_hint: None,
            can_retry: false,
            prompt_hash: "ph".into(),
        }
    }

    // ── T1: FLAG PLUMBING (default OFF + rollback) ───────────────────────────

    #[test]
    fn t1_flag_defaults_off() {
        assert!(!GuiSafetyPolishConfig::default().is_enabled());
        assert!(GuiSafetyPolishConfig::enabled().is_enabled());
        assert!(!GuiSafetyPolishConfig::disabled().is_enabled());
    }

    #[test]
    fn t1_from_env_lookup_default_off_unless_truthy() {
        let off = GuiSafetyPolishConfig::from_env_lookup(|_| None);
        assert!(!off.is_enabled(), "absent env => OFF on the default-off path");
        for falsy in ["0", "false", "no", "off", "", "garbage"] {
            let cfg = GuiSafetyPolishConfig::from_env_lookup(|_| Some(falsy.to_string()));
            assert!(!cfg.is_enabled(), "{falsy:?} must be OFF");
        }
        for truthy in ["1", "true", "yes", "on", "ON", "  true  "] {
            let cfg = GuiSafetyPolishConfig::from_env_lookup(|_| Some(truthy.to_string()));
            assert!(cfg.is_enabled(), "{truthy:?} must be ON");
        }
    }

    #[test]
    fn t1_from_env_lookup_default_on_rollback_switch() {
        // Absent => ON (the wave-gate default, Task 9.7).
        assert!(GuiSafetyPolishConfig::from_env_lookup_default_on(|_| None).is_enabled());
        // Explicit falsy => the documented rollback switch (OFF).
        for falsy in ["0", "false", "no", "off", ""] {
            let cfg = GuiSafetyPolishConfig::from_env_lookup_default_on(|_| Some(falsy.to_string()));
            assert!(!cfg.is_enabled(), "{falsy:?} must roll back to OFF");
        }
        assert!(
            GuiSafetyPolishConfig::from_env_lookup_default_on(|_| Some("1".to_string())).is_enabled()
        );
    }

    #[test]
    fn t1_env_flag_const_is_stable() {
        assert_eq!(SAFETY_POLISH_ENV_FLAG, "KRIA_GUI_COG_SAFETY_POLISH");
    }

    // ── T1: CONTRACT PER ACTION TYPE (predicate + evidence + wait + confidence)

    #[test]
    fn t1_contract_per_action_type_predicate_and_evidence_are_correct() {
        // OpenApp: predicate = active_window_match, evidence = active-window probe.
        let open = verification_contract_for(&GuiActionKind::OpenApp, false, 4_000, 12);
        assert_eq!(open.predicate, "active_window_match");
        assert_eq!(open.evidence_source, "active_window_probe");

        // FocusField: predicate = focused_control, evidence = accessibility.
        let focus = verification_contract_for(&GuiActionKind::FocusField, false, 4_000, 12);
        assert_eq!(focus.predicate, "focused_control");
        assert_eq!(focus.evidence_source, "accessibility");

        // TypeText (non-secret): predicate = text_present, evidence = accessibility.
        let typ = verification_contract_for(&GuiActionKind::TypeText, false, 4_000, 12);
        assert_eq!(typ.predicate, "text_present");
        assert_eq!(typ.evidence_source, "accessibility");

        // TypeText (secret): predicate = state_changed (never text_present so a
        // secret is never searched for), evidence = observation.
        let secret = verification_contract_for(&GuiActionKind::TypeText, true, 4_000, 12);
        assert_eq!(secret.predicate, "state_changed");
        assert_eq!(secret.evidence_source, "observation");

        // Copy: predicate = clipboard_changed, evidence = backend receipt only.
        let copy = verification_contract_for(&GuiActionKind::Copy, false, 4_000, 12);
        assert_eq!(copy.predicate, "clipboard_changed");
        assert_eq!(copy.evidence_source, "backend_receipt");

        // PressKey: predicate = screen_changed, evidence = observation.
        let key = verification_contract_for(&GuiActionKind::PressKey, false, 4_000, 12);
        assert_eq!(key.predicate, "screen_changed");
        assert_eq!(key.evidence_source, "observation");

        // CloseDialog: predicate = dialog_visible, evidence = observation.
        let dialog = verification_contract_for(&GuiActionKind::CloseDialog, false, 4_000, 12);
        assert_eq!(dialog.predicate, "dialog_visible");
        assert_eq!(dialog.evidence_source, "observation");
    }

    #[test]
    fn t1_contract_carries_bounded_wait_and_confidence_bar() {
        let contract = verification_contract_for(&GuiActionKind::OpenApp, false, 5_000, 9);
        // BOUNDED WAIT: the caller's Task 1 caps are threaded verbatim (never an
        // unbounded poll).
        assert_eq!(contract.bounded_wait_ms, 5_000);
        assert_eq!(contract.max_reobserve, 9);
        // CONFIDENCE: the contract carries the explicit confidence bar.
        assert_eq!(contract.min_confidence, VERIFICATION_CONTRACT_MIN_CONFIDENCE);
        assert!((0.0..=1.0).contains(&contract.min_confidence));
        // Telemetry summary exposes all four contract guarantees.
        let json = contract.summary_json();
        assert_eq!(json["predicate"], "active_window_match");
        assert_eq!(json["evidence_source"], "active_window_probe");
        assert_eq!(json["bounded_wait_ms"], 5_000);
        assert_eq!(json["max_reobserve"], 9);
    }

    // ── T2: INCONCLUSIVE FOR LOW-CONFIDENCE / UNRELIABLE EVIDENCE ─────────────

    #[test]
    fn t2_unreliable_active_window_probe_downgrades_verified_to_inconclusive() {
        // A window-match verified verdict, but the active-window probe is
        // UNRELIABLE: the honest verdict is `inconclusive`, never a false verified.
        let contract = verification_contract_for(&GuiActionKind::OpenApp, false, 4_000, 12);
        let verified = verified_result(GuiVerificationStrategy::ActiveWindowMatch, 0.9);
        assert!(verified.is_verified());

        let probe_ok = false;
        let adjusted = apply_verification_contract(&verified, &contract, probe_ok);
        assert_eq!(adjusted.status, VERIFICATION_INCONCLUSIVE);
        assert!(!adjusted.is_verified(), "must not stay a false verified");
        assert!(!adjusted.matched_expected_state);
        assert!(adjusted.safe_error_summary.is_some());
        assert!(adjusted.recovery_hint.is_some());
        assert!(adjusted
            .evidence
            .iter()
            .any(|e| e.contains("inconclusive")));
    }

    #[test]
    fn t2_reliable_active_window_probe_keeps_verified_unchanged() {
        let contract = verification_contract_for(&GuiActionKind::OpenApp, false, 4_000, 12);
        let verified = verified_result(GuiVerificationStrategy::ActiveWindowMatch, 0.9);
        // Probe reliable + confidence above the bar => verdict untouched.
        let adjusted = apply_verification_contract(&verified, &contract, true);
        assert_eq!(adjusted.status, VERIFICATION_VERIFIED);
        assert!(adjusted.is_verified());
        assert_eq!(adjusted.evidence, verified.evidence, "no extra downgrade evidence");
    }

    #[test]
    fn t2_confidence_below_bar_downgrades_to_inconclusive() {
        // A non-active-window predicate whose confidence is below the contract
        // bar => honest verdict is `inconclusive` (not a false verified).
        let contract = verification_contract_for(&GuiActionKind::PressKey, false, 4_000, 12);
        let weak = verified_result(GuiVerificationStrategy::ScreenChanged, 0.5);
        let adjusted = apply_verification_contract(&weak, &contract, true);
        assert_eq!(adjusted.status, VERIFICATION_INCONCLUSIVE);
        assert!(!adjusted.is_verified());
    }

    #[test]
    fn t2_failed_and_blocked_are_never_softened_or_upgraded() {
        let contract = verification_contract_for(&GuiActionKind::OpenApp, false, 4_000, 12);
        // A failed verdict stays failed (the contract never upgrades it).
        let mut failed = verified_result(GuiVerificationStrategy::ActiveWindowMatch, 0.2);
        failed.status = VERIFICATION_FAILED.into();
        failed.matched_expected_state = false;
        let adjusted = apply_verification_contract(&failed, &contract, false);
        assert_eq!(adjusted.status, VERIFICATION_FAILED);

        // A blocked verdict stays blocked.
        let mut blocked = verified_result(GuiVerificationStrategy::ActiveWindowMatch, 0.2);
        blocked.status = VERIFICATION_BLOCKED.into();
        let adjusted_blocked = apply_verification_contract(&blocked, &contract, true);
        assert_eq!(adjusted_blocked.status, VERIFICATION_BLOCKED);
    }

    #[test]
    fn t2_non_active_window_predicate_ignores_probe_reliability() {
        // ScreenChanged evidence is observation, not the active-window probe, so
        // an unreliable probe does NOT downgrade a confident verified verdict.
        let contract = verification_contract_for(&GuiActionKind::PressKey, false, 4_000, 12);
        let verified = verified_result(GuiVerificationStrategy::ScreenChanged, 0.9);
        let adjusted = apply_verification_contract(&verified, &contract, false);
        assert_eq!(adjusted.status, VERIFICATION_VERIFIED);
    }

    #[test]
    fn t2_inconclusive_input_stays_inconclusive() {
        // The contract never converts an existing `inconclusive` into a verified.
        let contract = verification_contract_for(&GuiActionKind::OpenApp, false, 4_000, 12);
        let mut inconclusive = verified_result(GuiVerificationStrategy::ActiveWindowMatch, 0.5);
        inconclusive.status = VERIFICATION_INCONCLUSIVE.into();
        inconclusive.matched_expected_state = false;
        let adjusted = apply_verification_contract(&inconclusive, &contract, true);
        assert_eq!(adjusted.status, VERIFICATION_INCONCLUSIVE);
    }
}

#[cfg(test)]
mod verify_live_predicate_tests {
    //! Phase 1 (Requirement 1.1/1.5) T1/T2: the `gui_cog_verify_live` flag
    //! plumbing (default OFF + rollback), the flag-aware OpenApp predicate/
    //! evidence (`window_visible` + `observation` when ON), `SwitchWindow`
    //! staying `active_window_match`, and flag-OFF being byte-for-byte the prior
    //! `active_window_match` verdict. All CI-safe: no live desktop required.
    use super::*;

    // ── T1: FLAG PLUMBING (default OFF + rollback) ───────────────────────────

    #[test]
    fn t1_verify_live_flag_defaults_off() {
        assert!(!GuiVerifyLiveConfig::default().is_enabled());
        assert!(GuiVerifyLiveConfig::enabled().is_enabled());
        assert!(!GuiVerifyLiveConfig::disabled().is_enabled());
    }

    #[test]
    fn t1_verify_live_env_flag_const_is_stable() {
        assert_eq!(VERIFY_LIVE_ENV_FLAG, "KRIA_GUI_COG_VERIFY_LIVE");
    }

    #[test]
    fn t1_verify_live_from_env_lookup_default_off_unless_truthy() {
        assert!(!GuiVerifyLiveConfig::from_env_lookup(|_| None).is_enabled());
        for falsy in ["0", "false", "no", "off", "", "garbage"] {
            let cfg = GuiVerifyLiveConfig::from_env_lookup(|_| Some(falsy.to_string()));
            assert!(!cfg.is_enabled(), "{falsy:?} must be OFF");
        }
        for truthy in ["1", "true", "yes", "on", "ON", "  true  "] {
            let cfg = GuiVerifyLiveConfig::from_env_lookup(|_| Some(truthy.to_string()));
            assert!(cfg.is_enabled(), "{truthy:?} must be ON");
        }
    }

    #[test]
    fn t1_verify_live_from_env_lookup_default_on_rollback_switch() {
        // Absent => ON (the live-gate default; desktop wires from_env_default_on).
        assert!(GuiVerifyLiveConfig::from_env_lookup_default_on(|_| None).is_enabled());
        // Explicit falsy => the documented rollback switch (OFF).
        for falsy in ["0", "false", "no", "off", ""] {
            let cfg = GuiVerifyLiveConfig::from_env_lookup_default_on(|_| Some(falsy.to_string()));
            assert!(!cfg.is_enabled(), "{falsy:?} must roll back to OFF");
        }
        assert!(
            GuiVerifyLiveConfig::from_env_lookup_default_on(|_| Some("1".to_string())).is_enabled()
        );
    }

    // ── T2: FLAG-AWARE OPENAPP PREDICATE + EVIDENCE ──────────────────────────

    #[test]
    fn t2_open_app_predicate_is_window_visible_when_flag_on() {
        // Flag ON: OpenApp predicate == window_visible, evidence == observation.
        assert_eq!(
            select_verification_strategy_with_flag(&GuiActionKind::OpenApp, false, true),
            GuiVerificationStrategy::WindowVisible
        );
        let contract =
            verification_contract_for_with_flag(&GuiActionKind::OpenApp, false, 4_000, 12, true);
        assert_eq!(contract.predicate, "window_visible");
        assert_eq!(contract.evidence_source, "observation");
        // The contract still threads the bounded wait verbatim (never unbounded).
        assert_eq!(contract.bounded_wait_ms, 4_000);
        assert_eq!(contract.max_reobserve, 12);
    }

    #[test]
    fn t2_window_visible_evidence_source_is_observation_not_active_window_probe() {
        assert_eq!(
            evidence_source_for_strategy(GuiVerificationStrategy::WindowVisible),
            GuiVerificationEvidenceSource::Observation
        );
        // ActiveWindowMatch still uses the active-window probe (SwitchWindow).
        assert_eq!(
            evidence_source_for_strategy(GuiVerificationStrategy::ActiveWindowMatch),
            GuiVerificationEvidenceSource::ActiveWindowProbe
        );
    }

    #[test]
    fn t2_switch_window_is_never_changed_by_flag() {
        // SwitchWindow stays active_window_match REGARDLESS of the flag (a later
        // phase fixes window activation; Phase 1 must not touch it).
        for flag in [false, true] {
            assert_eq!(
                select_verification_strategy_with_flag(&GuiActionKind::SwitchWindow, false, flag),
                GuiVerificationStrategy::ActiveWindowMatch
            );
            let contract = verification_contract_for_with_flag(
                &GuiActionKind::SwitchWindow,
                false,
                4_000,
                12,
                flag,
            );
            assert_eq!(contract.predicate, "active_window_match");
            assert_eq!(contract.evidence_source, "active_window_probe");
        }
    }

    // ── T2: FLAG-OFF BYTE-FOR-BYTE (prior active_window_match preserved) ─────

    #[test]
    fn t2_flag_off_open_app_is_byte_for_byte_prior_active_window_match() {
        // Flag OFF: the flag-aware selection is IDENTICAL to the prior
        // `select_verification_strategy` for every action kind.
        let kinds = [
            GuiActionKind::OpenApp,
            GuiActionKind::SwitchWindow,
            GuiActionKind::FocusField,
            GuiActionKind::TypeText,
            GuiActionKind::ClickControl,
            GuiActionKind::PressKey,
            GuiActionKind::Copy,
            GuiActionKind::CloseDialog,
        ];
        for kind in kinds {
            for secret in [false, true] {
                assert_eq!(
                    select_verification_strategy_with_flag(&kind, secret, false),
                    select_verification_strategy(&kind, secret),
                    "flag-OFF must match the prior strategy for {kind:?} secret={secret}"
                );
            }
        }
        // Specifically: OpenApp flag-OFF is still active_window_match + probe.
        let off = verification_contract_for_with_flag(&GuiActionKind::OpenApp, false, 4_000, 12, false);
        let prior = verification_contract_for(&GuiActionKind::OpenApp, false, 4_000, 12);
        assert_eq!(off, prior, "flag-OFF contract must be byte-for-byte the prior contract");
        assert_eq!(off.predicate, "active_window_match");
        assert_eq!(off.evidence_source, "active_window_probe");
    }
}

#[cfg(test)]
mod verify_evidence_tests {
    //! Task 4 (Issue #10): ordered evidence + honest `inconclusive` over a false
    //! `verification_failed`. Pure functions; no display/backend.
    use super::*;

    fn failed_result(strategy: GuiVerificationStrategy) -> GuiPostActionVerificationResult {
        GuiPostActionVerificationResult {
            verification_id: "v-1".into(),
            execution_id: "e-1".into(),
            proposal_id: "p-1".into(),
            status: VERIFICATION_FAILED.into(),
            verification_strategy: strategy.as_str().into(),
            evidence: vec!["no change observed".into()],
            pre_state_summary: "pre".into(),
            post_state_summary: "post".into(),
            matched_expected_state: false,
            target_still_present: true,
            target_identity_matches: true,
            confidence: 0.3,
            safe_error_summary: None,
            recovery_hint: None,
            can_retry: true,
            prompt_hash: "ph".into(),
        }
    }

    fn verified(strategy: GuiVerificationStrategy) -> GuiPostActionVerificationResult {
        let mut r = failed_result(strategy);
        r.status = VERIFICATION_VERIFIED.into();
        r.matched_expected_state = true;
        r.confidence = 0.9;
        r
    }

    // ── flag plumbing ────────────────────────────────────────────────────────

    #[test]
    fn flag_defaults_on_and_rolls_back_on_falsy() {
        assert!(verify_evidence_enabled_lookup(|_| None));
        for raw in ["0", "false", "no", "off", "", " OFF "] {
            assert!(!verify_evidence_enabled_lookup(|_| Some(raw.to_string())));
        }
        for raw in ["1", "true", "yes", "on", "anything"] {
            assert!(verify_evidence_enabled_lookup(|_| Some(raw.to_string())));
        }
    }

    // ── ordered evidence ─────────────────────────────────────────────────────

    #[test]
    fn ordered_evidence_primary_first_then_observation_for_accessibility() {
        let ev = ordered_evidence_for_strategy(GuiVerificationStrategy::TextPresent);
        assert_eq!(ev.first(), Some(&GuiVerificationEvidenceSource::Accessibility));
        // A screen change is the honest secondary when a11y is off.
        assert!(ev.contains(&GuiVerificationEvidenceSource::Observation));
    }

    #[test]
    fn ordered_evidence_clipboard_has_no_visual_fallback() {
        // Requirement 10.2: a state-change verdict is never confirmed by an
        // OCR/screenshot-only proxy where the truth is a backend receipt.
        let ev = ordered_evidence_for_strategy(GuiVerificationStrategy::ClipboardChanged);
        assert_eq!(ev, vec![GuiVerificationEvidenceSource::BackendReceipt]);
    }

    #[test]
    fn primary_reliable_tracks_capability_signals() {
        // accessibility predicate reliable only when a11y is up.
        assert!(primary_evidence_reliable(
            GuiVerificationStrategy::TextPresent,
            true,
            false,
            false
        ));
        assert!(!primary_evidence_reliable(
            GuiVerificationStrategy::TextPresent,
            false,
            true,
            true
        ));
        // observation predicate reliable only when a screenshot is available.
        assert!(primary_evidence_reliable(
            GuiVerificationStrategy::ScreenChanged,
            false,
            true,
            false
        ));
        assert!(!primary_evidence_reliable(
            GuiVerificationStrategy::ScreenChanged,
            true,
            false,
            true
        ));
    }

    // ── apply_evidence_fallback: failed → inconclusive only when primary unreliable

    #[test]
    fn failed_with_unreliable_primary_becomes_inconclusive() {
        // a11y off → a TextPresent `failed` is really "couldn't tell" → inconclusive.
        let r = failed_result(GuiVerificationStrategy::TextPresent);
        let out = apply_evidence_fallback(&r, false, true, true, true);
        assert_eq!(out.status, VERIFICATION_INCONCLUSIVE);
        assert!(out.evidence.iter().any(|e| e.contains("inconclusive")));
    }

    #[test]
    fn failed_with_reliable_primary_stays_failed() {
        // a11y up and still no text → a GENUINE failure; never softened.
        let r = failed_result(GuiVerificationStrategy::TextPresent);
        let out = apply_evidence_fallback(&r, true, true, true, true);
        assert_eq!(out.status, VERIFICATION_FAILED);
    }

    #[test]
    fn verified_is_never_touched() {
        // Never upgrades nor alters a verified verdict (no false verified).
        let r = verified(GuiVerificationStrategy::TextPresent);
        let out = apply_evidence_fallback(&r, false, false, false, true);
        assert_eq!(out.status, VERIFICATION_VERIFIED);
        assert_eq!(out, r);
    }

    #[test]
    fn flag_off_is_byte_for_byte_noop() {
        let r = failed_result(GuiVerificationStrategy::TextPresent);
        let out = apply_evidence_fallback(&r, false, false, false, false);
        assert_eq!(out, r, "flag-OFF must be a byte-for-byte no-op");
    }

    #[test]
    fn contract_carries_ordered_evidence_sources() {
        let c = verification_contract_for(&GuiActionKind::TypeText, false, 4_000, 12);
        assert_eq!(c.evidence_sources.first().map(String::as_str), Some("accessibility"));
        assert!(c.evidence_sources.iter().any(|s| s == "observation"));
    }

    // ── apply_ordered_evidence: ordered fallback (upgrade + honest inconclusive)

    fn inconclusive_result(strategy: GuiVerificationStrategy) -> GuiPostActionVerificationResult {
        let mut r = failed_result(strategy);
        r.status = VERIFICATION_INCONCLUSIVE.into();
        r.confidence = 0.5;
        r
    }

    /// WindowVisible adds Process AFTER the active-window probe (Issue #2 chain).
    #[test]
    fn ordered_evidence_window_visible_chain_is_observation_then_active_window_then_process() {
        let ev = ordered_evidence_for_strategy(GuiVerificationStrategy::WindowVisible);
        assert_eq!(
            ev,
            vec![
                GuiVerificationEvidenceSource::Observation,
                GuiVerificationEvidenceSource::ActiveWindowProbe,
                GuiVerificationEvidenceSource::Process,
            ]
        );
    }

    /// Weak/unavailable screenshot + a STRONG secondary (active-window change) ⇒
    /// the honest verdict is `verified` via the ordered fallback (Task 4.4).
    #[test]
    fn inconclusive_upgraded_to_verified_by_active_window_secondary() {
        // ScreenChanged primary is Observation; with no screenshot the core verdict
        // is inconclusive, but the active-window probe (secondary) confirms a real
        // navigation/change.
        let r = inconclusive_result(GuiVerificationStrategy::ScreenChanged);
        let ev = GuiSecondaryEvidence {
            screen_changed: false,
            active_window_changed: true,
            process_running: false,
            accessibility_ok: false,
            screenshot_available: false, // primary (observation) unavailable
            active_window_probe_ok: true,
        };
        let out = apply_ordered_evidence(&r, &ev, true);
        assert_eq!(out.status, VERIFICATION_VERIFIED);
        assert!(out.matched_expected_state);
        assert!(out.confidence >= 0.8);
        assert!(out
            .evidence
            .iter()
            .any(|e| e.contains("secondary source (active_window_probe)")));
    }

    /// A11y-off TextPresent (primary Accessibility unavailable) ⇒ upgraded to
    /// `verified` by a screen-HASH change (Observation secondary) — generalizes the
    /// Task-2 browser screen_changed override to ALL predicates (not browser-only).
    #[test]
    fn accessibility_off_text_present_upgraded_by_screen_change_secondary() {
        let r = inconclusive_result(GuiVerificationStrategy::TextPresent);
        let ev = GuiSecondaryEvidence {
            screen_changed: true,
            active_window_changed: false,
            process_running: false,
            accessibility_ok: false, // primary (accessibility) unavailable
            screenshot_available: true,
            active_window_probe_ok: true,
        };
        let out = apply_ordered_evidence(&r, &ev, true);
        assert_eq!(out.status, VERIFICATION_VERIFIED);
        assert!(out
            .evidence
            .iter()
            .any(|e| e.contains("secondary source (observation)")));
    }

    /// ALL evidence weak (primary unavailable AND no secondary confirms) ⇒ the
    /// honest `inconclusive`, NOT a false `verification_failed` (Task 4.4).
    #[test]
    fn all_weak_evidence_stays_inconclusive_not_false_failed() {
        // Start from a `failed` whose primary was unreliable and no secondary fires.
        let r = failed_result(GuiVerificationStrategy::TextPresent);
        let ev = GuiSecondaryEvidence {
            screen_changed: false,
            active_window_changed: false,
            process_running: false,
            accessibility_ok: false, // primary unreliable
            screenshot_available: false,
            active_window_probe_ok: false,
        };
        let out = apply_ordered_evidence(&r, &ev, true);
        assert_eq!(out.status, VERIFICATION_INCONCLUSIVE);
        assert!(!out.matched_expected_state);
    }

    /// A GENUINE failure (primary reliable, no change) is NEVER softened or
    /// overridden by a secondary source.
    #[test]
    fn reliable_primary_failure_is_never_overridden_by_secondary() {
        let r = failed_result(GuiVerificationStrategy::ScreenChanged);
        let ev = GuiSecondaryEvidence {
            screen_changed: false,
            active_window_changed: true, // a secondary "change" exists ...
            process_running: true,
            accessibility_ok: true,
            screenshot_available: true, // ... but the PRIMARY (observation) was reliable
            active_window_probe_ok: true,
        };
        let out = apply_ordered_evidence(&r, &ev, true);
        assert_eq!(out.status, VERIFICATION_FAILED, "reliable negative is honest");
    }

    /// Requirement 10.2 (Task 4.2): OCR text is NEVER the sole evidence for a
    /// state-change verdict. The secondary signals are screen-hash / active-window
    /// / process based; with NONE of those set (only OCR text would exist in the
    /// observation), a state-change predicate cannot be upgraded to `verified`.
    #[test]
    fn ocr_only_never_yields_a_state_change_verified() {
        for strategy in [
            GuiVerificationStrategy::ScreenChanged,
            GuiVerificationStrategy::StateChanged,
            GuiVerificationStrategy::ResultVisible,
        ] {
            let r = inconclusive_result(strategy);
            // screen_changed=false means NO screen-hash delta — any OCR text in the
            // observation is irrelevant to the ordered model (it is not an evidence
            // source). No secondary confirms.
            let ev = GuiSecondaryEvidence {
                screen_changed: false,
                active_window_changed: false,
                process_running: false,
                accessibility_ok: false,
                screenshot_available: false,
                active_window_probe_ok: false,
            };
            let out = apply_ordered_evidence(&r, &ev, true);
            assert_ne!(
                out.status, VERIFICATION_VERIFIED,
                "OCR-only must never confirm a {strategy:?} verdict"
            );
        }
        // Structural guarantee: no evidence source maps to OCR/coordinates.
        for strategy in [
            GuiVerificationStrategy::ScreenChanged,
            GuiVerificationStrategy::StateChanged,
            GuiVerificationStrategy::TextPresent,
        ] {
            for source in ordered_evidence_for_strategy(strategy) {
                assert!(
                    matches!(
                        source,
                        GuiVerificationEvidenceSource::Accessibility
                            | GuiVerificationEvidenceSource::Observation
                            | GuiVerificationEvidenceSource::ActiveWindowProbe
                            | GuiVerificationEvidenceSource::Process
                            | GuiVerificationEvidenceSource::BackendReceipt
                            | GuiVerificationEvidenceSource::None
                    ),
                    "evidence taxonomy has no OCR/coordinate variant"
                );
            }
        }
    }

    /// Clipboard (backend-receipt) has NO visual fallback — an inconclusive
    /// clipboard verdict is never upgraded by a screen change (Requirement 10.2).
    #[test]
    fn clipboard_inconclusive_not_upgraded_by_screen_change() {
        let r = inconclusive_result(GuiVerificationStrategy::ClipboardChanged);
        let ev = GuiSecondaryEvidence {
            screen_changed: true,
            active_window_changed: true,
            process_running: true,
            accessibility_ok: true,
            screenshot_available: true,
            active_window_probe_ok: true,
        };
        let out = apply_ordered_evidence(&r, &ev, true);
        assert_eq!(out.status, VERIFICATION_INCONCLUSIVE);
    }

    /// A `verified` verdict is never touched (no false-`verified` fabrication and
    /// no alteration of an honest positive).
    #[test]
    fn ordered_evidence_never_touches_verified() {
        let r = verified(GuiVerificationStrategy::TextPresent);
        let ev = GuiSecondaryEvidence::default();
        let out = apply_ordered_evidence(&r, &ev, true);
        assert_eq!(out, r);
    }

    /// Flag-OFF: `apply_ordered_evidence` is a byte-for-byte no-op for every
    /// verdict status (serialize-compare).
    #[test]
    fn ordered_evidence_flag_off_is_byte_for_byte_noop() {
        let strong = GuiSecondaryEvidence {
            screen_changed: true,
            active_window_changed: true,
            process_running: true,
            accessibility_ok: false,
            screenshot_available: false,
            active_window_probe_ok: true,
        };
        for r in [
            failed_result(GuiVerificationStrategy::TextPresent),
            inconclusive_result(GuiVerificationStrategy::ScreenChanged),
            verified(GuiVerificationStrategy::TextPresent),
        ] {
            let out = apply_ordered_evidence(&r, &strong, false);
            assert_eq!(
                serde_json::to_string(&out.summary_json()).unwrap(),
                serde_json::to_string(&r.summary_json()).unwrap(),
                "flag-OFF must be byte-for-byte the prior verdict"
            );
        }
    }
}
