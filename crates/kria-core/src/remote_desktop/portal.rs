//! xdg-desktop-portal ScreenCast + PipeWire capture backend (Phase 4.6 v3).
//!
//! Shares the **current** GNOME (or any portal-backed) session — X11 or
//! Wayland — by acquiring an `org.freedesktop.portal.ScreenCast` +
//! `org.freedesktop.portal.RemoteDesktop` session, then streaming the PipeWire
//! node to the browser over WebRTC (encode + transport handled by the signaling
//! layer). No gnome-remote-desktop / RDP involved, so the EGFX/AVC444 RDP
//! limitations do not apply, and the same portal input-injection path is what a
//! future GUI-cognition agent uses to drive the machine.
//!
//! Phase 5 status: lifecycle skeleton (compiles + keeps the session manager and
//! all safety/HITL/audit/idle/kill-switch logic intact). The real portal
//! acquisition + capture handle land in Phase 6.

use std::sync::atomic::{AtomicBool, Ordering};

use super::backend::DesktopBackend;
use crate::config::RemoteDesktopConfig;

/// Detect whether a desktop portal session bus is reachable on this host.
pub fn portal_available() -> bool {
    // The portal is a session-bus service; the real check happens at enable()
    // time. A session bus address (or XDG_RUNTIME_DIR bus) implies a graphical
    // login session where the portal can run.
    if std::env::var("DBUS_SESSION_BUS_ADDRESS").is_ok() {
        return true;
    }
    std::env::var("XDG_RUNTIME_DIR")
        .map(|dir| std::path::Path::new(&dir).join("bus").exists())
        .unwrap_or(false)
}

/// Portal ScreenCast + PipeWire + WebRTC backend.
pub struct PortalBackend {
    #[allow(dead_code)]
    config: RemoteDesktopConfig,
    running: AtomicBool,
}

impl PortalBackend {
    pub fn new(config: RemoteDesktopConfig) -> Self {
        Self {
            config,
            running: AtomicBool::new(false),
        }
    }
}

impl DesktopBackend for PortalBackend {
    fn enable(&self) -> Result<(), String> {
        if !portal_available() {
            return Err(
                "no desktop portal session bus found — remote desktop needs a \
                 logged-in graphical session (xdg-desktop-portal)."
                    .into(),
            );
        }
        // Phase 6: acquire ScreenCast + RemoteDesktop portal session here
        // (CreateSession -> SelectSources -> Start [consent] -> OpenPipeWireRemote),
        // store the PipeWire fd + node id + input grant for the signaling layer.
        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn disable(&self) {
        // Phase 6: tear down the WebRTC pipeline + close the portal session.
        self.running.store(false, Ordering::SeqCst);
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn label(&self) -> String {
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
        format!("WebRTC · portal ScreenCast · {session} session")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_mentions_webrtc() {
        let b = PortalBackend::new(RemoteDesktopConfig::default());
        assert!(b.label().contains("WebRTC"));
    }

    #[test]
    fn enable_disable_toggles_running() {
        let b = PortalBackend::new(RemoteDesktopConfig::default());
        // enable() may fail in a headless CI without a session bus; only assert
        // the running flag flips when it succeeds.
        if b.enable().is_ok() {
            assert!(b.is_running());
        }
        b.disable();
        assert!(!b.is_running());
    }
}
