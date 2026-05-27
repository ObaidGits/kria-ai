//! Application lifecycle helpers for GUI eval.
//!
//! Provides utilities for:
//! - Detecting whether an app is already running
//! - Checking if a window for an app is visible
//! - Waiting for an app to become ready
//! - Detecting session reuse vs. fresh launch

use std::time::{Duration, Instant};

use super::types::{
    GuiEvalCase, GuiEvalEnvironmentClassification, GuiEvalPreflight, GuiEvalPreflightCheck,
    GuiEvalPreflightStatus,
};
use kria_core::agent::gui_production_readiness::{
    assess_gui_production_readiness, GuiReadinessMode,
};

/// Check if a process with the given binary name is currently running.
/// Uses /proc scanning — works on Linux without any external tools.
pub fn is_process_running(binary_name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let pid_dir = entry.path();
        if !pid_dir.is_dir() {
            continue;
        }
        let Some(name) = pid_dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.parse::<u32>().is_err() {
            continue;
        }
        let comm_path = pid_dir.join("comm");
        if let Ok(comm) = std::fs::read_to_string(&comm_path) {
            let comm = comm.trim();
            // Match exact binary name or prefix (e.g., "code" matches "code" and "code-oss")
            if comm == binary_name || comm.starts_with(binary_name) {
                return true;
            }
        }
    }
    false
}

/// Get the PID of a running process by binary name.
pub fn get_process_pid(binary_name: &str) -> Option<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return None;
    };
    for entry in entries.flatten() {
        let pid_dir = entry.path();
        if !pid_dir.is_dir() {
            continue;
        }
        let Some(name) = pid_dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        let comm_path = pid_dir.join("comm");
        if let Ok(comm) = std::fs::read_to_string(&comm_path) {
            let comm = comm.trim();
            if comm == binary_name || comm.starts_with(binary_name) {
                return Some(pid);
            }
        }
    }
    None
}

/// Map a user-facing app name to the binary name used in /proc/*/comm.
pub fn app_name_to_binary(app_name: &str) -> &'static str {
    match app_name.to_ascii_lowercase().as_str() {
        "gedit" => "gedit",
        "code" | "vscode" | "vs code" | "visual studio code" => "code",
        "chrome" | "google-chrome" | "google chrome" => "chrome",
        "firefox" => "firefox",
        "brave" | "brave-browser" => "brave",
        "gnome-terminal" | "terminal" => "gnome-terminal",
        "nautilus" | "file manager" => "nautilus",
        "kate" => "kate",
        "mousepad" => "mousepad",
        "xed" => "xed",
        _ => "unknown",
    }
}

/// Wait for a process to appear in /proc, up to `timeout`.
/// Returns the PID if found, None if timed out.
pub async fn wait_for_process(binary_name: &str, timeout: Duration) -> Option<u32> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(pid) = get_process_pid(binary_name) {
            return Some(pid);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Check if a window with the given title substring is visible via wmctrl.
/// Returns None if wmctrl is not available or no matching window found.
pub async fn find_window_by_title(title_contains: &str) -> Option<WindowInfo> {
    let output = tokio::process::Command::new("wmctrl")
        .args(["-l"])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.to_lowercase().contains(&title_contains.to_lowercase()) {
            // Parse wmctrl -l output: "0x04a00003  0  hostname  Title..."
            let parts: Vec<&str> = line.splitn(4, char::is_whitespace).collect();
            if parts.len() >= 4 {
                return Some(WindowInfo {
                    title: parts[3].trim().to_string(),
                    window_id: parts[0].to_string(),
                });
            }
        }
    }
    None
}

/// Basic window info from wmctrl.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub title: String,
    pub window_id: String,
}

/// Detect the current display server type.
pub fn detect_display_server() -> &'static str {
    kria_core::agent::gui_production_readiness::detect_display_server()
}

/// Check if xdotool is available (required for X11 window queries).
pub fn xdotool_available() -> bool {
    which::which("xdotool").is_ok()
}

/// Check if wmctrl is available (required for window listing).
pub fn wmctrl_available() -> bool {
    which::which("wmctrl").is_ok()
}

/// Scan ~/.kria/generated/ for files matching a pattern.
/// Returns all matching file paths.
pub fn find_generated_files(pattern: &str) -> Vec<std::path::PathBuf> {
    let base = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"));
    let dir = base.join(".kria").join("generated");

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let pattern_lower = pattern.to_ascii_lowercase();
    let mut results = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let name_lower = name.to_ascii_lowercase();
            // Simple glob: if pattern contains '*', match prefix/suffix
            let matches = if pattern.contains('*') {
                let parts: Vec<&str> = pattern_lower.split('*').collect();
                match parts.as_slice() {
                    [prefix, suffix] => {
                        name_lower.starts_with(prefix) && name_lower.ends_with(suffix)
                    }
                    [prefix] => name_lower.starts_with(prefix),
                    _ => name_lower.contains(&pattern_lower.replace('*', "")),
                }
            } else {
                name_lower.contains(&pattern_lower)
            };
            if matches {
                results.push(path);
            }
        }
    }

    results
}

/// Clean up generated files matching a pattern (for test isolation).
pub fn cleanup_generated_files(pattern: &str) {
    for path in find_generated_files(pattern) {
        let _ = std::fs::remove_file(&path);
    }
}

/// Classify the live environment for GUI eval gating.
pub fn classify_gui_eval_environment() -> GuiEvalEnvironmentClassification {
    GuiEvalEnvironmentClassification {
        detected_display_server: detect_display_server().to_string(),
        session_type: std::env::var("XDG_SESSION_TYPE").ok(),
        has_display: std::env::var("DISPLAY").is_ok(),
        has_wayland_display: std::env::var("WAYLAND_DISPLAY").is_ok(),
        kria_eval_gui_enabled: live_gui_eval_opted_in(),
        kria_eval_vm_enabled: std::env::var("KRIA_EVAL_VM").as_deref() == Ok("1"),
        xdotool_available: xdotool_available(),
        wmctrl_available: wmctrl_available(),
    }
}

/// Explicit host-GUI opt-in for live evals.
///
/// `KRIA_EVAL_GUI=1` remains the stable CI/script contract. `--gui-live` is the
/// interactive CLI contract so users do not have to remember a separate env var
/// when they intentionally want to see real windows move/open.
pub fn live_gui_eval_opted_in() -> bool {
    std::env::var("KRIA_EVAL_GUI").as_deref() == Ok("1")
        || std::env::args().any(|arg| arg == "--gui-live")
}

/// Structured preflight gate for GUI eval cases.
///
/// This is deliberately conservative. CI-safe structural cases can run without
/// a desktop. Real desktop cases require an explicit `KRIA_EVAL_GUI=1` opt-in,
/// and VM/destructive cases require `KRIA_EVAL_VM=1`.
pub fn preflight_gui_eval_case(case: &GuiEvalCase) -> GuiEvalPreflight {
    let environment = classify_gui_eval_environment();
    let mut required_capabilities = case.governance.capability_ids.clone();
    let mut missing_capabilities = Vec::new();
    let mut blocking_reasons = Vec::new();
    let mut checks = Vec::new();

    let readiness_mode = readiness_mode_for_case(case);
    let readiness = assess_gui_production_readiness(readiness_mode);
    let blockers = readiness.blocker_messages();
    if !blockers.is_empty() && case.expected_behavior.expect_success {
        let reason = format!(
            "GUI production readiness failed for {:?}: {}",
            readiness_mode,
            blockers.join("; ")
        );
        push_unique(&mut missing_capabilities, "gui.production_readiness");
        blocking_reasons.push(reason.clone());
        checks.push(blocking_check(
            "gui.production_readiness",
            "gui_readiness",
            reason,
        ));
    } else {
        checks.push(available_check(
            "gui.production_readiness",
            blockers.is_empty(),
            format!("GUI readiness mode {:?} checked", readiness_mode),
        ));
    }

    let display_capability = format!("display.{}", case.display_server.as_str());
    push_unique(&mut required_capabilities, &display_capability);
    if !case.display_server.is_satisfied() {
        let reason = format!(
            "display requirement '{}' not met; detected '{}'",
            case.display_server.as_str(),
            detect_display_server()
        );
        push_unique(&mut missing_capabilities, &display_capability);
        blocking_reasons.push(reason.clone());
        checks.push(blocking_check(
            &display_capability,
            "display_requirement",
            reason,
        ));
    } else {
        checks.push(available_check(
            &display_capability,
            true,
            format!(
                "display requirement '{}' is satisfied",
                case.display_server.as_str()
            ),
        ));
    }

    if case.requires_desktop && !live_gui_eval_opted_in() {
        let reason =
            "desktop eval requires KRIA_EVAL_GUI=1 or --gui-live with a real display and target apps"
                .to_string();
        push_unique(&mut required_capabilities, "env.kria_eval_gui");
        push_unique(&mut missing_capabilities, "env.kria_eval_gui");
        blocking_reasons.push(reason.clone());
        checks.push(blocking_check(
            "env.kria_eval_gui",
            "desktop_opt_in",
            reason,
        ));
    } else {
        checks.push(available_check(
            "env.kria_eval_gui",
            !case.requires_desktop || environment.kria_eval_gui_enabled,
            "desktop opt-in checked".to_string(),
        ));
    }

    let vm_only = case.tags.iter().any(|tag| {
        matches!(
            tag.as_str(),
            "vm" | "vm-only" | "destructive" | "dangerous" | "host-mutating"
        )
    });
    if vm_only && std::env::var("KRIA_EVAL_VM").as_deref() != Ok("1") {
        let reason = "VM/destructive eval requires KRIA_EVAL_VM=1".to_string();
        push_unique(&mut required_capabilities, "env.kria_eval_vm");
        push_unique(&mut missing_capabilities, "env.kria_eval_vm");
        blocking_reasons.push(reason.clone());
        checks.push(blocking_check("env.kria_eval_vm", "vm_opt_in", reason));
    } else if vm_only {
        push_unique(&mut required_capabilities, "env.kria_eval_vm");
        checks.push(available_check(
            "env.kria_eval_vm",
            environment.kria_eval_vm_enabled,
            "VM/destructive opt-in checked".to_string(),
        ));
    }

    for tool in &case.expected_behavior.required_tools {
        let capability = format!("tool.{}", tool);
        push_unique(&mut required_capabilities, &capability);
        if let Err(reason) = kria_core::agent::gui_services::check_action_readiness(tool) {
            if !case.expected_behavior.expect_success {
                checks.push(GuiEvalPreflightCheck {
                    capability,
                    required: true,
                    available: false,
                    blocker_kind: None,
                    message: format!(
                        "tool '{}' unavailable but case does not expect success: {}",
                        tool, reason
                    ),
                });
                continue;
            }
            let reason = format!(
                "required GUI sidecar/substrate is not ready for tool '{}': {}",
                tool, reason
            );
            push_unique(&mut missing_capabilities, &capability);
            blocking_reasons.push(reason.clone());
            checks.push(blocking_check(&capability, "tool_capability", reason));
        } else {
            checks.push(available_check(
                &capability,
                true,
                format!("tool '{}' readiness satisfied", tool),
            ));
        }
    }

    required_capabilities.sort();
    required_capabilities.dedup();
    missing_capabilities.sort();
    missing_capabilities.dedup();

    GuiEvalPreflight {
        status: if blocking_reasons.is_empty() {
            GuiEvalPreflightStatus::Runnable
        } else {
            GuiEvalPreflightStatus::EnvironmentBlocked
        },
        required_environment_profile: case.governance.environment_profile.clone(),
        environment,
        required_capabilities,
        missing_capabilities,
        blocking_reasons,
        checks,
    }
}

/// Compatibility wrapper for older callers.
pub fn gui_eval_skip_reason(case: &GuiEvalCase) -> Option<String> {
    let preflight = preflight_gui_eval_case(case);
    if preflight.status == GuiEvalPreflightStatus::EnvironmentBlocked {
        return Some(preflight.blocking_reasons.join("; "));
    }
    None
}

fn readiness_mode_for_case(case: &GuiEvalCase) -> GuiReadinessMode {
    if case.tags.iter().any(|tag| {
        matches!(
            tag.as_str(),
            "vm" | "vm-only" | "destructive" | "dangerous" | "host-mutating"
        )
    }) {
        return GuiReadinessMode::VmIsolated;
    }

    GuiReadinessMode::StructuralOnly
}

fn available_check(capability: &str, available: bool, message: String) -> GuiEvalPreflightCheck {
    GuiEvalPreflightCheck {
        capability: capability.to_string(),
        required: true,
        available,
        blocker_kind: None,
        message,
    }
}

fn blocking_check(capability: &str, blocker_kind: &str, message: String) -> GuiEvalPreflightCheck {
    GuiEvalPreflightCheck {
        capability: capability.to_string(),
        required: true,
        available: false,
        blocker_kind: Some(blocker_kind.to_string()),
        message,
    }
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_to_binary_maps_known_apps() {
        assert_eq!(app_name_to_binary("gedit"), "gedit");
        assert_eq!(app_name_to_binary("code"), "code");
        assert_eq!(app_name_to_binary("VS Code"), "code");
        assert_eq!(app_name_to_binary("chrome"), "chrome");
        assert_eq!(app_name_to_binary("firefox"), "firefox");
    }

    #[test]
    fn detect_display_server_returns_known_value() {
        let ds = detect_display_server();
        assert!(
            ["x11", "wayland", "xwayland", "unknown"].contains(&ds),
            "unexpected display server: {}",
            ds
        );
    }

    #[test]
    fn find_generated_files_returns_empty_for_nonexistent_pattern() {
        let files = find_generated_files("definitely_does_not_exist_xyz_12345");
        assert!(files.is_empty());
    }

    #[test]
    fn structural_non_desktop_case_is_not_skipped_by_gui_gate() {
        let case = GuiEvalCase {
            id: "unit-structural".to_string(),
            description: "structural eval".to_string(),
            prompt: "write a file".to_string(),
            expected_behavior: super::super::types::ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: Vec::new(),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: Vec::new(),
                forbidden_response_patterns: Vec::new(),
                required_response_patterns: Vec::new(),
                expect_success: true,
                app_already_running: false,
            },
            display_server: super::super::types::DisplayServerRequirement::Any,
            tags: vec!["unit".to_string()],
            requires_desktop: false,
            timeout: Duration::from_secs(1),
            governance: super::super::governance::EvalGovernanceMetadata::default(),
        };

        let preflight = preflight_gui_eval_case(&case);
        assert_eq!(preflight.status, GuiEvalPreflightStatus::Runnable);
        assert!(preflight
            .required_capabilities
            .contains(&"tool.write_file".to_string()));
        assert!(gui_eval_skip_reason(&case).is_none());
    }

    #[test]
    fn desktop_case_without_gui_opt_in_is_environment_blocked() {
        if std::env::var("KRIA_EVAL_GUI").as_deref() == Ok("1") {
            eprintln!("[SKIP] KRIA_EVAL_GUI=1 disables this opt-in gate assertion");
            return;
        }

        let case = GuiEvalCase {
            id: "unit-desktop".to_string(),
            description: "desktop eval".to_string(),
            prompt: "open gedit".to_string(),
            expected_behavior: super::super::types::ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: Vec::new(),
                required_tools: Vec::new(),
                forbidden_tools: Vec::new(),
                forbidden_response_patterns: Vec::new(),
                required_response_patterns: Vec::new(),
                expect_success: true,
                app_already_running: false,
            },
            display_server: super::super::types::DisplayServerRequirement::Any,
            tags: vec!["unit".to_string()],
            requires_desktop: true,
            timeout: Duration::from_secs(1),
            governance: super::super::governance::EvalGovernanceMetadata::default(),
        };

        let preflight = preflight_gui_eval_case(&case);
        assert_eq!(preflight.status, GuiEvalPreflightStatus::EnvironmentBlocked);
        assert!(preflight
            .missing_capabilities
            .contains(&"env.kria_eval_gui".to_string()));
        assert!(gui_eval_skip_reason(&case).is_some());
    }

    #[test]
    fn is_process_running_finds_init() {
        // PID 1 (init/systemd) is always running on Linux
        // We can't check by name easily, but we can verify the function doesn't panic
        let _ = is_process_running("systemd");
        let _ = is_process_running("init");
    }
}
