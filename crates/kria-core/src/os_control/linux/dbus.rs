//! Live D-Bus transport seam (session + system bus).
//!
//! linux-os-control-production **Task 1.3** (OSC-003, OSC-033), design §§7, 18.
//!
//! # Host safety
//!
//! Opening a D-Bus connection is a **raw live transport**. Per Task 0.4 every
//! such constructor:
//!
//! 1. requires a [`LiveHostAccessToken`] by borrow — mintable **only** by the
//!    desktop/server composition roots under `os-control-live`, so no completion
//!    test can construct one; and
//! 2. calls [`deny_live_transport`] **before** touching the bus, so if a
//!    deny-live (`os-control-test`) build ever reached here it would trip the
//!    process-wide sentinel and abort rather than open a live connection.
//!
//! Because of (1) and (2) these functions are effectively live-only: they
//! compile in every feature configuration but can neither be invoked nor open a
//! bus under the deny-live test composition. The higher-level probe that uses
//! this transport is [`super::probe::LiveSessionProbe`].

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::capability::BusKind;

/// A live D-Bus connection pair. Constructed only from a live composition root;
/// each connection is opened behind the deny-live sentinel.
#[derive(Clone)]
pub struct LiveDbusTransport {
    session: Option<zbus::Connection>,
    system: Option<zbus::Connection>,
}

impl std::fmt::Debug for LiveDbusTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveDbusTransport")
            .field("session", &self.session.is_some())
            .field("system", &self.system.is_some())
            .finish()
    }
}

impl LiveDbusTransport {
    /// Open the session and system buses (best effort). A bus that fails to
    /// connect is recorded as absent rather than erroring, so capability probing
    /// can degrade only the affected domains (OSC-031.6).
    ///
    /// # Host safety
    /// Requires a [`LiveHostAccessToken`] and arms the deny-live sentinel for
    /// each bus kind before connecting.
    pub async fn connect(token: &LiveHostAccessToken) -> Self {
        let session = Self::connect_session(token).await;
        let system = Self::connect_system(token).await;
        Self { session, system }
    }

    /// Open only the session bus (deny-live guarded).
    pub async fn connect_session(_token: &LiveHostAccessToken) -> Option<zbus::Connection> {
        deny_live_transport(RawTransportKind::SessionBus);
        zbus::Connection::session().await.ok()
    }

    /// Open only the system bus (deny-live guarded).
    pub async fn connect_system(_token: &LiveHostAccessToken) -> Option<zbus::Connection> {
        deny_live_transport(RawTransportKind::SystemBus);
        zbus::Connection::system().await.ok()
    }

    /// Borrow the connection for a bus kind, if it is open.
    #[must_use]
    pub fn connection(&self, bus: BusKind) -> Option<&zbus::Connection> {
        match bus {
            BusKind::Session => self.session.as_ref(),
            BusKind::System => self.system.as_ref(),
        }
    }

    /// Whether a bus is currently connected.
    #[must_use]
    pub fn is_connected(&self, bus: BusKind) -> bool {
        self.connection(bus).is_some()
    }
}
