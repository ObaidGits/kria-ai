//! Wayland-safe window focus / switch abstraction (Task 4.1 — FOUNDATION).
//!
//! KRIA stays the authoritative orchestrator: the window-focus backends defined
//! here are pure execution **substrates**. They activate/switch to a target
//! window when the orchestrator decides to; they never gain orchestration
//! authority, never originate an action from raw prompt/OCR/coordinates, and are
//! always bounded + verifiable (Requirement 3, Requirement 15). The pipeline
//! shape is preserved: Intent → Capability → Policy → Substrate → Tool →
//! Verification. This module supplies the **Substrate** layer for window focus.
//!
//! Requirement 3 (Wayland-safe window focus):
//! - 3.1 SwitchWindow SHALL NOT depend on `wmctrl` on Wayland → [`X11Wmctrl`] is
//!   eligible ONLY on X11 sessions ([`select_focus_backends`]).
//! - 3.2 Prefer a compositor-native activate-by-window-identity path
//!   (GNOME-shell bridge / desktop portal); key-based switching (Alt+Tab via
//!   uinput/ydotool) is a last-resort fallback and SHALL always be followed by
//!   verification → backend preference order
//!   `GnomeBridge → Portal → UinputAltTab(verify) → X11Wmctrl(x11 only)`;
//!   [`WindowFocusBackend::requires_verification`] is true for
//!   [`UinputAltTab`].
//! - 3.3 WHEN no focus path is available, fail with a clear, actionable reason
//!   → [`WindowFocusError`] / an empty selection chain.
//! - 3.4 SwitchWindow SHALL be verified by re-observing that the requested
//!   window is active → [`WindowFocusVerification`] +
//!   [`verify_active_window`] (the interface 4.3 wires; here it is the grounded
//!   stub so the abstraction is verifiable by construction).
//!
//! Scope of THIS subtask (4.1): the abstraction (trait + backend enum), the
//! backend ordering, the session-based selection function, and the
//! `gui_cog_wayland_focus` feature flag (default OFF). Routing `SwitchWindow`
//! through it is Task 4.2; verify-by-reobserve execution is Task 4.3. Real
//! backend execution is therefore interface-level here.
//!
//! [`X11Wmctrl`]: WindowFocusBackend::X11Wmctrl
//! [`UinputAltTab`]: WindowFocusBackend::UinputAltTab

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::executor::GuiActionBackendStatus;
use super::perception::sanitize_gui_text;

/// Environment variable that enables the `gui_cog_wayland_focus` feature flag
/// (Task 4). Truthy (`1`/`true`/`yes`/`on`) turns the Wayland-safe window-focus
/// abstraction ON. Default (unset or any other value) keeps it OFF: the existing
/// SwitchWindow behavior is preserved until the Wave 3 gate (Task 4.5) flips it.
pub const WAYLAND_FOCUS_ENV_FLAG: &str = "KRIA_GUI_COG_WAYLAND_FOCUS";

/// Max sanitized length for a window/app identity hint surfaced in events.
const MAX_FOCUS_HINT_CHARS: usize = 120;

/// A window-focus execution **substrate** (Requirement 3.2 preference order).
///
/// The variants are listed in their global preference order — compositor-native
/// activate-by-identity paths first, the key-based Alt+Tab fallback next, and
/// the X11-only `wmctrl` path last. [`select_focus_backends`] filters this order
/// by session type + probed input capability; it never reorders it, so any
/// selection result is always a subsequence of [`ALL_BACKENDS_IN_PREFERENCE_ORDER`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowFocusBackend {
    /// GNOME-shell D-Bus bridge: compositor-native activate-by-window-identity.
    /// Wayland-capable (and works under GNOME-on-X11). Most preferred (3.2).
    GnomeBridge,
    /// xdg desktop portal activate path: compositor-native, Wayland-capable.
    Portal,
    /// Alt+Tab synthesized via the uinput/ydotool input substrate. Last-resort
    /// key-based fallback; MUST be verified afterwards (3.2) — see
    /// [`WindowFocusBackend::requires_verification`].
    UinputAltTab,
    /// `wmctrl` activate-by-window. X11-only (3.1): never eligible on Wayland.
    X11Wmctrl,
}

/// Every backend in global preference order (Requirement 3.2). Selection always
/// preserves this relative order.
pub const ALL_BACKENDS_IN_PREFERENCE_ORDER: [WindowFocusBackend; 4] = [
    WindowFocusBackend::GnomeBridge,
    WindowFocusBackend::Portal,
    WindowFocusBackend::UinputAltTab,
    WindowFocusBackend::X11Wmctrl,
];

impl WindowFocusBackend {
    /// Stable string tag (used in `backend_used` + events; keep stable).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GnomeBridge => "gnome_bridge",
            Self::Portal => "portal",
            Self::UinputAltTab => "uinput_alt_tab",
            Self::X11Wmctrl => "x11_wmctrl",
        }
    }

    /// Whether this backend is a compositor-native activate-by-window-identity
    /// path (the preferred class in Requirement 3.2).
    pub fn is_compositor_native(&self) -> bool {
        matches!(self, Self::GnomeBridge | Self::Portal)
    }

    /// Whether this backend is usable on Wayland sessions. Everything except the
    /// `wmctrl` path is Wayland-capable (Requirement 3.1).
    pub fn is_wayland_capable(&self) -> bool {
        !matches!(self, Self::X11Wmctrl)
    }

    /// Whether this backend is restricted to X11 sessions (Requirement 3.1:
    /// `wmctrl` only).
    pub fn is_x11_only(&self) -> bool {
        matches!(self, Self::X11Wmctrl)
    }

    /// Whether a successful focus via this backend MUST be followed by
    /// verification (Requirement 3.2: the key-based Alt+Tab fallback is never
    /// trusted blindly). Compositor-native activate-by-identity paths are
    /// authoritative, but the loop still verifies them under Requirement 3.4 —
    /// this flag marks the backends that may NOT be reported focused without
    /// re-observe confirmation.
    pub fn requires_verification(&self) -> bool {
        matches!(self, Self::UinputAltTab)
    }
}

/// Task 13 (Issue #11): the `gui_cog_backend_status` feature flag. Default ON;
/// an explicit falsy value (`0`/`false`/`no`/`off`/empty) in
/// `KRIA_GUI_COG_BACKEND_STATUS` rolls back to the prior behavior (the
/// backend-availability status is not surfaced), byte-for-byte. An absent env
/// value keeps it ON.
pub const BACKEND_STATUS_ENV_FLAG: &str = "KRIA_GUI_COG_BACKEND_STATUS";

/// Whether the backend-availability status surfacing is enabled.
pub fn backend_status_enabled() -> bool {
    backend_status_enabled_lookup(|key| std::env::var(key).ok())
}

/// Testable core of [`backend_status_enabled`] with an injectable lookup.
pub fn backend_status_enabled_lookup<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    match lookup(BACKEND_STATUS_ENV_FLAG) {
        Some(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off" | ""
        ),
        None => true,
    }
}

/// Task 13 (Issue #11): availability of the window-focus / capture / activate
/// backends. Lets the runtime degrade GRACEFULLY and surface an honest
/// capability notice instead of a silent failure when the GNOME extension (the
/// current single point of dependency) is absent. Pure/derived — no I/O.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiBackendStatus {
    /// The GNOME `kria-active-window` extension is reachable (activate+capture).
    pub extension_available: bool,
    /// The uinput daemon is up (keyboard, Alt+Tab focus fallback, abs click).
    pub uinput_available: bool,
    /// An xdg desktop portal is available (capture/activate fallback).
    pub portal_available: bool,
    /// The X11 `wmctrl`/`xdotool` path is usable (X11 sessions only).
    pub x11_available: bool,
    /// Window CAPTURE is available via SOME backend (extension or portal).
    pub capture_available: bool,
    /// Window ACTIVATION is available via SOME backend (extension/portal/altTab).
    pub activate_available: bool,
    /// The best window-focus backend given availability + session type, or
    /// `None` when no path exists (fully degraded).
    pub preferred_focus_backend: Option<WindowFocusBackend>,
    /// Honest, user-facing capability notices (empty when fully capable).
    pub capability_notices: Vec<String>,
}

impl GuiBackendStatus {
    /// Derive the backend status from probed availability + session type. Never
    /// fabricates a capability: a missing extension yields a clear notice and
    /// the best available fallback (or an explicit "no backend" notice).
    pub fn assess(
        extension_available: bool,
        uinput_available: bool,
        portal_available: bool,
        x11_available: bool,
        is_wayland: bool,
    ) -> Self {
        let capture_available = extension_available || portal_available;
        let activate_available = extension_available || portal_available || uinput_available;

        // Best focus backend in the global preference order, filtered by
        // availability + session type (X11 path never eligible on Wayland).
        let preferred_focus_backend = if extension_available {
            Some(WindowFocusBackend::GnomeBridge)
        } else if portal_available {
            Some(WindowFocusBackend::Portal)
        } else if uinput_available {
            Some(WindowFocusBackend::UinputAltTab)
        } else if x11_available && !is_wayland {
            Some(WindowFocusBackend::X11Wmctrl)
        } else {
            None
        };

        let mut capability_notices = Vec::new();
        if !extension_available {
            capability_notices.push(
                "GNOME window extension unavailable: window activation/capture is degraded; using the best available fallback".to_string(),
            );
        }
        if !capture_available {
            capability_notices
                .push("screen capture unavailable (no extension or portal backend)".to_string());
        }
        if !activate_available {
            capability_notices.push(
                "window activation unavailable (no extension, portal, or input backend)".to_string(),
            );
        }
        if preferred_focus_backend.is_none() {
            capability_notices
                .push("no window-focus backend available for this session".to_string());
        }

        Self {
            extension_available,
            uinput_available,
            portal_available,
            x11_available,
            capture_available,
            activate_available,
            preferred_focus_backend,
            capability_notices,
        }
    }

    /// Whether the system is fully capable (no degradation notices).
    pub fn fully_capable(&self) -> bool {
        self.capability_notices.is_empty()
    }
}

/// The `gui_cog_wayland_focus` feature-flag bundle (default OFF) — Task 4.1.
///
/// Mirrors [`GuiReobserveConfig`]/[`GuiSmartPlannerConfig`]/
/// [`GuiRuntimeGuardConfig`]. While OFF (the default) the existing SwitchWindow
/// behavior is preserved; while ON the Wayland-safe focus abstraction is used to
/// route SwitchWindow (Task 4.2) and verify the active window (Task 4.3). The
/// Wave 3 gate (Task 4.5) flips the live/desktop path to default ON.
///
/// [`GuiReobserveConfig`]: super::turn_budget::GuiReobserveConfig
/// [`GuiSmartPlannerConfig`]: super::llm_planner::GuiSmartPlannerConfig
/// [`GuiRuntimeGuardConfig`]: super::turn_budget::GuiRuntimeGuardConfig
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiWaylandFocusConfig {
    /// Whether the Wayland-safe window-focus abstraction is active.
    pub enabled: bool,
}

impl Default for GuiWaylandFocusConfig {
    fn default() -> Self {
        // Task 4: flag default OFF until the Wave 3 gate (Task 4.5) flips it.
        Self { enabled: false }
    }
}

impl GuiWaylandFocusConfig {
    /// Construct an explicitly-enabled Wayland-focus config.
    pub fn enabled() -> Self {
        Self { enabled: true }
    }

    /// Construct an explicitly-disabled Wayland-focus config.
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Whether the Wayland-safe window-focus abstraction should run.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Derive the config from the process environment. The flag is OFF unless
    /// [`WAYLAND_FOCUS_ENV_FLAG`] is truthy.
    pub fn from_env() -> Self {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env`](Self::from_env) with an injectable lookup.
    pub fn from_env_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        Self {
            enabled: wayland_focus_flag_truthy(lookup(WAYLAND_FOCUS_ENV_FLAG).as_deref()),
        }
    }

    /// Derive the config from the process environment with the flag defaulting
    /// **ON** (Task 4.5 gate flip). The Wayland-safe focus abstraction is active
    /// unless [`WAYLAND_FOCUS_ENV_FLAG`] is explicitly falsy
    /// (`0`/`false`/`no`/`off`/empty), which is the documented rollback switch to
    /// restore the prior SwitchWindow behavior without a code change. An absent
    /// env value keeps the flag ON.
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
            enabled: !wayland_focus_flag_falsy(lookup(WAYLAND_FOCUS_ENV_FLAG).as_deref()),
        }
    }
}

/// Parse a `gui_cog_wayland_focus` env value as truthy (`1`/`true`/`yes`/`on`).
fn wayland_focus_flag_truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// Whether a `gui_cog_wayland_focus` env value is an explicit opt-OUT, used by
/// the default-ON path ([`GuiWaylandFocusConfig::from_env_lookup_default_on`])
/// as the documented rollback switch: an empty or `0`/`false`/`no`/`off` value
/// disables the abstraction. An absent value (`None`) is NOT falsy — the default
/// stays ON.
fn wayland_focus_flag_falsy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0") | Some("false") | Some("no") | Some("off") | Some("")
    )
}

/// A sanitized window-identity target for activate-by-identity focus
/// (Requirement 3.2). Holds only sanitized app/window hints — never raw
/// prompt/OCR text, secrets, or coordinates (Property 7).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WindowIdentity {
    /// Sanitized target application hint (e.g. "Chrome").
    pub app_hint: Option<String>,
    /// Sanitized target window-title hint.
    pub window_hint: Option<String>,
}

impl WindowIdentity {
    /// Build a sanitized identity from optional app/window hints.
    pub fn new(app_hint: Option<&str>, window_hint: Option<&str>) -> Self {
        Self {
            app_hint: sanitize_hint(app_hint),
            window_hint: sanitize_hint(window_hint),
        }
    }

    /// Whether this identity carries any concrete target to activate-by-identity.
    /// When false, an identity-based backend cannot resolve a target and the
    /// loop must fall back / ask (Requirement 3.3).
    pub fn has_target(&self) -> bool {
        self.app_hint.is_some() || self.window_hint.is_some()
    }

    /// The best available human-readable (already sanitized) label.
    pub fn label(&self) -> Option<&str> {
        self.window_hint
            .as_deref()
            .or(self.app_hint.as_deref())
    }
}

fn sanitize_hint(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    let cleaned = sanitize_gui_text(value, MAX_FOCUS_HINT_CHARS).text;
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_string())
    }
}

/// Outcome of re-observing the active window after a focus attempt
/// (Requirement 3.4 / 23.2). Foundation-level interface that Task 4.3 wires to
/// real re-observation. `Verified` is only reached when the active window
/// matches the requested identity above the confidence bar; ambiguous evidence
/// yields `Inconclusive` (never a false `Verified`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowFocusVerification {
    /// Active window confirmed to match the requested identity.
    Verified,
    /// Evidence was ambiguous / low-confidence — not a confirmed success.
    Inconclusive,
    /// Active window did NOT match the requested identity.
    Failed,
    /// Verification has not been attempted yet (interface default).
    NotAttempted,
}

impl WindowFocusVerification {
    /// Stable string tag for events.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Inconclusive => "inconclusive",
            Self::Failed => "failed",
            Self::NotAttempted => "not_attempted",
        }
    }

    /// Whether the focus is confirmed (only `Verified` counts).
    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Verified)
    }
}

/// Grounded verification stub (Requirement 3.4): compare the requested identity
/// against the observed active-window label. Task 4.3 replaces the
/// label-equality heuristic with the real re-observe + confidence contract; the
/// signature is the interface 4.3 builds on. Matching is case-insensitive and
/// English-scoped for v1 (Requirement 26.3).
pub fn verify_active_window(
    requested: &WindowIdentity,
    observed_active_label: Option<&str>,
) -> WindowFocusVerification {
    let Some(expected) = requested.label() else {
        // No concrete identity to verify against → cannot confirm.
        return WindowFocusVerification::Inconclusive;
    };
    let Some(active) = observed_active_label.map(str::trim).filter(|s| !s.is_empty()) else {
        return WindowFocusVerification::Inconclusive;
    };
    let expected_l = expected.to_ascii_lowercase();
    let active_l = active.to_ascii_lowercase();
    if active_l == expected_l || active_l.contains(&expected_l) || expected_l.contains(&active_l) {
        WindowFocusVerification::Verified
    } else {
        WindowFocusVerification::Failed
    }
}

/// Result of a single backend's focus attempt (Task 4.2 reports `backend_used`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowFocusOutcome {
    /// Which substrate actually performed (or attempted) the focus.
    pub backend_used: WindowFocusBackend,
    /// Verification state after re-observe (Requirement 3.4).
    pub verification: WindowFocusVerification,
}

impl WindowFocusOutcome {
    /// Sanitized JSON summary for events/telemetry (no secrets, no raw payload).
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "backend_used": self.backend_used.as_str(),
            "verification": self.verification.as_str(),
        })
    }
}

/// Errors a window-focus substrate can surface. Requirement 3.3: when no focus
/// path is available the caller gets a clear, actionable reason — never a
/// generic "deterministic action backend failed".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WindowFocusError {
    /// No eligible backend for the session (e.g. Wayland with no input substrate
    /// and no compositor-native path) — Requirement 3.3.
    #[error("no window-focus backend is available for this session: {reason}")]
    NoBackendAvailable {
        /// Sanitized, actionable reason.
        reason: String,
    },
    /// The identity carried no concrete target to activate.
    #[error("no target window identity to focus")]
    NoTarget,
    /// A specific backend attempt failed; the chain may try the next backend.
    #[error("window-focus backend `{backend}` failed: {reason}")]
    BackendFailed {
        /// Stable backend tag that failed.
        backend: &'static str,
        /// Sanitized, actionable reason.
        reason: String,
    },
    /// Foundation placeholder: real execution is wired in Task 4.2.
    #[error("window-focus backend `{0}` execution is not wired yet (Task 4.2)")]
    NotImplemented(&'static str),
}

/// A window-focus execution substrate. KRIA orchestrates; an implementor only
/// activates the requested window and reports which backend it used + the
/// verification state. Real implementations land in Task 4.2/4.3; this trait is
/// the foundation contract so the loop can be written against it.
#[async_trait]
pub trait WindowFocusBackendHandler: Send + Sync {
    /// Which backend this handler represents.
    fn backend(&self) -> WindowFocusBackend;

    /// Attempt to focus/activate the target window identity.
    async fn focus_window(
        &self,
        identity: &WindowIdentity,
    ) -> Result<WindowFocusOutcome, WindowFocusError>;
}

/// Select the eligible window-focus backend chain for a session, in preference
/// order (Requirement 3.1/3.2). "Select by session":
///
/// - **Wayland**: compositor-native first (`GnomeBridge`, `Portal`), then the
///   key-based `UinputAltTab` fallback **only if** a usable input substrate was
///   probed ([`GuiActionBackendStatus::can_execute_actions`]). `X11Wmctrl` is
///   NEVER eligible (3.1) — the Wayland path never depends on X11-only tools.
/// - **X11**: same compositor-native + Alt+Tab order, then `X11Wmctrl` last as
///   the X11-only activate path.
/// - **Unknown/other**: conservative — compositor-native paths plus Alt+Tab (if
///   input usable). `X11Wmctrl` excluded since the session is not confirmed X11.
///
/// The result is always a subsequence of [`ALL_BACKENDS_IN_PREFERENCE_ORDER`].
/// An empty result means no focus path exists → the caller surfaces a clear,
/// actionable reason (Requirement 3.3) rather than a generic backend failure.
///
/// `session_type` is taken explicitly (per the design's selection signature);
/// `backend` supplies the reused probe signals (input substrate availability).
pub fn select_focus_backends(
    session_type: &str,
    backend: &GuiActionBackendStatus,
) -> Vec<WindowFocusBackend> {
    let session = session_type.trim().to_ascii_lowercase();
    // The Alt+Tab fallback needs a usable key-input substrate. Reuse the
    // existing aggregated probe signal rather than re-probing.
    let alt_tab_usable = backend.can_execute_actions;

    ALL_BACKENDS_IN_PREFERENCE_ORDER
        .iter()
        .copied()
        .filter(|candidate| match candidate {
            // X11-only path: eligible only on confirmed X11 sessions (3.1).
            WindowFocusBackend::X11Wmctrl => session == "x11",
            // Key-based fallback: only if a usable input substrate exists.
            WindowFocusBackend::UinputAltTab => alt_tab_usable,
            // Compositor-native paths are Wayland-capable and also valid under
            // GNOME-on-X11; always eligible as preferred options.
            WindowFocusBackend::GnomeBridge | WindowFocusBackend::Portal => true,
        })
        .collect()
}

/// Choose the window-focus backend that will ACTIVATE the requested identity
/// (Task 4.2 — routing + activate-by-window-identity preference).
///
/// The eligible `chain` (from [`select_focus_backends`]) is already in global
/// preference order — compositor-native **activate-by-window-identity** paths
/// first, the key-based Alt+Tab fallback last — so the first *available* backend
/// in the chain is returned. This structurally prefers a compositor-native
/// activate-by-identity path over a blind Alt+Tab fallback (Requirement 3.2);
/// the key-based fallback ([`WindowFocusBackend::UinputAltTab`]) /
/// [`WindowFocusBackend::X11Wmctrl`] is only chosen when every
/// higher-preference backend is unavailable.
///
/// `is_available` reports whether a real handler for a backend can run in this
/// session. Compositor-native handlers are wired in later subtasks (4.3+/4.5);
/// today only the input-substrate-backed [`WindowFocusBackend::UinputAltTab`] is
/// live, so a healthy session correctly falls back to it (and that backend
/// [`requires_verification`](WindowFocusBackend::requires_verification)).
///
/// Errors are clear + actionable (Requirement 3.3; the full no-path messaging /
/// re-observe verification is Task 4.3):
/// - [`WindowFocusError::NoTarget`] when the identity carries no app/window hint
///   to activate by — we NEVER blindly Alt+Tab without a resolved target
///   (KRIA-authority: identity comes from sanitized resolved-target data, never
///   a blind key spam loop).
/// - [`WindowFocusError::NoBackendAvailable`] when the chain is empty or every
///   eligible backend is currently unavailable.
pub fn select_window_focus_backend<F>(
    chain: &[WindowFocusBackend],
    identity: &WindowIdentity,
    is_available: F,
) -> Result<WindowFocusBackend, WindowFocusError>
where
    F: Fn(WindowFocusBackend) -> bool,
{
    if !identity.has_target() {
        return Err(WindowFocusError::NoTarget);
    }
    if chain.is_empty() {
        return Err(WindowFocusError::NoBackendAvailable {
            reason: "no eligible window-focus backend for this session".into(),
        });
    }
    chain
        .iter()
        .copied()
        .find(|backend| is_available(*backend))
        .ok_or_else(|| WindowFocusError::NoBackendAvailable {
            reason: "all eligible window-focus backends are currently unavailable".into(),
        })
}

/// Sanitized routing summary for the SwitchWindow window-focus decision, emitted
/// in the execution events so the chosen backend + chain are observable
/// (Requirement 3 / Task 4.2 `backend_used`). Carries only sanitized hints +
/// stable backend tags — never raw prompt/OCR text, secrets, or coordinates
/// (Property 7).
pub fn window_focus_routing_json(
    identity: &WindowIdentity,
    chain: &[WindowFocusBackend],
    backend_used: Option<WindowFocusBackend>,
    verification: WindowFocusVerification,
    error: Option<&WindowFocusError>,
) -> serde_json::Value {
    serde_json::json!({
        "routed": true,
        "identity_label": identity.label(),
        "has_target": identity.has_target(),
        "chain": chain.iter().map(|b| b.as_str()).collect::<Vec<_>>(),
        "backend_used": backend_used.map(|b| b.as_str()),
        "requires_verification": backend_used
            .map(|b| b.requires_verification())
            .unwrap_or(false),
        "verification": verification.as_str(),
        "error": error.map(|e| e.to_string()),
    })
}

/// Build a clear, actionable, sanitized reason for the SwitchWindow caller when
/// NO viable window-focus path exists (Requirement 3.3, Task 4.3). This
/// deliberately REPLACES the legacy generic "wmctrl required" /
/// "deterministic action backend failed" message: it explains that the session
/// has no usable window-activation backend and what is needed to enable one, so
/// the user gets an actionable next step instead of an opaque failure. The
/// message carries only sanitized session hints — never raw prompt/OCR text,
/// secrets, or coordinates (Property 7). KRIA stays authoritative: a missing
/// substrate surfaces a truthful reason, never a fabricated success.
pub fn no_focus_path_message(error: &WindowFocusError, session_type: &str) -> String {
    let session = sanitize_session_label(session_type);
    match error {
        WindowFocusError::NoTarget => "I could not identify which window to switch to, so I did \
             not switch anything. Name the target window or application (for example, \"switch to \
             the Chrome window\")."
            .to_string(),
        WindowFocusError::NoBackendAvailable { .. } => format!(
            "This {session} session has no usable window-activation backend, so I cannot switch \
             windows. None of the compositor-native paths (GNOME shell bridge, desktop portal) are \
             available, and no input substrate (uinput/ydotool) could be used for an Alt+Tab \
             fallback. Enable a compositor activation path or grant uinput input access to switch \
             windows."
        ),
        WindowFocusError::BackendFailed { backend, reason } => format!(
            "The window-activation backend `{backend}` could not switch windows on this {session} \
             session: {reason}. No other window-focus backend was available to try."
        ),
        WindowFocusError::NotImplemented(backend) => format!(
            "The window-activation backend `{backend}` is not available on this {session} session, \
             and no other window-focus backend could switch windows."
        ),
    }
}

/// Sanitize a session-type label for inclusion in an actionable error message.
/// Empty/blank input degrades to a neutral "current" rather than leaking raw
/// text.
fn sanitize_session_label(session_type: &str) -> String {
    let trimmed = session_type.trim();
    if trimmed.is_empty() {
        return "current".to_string();
    }
    let cleaned = sanitize_gui_text(trimmed, 32).text;
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "current".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Verdict of the Task 4.3 bounded verify-by-re-observe step: compare the
/// requested window identity against a single FRESH active-window observation
/// (Requirement 3.4 / 23.2). This is the integration-facing wrapper around
/// [`verify_active_window`] that encodes the re-observe trust rules:
///
/// - `probe_ok == false` ⇒ the fresh observation is unreliable, so the verdict
///   is [`WindowFocusVerification::Inconclusive`] (never a false `Verified`).
/// - otherwise the observed active-window label is matched against the requested
///   identity, yielding `Verified` / `Failed` / `Inconclusive` truthfully.
///
/// The caller supplies exactly ONE re-observed active label — this function
/// performs no polling and is therefore bounded by construction (Requirement
/// 21, Property 9). The Alt+Tab fallback ([`WindowFocusBackend::UinputAltTab`])
/// is never trusted blindly: its outcome is decided by this re-observe verdict
/// just like every other backend (Requirement 3.2).
pub fn verify_focus_by_reobserve(
    requested: &WindowIdentity,
    observed_active_label: Option<&str>,
    probe_ok: bool,
) -> WindowFocusVerification {
    if !probe_ok {
        return WindowFocusVerification::Inconclusive;
    }
    verify_active_window(requested, observed_active_label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // ---- Task 13 (Issue #11): backend availability status + capability notice -

    #[test]
    fn backend_status_flag_defaults_on_and_rolls_back() {
        assert!(backend_status_enabled_lookup(|_| None));
        for raw in ["0", "false", "no", "off", "", " OFF "] {
            assert!(!backend_status_enabled_lookup(|_| Some(raw.to_string())));
        }
        for raw in ["1", "true", "on", "yes"] {
            assert!(backend_status_enabled_lookup(|_| Some(raw.to_string())));
        }
    }

    #[test]
    fn backend_status_fully_capable_with_extension() {
        let s = GuiBackendStatus::assess(true, true, false, false, true);
        assert!(s.fully_capable(), "extension present => no degradation notices");
        assert_eq!(s.preferred_focus_backend, Some(WindowFocusBackend::GnomeBridge));
        assert!(s.capture_available && s.activate_available);
    }

    #[test]
    fn extension_absent_emits_notice_and_uses_fallback_not_silent_fail() {
        // No extension, but uinput is up: degrade to the Alt+Tab focus fallback
        // and surface an honest capability notice (never a silent failure).
        let s = GuiBackendStatus::assess(false, true, false, false, true);
        assert_eq!(s.preferred_focus_backend, Some(WindowFocusBackend::UinputAltTab));
        assert!(!s.capability_notices.is_empty());
        assert!(s
            .capability_notices
            .iter()
            .any(|n| n.contains("GNOME window extension unavailable")));
        // Capture has no fallback here (no portal) -> honest notice.
        assert!(!s.capture_available);
        assert!(s.activate_available, "uinput Alt+Tab can still activate");
    }

    #[test]
    fn extension_absent_with_portal_keeps_capture_and_activate() {
        let s = GuiBackendStatus::assess(false, false, true, false, true);
        assert_eq!(s.preferred_focus_backend, Some(WindowFocusBackend::Portal));
        assert!(s.capture_available && s.activate_available);
    }

    #[test]
    fn fully_degraded_has_no_backend_and_clear_notices() {
        let s = GuiBackendStatus::assess(false, false, false, false, true);
        assert_eq!(s.preferred_focus_backend, None);
        assert!(!s.capture_available && !s.activate_available);
        assert!(s
            .capability_notices
            .iter()
            .any(|n| n.contains("no window-focus backend available")));
    }

    #[test]
    fn x11_wmctrl_only_eligible_off_wayland() {
        // x11 available, Wayland session -> wmctrl NOT eligible (no path).
        let wayland = GuiBackendStatus::assess(false, false, false, true, true);
        assert_eq!(wayland.preferred_focus_backend, None);
        // Same on an X11 session -> wmctrl is the fallback.
        let x11 = GuiBackendStatus::assess(false, false, false, true, false);
        assert_eq!(x11.preferred_focus_backend, Some(WindowFocusBackend::X11Wmctrl));
    }

    fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    /// A backend status with a given session type and input-substrate usability.
    fn backend_with(session: &str, can_execute: bool) -> GuiActionBackendStatus {
        let mut status = if can_execute {
            GuiActionBackendStatus::available("test_backend")
        } else {
            GuiActionBackendStatus::blocked("unavailable", "no input substrate", session)
        };
        status.session_type = session.to_string();
        status.can_execute_actions = can_execute;
        status
    }

    fn is_subsequence_of_global(order: &[WindowFocusBackend]) -> bool {
        let mut it = ALL_BACKENDS_IN_PREFERENCE_ORDER.iter();
        order
            .iter()
            .all(|backend| it.by_ref().any(|candidate| candidate == backend))
    }

    #[test]
    fn wayland_excludes_wmctrl_and_keeps_preference_order() {
        let backend = backend_with("wayland", true);
        let chain = select_focus_backends("wayland", &backend);
        assert_eq!(
            chain,
            vec![
                WindowFocusBackend::GnomeBridge,
                WindowFocusBackend::Portal,
                WindowFocusBackend::UinputAltTab,
            ],
            "Wayland chain is compositor-native first, Alt+Tab last, no wmctrl"
        );
        // Requirement 3.1: Wayland path never depends on the X11-only tool.
        assert!(!chain.contains(&WindowFocusBackend::X11Wmctrl));
        assert!(chain.iter().all(|b| b.is_wayland_capable()));
        assert!(is_subsequence_of_global(&chain));
    }

    #[test]
    fn wayland_without_input_substrate_drops_alt_tab() {
        let backend = backend_with("wayland", false);
        let chain = select_focus_backends("wayland", &backend);
        assert_eq!(
            chain,
            vec![WindowFocusBackend::GnomeBridge, WindowFocusBackend::Portal],
            "no usable input substrate ⇒ Alt+Tab fallback is not eligible"
        );
        assert!(!chain.contains(&WindowFocusBackend::X11Wmctrl));
    }

    #[test]
    fn x11_includes_wmctrl_last() {
        let backend = backend_with("x11", true);
        let chain = select_focus_backends("x11", &backend);
        assert_eq!(
            chain,
            vec![
                WindowFocusBackend::GnomeBridge,
                WindowFocusBackend::Portal,
                WindowFocusBackend::UinputAltTab,
                WindowFocusBackend::X11Wmctrl,
            ],
            "X11 chain ends with the X11-only wmctrl path"
        );
        assert_eq!(chain.last(), Some(&WindowFocusBackend::X11Wmctrl));
        assert!(is_subsequence_of_global(&chain));
    }

    #[test]
    fn x11_without_input_substrate_keeps_compositor_and_wmctrl() {
        let backend = backend_with("x11", false);
        let chain = select_focus_backends("x11", &backend);
        assert_eq!(
            chain,
            vec![
                WindowFocusBackend::GnomeBridge,
                WindowFocusBackend::Portal,
                WindowFocusBackend::X11Wmctrl,
            ],
            "Alt+Tab dropped without input substrate; wmctrl still eligible on X11"
        );
    }

    #[test]
    fn unknown_session_never_selects_wmctrl() {
        for session in ["unknown", "", "mir", "tty"] {
            let backend = backend_with(session, true);
            let chain = select_focus_backends(session, &backend);
            assert!(
                !chain.contains(&WindowFocusBackend::X11Wmctrl),
                "session {session:?} must not select the X11-only wmctrl path"
            );
            assert!(is_subsequence_of_global(&chain));
        }
    }

    #[test]
    fn alt_tab_backend_requires_verification() {
        // Requirement 3.2: the key-based fallback is never trusted blindly.
        assert!(WindowFocusBackend::UinputAltTab.requires_verification());
        assert!(!WindowFocusBackend::GnomeBridge.requires_verification());
        assert!(!WindowFocusBackend::Portal.requires_verification());
        assert!(!WindowFocusBackend::X11Wmctrl.requires_verification());
    }

    #[test]
    fn backend_classification_invariants() {
        assert!(WindowFocusBackend::GnomeBridge.is_compositor_native());
        assert!(WindowFocusBackend::Portal.is_compositor_native());
        assert!(!WindowFocusBackend::UinputAltTab.is_compositor_native());
        assert!(WindowFocusBackend::X11Wmctrl.is_x11_only());
        assert!(!WindowFocusBackend::X11Wmctrl.is_wayland_capable());
    }

    #[test]
    fn verify_active_window_matches_requested_identity() {
        let id = WindowIdentity::new(Some("Chrome"), None);
        assert_eq!(
            verify_active_window(&id, Some("Google Chrome")),
            WindowFocusVerification::Verified
        );
        assert_eq!(
            verify_active_window(&id, Some("Files")),
            WindowFocusVerification::Failed
        );
        assert_eq!(
            verify_active_window(&id, None),
            WindowFocusVerification::Inconclusive
        );
        // No requested identity ⇒ cannot confirm.
        assert_eq!(
            verify_active_window(&WindowIdentity::default(), Some("Chrome")),
            WindowFocusVerification::Inconclusive
        );
    }

    #[test]
    fn window_identity_sanitizes_and_reports_target() {
        let id = WindowIdentity::new(Some("  Chrome  "), Some(""));
        assert_eq!(id.app_hint.as_deref(), Some("Chrome"));
        assert_eq!(id.window_hint, None);
        assert!(id.has_target());
        assert_eq!(id.label(), Some("Chrome"));
        assert!(!WindowIdentity::default().has_target());
    }

    #[test]
    fn flag_defaults_off() {
        assert!(!GuiWaylandFocusConfig::default().is_enabled());
        // Absent env ⇒ OFF on the default-off path.
        let cfg = GuiWaylandFocusConfig::from_env_lookup(lookup_from(&[]));
        assert!(!cfg.is_enabled());
    }

    #[test]
    fn flag_truthy_enables() {
        for value in ["1", "true", "yes", "on", "ON", " True "] {
            let cfg =
                GuiWaylandFocusConfig::from_env_lookup(lookup_from(&[(WAYLAND_FOCUS_ENV_FLAG, value)]));
            assert!(cfg.is_enabled(), "value {value:?} should enable the flag");
        }
        for value in ["0", "false", "no", "off", "", "maybe"] {
            let cfg =
                GuiWaylandFocusConfig::from_env_lookup(lookup_from(&[(WAYLAND_FOCUS_ENV_FLAG, value)]));
            assert!(!cfg.is_enabled(), "value {value:?} should NOT enable the flag");
        }
    }

    #[test]
    fn flag_default_on_and_rollback() {
        // Absent env ⇒ ON on the default-on (gate-flip) path.
        let cfg = GuiWaylandFocusConfig::from_env_lookup_default_on(lookup_from(&[]));
        assert!(cfg.is_enabled());
        // Truthy keeps it ON.
        let cfg = GuiWaylandFocusConfig::from_env_lookup_default_on(lookup_from(&[(
            WAYLAND_FOCUS_ENV_FLAG,
            "on",
        )]));
        assert!(cfg.is_enabled());
        // Explicit falsy is the documented rollback switch.
        for value in ["0", "false", "no", "off", ""] {
            let cfg = GuiWaylandFocusConfig::from_env_lookup_default_on(lookup_from(&[(
                WAYLAND_FOCUS_ENV_FLAG,
                value,
            )]));
            assert!(!cfg.is_enabled(), "value {value:?} should roll back to OFF");
        }
    }

    #[test]
    fn backend_str_tags_are_stable() {
        assert_eq!(WindowFocusBackend::GnomeBridge.as_str(), "gnome_bridge");
        assert_eq!(WindowFocusBackend::Portal.as_str(), "portal");
        assert_eq!(WindowFocusBackend::UinputAltTab.as_str(), "uinput_alt_tab");
        assert_eq!(WindowFocusBackend::X11Wmctrl.as_str(), "x11_wmctrl");
    }

    // ---- Task 4.2: routing / activate-by-identity preference ---------------

    fn chrome_identity() -> WindowIdentity {
        WindowIdentity::new(Some("Chrome"), Some("Google Chrome"))
    }

    #[test]
    fn routing_prefers_compositor_native_over_alt_tab() {
        // Requirement 3.2: when a compositor-native activate-by-identity backend
        // is available it is chosen ahead of the key-based Alt+Tab fallback.
        let chain = vec![
            WindowFocusBackend::GnomeBridge,
            WindowFocusBackend::Portal,
            WindowFocusBackend::UinputAltTab,
        ];
        let used = select_window_focus_backend(&chain, &chrome_identity(), |_| true)
            .expect("a backend should be selected");
        assert_eq!(used, WindowFocusBackend::GnomeBridge);
        assert!(used.is_compositor_native());
        assert!(!used.requires_verification());
    }

    #[test]
    fn routing_falls_back_to_alt_tab_when_compositor_unavailable() {
        // The compositor-native handlers are not wired yet (Task 4.3+/4.5), so a
        // healthy session correctly falls back to the key-based UinputAltTab
        // last-resort — which MUST be verified.
        let chain = vec![
            WindowFocusBackend::GnomeBridge,
            WindowFocusBackend::Portal,
            WindowFocusBackend::UinputAltTab,
        ];
        let used = select_window_focus_backend(&chain, &chrome_identity(), |backend| {
            backend == WindowFocusBackend::UinputAltTab
        })
        .expect("the Alt+Tab fallback should be selected");
        assert_eq!(used, WindowFocusBackend::UinputAltTab);
        assert!(used.requires_verification());
    }

    #[test]
    fn routing_respects_chain_order_for_portal_before_alt_tab() {
        let chain = vec![
            WindowFocusBackend::GnomeBridge,
            WindowFocusBackend::Portal,
            WindowFocusBackend::UinputAltTab,
        ];
        // GnomeBridge unavailable, Portal available ⇒ Portal wins over Alt+Tab.
        let used = select_window_focus_backend(&chain, &chrome_identity(), |backend| {
            backend != WindowFocusBackend::GnomeBridge
        })
        .expect("portal should be selected");
        assert_eq!(used, WindowFocusBackend::Portal);
    }

    #[test]
    fn routing_without_target_never_blindly_alt_tabs() {
        // Property: identity from sanitized resolved-target data only — no blind
        // Alt+Tab when there is no concrete window identity (Requirement 3.3).
        let chain = vec![WindowFocusBackend::UinputAltTab];
        let err = select_window_focus_backend(&chain, &WindowIdentity::default(), |_| true)
            .expect_err("no target ⇒ error, never a blind switch");
        assert_eq!(err, WindowFocusError::NoTarget);
    }

    #[test]
    fn routing_no_available_backend_is_actionable_error() {
        let chain = vec![
            WindowFocusBackend::GnomeBridge,
            WindowFocusBackend::Portal,
            WindowFocusBackend::UinputAltTab,
        ];
        let err = select_window_focus_backend(&chain, &chrome_identity(), |_| false)
            .expect_err("nothing available ⇒ NoBackendAvailable");
        assert!(matches!(err, WindowFocusError::NoBackendAvailable { .. }));
        // Empty chain (e.g. an unknown session with no input substrate) too.
        let err = select_window_focus_backend(&[], &chrome_identity(), |_| true)
            .expect_err("empty chain ⇒ NoBackendAvailable");
        assert!(matches!(err, WindowFocusError::NoBackendAvailable { .. }));
    }

    #[test]
    fn routing_json_is_sanitized_and_truthful() {
        let identity = chrome_identity();
        let chain = vec![
            WindowFocusBackend::GnomeBridge,
            WindowFocusBackend::Portal,
            WindowFocusBackend::UinputAltTab,
        ];
        let json = window_focus_routing_json(
            &identity,
            &chain,
            Some(WindowFocusBackend::UinputAltTab),
            WindowFocusVerification::NotAttempted,
            None,
        );
        assert_eq!(json["routed"], serde_json::json!(true));
        assert_eq!(json["backend_used"], serde_json::json!("uinput_alt_tab"));
        // UinputAltTab is the key-based fallback ⇒ must be verified (3.2).
        assert_eq!(json["requires_verification"], serde_json::json!(true));
        assert_eq!(
            json["chain"],
            serde_json::json!(["gnome_bridge", "portal", "uinput_alt_tab"])
        );
        assert_eq!(json["verification"], serde_json::json!("not_attempted"));
        assert_eq!(json["error"], serde_json::Value::Null);
    }

    // ---- Task 4.3: verify-by-reobserve verdict + no-path actionable error ---

    #[test]
    fn verify_focus_by_reobserve_reports_truthful_verdict() {
        let id = WindowIdentity::new(Some("Chrome"), Some("Google Chrome"));
        // Fresh active window matches the requested identity ⇒ Verified.
        assert_eq!(
            verify_focus_by_reobserve(&id, Some("Google Chrome"), true),
            WindowFocusVerification::Verified
        );
        // Fresh active window is a different window ⇒ Failed (not a false pass).
        assert_eq!(
            verify_focus_by_reobserve(&id, Some("Files"), true),
            WindowFocusVerification::Failed
        );
        // No observed active label even though the probe ran ⇒ Inconclusive.
        assert_eq!(
            verify_focus_by_reobserve(&id, None, true),
            WindowFocusVerification::Inconclusive
        );
    }

    #[test]
    fn verify_focus_by_reobserve_is_inconclusive_when_probe_failed() {
        // Requirement 3.4 / 23.2: an unreliable fresh observation (probe not ok)
        // is NEVER reported as a confirmed match — even if a stale label happens
        // to look right. The Alt+Tab fallback in particular must not be trusted
        // blindly, so a failed re-observe yields Inconclusive, not Verified.
        let id = WindowIdentity::new(Some("Chrome"), None);
        assert_eq!(
            verify_focus_by_reobserve(&id, Some("Google Chrome"), false),
            WindowFocusVerification::Inconclusive
        );
        assert_eq!(
            verify_focus_by_reobserve(&id, None, false),
            WindowFocusVerification::Inconclusive
        );
    }

    #[test]
    fn no_path_message_is_actionable_and_never_mentions_wmctrl() {
        // Requirement 3.3: no viable focus path ⇒ a clear, actionable, sanitized
        // reason that is explicitly NOT the legacy generic "wmctrl required"
        // failure.
        let no_backend = WindowFocusError::NoBackendAvailable {
            reason: "no eligible window-focus backend for this session".into(),
        };
        let msg = no_focus_path_message(&no_backend, "wayland");
        assert!(msg.contains("wayland"), "message names the session: {msg}");
        assert!(
            msg.contains("no usable window-activation backend"),
            "message explains the backend gap: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("portal") && msg.to_lowercase().contains("uinput"),
            "message states what is needed: {msg}"
        );
        assert!(
            !msg.to_lowercase().contains("wmctrl"),
            "message must NOT be the legacy wmctrl failure: {msg}"
        );
    }

    #[test]
    fn no_path_message_for_missing_target_asks_for_a_window() {
        let msg = no_focus_path_message(&WindowFocusError::NoTarget, "wayland");
        assert!(
            msg.to_lowercase().contains("which window")
                || msg.to_lowercase().contains("target window"),
            "no-target message asks the user to name a window: {msg}"
        );
        assert!(!msg.to_lowercase().contains("wmctrl"));
    }

    #[test]
    fn no_path_message_sanitizes_blank_session() {
        let err = WindowFocusError::NoBackendAvailable {
            reason: "all eligible window-focus backends are currently unavailable".into(),
        };
        let msg = no_focus_path_message(&err, "   ");
        // Blank session degrades to a neutral label, never an empty hole.
        assert!(msg.contains("current session"), "blank session ⇒ neutral: {msg}");
    }

    // ---- Task 4.4 gap-fill: remaining selection/verdict surfaces ------------

    #[test]
    fn verification_is_verified_only_for_verified_variant() {
        // Only `Verified` counts as a confirmed focus; everything else is not a
        // pass (Requirement 23.2 — ambiguous evidence is never a false verified).
        assert!(WindowFocusVerification::Verified.is_verified());
        assert!(!WindowFocusVerification::Inconclusive.is_verified());
        assert!(!WindowFocusVerification::Failed.is_verified());
        assert!(!WindowFocusVerification::NotAttempted.is_verified());
        // Stable tags for events.
        assert_eq!(WindowFocusVerification::Verified.as_str(), "verified");
        assert_eq!(WindowFocusVerification::Inconclusive.as_str(), "inconclusive");
        assert_eq!(WindowFocusVerification::Failed.as_str(), "failed");
        assert_eq!(WindowFocusVerification::NotAttempted.as_str(), "not_attempted");
    }

    #[test]
    fn outcome_summary_json_is_sanitized_and_truthful() {
        // The per-attempt outcome summary carries only the stable backend tag and
        // the verification verdict — no raw payload, no secrets (Property 7).
        let outcome = WindowFocusOutcome {
            backend_used: WindowFocusBackend::UinputAltTab,
            verification: WindowFocusVerification::Verified,
        };
        let json = outcome.summary_json();
        assert_eq!(json["backend_used"], serde_json::json!("uinput_alt_tab"));
        assert_eq!(json["verification"], serde_json::json!("verified"));
        // Exactly the two sanitized fields — nothing leaked.
        assert_eq!(json.as_object().map(|o| o.len()), Some(2));
    }

    #[test]
    fn routing_json_surfaces_error_and_null_backend_on_no_path() {
        // The no-viable-path routing JSON (Requirement 3.3) reports the actionable
        // error string and a null `backend_used` — the data that drives the
        // truthful `window_focus_unavailable` result; the chosen backend is never
        // fabricated when nothing ran.
        let identity = chrome_identity();
        let chain = vec![WindowFocusBackend::GnomeBridge, WindowFocusBackend::Portal];
        let err = WindowFocusError::NoBackendAvailable {
            reason: "all eligible window-focus backends are currently unavailable".into(),
        };
        let json = window_focus_routing_json(
            &identity,
            &chain,
            None,
            WindowFocusVerification::NotAttempted,
            Some(&err),
        );
        assert_eq!(json["backend_used"], serde_json::Value::Null);
        assert_eq!(json["requires_verification"], serde_json::json!(false));
        assert_eq!(json["verification"], serde_json::json!("not_attempted"));
        assert!(
            json["error"].as_str().is_some_and(|msg| msg.contains("window-focus backend")),
            "error string is surfaced: {json}"
        );
        // wmctrl is never named in the chain for this (Wayland-style) no-path.
        assert_eq!(json["chain"], serde_json::json!(["gnome_bridge", "portal"]));
    }

    #[test]
    fn no_path_message_for_backend_failed_and_not_implemented_are_actionable() {
        // Both remaining error variants surface a clear, session-scoped, sanitized
        // reason that is NOT the legacy generic "wmctrl required" failure.
        let failed = WindowFocusError::BackendFailed {
            backend: "uinput_alt_tab",
            reason: "input substrate rejected the key sequence".into(),
        };
        let failed_msg = no_focus_path_message(&failed, "wayland");
        assert!(failed_msg.contains("uinput_alt_tab"), "names the backend: {failed_msg}");
        assert!(failed_msg.contains("wayland"), "names the session: {failed_msg}");
        assert!(!failed_msg.to_lowercase().contains("wmctrl"), "no wmctrl: {failed_msg}");

        let not_impl = WindowFocusError::NotImplemented("gnome_bridge");
        let not_impl_msg = no_focus_path_message(&not_impl, "x11");
        assert!(not_impl_msg.contains("gnome_bridge"), "names the backend: {not_impl_msg}");
        assert!(not_impl_msg.contains("x11"), "names the session: {not_impl_msg}");
        assert!(!not_impl_msg.to_lowercase().contains("wmctrl"), "no wmctrl: {not_impl_msg}");
    }
}
