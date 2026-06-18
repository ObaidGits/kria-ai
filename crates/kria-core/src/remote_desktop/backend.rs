//! Transport-neutral desktop-capture backend abstraction.
//!
//! The session state machine ([`super::session::RemoteDesktopManager`]) only
//! cares about lifecycle (acquire/release the capture + input grant, report
//! whether it is serving). The concrete backend — currently
//! [`super::portal::PortalBackend`] (xdg-desktop-portal ScreenCast + PipeWire +
//! WebRTC) — owns the heavy machinery. This keeps the manager unit-testable
//! with a fake backend and decoupled from any specific capture technology.

/// A desktop-capture + input backend the session manager drives.
pub trait DesktopBackend: Send + Sync {
    /// Acquire the capture session (and input grant). For the portal backend
    /// this opens the ScreenCast + RemoteDesktop portal session (consent), so
    /// it must only be called on an explicit, confirmed user action.
    fn enable(&self) -> Result<(), String>;

    /// Release the capture session + tear down any active stream (idempotent).
    fn disable(&self);

    /// Whether the backend is currently serving (capture session is live).
    fn is_running(&self) -> bool;

    /// Human label for status/UX.
    fn label(&self) -> String;
}
