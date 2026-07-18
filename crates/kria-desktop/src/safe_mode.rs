//! Linux graphics safe-mode boot (kria-ui-redesign spec task 0.6).
//!
//! KRIA renders through **WebKitGTK** on Linux. On several GPU/driver combos —
//! notably **NVIDIA under Wayland** — WebKitGTK can paint a BLANK window or
//! crash unless the accelerated-compositing / DMABUF paths are disabled via env
//! flags (design.md §11.2 / §11.4 risk "Blank screen / crash (Wayland+NVIDIA)").
//!
//! This module:
//!   1. detects a likely-problematic environment (Wayland + NVIDIA),
//!   2. resolves whether to boot in **safe mode** (explicit `--safe-mode` CLI
//!      flag / `KRIA_SAFE_MODE=1`, or as an automatic recovery relaunch),
//!   3. decides which WebKit env flags to set (never clobbering an explicit
//!      user-provided value),
//!   4. offers a graceful relaunch-in-safe-mode path so a first-boot blank
//!      screen / build failure is recoverable instead of a dead white window.
//!
//! The decision logic is a pure function over a captured [`GraphicsEnv`] so it
//! is unit-testable without touching real process env.

use std::ffi::OsString;

/// Snapshot of the graphics-relevant environment, captured once at startup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphicsEnv {
    /// `XDG_SESSION_TYPE` (e.g. "wayland" / "x11"), if set.
    pub session_type: Option<String>,
    /// Whether `WAYLAND_DISPLAY` is set (strong Wayland signal).
    pub wayland_display: bool,
    /// Whether an NVIDIA GPU is present/selected on this session.
    pub has_nvidia: bool,
    /// Existing explicit `WEBKIT_DISABLE_DMABUF_RENDERER` value, if the user set one.
    pub explicit_dmabuf: Option<String>,
    /// Existing explicit `WEBKIT_DISABLE_COMPOSITING_MODE` value, if the user set one.
    pub explicit_compositing: Option<String>,
    /// True when safe mode was explicitly requested (`--safe-mode` / `KRIA_SAFE_MODE`).
    pub safe_mode_requested: bool,
}

/// Resolved boot decision: which env flags to set and why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SafeModeDecision {
    /// True when booting in the fuller safe mode (accelerated compositing off).
    pub safe_mode: bool,
    /// True when the environment looks blank-screen/crash-prone (Wayland+NVIDIA).
    pub problematic: bool,
    /// Set `WEBKIT_DISABLE_DMABUF_RENDERER=1` (reliable render path).
    pub set_disable_dmabuf: bool,
    /// Set `WEBKIT_DISABLE_COMPOSITING_MODE=1` (safe mode only; disables accel).
    pub set_disable_compositing: bool,
    /// Human-readable explanation for logs.
    pub reason: String,
}

/// Is this a Wayland session? True if `WAYLAND_DISPLAY` is set or
/// `XDG_SESSION_TYPE` reports wayland.
pub fn is_wayland(env: &GraphicsEnv) -> bool {
    if env.wayland_display {
        return true;
    }
    matches!(env.session_type.as_deref(), Some(s) if s.eq_ignore_ascii_case("wayland"))
}

/// Pure boot decision. Assumes a Linux target (caller gates by cfg).
///
/// Rules:
/// - The DMABUF renderer is disabled by default on Linux (proven blank-window
///   fix) unless the user set an explicit `WEBKIT_DISABLE_DMABUF_RENDERER`.
/// - Full safe mode (explicit request or recovery relaunch) additionally
///   disables accelerated compositing, unless the user set an explicit value.
/// - Wayland+NVIDIA is flagged `problematic` so we can log guidance even when
///   not in full safe mode.
pub fn decide(env: &GraphicsEnv) -> SafeModeDecision {
    let problematic = is_wayland(env) && env.has_nvidia;
    let safe_mode = env.safe_mode_requested;

    let set_disable_dmabuf = env.explicit_dmabuf.is_none();
    let set_disable_compositing = safe_mode && env.explicit_compositing.is_none();

    let reason = if safe_mode {
        "safe mode requested: disabling WebKitGTK DMABUF renderer + accelerated compositing"
            .to_string()
    } else if problematic {
        "Wayland + NVIDIA detected: disabling WebKitGTK DMABUF renderer (accelerated compositing left on)".to_string()
    } else {
        "Linux baseline: disabling WebKitGTK DMABUF renderer for reliable rendering".to_string()
    };

    SafeModeDecision {
        safe_mode,
        problematic,
        set_disable_dmabuf,
        set_disable_compositing,
        reason,
    }
}

/// Detect NVIDIA presence without extra dependencies: device nodes or the
/// GLX/PRIME vendor env hints commonly set on hybrid-graphics laptops.
#[cfg(target_os = "linux")]
fn detect_nvidia() -> bool {
    if std::path::Path::new("/dev/nvidia0").exists()
        || std::path::Path::new("/dev/nvidiactl").exists()
        || std::path::Path::new("/proc/driver/nvidia").exists()
    {
        return true;
    }
    if let Ok(v) = std::env::var("__GLX_VENDOR_LIBRARY_NAME") {
        if v.eq_ignore_ascii_case("nvidia") {
            return true;
        }
    }
    std::env::var_os("__NV_PRIME_RENDER_OFFLOAD").is_some()
}

/// True when the user asked for safe mode via CLI flag or env var.
pub fn safe_mode_requested() -> bool {
    if let Ok(v) = std::env::var("KRIA_SAFE_MODE") {
        if v == "1" || v.eq_ignore_ascii_case("true") {
            return true;
        }
    }
    std::env::args().any(|a| a == "--safe-mode")
}

/// Capture the current graphics environment from real process env (Linux).
#[cfg(target_os = "linux")]
pub fn detect_graphics_env() -> GraphicsEnv {
    GraphicsEnv {
        session_type: std::env::var("XDG_SESSION_TYPE").ok(),
        wayland_display: std::env::var_os("WAYLAND_DISPLAY").is_some(),
        has_nvidia: detect_nvidia(),
        explicit_dmabuf: std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").ok(),
        explicit_compositing: std::env::var("WEBKIT_DISABLE_COMPOSITING_MODE").ok(),
        safe_mode_requested: safe_mode_requested(),
    }
}

/// Apply the decided env flags to the current process (before the webview
/// initializes). Only sets a flag when the decision says so (never clobbers an
/// explicit user value). No-op fields are skipped.
#[cfg(target_os = "linux")]
pub fn apply(decision: &SafeModeDecision) {
    if decision.set_disable_dmabuf {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    if decision.set_disable_compositing {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }
}

/// Establish the Linux rendering baseline at the very start of `main`. Returns
/// the captured env so the caller can decide recovery behavior. No-op on
/// non-Linux platforms.
#[cfg(target_os = "linux")]
pub fn establish_baseline() -> GraphicsEnv {
    let env = detect_graphics_env();
    let decision = decide(&env);
    apply(&decision);

    if decision.safe_mode {
        eprintln!("[KRIA] SAFE MODE: {}", decision.reason);
    } else if decision.problematic {
        eprintln!(
            "[KRIA] {} — if the window is blank or crashes, relaunch with --safe-mode (see docs/LINUX_GRAPHICS.md).",
            decision.reason
        );
    } else {
        tracing::debug!("graphics baseline: {}", decision.reason);
    }
    env
}

/// Relaunch this executable in safe mode (`KRIA_SAFE_MODE=1`) and exit the
/// current process. Used as a graceful recovery when the first (accelerated)
/// boot fails to build/show the webview, turning a blank-screen/crash into a
/// self-healing restart. Returns `false` (without exiting) if the relaunch
/// could not be spawned, so the caller can fall through to a hard error.
#[cfg(target_os = "linux")]
pub fn relaunch_in_safe_mode() -> bool {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[KRIA] cannot locate current exe for safe-mode relaunch: {e}");
            return false;
        }
    };
    // Forward original args except a duplicate --safe-mode (env var carries it).
    let args: Vec<OsString> = std::env::args_os()
        .skip(1)
        .filter(|a| a != "--safe-mode")
        .collect();
    eprintln!("[KRIA] first boot failed — relaunching in safe mode (KRIA_SAFE_MODE=1)…");
    match std::process::Command::new(exe)
        .args(&args)
        .env("KRIA_SAFE_MODE", "1")
        .spawn()
    {
        Ok(_) => {
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("[KRIA] safe-mode relaunch failed to spawn: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(overrides: impl FnOnce(&mut GraphicsEnv)) -> GraphicsEnv {
        let mut e = GraphicsEnv::default();
        overrides(&mut e);
        e
    }

    #[test]
    fn is_wayland_via_display_or_session_type() {
        assert!(is_wayland(&env(|e| e.wayland_display = true)));
        assert!(is_wayland(
            &env(|e| e.session_type = Some("wayland".into()))
        ));
        assert!(is_wayland(
            &env(|e| e.session_type = Some("Wayland".into()))
        ));
        assert!(!is_wayland(&env(|e| e.session_type = Some("x11".into()))));
        assert!(!is_wayland(&env(|_| {})));
    }

    #[test]
    fn baseline_disables_dmabuf_but_not_compositing() {
        let d = decide(&env(|_| {}));
        assert!(d.set_disable_dmabuf);
        assert!(!d.set_disable_compositing);
        assert!(!d.safe_mode);
        assert!(!d.problematic);
    }

    #[test]
    fn wayland_nvidia_is_flagged_problematic_without_full_safe_mode() {
        let d = decide(&env(|e| {
            e.wayland_display = true;
            e.has_nvidia = true;
        }));
        assert!(d.problematic);
        assert!(d.set_disable_dmabuf);
        assert!(!d.set_disable_compositing); // accel left on unless safe mode
    }

    #[test]
    fn safe_mode_disables_compositing_too() {
        let d = decide(&env(|e| e.safe_mode_requested = true));
        assert!(d.safe_mode);
        assert!(d.set_disable_dmabuf);
        assert!(d.set_disable_compositing);
        assert!(d.reason.contains("safe mode"));
    }

    #[test]
    fn explicit_user_values_are_never_clobbered() {
        let d = decide(&env(|e| {
            e.safe_mode_requested = true;
            e.explicit_dmabuf = Some("0".into());
            e.explicit_compositing = Some("0".into());
        }));
        assert!(
            !d.set_disable_dmabuf,
            "must not override explicit DMABUF value"
        );
        assert!(
            !d.set_disable_compositing,
            "must not override explicit compositing value"
        );
    }

    #[test]
    fn nvidia_without_wayland_is_not_problematic() {
        let d = decide(&env(|e| {
            e.session_type = Some("x11".into());
            e.has_nvidia = true;
        }));
        assert!(!d.problematic);
        assert!(d.set_disable_dmabuf);
    }
}
