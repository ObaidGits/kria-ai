//! RFC 008: Tauri commands for the GUI Automation master toggle + status.
//!
//! Frontend bindings (see UI):
//!   - `get_gui_automation_status` → returns `{ vision, uinput, enabled, ... }`
//!   - `set_gui_automation_enabled(enabled: bool)` → master kill switch

use super::AppStateCell;
use kria_core::agent::gui_cognition::backend_status::{select_gui_action_backend, GuiBackendProbeInput};
use kria_core::orchestrator::ServiceStatus;
use tauri::{AppHandle, Manager};

/// Snapshot of GUI automation health for the UI status indicator.
#[derive(serde::Serialize)]
pub struct GuiAutomationStatus {
    /// Current liveness of the Python vision sidecar.
    pub vision_sidecar: String,
    /// Current liveness of the uinput daemon.
    pub uinput_daemon: String,
    /// User-controlled master enable toggle.
    pub automation_enabled: bool,
    /// `true` iff the GlobalSafetyHalt flag is set (any reason).
    pub global_halt_engaged: bool,
    /// Machine-readable halt classification for UI copy.
    pub halt_kind: String,
    /// Human-readable reason the halt is currently engaged (if any).
    pub halt_reason: Option<String>,
    /// Short remediation hints required before GUI actions can run.
    pub release_conditions: Vec<String>,
    /// PID of vision sidecar process, if running.
    pub vision_pid: Option<u32>,
    /// PID of uinput daemon process, if running.
    pub uinput_pid: Option<u32>,
    /// `true` if the ServiceOrchestrator initialized successfully at boot.
    pub orchestrator_available: bool,
    pub session_type: String,
    pub selected_backend: String,
    pub backend_selection_reason: String,
    pub backend_probe_status: String,
    pub backend_probe_errors: Vec<String>,
    pub xdotool_available: bool,
    pub xdotool_usable_for_actions: bool,
    pub ydotool_available: bool,
    pub ydotool_usable_for_actions: bool,
    pub uinput_socket_path: Option<String>,
    pub uinput_socket_accessible: bool,
    pub can_execute_actions: bool,
}

fn command_available(name: &str) -> bool {
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|path| path.join(name))
                .find(|candidate| candidate.is_file())
        })
        .is_some()
}

fn current_gui_session_type() -> String {
    let explicit = std::env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if !explicit.is_empty() {
        return explicit;
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return "wayland".into();
    }
    if std::env::var_os("DISPLAY").is_some() {
        return "x11".into();
    }
    "unknown".into()
}

fn gui_halt_kind(
    orchestrator_available: bool,
    automation_enabled: bool,
    global_halt_engaged: bool,
    halt_reason: Option<&str>,
    vision_sidecar: &str,
    uinput_daemon: &str,
) -> String {
    if !orchestrator_available {
        return "orchestrator_unavailable".into();
    }
    if !automation_enabled || halt_reason.is_some_and(|reason| reason.contains("user disabled")) {
        return "user_disabled".into();
    }
    if !global_halt_engaged {
        return "none".into();
    }
    if vision_sidecar == "starting"
        || uinput_daemon == "starting"
        || halt_reason.is_some_and(|reason| {
            reason.contains("warming")
                || reason.contains("startup")
                || reason.contains("re-spawning")
        })
    {
        return "startup_warming".into();
    }
    if vision_sidecar == "failed"
        || uinput_daemon == "failed"
        || vision_sidecar == "stopped"
        || uinput_daemon == "stopped"
        || halt_reason.is_some_and(|reason| reason.contains("service not ready"))
    {
        return "service_not_ready".into();
    }
    "emergency".into()
}

fn gui_release_conditions(
    automation_enabled: bool,
    vision_sidecar: &str,
    uinput_daemon: &str,
) -> Vec<String> {
    if !automation_enabled {
        return vec!["Enable GUI automation in Settings.".into()];
    }
    let mut conditions = Vec::new();
    if vision_sidecar == "starting" || uinput_daemon == "starting" {
        conditions.push("Wait for vision sidecar and uinput daemon to report running.".into());
    }
    if vision_sidecar != "running" && vision_sidecar != "starting" {
        conditions.push("Start or repair the vision sidecar.".into());
    }
    if uinput_daemon != "running" && uinput_daemon != "starting" {
        conditions.push("Start or repair the uinput daemon and sudoers/socket permissions.".into());
    }
    conditions
}

impl From<ServiceStatus> for GuiAutomationStatus {
    fn from(s: ServiceStatus) -> Self {
        fn label(v: kria_core::orchestrator::ServiceLiveness) -> String {
            use kria_core::orchestrator::ServiceLiveness::*;
            match v {
                Stopped => "stopped",
                Starting => "starting",
                Running => "running",
                Failed => "failed",
            }
            .to_string()
        }
        let vision_sidecar = label(s.vision_sidecar);
        let uinput_daemon = label(s.uinput_daemon);
        let session_type = current_gui_session_type();
        let xdotool_available = command_available("xdotool");
        let ydotool_available = command_available("ydotool");
        let uinput_socket_path = kria_core::agent::gui_services::default_uinput_socket_path();
        let uinput_socket_accessible = uinput_socket_path.exists();
        let global_halt_engaged = kria_core::safety::is_halted();
        let halt_reason = kria_core::safety::halt_reason();
        let backend_selection = select_gui_action_backend(&GuiBackendProbeInput {
            global_halt_engaged,
            halt_reason: halt_reason.clone(),
            automation_enabled: s.automation_enabled,
            orchestrator_available: true,
            session_type: session_type.clone(),
            vision_sidecar: vision_sidecar.clone(),
            uinput_daemon: uinput_daemon.clone(),
            xdotool_available,
            xdotool_display_usable: session_type == "x11"
                && xdotool_available
                && std::env::var_os("DISPLAY").is_some(),
            ydotool_available,
            ydotool_permission_ok: false,
            uinput_available: uinput_daemon == "running",
            uinput_socket_path: Some(uinput_socket_path.display().to_string()),
            uinput_socket_accessible,
        });
        Self {
            vision_sidecar: vision_sidecar.clone(),
            uinput_daemon: uinput_daemon.clone(),
            automation_enabled: s.automation_enabled,
            global_halt_engaged,
            halt_kind: gui_halt_kind(
                true,
                s.automation_enabled,
                global_halt_engaged,
                halt_reason.as_deref(),
                vision_sidecar.as_str(),
                uinput_daemon.as_str(),
            ),
            halt_reason,
            release_conditions: gui_release_conditions(
                s.automation_enabled,
                vision_sidecar.as_str(),
                uinput_daemon.as_str(),
            ),
            vision_pid: s.vision_pid,
            uinput_pid: s.uinput_pid,
            orchestrator_available: true,
            session_type,
            selected_backend: backend_selection.selected_backend,
            backend_selection_reason: backend_selection.backend_selection_reason,
            backend_probe_status: backend_selection.backend_probe_status,
            backend_probe_errors: backend_selection.backend_probe_errors,
            xdotool_available,
            xdotool_usable_for_actions: backend_selection.xdotool_usable_for_actions,
            ydotool_available,
            ydotool_usable_for_actions: backend_selection.ydotool_usable_for_actions,
            uinput_socket_path: Some(uinput_socket_path.display().to_string()),
            uinput_socket_accessible,
            can_execute_actions: backend_selection.can_execute_actions,
        }
    }
}

fn orchestrator_unavailable_status() -> GuiAutomationStatus {
    let session_type = current_gui_session_type();
    let xdotool_available = command_available("xdotool");
    let ydotool_available = command_available("ydotool");
    let halt_reason = kria_core::safety::halt_reason();
    let backend_selection = select_gui_action_backend(&GuiBackendProbeInput {
        global_halt_engaged: kria_core::safety::is_halted(),
        halt_reason: halt_reason.clone(),
        automation_enabled: false,
        orchestrator_available: false,
        session_type: session_type.clone(),
        vision_sidecar: "stopped".into(),
        uinput_daemon: "stopped".into(),
        xdotool_available,
        xdotool_display_usable: false,
        ydotool_available,
        ydotool_permission_ok: false,
        uinput_available: false,
        uinput_socket_path: None,
        uinput_socket_accessible: false,
    });
    GuiAutomationStatus {
        vision_sidecar: "stopped".to_string(),
        uinput_daemon: "stopped".to_string(),
        automation_enabled: false,
        global_halt_engaged: kria_core::safety::is_halted(),
        halt_kind: "orchestrator_unavailable".into(),
        halt_reason,
        release_conditions: vec!["Restart KRIA with the GUI service orchestrator available.".into()],
        vision_pid: None,
        uinput_pid: None,
        orchestrator_available: false,
        session_type,
        selected_backend: backend_selection.selected_backend,
        backend_selection_reason: backend_selection.backend_selection_reason,
        backend_probe_status: backend_selection.backend_probe_status,
        backend_probe_errors: backend_selection.backend_probe_errors,
        xdotool_available,
        xdotool_usable_for_actions: backend_selection.xdotool_usable_for_actions,
        ydotool_available,
        ydotool_usable_for_actions: backend_selection.ydotool_usable_for_actions,
        uinput_socket_path: None,
        uinput_socket_accessible: false,
        can_execute_actions: backend_selection.can_execute_actions,
    }
}

/// Get the current automation/services status for the UI.
#[tauri::command]
pub async fn get_gui_automation_status(handle: AppHandle) -> Result<GuiAutomationStatus, String> {
    let cell: tauri::State<'_, AppStateCell> = handle.state();
    let Some(state) = cell.get() else {
        return Ok(orchestrator_unavailable_status());
    };

    match state.gui_orchestrator.as_ref() {
        Some(orch) => Ok(orch.status().await.into()),
        None => Ok(orchestrator_unavailable_status()),
    }
}

/// Master enable/disable toggle for GUI automation.
///
/// When `enabled = false`:
///   - GlobalSafetyHalt is engaged immediately
///   - Both child services are SIGKILLed
///   - Stale sockets are removed
///
/// When `enabled = true`:
///   - Services are re-spawned
///   - GlobalSafetyHalt remains engaged until both services pass health check
#[tauri::command]
pub async fn set_gui_automation_enabled(
    handle: AppHandle,
    enabled: bool,
) -> Result<GuiAutomationStatus, String> {
    let cell: tauri::State<'_, AppStateCell> = handle.state();
    let Some(state) = cell.get() else {
        return Err("Runtime not initialized".to_string());
    };

    let Some(orch) = state.gui_orchestrator.as_ref() else {
        // No orchestrator: enforce halt regardless of requested state
        kria_core::safety::engage_halt("orchestrator unavailable");
        return Err(
            "GUI orchestrator is not available (binaries missing or sudo not configured)"
                .to_string(),
        );
    };

    orch.set_automation_enabled(enabled)
        .await
        .map_err(|e| format!("set_automation_enabled failed: {e}"))?;

    Ok(orch.status().await.into())
}

/// P2g: Get the current environment grounding status for operational visibility.
///
/// Returns a lightweight snapshot of:
/// - cache freshness (generation, age, stale flag)
/// - system capabilities (xdotool, wmctrl, xrandr availability)
/// - focused window/app
/// - visible window and monitor counts
/// - terminal CWD and IDE project hints
///
/// This is strictly operational debugging data. No semantic reasoning,
/// no confidence scores, no ontology classification.
#[tauri::command]
pub async fn get_grounding_status(
    handle: AppHandle,
) -> Result<kria_core::agent::environment_grounder::GroundingStatus, String> {
    use kria_core::agent::environment_grounder::{GroundingCapabilities, GroundingStatus};

    let cell: tauri::State<'_, AppStateCell> = handle.state();
    let Some(_state) = cell.get() else {
        // Runtime not initialized — return degraded status
        return Ok(GroundingStatus {
            cache_generation: 0,
            cache_age_ms: 0,
            cache_stale: true,
            capabilities: GroundingCapabilities::none(),
            focused_app: None,
            focused_window_title: None,
            visible_window_count: 0,
            monitor_count: 0,
            terminal_cwd: None,
            open_project: None,
            process_count: 0,
        });
    };

    // The grounder is wired into GuiExecutionCoordinator which is created
    // per-session. For now, do a one-shot ground query for the status endpoint.
    // In the future this will read from the shared LiveEnvironmentGrounder cache.
    use kria_core::agent::environment_grounder::LiveEnvironmentGrounder;
    let grounder = LiveEnvironmentGrounder::new();
    Ok(grounder.grounding_status())
}

/// Whether the GUI Cognition "readiness safety gate" is currently BYPASSED.
///
/// The gate (in the core runtime) downgrades a live turn to `safety_only` when
/// per-turn preconditions (uinput / AT-SPI / focus / display) are not yet ready —
/// this is what produces "Workflow paused safely: execution_mode is safety_only"
/// and is why a first action prompt after launch does nothing. The gate is the
/// `KRIA_GUI_COG_RUNTIME_GUARDS` runtime-guard flag, read fresh each turn; an
/// explicit falsy value bypasses it (and the other runaway guards).
fn readiness_bypass_enabled() -> bool {
    match std::env::var("KRIA_GUI_COG_RUNTIME_GUARDS") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off" | ""
        ),
        Err(_) => false,
    }
}

/// Read the current readiness-gate bypass state for the Settings toggle.
#[tauri::command]
pub async fn get_gui_cognition_readiness_bypass() -> Result<bool, String> {
    Ok(readiness_bypass_enabled())
}

/// Enable/disable the GUI Cognition readiness-gate bypass (developer/test mode).
///
/// `enabled = true`  → set `KRIA_GUI_COG_RUNTIME_GUARDS=0`: GUI Cognition runs
///   live actions on the FIRST prompt without waiting for the readiness
///   preconditions (no `safety_only` downgrade). NOTE: this also relaxes the
///   per-turn runaway guards (cancel/watchdog/abort budget), so it is a test/dev
///   switch, not a production setting.
/// `enabled = false` → set `KRIA_GUI_COG_RUNTIME_GUARDS=1`: restore the default
///   safe behavior (readiness gate + runaway guards enforced).
///
/// The value is read fresh by the runtime at the start of every turn, so the
/// change takes effect on the NEXT prompt (no restart needed). It is process
/// scoped and resets to the safe default on app restart.
#[tauri::command]
pub async fn set_gui_cognition_readiness_bypass(enabled: bool) -> Result<bool, String> {
    if enabled {
        std::env::set_var("KRIA_GUI_COG_RUNTIME_GUARDS", "0");
    } else {
        std::env::set_var("KRIA_GUI_COG_RUNTIME_GUARDS", "1");
    }
    tracing::warn!(
        target: "gui_cognition_safety",
        bypass = enabled,
        "GUI Cognition readiness-gate bypass toggled from Settings"
    );
    Ok(readiness_bypass_enabled())
}

/// Cancel the active GUI Cognition turn for a session mid-flight. Cancellation
/// is cooperative — the workflow loop checks the cancel token before each
/// action, so this halts the turn *before its next action* without interrupting
/// an action already in progress.
///
/// This is a NEW command (it does not replace the existing `cancel_turn` /
/// `cancel_request` chat commands, which target the chat/agent loop). It reaches
/// the active turn through the process-local GUI Cognition cancel registry keyed
/// by `session_id`.
///
/// Returns `{ requested, found, reason }`:
///   - `found = false` means no active GUI Cognition turn was registered for the
///     session (nothing to cancel).
#[tauri::command]
pub async fn cancel_gui_cognition_turn(
    session_id: String,
    reason: Option<String>,
) -> Result<serde_json::Value, String> {
    let reason = reason
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            kria_core::agent::gui_cognition::cancel::DEFAULT_CANCEL_REASON.to_string()
        });
    let found = kria_core::agent::gui_cognition::cancel::gui_cancel_registry()
        .request_cancel(&session_id, &reason);
    tracing::info!(
        target: "gui_cognition_cancel",
        session_id = %session_id,
        found,
        "GUI Cognition turn cancellation requested from UI/API"
    );
    Ok(serde_json::json!({
        "requested": true,
        "found": found,
        "session_id": session_id,
        "reason": reason,
    }))
}
