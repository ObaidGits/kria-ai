//! RFC 008: Tauri commands for the GUI Automation master toggle + status.
//!
//! Frontend bindings (see UI):
//!   - `get_gui_automation_status` → returns `{ vision, uinput, enabled, ... }`
//!   - `set_gui_automation_enabled(enabled: bool)` → master kill switch

use super::{AppStateCell};
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
    /// Human-readable reason the halt is currently engaged (if any).
    pub halt_reason: Option<String>,
    /// PID of vision sidecar process, if running.
    pub vision_pid: Option<u32>,
    /// PID of uinput daemon process, if running.
    pub uinput_pid: Option<u32>,
    /// `true` if the ServiceOrchestrator initialized successfully at boot.
    pub orchestrator_available: bool,
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
        Self {
            vision_sidecar: label(s.vision_sidecar),
            uinput_daemon: label(s.uinput_daemon),
            automation_enabled: s.automation_enabled,
            global_halt_engaged: kria_core::safety::is_halted(),
            halt_reason: kria_core::safety::halt_reason(),
            vision_pid: s.vision_pid,
            uinput_pid: s.uinput_pid,
            orchestrator_available: true,
        }
    }
}

/// Get the current automation/services status for the UI.
#[tauri::command]
pub async fn get_gui_automation_status(handle: AppHandle) -> Result<GuiAutomationStatus, String> {
    let cell: tauri::State<'_, AppStateCell> = handle.state();
    let Some(state) = cell.get() else {
        return Ok(GuiAutomationStatus {
            vision_sidecar: "stopped".to_string(),
            uinput_daemon: "stopped".to_string(),
            automation_enabled: false,
            global_halt_engaged: kria_core::safety::is_halted(),
            halt_reason: kria_core::safety::halt_reason(),
            vision_pid: None,
            uinput_pid: None,
            orchestrator_available: false,
        });
    };

    match state.gui_orchestrator.as_ref() {
        Some(orch) => Ok(orch.status().await.into()),
        None => Ok(GuiAutomationStatus {
            vision_sidecar: "stopped".to_string(),
            uinput_daemon: "stopped".to_string(),
            automation_enabled: false,
            global_halt_engaged: kria_core::safety::is_halted(),
            halt_reason: kria_core::safety::halt_reason(),
            vision_pid: None,
            uinput_pid: None,
            orchestrator_available: false,
        }),
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
