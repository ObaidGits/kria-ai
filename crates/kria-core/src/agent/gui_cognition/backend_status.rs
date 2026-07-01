//! GUI action/focus **backend availability status** — the lean, self-contained
//! status surface used by the live desktop `gui-automation-status` endpoint and
//! the V2 GUI-cognition path.
//!
//! Task 13: this module was extracted out of the (removed) V1 `executor` /
//! `window_focus` runtime so the backend-availability probing survives the
//! deletion of the over-built V1 pipeline. It is pure/derived — no I/O, no
//! dependency on any other `gui_cognition` submodule.

use serde::{Deserialize, Serialize};

/// The window-focus backend kinds in global preference order (most preferred
/// first). Pure/derived — selection preserves this relative order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowFocusBackend {
    /// GNOME-shell D-Bus bridge: compositor-native activate-by-window-identity.
    /// Wayland-capable (and works under GNOME-on-X11). Most preferred.
    GnomeBridge,
    /// xdg desktop portal activate path: compositor-native, Wayland-capable.
    Portal,
    /// Alt+Tab synthesized via the uinput/ydotool input substrate. Last-resort
    /// key-based fallback; MUST be verified afterwards.
    UinputAltTab,
    /// `wmctrl` activate-by-window. X11-only: never eligible on Wayland.
    X11Wmctrl,
}

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
    /// path (the preferred class).
    pub fn is_compositor_native(&self) -> bool {
        matches!(self, Self::GnomeBridge | Self::Portal)
    }

    /// Whether this backend is usable on Wayland sessions. Everything except the
    /// `wmctrl` path is Wayland-capable.
    pub fn is_wayland_capable(&self) -> bool {
        !matches!(self, Self::X11Wmctrl)
    }

    /// Whether this backend is restricted to X11 sessions (`wmctrl` only).
    pub fn is_x11_only(&self) -> bool {
        matches!(self, Self::X11Wmctrl)
    }
}

/// Env flag that gates whether backend-availability status is surfaced.
pub const BACKEND_STATUS_ENV_FLAG: &str = "KRIA_GUI_COG_BACKEND_STATUS";

/// Whether the backend-availability status surfacing is enabled (default ON;
/// rolled back by a falsy `KRIA_GUI_COG_BACKEND_STATUS`).
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

/// Availability of the window-focus / capture / activate backends. Lets the
/// runtime degrade GRACEFULLY and surface an honest capability notice instead of
/// a silent failure when the GNOME extension is absent. Pure/derived — no I/O.
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
                "window activation unavailable (no extension, portal, or input backend)"
                    .to_string(),
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

/// The capability matrix of a selected GUI action backend. Pure/derived.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiExecutorCapabilityMatrix {
    pub observe: bool,
    pub focus_field: bool,
    pub fill_field: bool,
    pub click_control: bool,
    pub post_action_observe: bool,
    pub verify: bool,
    pub recovery_focus: bool,
    pub recovery_modal: bool,
}

impl GuiExecutorCapabilityMatrix {
    pub fn all_available() -> Self {
        Self {
            observe: true,
            focus_field: true,
            fill_field: true,
            click_control: true,
            post_action_observe: true,
            verify: true,
            recovery_focus: true,
            recovery_modal: true,
        }
    }

    pub fn observe_only() -> Self {
        Self {
            observe: true,
            focus_field: false,
            fill_field: false,
            click_control: false,
            post_action_observe: true,
            verify: true,
            recovery_focus: false,
            recovery_modal: true,
        }
    }
}

/// The live GUI-automation backend availability + halt/release status surfaced
/// by the desktop status endpoint. Pure/derived serialization surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiActionBackendStatus {
    pub global_halt_engaged: bool,
    pub halt_kind: String,
    pub halt_reason: Option<String>,
    pub release_conditions: Vec<String>,
    pub startup_elapsed_ms: Option<u64>,
    pub can_observe: bool,
    pub can_plan: bool,
    pub automation_enabled: bool,
    pub vision_sidecar: String,
    pub uinput_daemon: String,
    pub orchestrator_available: bool,
    pub session_type: String,
    pub xdotool_available: bool,
    pub ydotool_available: bool,
    pub uinput_available: bool,
    pub selected_backend: String,
    pub backend_selection_reason: String,
    pub backend_probe_status: String,
    pub backend_probe_errors: Vec<String>,
    pub input_backend_kind: String,
    pub focus_supported: bool,
    pub typing_supported: bool,
    pub click_supported: bool,
    pub verification_supported: bool,
    pub xdotool_usable_for_actions: bool,
    pub ydotool_usable_for_actions: bool,
    pub uinput_socket_path: Option<String>,
    pub uinput_socket_accessible: bool,
    pub can_execute_actions: bool,
    pub blockers: Vec<String>,
    pub capabilities: GuiExecutorCapabilityMatrix,
}

impl GuiActionBackendStatus {
    pub fn available(selected_backend: impl Into<String>) -> Self {
        let selected_backend = selected_backend.into();
        Self {
            global_halt_engaged: false,
            halt_kind: "none".into(),
            halt_reason: None,
            release_conditions: Vec::new(),
            startup_elapsed_ms: None,
            can_observe: true,
            can_plan: true,
            automation_enabled: true,
            vision_sidecar: "unknown".into(),
            uinput_daemon: "unknown".into(),
            orchestrator_available: true,
            session_type: "test".into(),
            xdotool_available: true,
            ydotool_available: true,
            uinput_available: true,
            selected_backend: selected_backend.clone(),
            backend_selection_reason: format!("Test backend {selected_backend} is available."),
            backend_probe_status: "test_backend_ready".into(),
            backend_probe_errors: Vec::new(),
            input_backend_kind: "test".into(),
            focus_supported: true,
            typing_supported: true,
            click_supported: true,
            verification_supported: true,
            xdotool_usable_for_actions: true,
            ydotool_usable_for_actions: true,
            uinput_socket_path: None,
            uinput_socket_accessible: true,
            can_execute_actions: true,
            blockers: Vec::new(),
            capabilities: GuiExecutorCapabilityMatrix::all_available(),
        }
    }

    pub fn blocked(
        selected_backend: impl Into<String>,
        blocker: impl Into<String>,
        session_type: impl Into<String>,
    ) -> Self {
        let selected_backend = selected_backend.into();
        let blocker = blocker.into();
        Self {
            global_halt_engaged: false,
            halt_kind: "service_not_ready".into(),
            halt_reason: None,
            release_conditions: vec!["Resolve the GUI action backend blocker, then retry.".into()],
            startup_elapsed_ms: None,
            can_observe: true,
            can_plan: true,
            automation_enabled: false,
            vision_sidecar: "unknown".into(),
            uinput_daemon: "unknown".into(),
            orchestrator_available: false,
            session_type: session_type.into(),
            xdotool_available: false,
            ydotool_available: false,
            uinput_available: false,
            selected_backend: selected_backend.clone(),
            backend_selection_reason: blocker.clone(),
            backend_probe_status: "test_backend_blocked".into(),
            backend_probe_errors: vec![blocker.clone()],
            input_backend_kind: "none".into(),
            focus_supported: false,
            typing_supported: false,
            click_supported: false,
            verification_supported: true,
            xdotool_usable_for_actions: false,
            ydotool_usable_for_actions: false,
            uinput_socket_path: None,
            uinput_socket_accessible: false,
            can_execute_actions: false,
            blockers: vec![blocker],
            capabilities: GuiExecutorCapabilityMatrix::observe_only(),
        }
    }

    /// The primary blocker preventing this backend from executing actions,
    /// independent of a specific action kind. Prefers the global-halt reason,
    /// then the first recorded blocker, then the backend selection reason.
    pub fn primary_backend_blocker(&self) -> String {
        if self.global_halt_engaged {
            return self
                .halt_reason
                .clone()
                .unwrap_or_else(|| "global safety halt is engaged".into());
        }
        if let Some(blocker) = self.blockers.first() {
            return blocker.clone();
        }
        if !self.backend_selection_reason.trim().is_empty() {
            return self.backend_selection_reason.clone();
        }
        "GUI action backend is unavailable".into()
    }
}

/// The probed inputs used to deterministically select a GUI action backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiBackendProbeInput {
    pub global_halt_engaged: bool,
    pub halt_reason: Option<String>,
    pub automation_enabled: bool,
    pub orchestrator_available: bool,
    pub session_type: String,
    pub vision_sidecar: String,
    pub uinput_daemon: String,
    pub xdotool_available: bool,
    pub xdotool_display_usable: bool,
    pub ydotool_available: bool,
    pub ydotool_permission_ok: bool,
    pub uinput_available: bool,
    pub uinput_socket_path: Option<String>,
    pub uinput_socket_accessible: bool,
}

/// The deterministic backend selection derived from a [`GuiBackendProbeInput`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiBackendSelection {
    pub selected_backend: String,
    pub backend_selection_reason: String,
    pub backend_probe_status: String,
    pub backend_probe_errors: Vec<String>,
    pub input_backend_kind: String,
    pub focus_supported: bool,
    pub typing_supported: bool,
    pub click_supported: bool,
    pub verification_supported: bool,
    pub xdotool_usable_for_actions: bool,
    pub ydotool_usable_for_actions: bool,
    pub can_execute_actions: bool,
    pub blockers: Vec<String>,
    pub capabilities: GuiExecutorCapabilityMatrix,
}

/// Deterministically select the GUI action backend for the probed session.
pub fn select_gui_action_backend(input: &GuiBackendProbeInput) -> GuiBackendSelection {
    let session = input.session_type.trim().to_lowercase();
    let xdotool_usable_for_actions =
        session == "x11" && input.xdotool_available && input.xdotool_display_usable;
    let ydotool_usable_for_actions =
        session == "wayland" && input.ydotool_available && input.ydotool_permission_ok;
    let uinput_usable_for_actions =
        session == "wayland" && input.uinput_available && input.uinput_socket_accessible;

    let mut backend_probe_errors = Vec::new();
    if session == "wayland" && input.xdotool_available {
        backend_probe_errors
            .push("xdotool detected but not usable for Wayland GUI actions".to_string());
    }
    if input.uinput_available && !input.uinput_socket_accessible {
        backend_probe_errors
            .push("uinput daemon reported running but socket is not accessible".into());
    }
    if input.ydotool_available && !input.ydotool_permission_ok {
        backend_probe_errors
            .push("ydotool detected but permission/usability probe did not pass".into());
    }
    if session == "x11" && input.xdotool_available && !input.xdotool_display_usable {
        backend_probe_errors.push("xdotool detected but DISPLAY/active-window probe failed".into());
    }

    let mut selection = GuiBackendSelection {
        selected_backend: "unavailable".into(),
        backend_selection_reason: "No deterministic GUI action backend is available.".into(),
        backend_probe_status: "unknown_session_blocked".into(),
        backend_probe_errors,
        input_backend_kind: "none".into(),
        focus_supported: false,
        typing_supported: false,
        click_supported: false,
        verification_supported: true,
        xdotool_usable_for_actions,
        ydotool_usable_for_actions,
        can_execute_actions: false,
        blockers: Vec::new(),
        capabilities: GuiExecutorCapabilityMatrix::observe_only(),
    };

    if input.global_halt_engaged {
        selection.selected_backend = "blocked_global_halt".into();
        selection.backend_probe_status = "global_halt_blocked".into();
        selection.backend_selection_reason = input
            .halt_reason
            .clone()
            .unwrap_or_else(|| "Global safety halt is engaged.".into());
        selection
            .blockers
            .push(selection.backend_selection_reason.clone());
        return selection;
    }

    if !input.orchestrator_available {
        selection.backend_probe_status = "orchestrator_unavailable".into();
        selection.backend_selection_reason = "GUI service orchestrator is unavailable.".into();
        selection
            .blockers
            .push(selection.backend_selection_reason.clone());
        return selection;
    }

    if !input.automation_enabled {
        selection.selected_backend = "automation_disabled".into();
        selection.backend_probe_status = "automation_disabled".into();
        selection.backend_selection_reason = "GUI automation is disabled by user setting.".into();
        selection
            .blockers
            .push(selection.backend_selection_reason.clone());
        return selection;
    }

    match session.as_str() {
        "wayland" if uinput_usable_for_actions => {
            selection.selected_backend = "uinput_accessibility".into();
            selection.backend_probe_status = "wayland_uinput_ready".into();
            selection.backend_selection_reason =
                "Wayland session selected uinput because the daemon and socket are healthy.".into();
            selection.input_backend_kind = "uinput".into();
        }
        "wayland" if ydotool_usable_for_actions => {
            selection.selected_backend = "ydotool_accessibility".into();
            selection.backend_probe_status = "wayland_ydotool_ready".into();
            selection.backend_selection_reason =
                "Wayland session selected ydotool because its usability probe passed.".into();
            selection.input_backend_kind = "ydotool".into();
        }
        "wayland" => {
            selection.backend_probe_status = "wayland_no_input_backend".into();
            selection.backend_selection_reason =
                "Wayland session has no usable uinput socket or validated ydotool backend.".into();
            selection
                .blockers
                .push(selection.backend_selection_reason.clone());
            return selection;
        }
        "x11" if xdotool_usable_for_actions => {
            selection.selected_backend = "xdotool_accessibility".into();
            selection.backend_probe_status = "x11_xdotool_ready".into();
            selection.backend_selection_reason =
                "X11 session selected xdotool because DISPLAY and active-window probe passed."
                    .into();
            selection.input_backend_kind = "xdotool".into();
        }
        "x11" => {
            selection.backend_probe_status = "x11_no_xdotool".into();
            selection.backend_selection_reason =
                "X11 session has no usable xdotool action backend.".into();
            selection
                .blockers
                .push(selection.backend_selection_reason.clone());
            return selection;
        }
        _ => {
            selection.backend_probe_status = "unknown_session_blocked".into();
            selection.backend_selection_reason =
                "GUI session type is unknown and no deterministic action backend is available."
                    .into();
            selection
                .blockers
                .push(selection.backend_selection_reason.clone());
            return selection;
        }
    }

    selection.can_execute_actions = true;
    selection.focus_supported = true;
    selection.typing_supported = true;
    selection.click_supported = true;
    selection.capabilities = GuiExecutorCapabilityMatrix::all_available();
    selection
}
