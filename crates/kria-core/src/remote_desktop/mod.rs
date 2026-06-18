//! Remote desktop view & takeover (Phase 4.6).
//!
//! Shares the **current** session (X11 or Wayland) via xdg-desktop-portal
//! ScreenCast + PipeWire, streamed to a phone over WebRTC through KRIA's
//! token-gated signaling on the phone gateway. Highest-risk capability, gated:
//!
//!   * disabled by default (`[remote_desktop].enabled`);
//!   * the capture/input portal session is acquired only on explicit confirm;
//!   * two-step HITL start (request → confirm) with a plain description;
//!   * capture enabled on confirm, released on stop / idle / halt (never left
//!     running), reconciled on startup/shutdown;
//!   * idle auto-expire, `global_halt` kill switch, full audit.
//!
//! The backend is behind [`DesktopBackend`] so the session state machine is
//! unit-testable without a live portal/compositor.

pub mod backend;
pub mod portal;
pub mod session;

pub use backend::DesktopBackend;
pub use portal::{portal_available, PortalBackend};
pub use session::{
    RemoteDesktopError, RemoteDesktopManager, RemoteSessionStatus, SessionActivation, SessionState,
};

use std::path::Path;
use std::sync::Arc;

/// Open an [`crate::safety::AuditLogger`] at `path` for remote-desktop events,
/// so hosts (e.g. `kria-server`) don't need a direct `rusqlite` dependency.
pub fn audit_logger_at(path: &Path) -> Option<Arc<crate::safety::AuditLogger>> {
    match rusqlite::Connection::open(path) {
        Ok(conn) => Some(Arc::new(crate::safety::AuditLogger::new(conn))),
        Err(e) => {
            tracing::warn!(error = %e, "remote desktop: failed to open audit db");
            None
        }
    }
}
