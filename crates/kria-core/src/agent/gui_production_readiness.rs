//! Production readiness contract for GUI cognition.
//!
//! This module is intentionally deterministic and read-only. It does not start
//! daemons, open apps, or probe with synthetic input. It answers one bounded
//! question: "is this runtime environment ready for the class of GUI cognition
//! the caller wants to run?"

use serde::{Deserialize, Serialize};

use crate::agent::gui_services::{self, GuiServiceDependency, GuiServiceReadiness};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiReadinessMode {
    /// Structural workflows only: file/shell/CDP/API/LSP paths. No GUI input.
    StructuralOnly,
    /// Real desktop workflow with app windows and semantic GUI observation.
    LiveDesktop,
    /// Keyboard/mouse/AT-SPI/OCR-heavy workflow. Highest local readiness bar.
    InteractionHeavy,
    /// VM-backed destructive or host-mutating evaluation.
    VmIsolated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiReadinessSeverity {
    Info,
    Warning,
    Blocker,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiReadinessIssue {
    pub severity: GuiReadinessSeverity,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiProductionReadinessReport {
    pub mode: GuiReadinessMode,
    pub production_ready: bool,
    pub display_server: String,
    pub dependencies: Vec<GuiServiceReadiness>,
    pub issues: Vec<GuiReadinessIssue>,
}

impl GuiProductionReadinessReport {
    pub fn blocker_messages(&self) -> Vec<String> {
        self.issues
            .iter()
            .filter(|i| i.severity == GuiReadinessSeverity::Blocker)
            .map(|i| format!("{}: {}", i.code, i.message))
            .collect()
    }

    pub fn as_short_status(&self) -> String {
        if self.production_ready {
            format!("{:?} ready on {}", self.mode, self.display_server)
        } else {
            format!(
                "{:?} not ready: {}",
                self.mode,
                self.blocker_messages().join("; ")
            )
        }
    }
}

pub fn assess_gui_production_readiness(mode: GuiReadinessMode) -> GuiProductionReadinessReport {
    let display_server = detect_display_server().to_string();
    let dependencies = gui_services::all_dependency_readiness();
    let mut issues = Vec::new();

    if display_server == "unknown" && mode != GuiReadinessMode::StructuralOnly {
        issues.push(GuiReadinessIssue {
            severity: GuiReadinessSeverity::Blocker,
            code: "NO_DISPLAY_SERVER".to_string(),
            message: "No X11/Wayland display server detected for live GUI cognition".to_string(),
        });
    }

    if matches!(
        mode,
        GuiReadinessMode::LiveDesktop | GuiReadinessMode::InteractionHeavy
    ) && std::env::var("KRIA_EVAL_GUI").as_deref() != Ok("1")
    {
        issues.push(GuiReadinessIssue {
            severity: GuiReadinessSeverity::Warning,
            code: "LIVE_GUI_NOT_OPTED_IN".to_string(),
            message: "Live GUI evals require KRIA_EVAL_GUI=1 to avoid accidental host automation"
                .to_string(),
        });
    }

    if mode == GuiReadinessMode::VmIsolated && std::env::var("KRIA_EVAL_VM").as_deref() != Ok("1") {
        issues.push(GuiReadinessIssue {
            severity: GuiReadinessSeverity::Blocker,
            code: "VM_EVAL_NOT_OPTED_IN".to_string(),
            message: "VM/destructive GUI evals require KRIA_EVAL_VM=1".to_string(),
        });
    }

    require_dependency(
        &dependencies,
        &mut issues,
        GuiServiceDependency::AtSpiBus,
        matches!(
            mode,
            GuiReadinessMode::LiveDesktop | GuiReadinessMode::InteractionHeavy
        ),
    );
    require_dependency(
        &dependencies,
        &mut issues,
        GuiServiceDependency::UinputDaemon,
        mode == GuiReadinessMode::InteractionHeavy,
    );
    require_dependency(
        &dependencies,
        &mut issues,
        GuiServiceDependency::VisionOrOcr,
        mode == GuiReadinessMode::InteractionHeavy,
    );

    if display_server == "wayland" && mode == GuiReadinessMode::InteractionHeavy {
        issues.push(GuiReadinessIssue {
            severity: GuiReadinessSeverity::Warning,
            code: "WAYLAND_INTERACTION_DEGRADED".to_string(),
            message:
                "Wayland interaction-heavy automation is compositor-dependent; prefer CDP/LSP/filesystem paths"
                    .to_string(),
        });
    }

    let production_ready = !issues
        .iter()
        .any(|i| i.severity == GuiReadinessSeverity::Blocker);

    GuiProductionReadinessReport {
        mode,
        production_ready,
        display_server,
        dependencies,
        issues,
    }
}

fn require_dependency(
    dependencies: &[GuiServiceReadiness],
    issues: &mut Vec<GuiReadinessIssue>,
    dependency: GuiServiceDependency,
    required: bool,
) {
    if !required {
        return;
    }
    let Some(readiness) = dependencies.iter().find(|d| d.dependency == dependency) else {
        issues.push(GuiReadinessIssue {
            severity: GuiReadinessSeverity::Blocker,
            code: format!("{:?}_MISSING_PROBE", dependency).to_ascii_uppercase(),
            message: "Readiness probe did not run".to_string(),
        });
        return;
    };
    if !readiness.ready {
        issues.push(GuiReadinessIssue {
            severity: GuiReadinessSeverity::Blocker,
            code: format!("{:?}_NOT_READY", dependency).to_ascii_uppercase(),
            message: readiness.evidence.clone(),
        });
    }
}

pub fn detect_display_server() -> &'static str {
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let has_display = std::env::var("DISPLAY").is_ok();
    let has_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();

    match session.as_str() {
        "x11" => "x11",
        "wayland" if has_display => "xwayland",
        "wayland" => "wayland",
        _ if has_display && has_wayland => "xwayland",
        _ if has_display => "x11",
        _ if has_wayland => "wayland",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_only_does_not_require_gui_sidecars() {
        let report = assess_gui_production_readiness(GuiReadinessMode::StructuralOnly);
        assert_eq!(report.mode, GuiReadinessMode::StructuralOnly);
        assert!(!report
            .issues
            .iter()
            .any(|i| i.code.contains("UINPUT") || i.code.contains("ATSPI")));
    }

    #[test]
    fn vm_mode_requires_explicit_vm_gate() {
        let report = assess_gui_production_readiness(GuiReadinessMode::VmIsolated);
        if std::env::var("KRIA_EVAL_VM").as_deref() != Ok("1") {
            assert!(report
                .issues
                .iter()
                .any(|i| i.code == "VM_EVAL_NOT_OPTED_IN"));
        }
    }
}
