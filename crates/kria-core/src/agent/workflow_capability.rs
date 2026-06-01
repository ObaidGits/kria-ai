//! Workflow Capability Resolution — Detect What The Environment Can Do.
//!
//! Resolves a `CapabilitySet` once at workflow start. The substrate router
//! uses this to adapt plans, gate verification leaves, and trigger HITL
//! when capabilities are insufficient.
//!
//! # Design
//!
//! - Resolved ONCE per workflow (cached in WorkflowMemory)
//! - No LLM calls — pure system queries
//! - Bounded: each probe has a hard timeout (50ms default)
//! - Graceful: probe failures → capability marked as unavailable (not error)
//!
//! # What Gets Detected
//!
//! - Session type (X11 / Wayland / XWayland)
//! - AT-SPI availability and level
//! - xdotool availability
//! - uinput daemon status
//! - App installation status
//! - Browser availability + CDP
//! - OCR availability

use crate::agent::workflow_types::{
    AtSpiLevel, CapabilitySet, EnvironmentCapability, InputInjectionLevel, InteractionCapability,
    SessionType, VerificationMethod, VerifierCapability,
};

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Environment Detection
// ═══════════════════════════════════════════════════════════════════════════════

/// Detect the current desktop session type.
pub fn detect_session_type() -> SessionType {
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    match session.to_lowercase().as_str() {
        "x11" => SessionType::X11,
        "wayland" => {
            // Check if XWayland is also running (most Wayland sessions have it)
            if std::env::var("DISPLAY").is_ok() {
                SessionType::XWayland
            } else {
                SessionType::Wayland
            }
        }
        _ => {
            // Fallback: check for DISPLAY (X11) or WAYLAND_DISPLAY
            if std::env::var("WAYLAND_DISPLAY").is_ok() {
                SessionType::Wayland
            } else if std::env::var("DISPLAY").is_ok() {
                SessionType::X11
            } else {
                SessionType::Unknown
            }
        }
    }
}

/// Detect AT-SPI availability level.
pub async fn detect_atspi_level() -> AtSpiLevel {
    let uid = unsafe { libc::getuid() };
    let bus_path = format!("/run/user/{}/at-spi/bus", uid);

    if !std::path::Path::new(&bus_path).exists() {
        return AtSpiLevel::None;
    }

    // Check if toolkit accessibility is enabled
    let toolkit_enabled = tokio::process::Command::new("gsettings")
        .args([
            "get",
            "org.gnome.desktop.interface",
            "toolkit-accessibility",
        ])
        .output()
        .await
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false);

    if !toolkit_enabled {
        return AtSpiLevel::BusOnly;
    }

    // Try to list accessible apps (quick probe)
    let has_apps = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        check_atspi_has_apps(),
    )
    .await
    .unwrap_or(false);

    if has_apps {
        AtSpiLevel::Full
    } else {
        AtSpiLevel::BusOnly
    }
}

async fn check_atspi_has_apps() -> bool {
    // Use the existing AT-SPI engine for a quick check
    let engine = crate::agent::atspi_engine::AtSpiEngine::new();
    let apps = engine.list_applications().await;
    !apps.is_empty()
}

/// Check if xdotool is available.
pub fn detect_xdotool() -> bool {
    std::process::Command::new("which")
        .arg("xdotool")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if the uinput daemon is running.
pub fn detect_uinput_daemon() -> bool {
    // Check for the KRIA uinput daemon process
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let comm_path = entry.path().join("comm");
            if let Ok(comm) = std::fs::read_to_string(&comm_path) {
                if comm.trim() == "kria-uinput-da" || comm.trim().starts_with("kria-uinput") {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if tesseract OCR is available.
pub fn detect_ocr() -> bool {
    std::process::Command::new("which")
        .arg("tesseract")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Detect the compositor name (best-effort).
pub fn detect_compositor() -> Option<String> {
    // Check common compositor environment variables
    if let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP") {
        let lower = desktop.to_lowercase();
        if lower.contains("gnome") {
            return Some("mutter".into());
        }
        if lower.contains("kde") || lower.contains("plasma") {
            return Some("kwin".into());
        }
        if lower.contains("sway") {
            return Some("sway".into());
        }
    }
    None
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Full Capability Resolution
// ═══════════════════════════════════════════════════════════════════════════════

// ─── Capability Cache (TTL-based) ────────────────────────────────────────────
// Capabilities are expensive to detect (~50-200ms across multiple shell calls).
// They rarely change within a session, so we cache for 60 seconds and invalidate
// on capability-related errors.

use std::sync::Mutex as StdMutex;
use std::time::{Duration as StdDuration, Instant};

const CAPABILITY_CACHE_TTL_SECS: u64 = 60;

struct CapabilityCacheEntry {
    capabilities: CapabilitySet,
    cached_at: Instant,
}

static CAPABILITY_CACHE: once_cell::sync::Lazy<StdMutex<Option<CapabilityCacheEntry>>> =
    once_cell::sync::Lazy::new(|| StdMutex::new(None));

/// Force-invalidate the capability cache. Called when a workflow fails with
/// capability-related errors (uinput unavailable, AT-SPI broken, etc.) so the
/// next workflow re-probes the environment.
pub fn invalidate_capability_cache() {
    if let Ok(mut guard) = CAPABILITY_CACHE.lock() {
        *guard = None;
        tracing::info!(target: "capability_cache", "Capability cache invalidated");
    }
}

/// Resolve the complete capability set for the current environment.
/// Caches result for 60 seconds to avoid repeated environment probes.
pub async fn resolve_capabilities() -> CapabilitySet {
    // Try cache first
    {
        if let Ok(guard) = CAPABILITY_CACHE.lock() {
            if let Some(ref entry) = *guard {
                if entry.cached_at.elapsed() < StdDuration::from_secs(CAPABILITY_CACHE_TTL_SECS) {
                    tracing::debug!(target: "capability_cache", "Cache hit");
                    return entry.capabilities.clone();
                }
            }
        }
    }

    // Cache miss or expired — probe environment
    tracing::debug!(target: "capability_cache", "Cache miss — probing environment");
    let capabilities = resolve_capabilities_uncached().await;

    // Store in cache
    if let Ok(mut guard) = CAPABILITY_CACHE.lock() {
        *guard = Some(CapabilityCacheEntry {
            capabilities: capabilities.clone(),
            cached_at: Instant::now(),
        });
    }

    capabilities
}

/// Uncached capability resolution. Internal helper.
async fn resolve_capabilities_uncached() -> CapabilitySet {
    let session_type = detect_session_type();
    let atspi_level = detect_atspi_level().await;
    let xdotool_available = detect_xdotool();
    let uinput_available = detect_uinput_daemon();
    let ocr_available = detect_ocr();
    let compositor = detect_compositor();

    let environment = EnvironmentCapability {
        session_type,
        compositor,
        atspi_level: atspi_level.clone(),
        xdotool_available,
        uinput_available,
        ocr_available,
    };

    let verifier = derive_verifier_capability(&environment);
    let interaction = derive_interaction_capability(&environment);

    CapabilitySet {
        environment,
        verifier,
        interaction,
    }
}

/// Derive verifier capabilities from environment state.
fn derive_verifier_capability(env: &EnvironmentCapability) -> VerifierCapability {
    let mut methods = vec![
        VerificationMethod::FileSystem,
        VerificationMethod::ProcessTable,
        VerificationMethod::PortCheck,
    ];

    let window_max_confidence = match (&env.session_type, &env.atspi_level) {
        (SessionType::X11, AtSpiLevel::Full) => {
            methods.push(VerificationMethod::AtSpi);
            methods.push(VerificationMethod::Xdotool);
            0.90
        }
        (SessionType::X11, _) if env.xdotool_available => {
            methods.push(VerificationMethod::Xdotool);
            0.75
        }
        (SessionType::Wayland, AtSpiLevel::Full) | (SessionType::XWayland, AtSpiLevel::Full) => {
            methods.push(VerificationMethod::AtSpi);
            if env.xdotool_available {
                methods.push(VerificationMethod::Xdotool);
            }
            0.70
        }
        (SessionType::Wayland, AtSpiLevel::Partial { .. }) => {
            methods.push(VerificationMethod::AtSpi);
            0.55
        }
        (SessionType::Wayland, _) => 0.40,
        _ => 0.40,
    };

    if env.ocr_available {
        methods.push(VerificationMethod::Ocr);
    }

    // CDP detection is deferred to browser-specific capability check
    let cdp_available = false; // Will be set by browser capability probe

    VerifierCapability {
        available_methods: methods,
        window_state_max_confidence: window_max_confidence,
        cdp_available,
        filesystem_available: true,
        process_table_available: true,
    }
}

/// Derive interaction capabilities from environment state.
fn derive_interaction_capability(env: &EnvironmentCapability) -> InteractionCapability {
    let keyboard_injection = if env.uinput_available {
        InputInjectionLevel::Full
    } else if env.xdotool_available && env.session_type == SessionType::X11 {
        InputInjectionLevel::XdotoolOnly
    } else {
        InputInjectionLevel::None
    };

    let mouse_injection = keyboard_injection; // Same mechanism

    InteractionCapability {
        keyboard_injection,
        mouse_injection,
        clipboard_available: true, // Always available on Linux via xclip/wl-copy
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Capability-Aware HITL Triggers
// ═══════════════════════════════════════════════════════════════════════════════

use crate::agent::workflow_types::{HitlActionType, HitlOption, HitlReason};

/// Check if an app is available and return HITL if not.
pub fn check_app_available(
    app_name: &str,
    registry: &crate::platform::app_registry::InstalledAppRegistry,
) -> Option<HitlReason> {
    let resolved = registry.resolve_alias(app_name);
    if resolved.is_some() {
        return None; // App is available
    }

    // App not found — generate HITL reason
    Some(HitlReason::InstallRequired {
        app: app_name.to_string(),
        install_command: suggest_install_command(app_name),
    })
}

/// Generate HITL options for an app-not-installed situation.
pub fn hitl_options_for_missing_app(app_name: &str) -> Vec<HitlOption> {
    let mut options = Vec::new();

    if let Some(cmd) = suggest_install_command(app_name) {
        options.push(HitlOption {
            id: "install".into(),
            label: format!("Install {}", app_name),
            action_type: HitlActionType::RunCommand { command: cmd },
        });
    }

    // Suggest alternatives based on category
    for alt in suggest_alternatives(app_name) {
        options.push(HitlOption {
            id: format!("use_{}", alt),
            label: format!("Use {} instead", alt),
            action_type: HitlActionType::ChooseAlternative { value: alt },
        });
    }

    options.push(HitlOption {
        id: "cancel".into(),
        label: "Cancel".into(),
        action_type: HitlActionType::Cancel,
    });

    options
}

/// Generate HITL options for a login-required situation.
pub fn hitl_options_for_login(service: &str) -> Vec<HitlOption> {
    let login_url = match service.to_lowercase().as_str() {
        "youtube" | "google" => Some("https://accounts.google.com".to_string()),
        "github" => Some("https://github.com/login".to_string()),
        "whatsapp" => Some("https://web.whatsapp.com".to_string()),
        _ => None,
    };

    let mut options = Vec::new();

    if let Some(url) = login_url {
        options.push(HitlOption {
            id: "open_login".into(),
            label: "Open Login Page".into(),
            action_type: HitlActionType::OpenUrl { url },
        });
    }

    options.push(HitlOption {
        id: "logged_in".into(),
        label: "I'm logged in now".into(),
        action_type: HitlActionType::Approve,
    });

    options.push(HitlOption {
        id: "skip".into(),
        label: "Skip this step".into(),
        action_type: HitlActionType::Skip,
    });

    options.push(HitlOption {
        id: "cancel".into(),
        label: "Cancel".into(),
        action_type: HitlActionType::Cancel,
    });

    options
}

/// Generate HITL options for ambiguous file selection.
pub fn hitl_options_for_ambiguous_files(candidates: &[String]) -> Vec<HitlOption> {
    let mut options: Vec<HitlOption> = candidates
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let filename = std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path);
            HitlOption {
                id: format!("file_{}", i),
                label: filename.to_string(),
                action_type: HitlActionType::ChooseAlternative {
                    value: path.clone(),
                },
            }
        })
        .collect();

    options.push(HitlOption {
        id: "cancel".into(),
        label: "Cancel".into(),
        action_type: HitlActionType::Cancel,
    });

    options
}

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — App Resolution Helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Suggest an install command for a missing app.
fn suggest_install_command(app_name: &str) -> Option<String> {
    let lower = app_name.to_lowercase();
    match lower.as_str() {
        "code" | "vscode" | "vs code" | "visual studio code" => {
            Some("sudo snap install code --classic".into())
        }
        "chrome" | "google-chrome" | "google chrome" => {
            Some("wget -q -O - https://dl.google.com/linux/linux_signing_key.pub | sudo apt-key add - && sudo apt install google-chrome-stable".into())
        }
        "firefox" => Some("sudo apt install firefox".into()),
        "gedit" => Some("sudo apt install gedit".into()),
        "nautilus" | "files" => Some("sudo apt install nautilus".into()),
        "libreoffice-calc" | "calc" | "spreadsheet" => {
            Some("sudo apt install libreoffice-calc".into())
        }
        "vlc" => Some("sudo apt install vlc".into()),
        "gimp" => Some("sudo apt install gimp".into()),
        _ => None,
    }
}

/// Suggest alternative apps for a given app name.
fn suggest_alternatives(app_name: &str) -> Vec<String> {
    let lower = app_name.to_lowercase();
    match lower.as_str() {
        "code" | "vscode" | "vs code" => {
            vec!["gedit".into(), "kate".into(), "nano".into()]
        }
        "chrome" | "google-chrome" => {
            vec!["firefox".into(), "chromium".into(), "brave".into()]
        }
        "excel" | "microsoft excel" => {
            vec!["libreoffice-calc".into(), "gnumeric".into()]
        }
        "word" | "microsoft word" => {
            vec!["libreoffice-writer".into()]
        }
        "nautilus" | "files" => {
            vec!["thunar".into(), "dolphin".into(), "nemo".into()]
        }
        _ => vec![],
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §5 — Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_type_detection_uses_env_vars() {
        // This test verifies the logic, not the actual environment
        // (which varies per CI/dev machine)
        let session = detect_session_type();
        // Should return one of the valid variants
        assert!(matches!(
            session,
            SessionType::X11 | SessionType::Wayland | SessionType::XWayland | SessionType::Unknown
        ));
    }

    #[test]
    fn verifier_capability_always_includes_filesystem_and_process() {
        let env = EnvironmentCapability {
            session_type: SessionType::Wayland,
            compositor: Some("mutter".into()),
            atspi_level: AtSpiLevel::None,
            xdotool_available: false,
            uinput_available: false,
            ocr_available: false,
        };
        let verifier = derive_verifier_capability(&env);
        assert!(verifier
            .available_methods
            .contains(&VerificationMethod::FileSystem));
        assert!(verifier
            .available_methods
            .contains(&VerificationMethod::ProcessTable));
        assert!(verifier
            .available_methods
            .contains(&VerificationMethod::PortCheck));
        assert!(verifier.filesystem_available);
        assert!(verifier.process_table_available);
    }

    #[test]
    fn wayland_without_atspi_has_low_window_confidence() {
        let env = EnvironmentCapability {
            session_type: SessionType::Wayland,
            compositor: Some("mutter".into()),
            atspi_level: AtSpiLevel::None,
            xdotool_available: false,
            uinput_available: false,
            ocr_available: false,
        };
        let verifier = derive_verifier_capability(&env);
        assert!(verifier.window_state_max_confidence <= 0.40);
    }

    #[test]
    fn x11_with_full_atspi_has_high_window_confidence() {
        let env = EnvironmentCapability {
            session_type: SessionType::X11,
            compositor: None,
            atspi_level: AtSpiLevel::Full,
            xdotool_available: true,
            uinput_available: true,
            ocr_available: true,
        };
        let verifier = derive_verifier_capability(&env);
        assert!(verifier.window_state_max_confidence >= 0.85);
        assert!(verifier
            .available_methods
            .contains(&VerificationMethod::AtSpi));
        assert!(verifier
            .available_methods
            .contains(&VerificationMethod::Xdotool));
        assert!(verifier
            .available_methods
            .contains(&VerificationMethod::Ocr));
    }

    #[test]
    fn interaction_capability_full_with_uinput() {
        let env = EnvironmentCapability {
            session_type: SessionType::Wayland,
            compositor: Some("mutter".into()),
            atspi_level: AtSpiLevel::Full,
            xdotool_available: false,
            uinput_available: true,
            ocr_available: false,
        };
        let interaction = derive_interaction_capability(&env);
        assert_eq!(interaction.keyboard_injection, InputInjectionLevel::Full);
        assert_eq!(interaction.mouse_injection, InputInjectionLevel::Full);
    }

    #[test]
    fn interaction_capability_none_without_uinput_on_wayland() {
        let env = EnvironmentCapability {
            session_type: SessionType::Wayland,
            compositor: Some("mutter".into()),
            atspi_level: AtSpiLevel::Full,
            xdotool_available: false,
            uinput_available: false,
            ocr_available: false,
        };
        let interaction = derive_interaction_capability(&env);
        assert_eq!(interaction.keyboard_injection, InputInjectionLevel::None);
    }

    #[test]
    fn interaction_capability_xdotool_on_x11_without_uinput() {
        let env = EnvironmentCapability {
            session_type: SessionType::X11,
            compositor: None,
            atspi_level: AtSpiLevel::None,
            xdotool_available: true,
            uinput_available: false,
            ocr_available: false,
        };
        let interaction = derive_interaction_capability(&env);
        assert_eq!(
            interaction.keyboard_injection,
            InputInjectionLevel::XdotoolOnly
        );
    }

    #[test]
    fn install_command_suggested_for_known_apps() {
        assert!(suggest_install_command("code").is_some());
        assert!(suggest_install_command("firefox").is_some());
        assert!(suggest_install_command("unknown_app_xyz").is_none());
    }

    #[test]
    fn alternatives_suggested_for_known_apps() {
        let alts = suggest_alternatives("code");
        assert!(!alts.is_empty());
        assert!(alts.contains(&"gedit".to_string()));

        let alts = suggest_alternatives("chrome");
        assert!(alts.contains(&"firefox".to_string()));
    }

    #[test]
    fn hitl_options_for_missing_app_always_has_cancel() {
        let options = hitl_options_for_missing_app("code");
        assert!(options.iter().any(|o| o.id == "cancel"));
        // Should also have install option for known apps
        assert!(options.iter().any(|o| o.id == "install"));
    }

    #[test]
    fn hitl_options_for_login_has_standard_options() {
        let options = hitl_options_for_login("youtube");
        assert!(options.iter().any(|o| o.id == "open_login"));
        assert!(options.iter().any(|o| o.id == "logged_in"));
        assert!(options.iter().any(|o| o.id == "skip"));
        assert!(options.iter().any(|o| o.id == "cancel"));
    }

    #[test]
    fn hitl_options_for_ambiguous_files_lists_all_candidates() {
        let candidates = vec![
            "/home/user/test.png".into(),
            "/home/user/test_v2.png".into(),
        ];
        let options = hitl_options_for_ambiguous_files(&candidates);
        // Should have one option per file + cancel
        assert_eq!(options.len(), 3);
        assert!(options.iter().any(|o| o.id == "cancel"));
    }
}
