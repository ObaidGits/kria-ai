//! GUI cognition service dependency model.
//!
//! This is a lightweight readiness gate for GUI-side dependencies. It does not
//! start services and it does not replace the ServiceOrchestrator; it simply
//! tells execution whether a particular GUI action has the local substrate it
//! needs. Structural actions intentionally have no GUI sidecar dependency.

use std::path::PathBuf;

use crate::infra::health::{HealthRegistry, ServiceStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GuiServiceDependency {
    UinputDaemon,
    AtSpiBus,
    VisionOrOcr,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiServiceReadiness {
    pub dependency: GuiServiceDependency,
    pub ready: bool,
    pub evidence: String,
}

impl GuiServiceReadiness {
    pub fn ok(dependency: GuiServiceDependency, evidence: impl Into<String>) -> Self {
        Self {
            dependency,
            ready: true,
            evidence: evidence.into(),
        }
    }

    pub fn missing(dependency: GuiServiceDependency, evidence: impl Into<String>) -> Self {
        Self {
            dependency,
            ready: false,
            evidence: evidence.into(),
        }
    }
}

pub fn dependencies_for_action(action: &str) -> &'static [GuiServiceDependency] {
    use GuiServiceDependency::*;
    match action {
        "type_text" | "click_mouse" | "press_shortcut" | "focus_window" => &[UinputDaemon],
        "click_element"
        | "click_ui_element"
        | "fill_form_field"
        | "detect_dialog"
        | "dismiss_dialog"
        | "get_desktop_state"
        | "find_ui_elements"
        | "check_app_responding"
        | "get_screen_elements"
        | "get_active_window" => &[AtSpiBus],
        "screenshot_analyze" | "ocr_image" | "analyze_image" => &[VisionOrOcr],
        _ => &[],
    }
}

pub fn action_is_structural(action: &str) -> bool {
    matches!(
        action,
        "write_file"
            | "execute_bash"
            | "open_application"
            | "open_application_with_file"
            | "browser_search"
            | "managed_browser_navigate"
            | "open_url"
            | "run_command"
    )
}

pub fn default_uinput_socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("KRIA_UINPUT_SOCKET") {
        return PathBuf::from(path);
    }
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime_dir).join("kria-uinput.sock");
    }
    if let Ok(cache_dir) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(cache_dir)
            .join("kria")
            .join("kria-uinput.sock");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".cache")
            .join("kria")
            .join("kria-uinput.sock");
    }
    PathBuf::from("/tmp/kria-uinput.sock")
}

pub fn check_action_readiness(action: &str) -> Result<Vec<GuiServiceReadiness>, String> {
    if action_is_structural(action) {
        return Ok(Vec::new());
    }

    let mut evidence = Vec::new();
    let mut missing = Vec::new();
    for dep in dependencies_for_action(action) {
        let ready = check_dependency(*dep);
        if ready.ready {
            evidence.push(ready);
        } else {
            missing.push(ready);
        }
    }

    if missing.is_empty() {
        Ok(evidence)
    } else {
        let joined = missing
            .iter()
            .map(|m| format!("{:?}: {}", m.dependency, m.evidence))
            .collect::<Vec<_>>()
            .join("; ");
        Err(format!(
            "GUI_SERVICE_NOT_READY for action '{}': {}",
            action, joined
        ))
    }
}

pub fn check_dependency(dep: GuiServiceDependency) -> GuiServiceReadiness {
    match dep {
        GuiServiceDependency::UinputDaemon => {
            let socket = default_uinput_socket_path();
            if socket.exists() {
                GuiServiceReadiness::ok(
                    dep,
                    format!("uinput socket exists at {}", socket.display()),
                )
            } else {
                GuiServiceReadiness::missing(
                    dep,
                    format!(
                        "uinput socket missing at {}; start kria-uinput-daemon or use structural substrate",
                        socket.display()
                    ),
                )
            }
        }
        GuiServiceDependency::AtSpiBus => {
            let uid = unsafe { libc::getuid() };
            let atspi_dir = PathBuf::from(format!("/run/user/{}/at-spi", uid));
            let legacy_socket = atspi_dir.join("bus");
            let bus_socket = if legacy_socket.exists() {
                Some(legacy_socket)
            } else {
                std::fs::read_dir(&atspi_dir).ok().and_then(|entries| {
                    entries.flatten().map(|entry| entry.path()).find(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .map(|name| name == "bus" || name.starts_with("bus_"))
                            .unwrap_or(false)
                    })
                })
            };
            if let Some(socket) = bus_socket {
                GuiServiceReadiness::ok(dep, format!("AT-SPI bus exists at {}", socket.display()))
            } else {
                GuiServiceReadiness::missing(
                    dep,
                    "AT-SPI bus unavailable; enable desktop accessibility before semantic GUI element actions",
                )
            }
        }
        GuiServiceDependency::VisionOrOcr => {
            if which::which("tesseract").is_ok() {
                GuiServiceReadiness::ok(dep, "tesseract CLI available")
            } else {
                GuiServiceReadiness::missing(
                    dep,
                    "no OCR/vision fallback detected; start vision sidecar or install tesseract",
                )
            }
        }
    }
}

pub fn all_dependency_readiness() -> Vec<GuiServiceReadiness> {
    [
        GuiServiceDependency::UinputDaemon,
        GuiServiceDependency::AtSpiBus,
        GuiServiceDependency::VisionOrOcr,
    ]
    .into_iter()
    .map(check_dependency)
    .collect()
}

pub fn refresh_gui_service_health(health: &HealthRegistry) {
    let services = [
        ("gui_uinput_daemon", GuiServiceDependency::UinputDaemon),
        ("gui_atspi_bus", GuiServiceDependency::AtSpiBus),
        ("gui_vision_ocr", GuiServiceDependency::VisionOrOcr),
    ];

    for (name, dep) in services {
        if health.get(name).is_none() {
            health.register(name);
        }
        let readiness = check_dependency(dep);
        let status = if readiness.ready {
            ServiceStatus::Healthy
        } else {
            ServiceStatus::Degraded
        };
        health.update(name, status, Some(readiness.evidence));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_actions_have_no_gui_sidecar_dependency() {
        assert!(dependencies_for_action("write_file").is_empty());
        assert!(dependencies_for_action("execute_bash").is_empty());
        assert!(action_is_structural("browser_search"));
    }

    #[test]
    fn input_actions_require_uinput() {
        assert_eq!(
            dependencies_for_action("type_text"),
            &[GuiServiceDependency::UinputDaemon]
        );
    }

    #[test]
    fn atspi_actions_require_atspi_bus() {
        assert_eq!(
            dependencies_for_action("click_element"),
            &[GuiServiceDependency::AtSpiBus]
        );
        assert_eq!(
            dependencies_for_action("click_ui_element"),
            &[GuiServiceDependency::AtSpiBus]
        );
    }

    #[test]
    fn refresh_gui_service_health_registers_all_gui_dependencies() {
        let health = HealthRegistry::new();
        refresh_gui_service_health(&health);
        assert!(health.get("gui_uinput_daemon").is_some());
        assert!(health.get("gui_atspi_bus").is_some());
        assert!(health.get("gui_vision_ocr").is_some());
    }
}
