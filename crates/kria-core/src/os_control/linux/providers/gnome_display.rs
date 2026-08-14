//! Live GNOME session D-Bus / hardware / XRandR brightness adapter (raw
//! transport seam).
//!
//! linux-os-control-production **Task 2.2** (OSC-019, OSC-031, OSC-032), design
//! §3, §9.6 (`linux/providers/gnome_display.rs`).
//!
//! # Host safety
//!
//! Driving brightness (the GNOME session-bus property, `brightnessctl`, or
//! `xrandr`) is a **raw live transport**. Like
//! [`crate::os_control::linux::providers::pipewire`], this adapter:
//!
//! 1. can be constructed **only** with a
//!    [`crate::os_control::access::LiveHostAccessToken`] (mintable solely in a
//!    live composition root under `os-control-live`), so no completion test can
//!    build it; and
//! 2. calls [`crate::os_control::access::deny_live_transport`] **before** any
//!    read or dispatch — naming the transport kind the selected backend would
//!    actually use — so a deny-live (`os-control-test`) build that reached here
//!    would trip the sentinel and abort rather than touch the host.
//!
//! # Reads
//!
//! [`BrightnessBackend::GnomeSettingsDaemon`] reads
//! `org.gnome.SettingsDaemon.Power.Screen.Brightness` over the **session bus**
//! connection a live composition root opened with [`LiveDbusTransport`] (see
//! [`LiveGnomeDisplay::with_bus`]).
//!
//! The two subprocess backends (`brightnessctl`, `xrandr`) have **no read path
//! in this adapter**: it owns a bus transport, not a governed query launcher, so
//! it reports [`OsControlError::Unavailable`] instead of assuming a binary is
//! installed — `brightnessctl` is absent on the owner's host, and an
//! `Unavailable` read is the honest answer there. There is no ungoverned
//! subprocess fallback.
//!
//! **A brightness that cannot be read is `Unavailable`, never `0`.** Zero is a
//! legitimate brightness, so substituting it would let a later
//! `set_brightness` "verify" against a percentage the host never reported. That
//! rule is enforced by [`parse_gnome_brightness_percent`], which also rejects
//! the `-1` GNOME reports when the session has no controllable backlight.
//!
//! Deny-live tests inject [`crate::os_control::display::fake::FakeDisplayTransport`].
//!
//! # No XRandR on Wayland (OSC-019.3, OSC-032.3)
//!
//! This adapter never selects [`BrightnessBackend::XrandrGamma`] outside an
//! X11 session: construction takes the session's confirmed
//! [`crate::os_control::capability::DisplayServer`] and resolves the backend
//! through [`crate::os_control::display::select_brightness_backend`], the same
//! choke point the deny-live tests assert against.

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::capability::BusKind;
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{ProviderId, SafeOperation, SafeText};
use crate::os_control::display::selection::parse_gnome_brightness_percent;
use crate::os_control::display::{BrightnessBackend, DisplayTransport};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::dbus::LiveDbusTransport;
use crate::os_control::linux::structured_command::StructuredCommandRequest;
use crate::os_control::receipt::ApplyOutcome;

/// The GNOME power daemon's bus name (session bus).
const GSD_POWER_SERVICE: &str = "org.gnome.SettingsDaemon.Power";
/// The GNOME power daemon's object.
const GSD_POWER_PATH: &str = "/org/gnome/SettingsDaemon/Power";
/// The interface owning the `Brightness` property.
const GSD_SCREEN_IFACE: &str = "org.gnome.SettingsDaemon.Power.Screen";
/// The standard freedesktop property interface.
const PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";

/// The live GNOME/hardware/XRandR brightness adapter. Constructible only in a
/// live composition; a value cannot exist under `os-control-test`.
pub struct LiveGnomeDisplay {
    backend: BrightnessBackend,
    /// The session-bus connection the GNOME property read runs over, when a live
    /// composition root handed this adapter one.
    session_bus: Option<zbus::Connection>,
    _seal: (),
}

impl LiveGnomeDisplay {
    /// Construct in a live composition root over a selected backend, **without**
    /// a bus connection: mutations dispatch through the governed structured
    /// command, and the brightness read reports `Unavailable`. Requires a
    /// [`LiveHostAccessToken`], so no completion test can build one. `backend`
    /// must already have been resolved through
    /// [`crate::os_control::display::select_brightness_backend`] against the
    /// session's confirmed display server, so XRandR can never reach this
    /// adapter for a Wayland session.
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken, backend: BrightnessBackend) -> Self {
        Self {
            backend,
            session_bus: None,
            _seal: (),
        }
    }

    /// Construct over the session bus a live composition root already opened
    /// with [`LiveDbusTransport`]. This is the constructor that makes the GNOME
    /// `Brightness` property read real; the transport was opened behind the
    /// deny-live sentinel and the live token, so this adapter never opens a bus
    /// of its own.
    #[must_use]
    pub fn with_bus(
        _token: &LiveHostAccessToken,
        backend: BrightnessBackend,
        transport: &LiveDbusTransport,
    ) -> Self {
        Self {
            backend,
            session_bus: transport.connection(BusKind::Session).cloned(),
            _seal: (),
        }
    }

    /// The transport kind a read on the selected backend would actually open.
    /// Naming it precisely keeps the deny-live sentinel's panic auditable.
    fn read_transport_kind(&self) -> RawTransportKind {
        match self.backend {
            BrightnessBackend::GnomeSettingsDaemon => RawTransportKind::SessionBus,
            // logind reads brightness from a sysfs DEVICE node, not a bus or a
            // process.
            // Named precisely so a sentinel panic points at the real transport.
            BrightnessBackend::LogindSession => RawTransportKind::Device,
            BrightnessBackend::Brightnessctl | BrightnessBackend::XrandrGamma => {
                RawTransportKind::Process
            }
        }
    }

    /// Borrow the session bus, or fail closed. Never a substituted percentage.
    fn bus(&self) -> Result<&zbus::Connection, OsControlError> {
        self.session_bus
            .as_ref()
            .ok_or_else(|| OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: SafeText::new(
                    "no session-bus connection was composed for the GNOME brightness adapter; brightness is unknown, not zero",
                ),
                retryable: false,
            })
    }

    /// Read the GNOME `Brightness` property, deadline- and cancellation-bounded
    /// by the observation context (a provider cannot grant itself more time). A
    /// read takes no grant because it changes nothing.
    async fn read_gnome_percent(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<u8, OsControlError> {
        if ctx.cancellation.is_cancelled() {
            return Err(OsControlError::CancelledBeforeMutation);
        }
        let conn = self.bus()?;
        let args = (GSD_SCREEN_IFACE, "Brightness");
        let call = conn.call_method(
            Some(GSD_POWER_SERVICE),
            GSD_POWER_PATH,
            Some(PROPERTIES_IFACE),
            "Get",
            &args,
        );
        let deadline = tokio::time::Instant::from_std(ctx.deadline);
        let reply = tokio::select! {
            biased;
            () = ctx.cancellation.cancelled() => {
                return Err(OsControlError::CancelledBeforeMutation);
            }
            outcome = tokio::time::timeout_at(deadline, call) => outcome,
        }
        .map_err(|_| OsControlError::TimedOutBeforeMutation {
            operation: SafeOperation::new("get_display_state"),
            timeout_ms: 0,
        })?
        .map_err(|_| OsControlError::Unavailable {
            provider: Some(self.provider_id()),
            reason: SafeText::new(
                "the GNOME power daemon did not answer the brightness property read",
            ),
            retryable: true,
        })?;

        let value: zbus::zvariant::OwnedValue =
            reply.body().deserialize().map_err(|_| self.protocol())?;
        // GNOME declares `Brightness` as `i` (a percentage). A different type is
        // an unrecognized reading, not a value to coerce.
        let raw = i32::try_from(value).map_err(|_| self.protocol())?;
        parse_gnome_brightness_percent(self.backend, raw)
    }

    /// The daemon answered with something this adapter cannot interpret.
    fn protocol(&self) -> OsControlError {
        OsControlError::ProtocolBeforeMutation {
            provider: self.provider_id(),
            operation: SafeOperation::new("get_display_state"),
        }
    }
}

#[async_trait::async_trait]
impl DisplayTransport for LiveGnomeDisplay {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(format!("display-{}", self.backend.as_str()))
    }

    fn selected_backend(&self) -> BrightnessBackend {
        self.backend
    }

    async fn read_brightness(&self, ctx: &HostExecutionContext) -> Result<u8, OsControlError> {
        // A state read opens the session bus (GNOME) or would launch a query
        // child process (brightnessctl / xrandr).
        deny_live_transport(self.read_transport_kind());

        match self.backend {
            BrightnessBackend::GnomeSettingsDaemon => self.read_gnome_percent(ctx).await,
            // A direct sysfs read: no process, no parsing ambiguity. An absent or
            // unreadable device stays unknown rather than becoming a percentage.
            BrightnessBackend::LogindSession => {
                let (device, max) = crate::os_control::display::selection::discover_backlight_device().ok_or_else(|| {
                    OsControlError::Unavailable {
                        provider: Some(self.provider_id()),
                        reason: SafeText::new(
                            "this machine exposes no controllable backlight device",
                        ),
                        retryable: false,
                    }
                })?;
                crate::os_control::display::selection::read_backlight_percent(&device, max).ok_or_else(|| {
                    OsControlError::Unavailable {
                        provider: Some(self.provider_id()),
                        reason: SafeText::new("the backlight brightness could not be read"),
                        retryable: true,
                    }
                })
            }
            // This adapter holds a bus transport, not a governed query launcher.
            // Refusing is the only honest answer: `brightnessctl` may not even
            // be installed, and inventing a percentage (especially `0`) would
            // give a later `set_brightness` a fabricated fact to verify against.
            BrightnessBackend::Brightnessctl | BrightnessBackend::XrandrGamma => {
                Err(OsControlError::Unavailable {
                    provider: Some(self.provider_id()),
                    reason: SafeText::new(
                        "this backend needs a governed subprocess read that the bus adapter does not provide; the tool's presence is not assumed and no brightness is guessed",
                    ),
                    retryable: false,
                })
            }
        }
    }

    async fn dispatch(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError> {
        // The governed request's own launch trips the deny-live sentinel; keep an
        // explicit guard here too so the adapter is unreachable under test.
        deny_live_transport(RawTransportKind::Process);
        request.dispatch().await
    }
}
