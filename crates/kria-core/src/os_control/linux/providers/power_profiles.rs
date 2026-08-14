//! Live `power-profiles-daemon` D-Bus / `powerprofilesctl` adapter (raw
//! transport seam).
//!
//! linux-os-control-production **Task 2.3** — "Migrate Wi-Fi and power-profile
//! controls" (OSC-020, OSC-031), design §3, §9.7
//! (`linux/providers/power_profiles.rs`).
//!
//! # Host safety
//!
//! Driving the power profile (`power-profiles-daemon` over D-Bus, or
//! `powerprofilesctl`) is a **raw live transport**. Like
//! [`crate::os_control::linux::providers::pipewire`] and
//! [`crate::os_control::linux::providers::gnome_display`], this adapter:
//!
//! 1. can be constructed **only** with a
//!    [`crate::os_control::access::LiveHostAccessToken`] (mintable solely in a
//!    live composition root under `os-control-live`), so no completion test can
//!    build it; and
//! 2. calls [`crate::os_control::access::deny_live_transport`] **before** any
//!    read or dispatch, so a deny-live (`os-control-test`) build that reached
//!    here would trip the sentinel and abort rather than touch the host.
//!
//! # Reads
//!
//! [`LivePowerProfiles::read_profile`] is a live **system-bus** read over the
//! connection a live composition root opened with [`LiveDbusTransport`] (see
//! [`LivePowerProfiles::with_bus`]). Mutations continue to dispatch through the
//! governed [`StructuredCommandRequest`]; there is no ungoverned subprocess
//! fallback.
//!
//! Both the renamed `org.freedesktop.UPower.PowerProfiles` interface and the
//! historical `net.hadess.PowerProfiles` one are tried, because which name a
//! host owns depends on its `power-profiles-daemon` version. Both are the same
//! authoritative daemon on the same guarded bus, so this is interface-version
//! tolerance, not a fallback transport.
//!
//! # The advertised profile set is hardware-dependent
//!
//! `power-profiles-daemon` decides which profiles exist from the platform
//! driver: a machine may advertise only `balanced` and `performance`, and some
//! vendors expose profiles outside this contract's closed set. So the read never
//! assumes a fixed set — it reads the daemon's own `Profiles` list and confirms
//! `ActiveProfile` against it through
//! [`crate::os_control::power::selection::parse_active_profile`]. A profile that
//! is not advertised here is never reported as active.
//!
//! Deny-live tests inject [`crate::os_control::power::fake::FakePowerProfileTransport`].

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::capability::BusKind;
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{ProviderId, SafeOperation, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::dbus::LiveDbusTransport;
use crate::os_control::linux::structured_command::StructuredCommandRequest;
use crate::os_control::power::selection::{
    parse_active_profile, parse_battery_capacity, parse_charge_cycles,
    UPOWER_DEVICE_TYPE_BATTERY,
};
use crate::os_control::power::{
    BatteryHealth, PowerProfile, PowerProfileBackend, PowerProfileTransport,
};
use crate::os_control::receipt::ApplyOutcome;

/// The standard freedesktop property interface.
const PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";

/// UPower's bus name / manager object / manager interface (system bus). UPower is
/// the authoritative source for battery presence and design-capacity health; it
/// is a different service from `power-profiles-daemon` on the same guarded bus,
/// not a second transport.
const UPOWER_SERVICE: &str = "org.freedesktop.UPower";
const UPOWER_PATH: &str = "/org/freedesktop/UPower";
const UPOWER_IFACE: &str = "org.freedesktop.UPower";
/// The per-device interface owning `Type`, `IsPresent`, `Capacity`,
/// `ChargeCycles` and `PowerSupply`.
const UPOWER_DEVICE_IFACE: &str = "org.freedesktop.UPower.Device";

/// `power-profiles-daemon`'s bus name / object path / interface, newest first.
/// The interface name equals the bus name on both generations of the daemon.
const PPD_ENDPOINTS: [(&str, &str); 2] = [
    (
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
    ),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"),
];

/// The live `power-profiles-daemon`/`powerprofilesctl` adapter. Constructible
/// only in a live composition; a value cannot exist under `os-control-test`.
pub struct LivePowerProfiles {
    backend: PowerProfileBackend,
    /// The system-bus connection the profile read runs over, when a live
    /// composition root handed this adapter one.
    system_bus: Option<zbus::Connection>,
    _seal: (),
}

impl LivePowerProfiles {
    /// Construct in a live composition root over a selected backend, **without**
    /// a bus connection: mutations dispatch through the governed structured
    /// command, and the profile read reports `Unavailable`. Requires a
    /// [`LiveHostAccessToken`], so no completion test can build one.
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken, backend: PowerProfileBackend) -> Self {
        Self {
            backend,
            system_bus: None,
            _seal: (),
        }
    }

    /// Construct over the system bus a live composition root already opened with
    /// [`LiveDbusTransport`]. This is the constructor that makes the
    /// `ActiveProfile`/`Profiles` read real; the transport was opened behind the
    /// deny-live sentinel and the live token, so this adapter never opens a bus
    /// of its own.
    #[must_use]
    pub fn with_bus(
        _token: &LiveHostAccessToken,
        backend: PowerProfileBackend,
        transport: &LiveDbusTransport,
    ) -> Self {
        Self {
            backend,
            system_bus: transport.connection(BusKind::System).cloned(),
            _seal: (),
        }
    }

    /// Borrow the system bus, or fail closed.
    fn bus(&self) -> Result<&zbus::Connection, OsControlError> {
        self.system_bus
            .as_ref()
            .ok_or_else(|| OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: SafeText::new(
                    "no system-bus connection was composed for the power-profile adapter; the active profile is unknown",
                ),
                retryable: false,
            })
    }

    /// The daemon answered with something this adapter cannot interpret.
    fn protocol(&self, member: &str) -> OsControlError {
        OsControlError::ProtocolBeforeMutation {
            provider: self.provider_id(),
            operation: SafeOperation::new(member),
        }
    }

    /// The daemon could not be read on either interface name.
    fn unreachable(&self) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(self.provider_id()),
            reason: SafeText::new(
                "power-profiles-daemon did not answer the profile read on either of its interface names",
            ),
            retryable: true,
        }
    }

    /// Read one property, deadline- and cancellation-bounded by the observation
    /// context (a provider cannot grant itself more time). A read takes no grant
    /// because it changes nothing.
    async fn property<R>(
        &self,
        ctx: &HostExecutionContext,
        service: &str,
        path: &str,
        property: &str,
    ) -> Result<R, OsControlError>
    where
        R: TryFrom<zbus::zvariant::OwnedValue>,
    {
        if ctx.cancellation.is_cancelled() {
            return Err(OsControlError::CancelledBeforeMutation);
        }
        let conn = self.bus()?;
        let args = (service, property);
        let call = conn.call_method(Some(service), path, Some(PROPERTIES_IFACE), "Get", &args);
        let deadline = tokio::time::Instant::from_std(ctx.deadline);
        let reply = tokio::select! {
            biased;
            () = ctx.cancellation.cancelled() => {
                return Err(OsControlError::CancelledBeforeMutation);
            }
            outcome = tokio::time::timeout_at(deadline, call) => outcome,
        }
        .map_err(|_| OsControlError::TimedOutBeforeMutation {
            operation: SafeOperation::new("get_power_plan"),
            timeout_ms: 0,
        })?
        .map_err(|_| self.unreachable())?;

        let value: zbus::zvariant::OwnedValue = reply
            .body()
            .deserialize()
            .map_err(|_| self.protocol(property))?;
        R::try_from(value).map_err(|_| self.protocol(property))
    }

    /// One deadline- and cancellation-bounded UPower method call.
    ///
    /// The bound comes from the observation context, never from this provider: an
    /// adapter cannot grant itself more time than the admitted action has. A read
    /// takes no grant because it changes nothing.
    async fn call<A, R>(
        &self,
        ctx: &HostExecutionContext,
        path: &str,
        interface: &str,
        member: &str,
        args: &A,
    ) -> Result<R, OsControlError>
    where
        A: serde::Serialize + zbus::zvariant::DynamicType,
        R: zbus::zvariant::Type + for<'d> serde::Deserialize<'d>,
    {
        if ctx.cancellation.is_cancelled() {
            return Err(OsControlError::CancelledBeforeMutation);
        }
        let conn = self.bus()?;
        let call = conn.call_method(Some(UPOWER_SERVICE), path, Some(interface), member, args);
        let deadline = tokio::time::Instant::from_std(ctx.deadline);
        let reply = tokio::select! {
            biased;
            () = ctx.cancellation.cancelled() => {
                return Err(OsControlError::CancelledBeforeMutation);
            }
            outcome = tokio::time::timeout_at(deadline, call) => outcome,
        }
        .map_err(|_| OsControlError::TimedOutBeforeMutation {
            operation: SafeOperation::new(member),
            timeout_ms: 0,
        })?
        .map_err(|_| OsControlError::Unavailable {
            provider: Some(self.provider_id()),
            reason: SafeText::new("UPower did not answer the battery state read"),
            retryable: true,
        })?;
        reply
            .body()
            .deserialize::<R>()
            .map_err(|_| self.protocol(member))
    }

    /// Read one `org.freedesktop.UPower.Device` property as a raw variant.
    ///
    /// Kept separate from [`Self::property`] because that helper assumes the
    /// interface name equals the bus name (true for `power-profiles-daemon`, not
    /// for UPower's per-device interface).
    async fn device_property(
        &self,
        ctx: &HostExecutionContext,
        path: &str,
        property: &str,
    ) -> Result<zbus::zvariant::OwnedValue, OsControlError> {
        self.call(
            ctx,
            path,
            PROPERTIES_IFACE,
            "Get",
            &(UPOWER_DEVICE_IFACE, property),
        )
        .await
    }

    /// The profile tokens this machine actually advertises (`Profiles`, `aa{sv}`,
    /// each entry carrying a `Profile` string). Entries without a readable
    /// `Profile` name are skipped rather than guessed at.
    async fn advertised_profiles(
        &self,
        ctx: &HostExecutionContext,
        service: &str,
        path: &str,
    ) -> Result<Vec<String>, OsControlError> {
        let entries: Vec<std::collections::HashMap<String, zbus::zvariant::OwnedValue>> =
            self.property(ctx, service, path, "Profiles").await?;
        Ok(entries
            .iter()
            .filter_map(|entry| entry.get("Profile"))
            .filter_map(|name| <&str>::try_from(name).ok())
            .map(str::to_string)
            .collect())
    }
}

#[async_trait::async_trait]
impl PowerProfileTransport for LivePowerProfiles {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(format!("power-{}", self.backend.as_str()))
    }

    fn selected_backend(&self) -> PowerProfileBackend {
        self.backend
    }

    async fn read_profile(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<PowerProfile, OsControlError> {
        // A profile read opens a system-bus round trip.
        deny_live_transport(RawTransportKind::SystemBus);

        // Fail before the loop when there is no bus at all, so the caller gets
        // "no connection" rather than "the daemon did not answer".
        self.bus()?;

        let mut last_error = None;
        for (service, path) in PPD_ENDPOINTS {
            // The advertised set is read FIRST: without it there is nothing to
            // confirm the active profile against.
            let advertised = match self.advertised_profiles(ctx, service, path).await {
                Ok(advertised) => advertised,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            let active: String = match self.property(ctx, service, path, "ActiveProfile").await {
                Ok(active) => active,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            return parse_active_profile(self.backend, &active, &advertised);
        }
        Err(last_error.unwrap_or_else(|| self.unreachable()))
    }

    async fn dispatch(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        request.dispatch().await
    }

    async fn read_battery_health(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<BatteryHealth, OsControlError> {
        // A UPower device enumeration opens a system-bus round trip.
        deny_live_transport(RawTransportKind::SystemBus);

        // Fail before enumerating when there is no bus at all, so the caller gets
        // "no connection" rather than "no battery" — those are different facts,
        // and only one of them is safe to report as `Absent`.
        self.bus()?;

        let devices: Vec<zbus::zvariant::OwnedObjectPath> = self
            .call(ctx, UPOWER_PATH, UPOWER_IFACE, "EnumerateDevices", &())
            .await?;

        for device in &devices {
            let path = device.as_str();
            // Match on the numeric device type, never on the model string: a
            // device label is neither unique nor stable.
            let kind: u32 = self
                .device_property(ctx, path, "Type")
                .await
                .and_then(|value: zbus::zvariant::OwnedValue| {
                    u32::try_from(value).map_err(|_| self.protocol("Type"))
                })?;
            if kind != UPOWER_DEVICE_TYPE_BATTERY {
                continue;
            }
            // A battery that is not a power supply is a peripheral's battery
            // (mouse, headset); it is not this host's battery.
            let power_supply: bool = self
                .device_property(ctx, path, "PowerSupply")
                .await
                .and_then(|value: zbus::zvariant::OwnedValue| {
                    bool::try_from(value).map_err(|_| self.protocol("PowerSupply"))
                })?;
            if !power_supply {
                continue;
            }
            let present: bool = self
                .device_property(ctx, path, "IsPresent")
                .await
                .and_then(|value: zbus::zvariant::OwnedValue| {
                    bool::try_from(value).map_err(|_| self.protocol("IsPresent"))
                })?;
            if !present {
                // The bay exists but holds no pack. A positive absence fact.
                return Ok(BatteryHealth::Absent);
            }

            let capacity_raw: f64 = self
                .device_property(ctx, path, "Capacity")
                .await
                .and_then(|value: zbus::zvariant::OwnedValue| {
                    f64::try_from(value).map_err(|_| self.protocol("Capacity"))
                })?;
            let capacity = parse_battery_capacity(self.backend, capacity_raw)?;

            // `ChargeCycles` is a newer property; a host whose UPower does not
            // expose it reports no cycle count rather than a fabricated zero.
            let cycles = match self.device_property(ctx, path, "ChargeCycles").await {
                Ok(value) => i32::try_from(value).ok().and_then(parse_charge_cycles),
                Err(_) => None,
            };

            return Ok(BatteryHealth::present(capacity, cycles));
        }

        // UPower answered, and its own inventory contains no power-supply
        // battery: a desktop. "No battery present" is the read, not 0% health.
        Ok(BatteryHealth::Absent)
    }
}
