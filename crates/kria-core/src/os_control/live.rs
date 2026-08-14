//! The **live** host composition root (design §4, §18; OSC-033/OSC-034).
//!
//! Compiled only under `os-control-live`. This is the single place in KRIA where
//! live OS transports are constructed, and therefore the only place a
//! [`LiveHostAccessToken`] is minted. Everything else reaches the host only
//! *through* the aggregate this module returns, which is why the seam is narrow.
//!
//! # Availability policy
//!
//! Two kinds of backend, treated differently on purpose:
//!
//! * **CLI-backed** (`wpctl`, `brightnessctl`, …) — composed only when the
//!   trusted executable actually exists on disk. `trusted_executable()` alone
//!   validates the path's *shape*, not its presence, so filtering on it would
//!   report a backend that is not installed.
//! * **Bus-backed** (logind, NetworkManager, power-profiles-daemon, GNOME
//!   SettingsDaemon) — composed optimistically when a session exists. There is no
//!   cheap way to prove a bus name is claimable without opening a transport, and
//!   the provider already fails closed with the frozen `Unavailable` envelope if
//!   the bus is absent. Composing is a statement of intent, never of guarantee.
//!
//! A domain left uncomposed answers `Unavailable` rather than degrading to a raw
//! shell — the same posture as
//! [`crate::os_control::testing::FakeHostOsControl`].

use std::sync::Arc;

use crate::os_control::access::LiveHostAccessToken;
use crate::os_control::applications::{ApplicationCloseControl, ApplicationCloseControlPort};
use crate::os_control::audio::selection::{select_backend as select_audio_backend, AudioBackend};
use crate::os_control::audio::{AudioControl, AudioControlPort};
use crate::os_control::clipboard::{ClipboardControl, ClipboardControlPort};
use crate::os_control::connectivity::selection::ConnectivityBackend;
use crate::os_control::connectivity::{ConnectivityControl, ConnectivityControlPort};
use crate::os_control::contract::ProviderId;
use crate::os_control::display::selection::{
    select_backend as select_brightness_backend, BrightnessBackend,
};
use crate::os_control::display::{DisplayControl, DisplayControlPort};
use crate::os_control::bluetooth::{BluetoothBackend, BluetoothControl, BluetoothControlPort};
use crate::os_control::broker::transport::LiveBrokerTransport;
use crate::os_control::files::{
    ArchiveControl, ArchiveControlPort, OwnershipControl, OwnershipControlPort,
    RealArchiveTransport, RealOwnershipTransport, RealTrashTransport, TrashControl,
    TrashControlPort,
};
use crate::os_control::capability::{BusKind, SessionProbe};
use crate::os_control::linux::dbus::LiveDbusTransport;
use crate::os_control::linux::probe::LiveSessionProbe;
use crate::os_control::linux::providers::{
    application_control::LiveApplicationControl, bluez::LiveBluez, clipboard::LiveClipboard,
    gnome_display::LiveGnomeDisplay, logind::LiveLogind, network_manager::LiveNetworkManager,
    notifications::LiveNotifications, packagekit::LivePackageKit, pipewire::LivePipewireAudio,
    power_profiles::LivePowerProfiles, process_control::LiveProcessControl, udisks::LiveUdisks,
};
use crate::os_control::notifications::{NotificationControl, NotificationControlPort};
use crate::os_control::packages::{PackageControl, PackageControlPort};
use crate::os_control::power::session::selection::PowerSessionBackend;
use crate::os_control::power::session::{PowerSessionControl, PowerSessionControlPort};
use crate::os_control::power::selection::PowerProfileBackend;
use crate::os_control::power::{PowerControl, PowerControlPort};
use crate::os_control::processes::{ProcessControl, ProcessControlPort};
use crate::os_control::runtime::HostOsControl;
use crate::os_control::storage::{StorageControl, StorageControlPort};

/// Observed host facts used to decide what to compose.
///
/// Built from a completed [`LiveSessionProbe`] so composition asks "does this bus
/// name actually have an owner on this machine" instead of composing optimistically
/// and hoping. `None` anywhere means the probe could not confirm it, which is
/// treated as absent — fail closed, never assume.
pub struct HostFacts {
    session_bus: bool,
    system_bus: bool,
    owned_services: Vec<String>,
}

impl HostFacts {
    /// Collect the facts the composition cares about from a completed probe.
    fn from_probe(probe: &LiveSessionProbe, plan: &crate::os_control::linux::probe::LiveProbePlan) -> Self {
        let mut owned_services = Vec::new();
        for (bus, service, _path) in &plan.services {
            // Owned OR activatable: an on-demand service (PackageKit, UDisks2) has
            // no owner until first call, and refusing to compose it would disable
            // a capability that actually works.
            if probe.service_reachable(*bus, service) {
                owned_services.push(service.clone());
            }
        }
        Self {
            session_bus: matches!(
                probe.bus_status(BusKind::Session),
                crate::os_control::capability::BusStatus::Available
            ),
            system_bus: matches!(
                probe.bus_status(BusKind::System),
                crate::os_control::capability::BusStatus::Available
            ),
            owned_services,
        }
    }

    fn owns(&self, service: &str) -> bool {
        self.owned_services.iter().any(|s| s == service)
    }
}

/// Stable identity of the live Linux host aggregate.
pub const LIVE_HOST_PROVIDER_ID: &str = "linux-host";

/// Whether a trusted executable path actually exists as a file on this host.
///
/// Presence is deliberately separate from `trusted_executable()`: that validates
/// the path is safe to execute (absolute, no shell metacharacters), which is a
/// *shape* check and stays true for a program that is not installed.
fn executable_present(path: &str) -> bool {
    std::path::Path::new(path).is_file()
}

/// Whether the process environment advertises a graphical session at all.
fn graphical_session_advertised() -> bool {
    !matches!(
        LiveSessionProbe::env_from_process().display_server_hint(),
        crate::os_control::capability::DisplayServer::Headless
    )
}

/// Whether this machine exposes a battery with charge-threshold control.
///
/// A desktop with no battery must not advertise the capability: the tool would be
/// offered, accepted, and then fail at the sysfs write. Checking for the threshold
/// attribute specifically — not merely for a battery — because many batteries do
/// not support charge limiting at all.
fn battery_present() -> bool {
    ["BAT0", "BAT1"].iter().any(|battery| {
        std::path::Path::new(&format!(
            "/sys/class/power_supply/{battery}/charge_control_end_threshold"
        ))
        .exists()
    })
}

/// The XDG config root for the current user.
///
/// Honours `XDG_CONFIG_HOME` before falling back to `~/.config`, because a user
/// who has moved their config directory would otherwise have KRIA write default
/// applications and autostart entries somewhere their desktop never reads.
fn xdg_config_root() -> Option<std::path::PathBuf> {
    if let Some(explicit) = std::env::var_os("XDG_CONFIG_HOME") {
        let path = std::path::PathBuf::from(explicit);
        // A relative XDG_CONFIG_HOME is invalid per the spec; ignore it rather
        // than resolving it against an arbitrary working directory.
        if path.is_absolute() {
            return Some(path);
        }
    }
    std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
}

/// The XDG trash root for the current user.
fn xdg_trash_root() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| {
        std::path::PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("Trash")
    })
}

/// The live aggregate of composed OS domain ports.
///
/// Construct with [`LiveHostOsControl::compose`] at desktop/server startup and
/// hand it to `OsControlRuntime::with_host`.
pub struct LiveHostOsControl {
    provider: ProviderId,
    audio: Option<Arc<dyn AudioControlPort>>,
    display: Option<Arc<dyn DisplayControlPort>>,
    connectivity: Option<Arc<dyn ConnectivityControlPort>>,
    power: Option<Arc<dyn PowerControlPort>>,
    power_session: Option<Arc<dyn PowerSessionControlPort>>,
    processes: Option<Arc<dyn ProcessControlPort>>,
    application_close: Option<Arc<dyn ApplicationCloseControlPort>>,
    clipboard: Option<Arc<dyn ClipboardControlPort>>,
    notifications: Option<Arc<dyn NotificationControlPort>>,
    packages: Option<Arc<dyn PackageControlPort>>,
    storage: Option<Arc<dyn StorageControlPort>>,
    trash: Option<Arc<dyn TrashControlPort>>,
    archive: Option<Arc<dyn ArchiveControlPort>>,
    ownership: Option<Arc<dyn OwnershipControlPort>>,
    bluetooth: Option<Arc<dyn BluetoothControlPort>>,
    secrets: Option<Arc<dyn crate::os_control::secrets::CredentialStore>>,
    file_attributes:
        Option<Arc<dyn crate::os_control::files::attributes::FileAttributeControlPort>>,
    search_control: Option<Arc<dyn crate::os_control::search::SearchControlPort>>,
    health: Option<Arc<dyn crate::os_control::health::SystemHealthControlPort>>,
    backup_scan: Option<Arc<dyn crate::os_control::backup::BackupScanControlPort>>,
    firmware: Option<Arc<dyn crate::os_control::hardware::FirmwareAwarenessPort>>,
    hardware: Option<Arc<dyn crate::os_control::hardware::HardwareControlPort>>,
    print_control: Option<Arc<dyn crate::os_control::print::PrintControlPort>>,
    privacy: Option<Arc<dyn crate::os_control::privacy::PrivacyControlPort>>,
    firewall: Option<Arc<dyn crate::os_control::firewall::FirewallControlPort>>,
    display_configuration: Option<Arc<dyn crate::os_control::display::configuration::DisplayConfigControlPort>>,
    desktop_association: Option<Arc<dyn crate::os_control::applications::DesktopAssociationControlPort>>,
    automation: Option<Arc<dyn crate::os_control::automation::AutomationControlPort>>,
    charge_thresholds: Option<Arc<dyn crate::os_control::power::charge::ChargeThresholdControlPort>>,
    snapshot: Option<crate::os_control::capability::CapabilitySnapshot>,
}

impl LiveHostOsControl {
    /// Probe the host over D-Bus, then compose every domain the probe confirms
    /// (Task 1.3, OSC-003).
    ///
    /// This is the composition path a live root should use: bus-backed domains are
    /// composed only when the service actually has an owner, so an absent
    /// NetworkManager or logind yields `Unavailable` for that domain instead of a
    /// provider that fails on first call. The resulting capability snapshot is
    /// carried on the aggregate and reaches every admitted action's observation
    /// context through the runtime.
    ///
    /// Each probe round trip is deadline-bounded, so a hung bus degrades the
    /// affected domain rather than blocking startup.
    pub async fn compose_probed() -> Self {
        let token = LiveHostAccessToken::mint();
        let transport = LiveDbusTransport::connect(&token).await;
        let plan = crate::os_control::catalog::probe_plan();
        let probe =
            LiveSessionProbe::probe(&transport, &plan, std::time::Duration::from_millis(1500))
                .await;
        let facts = HostFacts::from_probe(&probe, &plan);
        let prober = crate::os_control::capability::CapabilityProber::new(
            probe,
            crate::os_control::catalog::capability_catalog(),
        );
        let snapshot = prober.snapshot();
        tracing::info!(
            target: "authority_trace",
            session_bus = facts.session_bus,
            system_bus = facts.system_bus,
            owned_services = facts.owned_services.len(),
            revision = snapshot.revision.0,
            "capability probe completed"
        );
        Self::compose_with(&token, Some(&facts), Some(snapshot), Some(&transport))
    }

    /// Compose without probing: environment hints only, bus-backed domains
    /// composed optimistically.
    ///
    /// Prefer [`Self::compose_probed`]. This variant exists for synchronous
    /// callers and for a headless build with no reachable bus, where composing
    /// optimistically and letting the provider fail closed is the honest fallback.
    #[must_use]
    pub fn compose() -> Self {
        let token = LiveHostAccessToken::mint();
        Self::compose_with(&token, None, None, None)
    }

    fn compose_with(
        token: &LiveHostAccessToken,
        facts: Option<&HostFacts>,
        snapshot: Option<crate::os_control::capability::CapabilitySnapshot>,
            // The already-open session/system bus transport, when the caller probed.
        dbus: Option<&LiveDbusTransport>,
    ) -> Self {
        let graphical = graphical_session_advertised();
        // With probe facts, a bus-backed domain is composed only when its service
        // actually has an owner. Without them, compose optimistically: the provider
        // still fails closed with the frozen envelope if the bus is absent.
        let owns = |service: &str| facts.map_or(true, |f| f.owns(service));
        let session_ok = facts.map_or(graphical, |f| f.session_bus) && graphical;
        let system_ok = facts.map_or(true, |f| f.system_bus);

        Self {
            provider: ProviderId::new(LIVE_HOST_PROVIDER_ID),
            audio: Self::compose_audio(token),
            display: Self::compose_display(token, dbus),
            connectivity: (system_ok && owns("org.freedesktop.NetworkManager")).then(|| {
                Arc::new(ConnectivityControl::new(LiveNetworkManager::new(
                    token,
                    ConnectivityBackend::PREFERENCE[0],
                ))) as Arc<dyn ConnectivityControlPort>
            }),
            power: (system_ok && owns("org.freedesktop.UPower.PowerProfiles")).then(|| {
                Arc::new(PowerControl::new(LivePowerProfiles::new(
                    token,
                    PowerProfileBackend::PREFERENCE[0],
                ))) as Arc<dyn PowerControlPort>
            }),
            power_session: (system_ok && owns("org.freedesktop.login1")).then(|| {
                Arc::new(PowerSessionControl::new(LiveLogind::new(
                    token,
                    PowerSessionBackend::PREFERENCE[0],
                ))) as Arc<dyn PowerSessionControlPort>
            }),
            // /proc + kernel signals: always present on Linux.
            processes: Some(Arc::new(ProcessControl::new(LiveProcessControl::new(
                token,
            )))),
            application_close: graphical.then(|| {
                Arc::new(ApplicationCloseControl::new(LiveApplicationControl::new(
                    token,
                ))) as Arc<dyn ApplicationCloseControlPort>
            }),
            clipboard: graphical.then(|| {
                Arc::new(ClipboardControl::new(LiveClipboard::new(token)))
                    as Arc<dyn ClipboardControlPort>
            }),
            notifications: (session_ok && owns("org.freedesktop.Notifications")).then(|| {
                Arc::new(NotificationControl::new(LiveNotifications::new(token)))
                    as Arc<dyn NotificationControlPort>
            }),
            packages: (system_ok && owns("org.freedesktop.PackageKit")).then(|| {
                Arc::new(PackageControl::new(LivePackageKit::new(token)))
                    as Arc<dyn PackageControlPort>
            }),
            storage: (system_ok && owns("org.freedesktop.UDisks2")).then(|| {
                Arc::new(StorageControl::new(LiveUdisks::new(token)))
                    as Arc<dyn StorageControlPort>
            }),
            trash: Self::compose_trash(),
            archive: Some(Arc::new(ArchiveControl::new(RealArchiveTransport::new()))),
            // Ownership changes route through the privilege broker. The broker's
            // socket client is present but its Polkit-activated service is not
            // built yet (Task 1.5), so calls fail closed with a broker error
            // rather than silently doing nothing — which is why the port is
            // composed rather than left absent: the caller gets a precise reason.
            ownership: Some(Arc::new(OwnershipControl::new(
                RealOwnershipTransport::new(LiveBrokerTransport::new(token)),
            ))),
            // BlueZ owns `org.bluez` on the system bus. Composed only when that
            // service is reachable, so a machine without Bluetooth reports
            // Unavailable rather than a provider that cannot answer.
            bluetooth: (system_ok && owns("org.bluez")).then(|| {
                Arc::new(BluetoothControl::new(LiveBluez::new(
                    token,
                    BluetoothBackend::PREFERENCE[0],
                ))) as Arc<dyn BluetoothControlPort>
            }),
            // The Secret Service lives on the SESSION bus and is the user's own
            // keyring; composed only when that bus is reachable and the service
            // answers, so a headless session reports Unavailable rather than
            // silently failing to store a credential.
            // Requires the caller's already-open transport: opening a SECOND bus
            // connection here would derive a different session and the keyring
            // would refuse it.
            secrets: dbus
                .filter(|_| session_ok && owns("org.freedesktop.secrets"))
                .map(|transport| {
                    Arc::new(
                        crate::os_control::linux::providers::secret_service::LiveSecretService::new(
                            token, transport,
                        ),
                    ) as Arc<dyn crate::os_control::secrets::CredentialStore>
                }),
            // Always composed: these are plain filesystem operations with no
            // service dependency, so there is nothing to probe for.
            file_attributes: Some(Arc::new(
                crate::os_control::files::attributes::FileAttributeControl::new(
                    crate::os_control::files::attributes::RealFileAttributeTransport::new(),
                ),
            )),
            search_control: session_ok
                .then(crate::os_control::linux::providers::tracker_search::LiveSearch::discover)
                .flatten()
                .map(|transport| {
                    Arc::new(crate::os_control::search::SearchControl::new(transport))
                        as Arc<dyn crate::os_control::search::SearchControlPort>
                }),
            health: Some(Arc::new(crate::os_control::health::SystemHealthControl::new(
                crate::os_control::linux::providers::system_health::LiveHealth::discover(),
            )) as Arc<dyn crate::os_control::health::SystemHealthControlPort>),
            backup_scan: crate::os_control::linux::providers::backup_scan::LiveBackupScan::discover().map(
                |transport| {
                    Arc::new(crate::os_control::backup::BackupScanControl::new(transport))
                        as Arc<dyn crate::os_control::backup::BackupScanControlPort>
                },
            ),
            firmware: crate::os_control::linux::providers::firmware_sensors::LiveFirmware::discover().map(
                |provider| {
                    Arc::new(provider) as Arc<dyn crate::os_control::hardware::FirmwareAwarenessPort>
                },
            ),
            hardware: crate::os_control::linux::providers::firmware_sensors::LiveHardwareSensors::discover()
                .map(|provider| {
                    Arc::new(provider) as Arc<dyn crate::os_control::hardware::HardwareControlPort>
                }),
            print_control: crate::os_control::linux::providers::cups_print::LivePrint::discover().map(
                |transport| {
                    Arc::new(crate::os_control::print::PrintControl::new(transport))
                        as Arc<dyn crate::os_control::print::PrintControlPort>
                },
            ),
            privacy: session_ok
                .then(
                    crate::os_control::linux::providers::privacy_firewall::LivePrivacy::discover,
                )
                .flatten()
                .map(|transport| {
                    Arc::new(crate::os_control::privacy::PrivacyControl::new(transport))
                        as Arc<dyn crate::os_control::privacy::PrivacyControlPort>
                }),
            firewall: crate::os_control::linux::providers::privacy_firewall::LiveFirewall::discover().map(
                |transport| {
                    Arc::new(crate::os_control::firewall::FirewallControl::new(transport))
                        as Arc<dyn crate::os_control::firewall::FirewallControlPort>
                },
            ),
            display_configuration: session_ok
                .then(
                    crate::os_control::linux::providers::display_config::LiveDisplayConfig::discover,
                )
                .flatten()
                .map(|transport| {
                    Arc::new(
                        crate::os_control::display::configuration::DisplayConfigControl::new(
                            transport,
                        ),
                    )
                        as Arc<
                            dyn crate::os_control::display::configuration::DisplayConfigControlPort,
                        >
                }),
            desktop_association: session_ok
                .then(xdg_config_root)
                .flatten()
                .and_then(|root| {
                    // Creating `autostart/` can fail (read-only home, no space).
                    // A failure leaves the domain uncomposed and answering
                    // `Unavailable`, rather than a provider that fails on first
                    // write after the user was told the action was available.
                    crate::os_control::applications::RealDesktopAssociationTransport::new(root).ok()
                })
                .map(|transport| {
                    Arc::new(crate::os_control::applications::DesktopAssociationControl::new(
                        transport,
                    ))
                        as Arc<dyn crate::os_control::applications::DesktopAssociationControlPort>
                }),
            automation: Some(Arc::new(crate::os_control::automation::AutomationControl::new(
                crate::os_control::linux::providers::automation::LiveAutomation::new(token),
            )) as Arc<dyn crate::os_control::automation::AutomationControlPort>),
            charge_thresholds: battery_present().then(|| {
                Arc::new(crate::os_control::power::charge::ChargeThresholdControl::new(
                    crate::os_control::power::charge::RealChargeThresholdTransport::new(
                        crate::os_control::broker::transport::LiveBrokerTransport::new(token),
                    ),
                ))
                    as Arc<dyn crate::os_control::power::charge::ChargeThresholdControlPort>
            }),
            snapshot,
        }
    }


    /// Which audio backends are installed on this host, in preference order.
    #[must_use]
    pub fn available_audio_backends() -> Vec<AudioBackend> {
        AudioBackend::PREFERENCE
            .into_iter()
            .filter(|backend| {
                backend
                    .trusted_executable()
                    .is_ok_and(|exe| executable_present(exe.path()))
            })
            .collect()
    }

    /// Which brightness backends are usable on this host.
    ///
    /// `GnomeSettingsDaemon` is a session D-Bus property rather than a binary, so
    /// it counts as present whenever a graphical session is advertised; the
    /// eligibility rule then decides whether it is actually selected.
    #[must_use]
    pub fn available_brightness_backends() -> Vec<BrightnessBackend> {
        BrightnessBackend::PREFERENCE
            .into_iter()
            .filter(|backend| match backend {
                BrightnessBackend::GnomeSettingsDaemon => graphical_session_advertised(),
                // `busctl` being installed is not enough: a desktop with no panel
                // exposes no backlight device, and offering the backend there would
                // advertise a capability that fails on first use.
                BrightnessBackend::LogindSession => {
                    backend
                        .trusted_executable()
                        .is_ok_and(|exe| executable_present(exe.path()))
                        && crate::os_control::display::selection::discover_backlight_device()
                            .is_some()
                }
                other => other
                    .trusted_executable()
                    .is_ok_and(|exe| executable_present(exe.path())),
            })
            .collect()
    }

    fn compose_audio(token: &LiveHostAccessToken) -> Option<Arc<dyn AudioControlPort>> {
        let backend = select_audio_backend(&Self::available_audio_backends())?;
        Some(Arc::new(AudioControl::new(LivePipewireAudio::new(
            token, backend,
        ))))
    }

    /// Compose brightness, honouring the display-server eligibility rule: XRandR
    /// gamma scaling is never selected in a native Wayland session (OSC-019.3 /
    /// OSC-032.3), because it dims the gamma ramp rather than the real backlight.
    fn compose_display(
        token: &LiveHostAccessToken,
        dbus: Option<&LiveDbusTransport>,
    ) -> Option<Arc<dyn DisplayControlPort>> {
        let display_server = LiveSessionProbe::env_from_process().display_server_hint();
        let backend =
            select_brightness_backend(display_server, &Self::available_brightness_backends())?;
        // The GNOME brightness property lives on the SESSION bus. Without the
        // probed connection the adapter has no bus to read it over and every
        // brightness read and write answers `Unavailable` — which is what happened
        // before this was threaded through.
        Some(Arc::new(DisplayControl::new(match dbus {
            Some(transport) => LiveGnomeDisplay::with_bus(token, backend, transport),
            // No probe ran (the synchronous `compose()` path). The adapter still
            // composes and fails closed on first use rather than silently
            // reporting a brightness nobody measured.
            None => LiveGnomeDisplay::new(token, backend),
        })))
    }

    /// Compose Trash over the user's XDG trash root, creating it if absent.
    fn compose_trash() -> Option<Arc<dyn TrashControlPort>> {
        let root = xdg_trash_root()?;
        match RealTrashTransport::new(&root) {
            Ok(transport) => Some(Arc::new(TrashControl::new(transport))),
            Err(error) => {
                tracing::warn!(
                    target: "authority_trace",
                    root = %root.display(),
                    error = %error,
                    "could not open the XDG trash root; trash operations report unavailable"
                );
                None
            }
        }
    }

    /// A compact report of which domains composed, for the startup log.
    #[must_use]
    pub fn composed_domains(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.search_control.is_some() {
            out.push("search");
        }
        if self.health.is_some() {
            out.push("health");
        }
        if self.backup_scan.is_some() {
            out.push("backup_scan");
        }
        if self.firmware.is_some() {
            out.push("firmware");
        }
        if self.hardware.is_some() {
            out.push("hardware");
        }
        if self.print_control.is_some() {
            out.push("print");
        }
        if self.privacy.is_some() {
            out.push("privacy");
        }
        if self.firewall.is_some() {
            out.push("firewall");
        }
        if self.display_configuration.is_some() {
            out.push("display_configuration");
        }
        if self.desktop_association.is_some() {
            out.push("desktop_association");
        }
        if self.automation.is_some() {
            out.push("automation");
        }
        if self.charge_thresholds.is_some() {
            out.push("charge_thresholds");
        }
        if self.audio.is_some() {
            out.push("audio");
        }
        if self.display.is_some() {
            out.push("display");
        }
        if self.connectivity.is_some() {
            out.push("connectivity");
        }
        if self.power.is_some() {
            out.push("power");
        }
        if self.power_session.is_some() {
            out.push("power_session");
        }
        if self.processes.is_some() {
            out.push("processes");
        }
        if self.application_close.is_some() {
            out.push("application_close");
        }
        if self.clipboard.is_some() {
            out.push("clipboard");
        }
        if self.notifications.is_some() {
            out.push("notifications");
        }
        if self.packages.is_some() {
            out.push("packages");
        }
        if self.storage.is_some() {
            out.push("storage");
        }
        if self.trash.is_some() {
            out.push("trash");
        }
        if self.archive.is_some() {
            out.push("archive");
        }
        if self.ownership.is_some() {
            out.push("ownership(broker-pending)");
        }
        if self.bluetooth.is_some() {
            out.push("bluetooth");
        }
        if self.secrets.is_some() {
            out.push("secrets");
        }
        if self.file_attributes.is_some() {
            out.push("file_attributes");
        }
        out
    }

    /// Whether an audio port was composed on this host.
    #[must_use]
    pub fn has_audio(&self) -> bool {
        self.audio.is_some()
    }

    /// Whether a brightness port was composed on this host.
    #[must_use]
    pub fn has_display(&self) -> bool {
        self.display.is_some()
    }

    /// Whether a process port was composed on this host.
    #[must_use]
    pub fn has_processes(&self) -> bool {
        self.processes.is_some()
    }
}

impl HostOsControl for LiveHostOsControl {
    fn search_control(&self) -> Option<&dyn crate::os_control::search::SearchControlPort> {
        self.search_control.as_deref()
    }

    fn health(&self) -> Option<&dyn crate::os_control::health::SystemHealthControlPort> {
        self.health.as_deref()
    }

    fn backup_scan(&self) -> Option<&dyn crate::os_control::backup::BackupScanControlPort> {
        self.backup_scan.as_deref()
    }

    fn firmware(&self) -> Option<&dyn crate::os_control::hardware::FirmwareAwarenessPort> {
        self.firmware.as_deref()
    }

    fn hardware(&self) -> Option<&dyn crate::os_control::hardware::HardwareControlPort> {
        self.hardware.as_deref()
    }

    fn print_control(&self) -> Option<&dyn crate::os_control::print::PrintControlPort> {
        self.print_control.as_deref()
    }

    fn privacy(&self) -> Option<&dyn crate::os_control::privacy::PrivacyControlPort> {
        self.privacy.as_deref()
    }

    fn firewall(&self) -> Option<&dyn crate::os_control::firewall::FirewallControlPort> {
        self.firewall.as_deref()
    }

    fn display_configuration(&self) -> Option<&dyn crate::os_control::display::configuration::DisplayConfigControlPort> {
        self.display_configuration.as_deref()
    }

    fn desktop_association(&self) -> Option<&dyn crate::os_control::applications::DesktopAssociationControlPort> {
        self.desktop_association.as_deref()
    }

    fn automation(&self) -> Option<&dyn crate::os_control::automation::AutomationControlPort> {
        self.automation.as_deref()
    }

    fn charge_thresholds(&self) -> Option<&dyn crate::os_control::power::charge::ChargeThresholdControlPort> {
        self.charge_thresholds.as_deref()
    }

    fn provider_id(&self) -> ProviderId {
        self.provider.clone()
    }

    fn capability_snapshot(&self) -> Option<&crate::os_control::capability::CapabilitySnapshot> {
        self.snapshot.as_ref()
    }

    fn audio(&self) -> Option<&dyn AudioControlPort> {
        self.audio.as_deref()
    }

    fn display(&self) -> Option<&dyn DisplayControlPort> {
        self.display.as_deref()
    }

    fn connectivity(&self) -> Option<&dyn ConnectivityControlPort> {
        self.connectivity.as_deref()
    }

    fn power(&self) -> Option<&dyn PowerControlPort> {
        self.power.as_deref()
    }

    fn power_session(&self) -> Option<&dyn PowerSessionControlPort> {
        self.power_session.as_deref()
    }

    fn processes(&self) -> Option<&dyn ProcessControlPort> {
        self.processes.as_deref()
    }

    fn application_close(&self) -> Option<&dyn ApplicationCloseControlPort> {
        self.application_close.as_deref()
    }

    fn clipboard(&self) -> Option<&dyn ClipboardControlPort> {
        self.clipboard.as_deref()
    }

    fn notifications(&self) -> Option<&dyn NotificationControlPort> {
        self.notifications.as_deref()
    }

    fn packages(&self) -> Option<&dyn PackageControlPort> {
        self.packages.as_deref()
    }

    fn storage(&self) -> Option<&dyn StorageControlPort> {
        self.storage.as_deref()
    }

    fn trash(&self) -> Option<&dyn TrashControlPort> {
        self.trash.as_deref()
    }

    fn archive(&self) -> Option<&dyn ArchiveControlPort> {
        self.archive.as_deref()
    }

    fn ownership(&self) -> Option<&dyn OwnershipControlPort> {
        self.ownership.as_deref()
    }

    fn bluetooth(&self) -> Option<&dyn BluetoothControlPort> {
        self.bluetooth.as_deref()
    }

    fn secrets(&self) -> Option<&dyn crate::os_control::secrets::CredentialStore> {
        self.secrets.as_deref()
    }

    fn file_attributes(
        &self,
    ) -> Option<&dyn crate::os_control::files::attributes::FileAttributeControlPort> {
        self.file_attributes.as_deref()
    }
}
