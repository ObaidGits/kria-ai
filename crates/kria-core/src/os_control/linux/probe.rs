//! [`SessionProbe`] implementations: the live D-Bus/binary/env probe and the
//! host-safe scripted probe matrix.
//!
//! linux-os-control-production **Task 1.3** (OSC-003, OSC-031, OSC-032, OSC-033),
//! design §§7, 8, 18.
//!
//! * [`LiveSessionProbe`] gathers a **bounded** set of capability facts from the
//!   live buses/binaries/environment behind the deny-live sentinel + live token
//!   ([`super::dbus`]), then answers the synchronous [`SessionProbe`] questions
//!   from the cached facts. It is live-only.
//! * [`ScriptedProbeMatrix`] is the test double every capability-probing
//!   completion test injects. It performs **zero** live access and encodes a
//!   deterministic fact matrix (GNOME Wayland/X11, KDE Wayland, absent bus,
//!   stale env, service restart, unknown future fields, timeouts).

use std::collections::{HashMap, HashSet};

use crate::os_control::capability::{
    BusKind, BusStatus, DesktopFamily, DisplayServer, EnvHints, ServiceOwner, SessionProbe,
};

// ─────────────────────────────────────────────────────────────────────────────
// Live probe (live-only; never reachable under os-control-test)
// ─────────────────────────────────────────────────────────────────────────────

/// A bounded plan describing which services/portals/binaries the live probe
/// should gather facts for. Supplied by the composition root so the probe only
/// performs bounded, deterministic work (OSC-034).
#[derive(Debug, Clone, Default)]
pub struct LiveProbePlan {
    /// `(bus, well-known service, object path)` triples to own-check + introspect.
    pub services: Vec<(BusKind, String, String)>,
    /// Portals to check for availability.
    pub portals: Vec<String>,
    /// Binaries to check for presence.
    pub binaries: Vec<String>,
}

/// Facts gathered once (asynchronously) from the live system and then served
/// synchronously. Everything here is bounded and control-char-free.
#[derive(Debug, Default, Clone)]
struct LiveFacts {
    session_bus: bool,
    system_bus: bool,
    owners: HashMap<(BusKind, String), ServiceOwner>,
    /// Introspection XML per `(bus, service)` (bounded), used for presence-only
    /// method/property checks that tolerate unknown additive members.
    introspection: HashMap<(BusKind, String), String>,
    portals: HashSet<String>,
    binaries: HashSet<String>,
    /// Bus names that are **activatable**: not currently owned, but D-Bus will
    /// start their service on first call. Treating these as absent would wrongly
    /// mark on-demand services (PackageKit, UDisks2) unavailable.
    activatable: HashSet<(BusKind, String)>,
    display_server: DisplayServer,
    desktop_family: DesktopFamily,
    xwayland: bool,
}

/// The live [`SessionProbe`]: caches bounded facts gathered behind the deny-live
/// transport seam and answers probe questions from that cache.
#[derive(Debug, Clone)]
pub struct LiveSessionProbe {
    env: EnvHints,
    facts: LiveFacts,
}

impl LiveSessionProbe {
    /// Whether `service` is owned **or** activatable on `bus`.
    ///
    /// This is the question a composition root actually cares about: an
    /// activatable service will start on first call, so refusing to compose its
    /// domain would disable a working capability.
    #[must_use]
    pub fn service_reachable(&self, bus: BusKind, service: &str) -> bool {
        self.facts.owners.contains_key(&(bus, service.to_string()))
            || self
                .facts
                .activatable
                .contains(&(bus, service.to_string()))
    }

    /// Read the raw session environment as **hints** (OSC-003.3). Reading env
    /// vars is not a transport, so this needs no live token.
    #[must_use]
    pub fn env_from_process() -> EnvHints {
        EnvHints::from_raw(
            std::env::var("XDG_SESSION_TYPE").ok(),
            std::env::var("WAYLAND_DISPLAY").ok(),
            std::env::var("DISPLAY").ok(),
            std::env::var("XDG_CURRENT_DESKTOP").ok(),
        )
    }

    /// Gather bounded facts from the live system, then build a probe that serves
    /// them synchronously.
    ///
    /// # Host safety
    /// The transport was opened behind the deny-live sentinel + live token
    /// ([`super::dbus::LiveDbusTransport`]); this method never opens a bus of its
    /// own. All D-Bus round trips are individually deadline-bounded (OSC-034).
    pub async fn probe(
        transport: &super::dbus::LiveDbusTransport,
        plan: &LiveProbePlan,
        per_call_timeout: std::time::Duration,
    ) -> Self {
        let env = Self::env_from_process();
        let mut facts = LiveFacts {
            session_bus: transport.is_connected(BusKind::Session),
            system_bus: transport.is_connected(BusKind::System),
            display_server: confirm_display_server_from_env(&env),
            desktop_family: env.desktop_family_hint(),
            xwayland: env.display.is_some(),
            ..Default::default()
        };

        // Gather activatable names once per bus so an on-demand service is not
        // mistaken for an absent one.
        for bus in [BusKind::Session, BusKind::System] {
            if let Some(conn) = transport.connection(bus) {
                for name in list_activatable_names(conn, per_call_timeout).await {
                    facts.activatable.insert((bus, name));
                }
            }
        }

        for (bus, service, path) in &plan.services {
            let Some(conn) = transport.connection(*bus) else {
                continue;
            };
            if let Some(owner) = get_name_owner(conn, service, per_call_timeout).await {
                facts.owners.insert((*bus, service.clone()), owner);
                if let Some(xml) = introspect(conn, service, path, per_call_timeout).await {
                    facts.introspection.insert((*bus, service.clone()), xml);
                }
            }
        }

        for portal in &plan.portals {
            // A portal is available when its bus name is owned on the session bus.
            if let Some(conn) = transport.connection(BusKind::Session) {
                if get_name_owner(conn, portal, per_call_timeout)
                    .await
                    .is_some()
                {
                    facts.portals.insert(portal.clone());
                }
            }
        }

        for binary in &plan.binaries {
            if which::which(binary).is_ok() {
                facts.binaries.insert(binary.clone());
            }
        }

        Self { env, facts }
    }
}

/// Confirm a display-server family from env hints. Deeper compositor-service
/// confirmation lands with the display providers (later tasks); this remains a
/// truthful, non-fabricating derivation and never forces access.
fn confirm_display_server_from_env(env: &EnvHints) -> DisplayServer {
    env.display_server_hint()
}

/// `org.freedesktop.DBus.GetNameOwner`, deadline-bounded. `None` on
/// timeout/absent.
async fn get_name_owner(
    conn: &zbus::Connection,
    service: &str,
    timeout: std::time::Duration,
) -> Option<ServiceOwner> {
    let fut = conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "GetNameOwner",
        &(service),
    );
    let reply = tokio::time::timeout(timeout, fut).await.ok()?.ok()?;
    let owner: String = reply.body().deserialize().ok()?;
    Some(ServiceOwner::new(owner))
}

/// `org.freedesktop.DBus.ListActivatableNames`, deadline-bounded.
///
/// A D-Bus **activatable** name has no current owner until something calls it, so
/// an own-check alone reports on-demand services (PackageKit, UDisks2) as absent.
/// Returns an empty set on timeout rather than failing the whole probe.
async fn list_activatable_names(
    conn: &zbus::Connection,
    timeout: std::time::Duration,
) -> Vec<String> {
    let fut = conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "ListActivatableNames",
        &(),
    );
    match tokio::time::timeout(timeout, fut).await {
        Ok(Ok(reply)) => reply.body().deserialize::<Vec<String>>().unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// `org.freedesktop.DBus.Introspectable.Introspect`, deadline-bounded, returning
/// the bounded XML for presence-only member checks.
async fn introspect(
    conn: &zbus::Connection,
    service: &str,
    path: &str,
    timeout: std::time::Duration,
) -> Option<String> {
    let fut = conn.call_method(
        Some(service),
        path,
        Some("org.freedesktop.DBus.Introspectable"),
        "Introspect",
        &(),
    );
    let reply = tokio::time::timeout(timeout, fut).await.ok()?.ok()?;
    let xml: String = reply.body().deserialize().ok()?;
    // Bound the retained XML so a hostile/huge introspection cannot blow memory.
    Some(xml.chars().take(64 * 1024).collect())
}

impl SessionProbe for LiveSessionProbe {
    fn env_hints(&self) -> EnvHints {
        self.env.clone()
    }

    fn bus_status(&self, bus: BusKind) -> BusStatus {
        let up = match bus {
            BusKind::Session => self.facts.session_bus,
            BusKind::System => self.facts.system_bus,
        };
        if up {
            BusStatus::Available
        } else {
            BusStatus::Unavailable
        }
    }

    fn service_owner(&self, bus: BusKind, service: &str) -> Option<ServiceOwner> {
        self.facts.owners.get(&(bus, service.to_string())).cloned()
    }

    fn has_method(&self, bus: BusKind, service: &str, interface: &str, method: &str) -> bool {
        introspection_has_member(
            self.facts.introspection.get(&(bus, service.to_string())),
            interface,
            "method",
            method,
        )
    }

    fn has_property(&self, bus: BusKind, service: &str, interface: &str, property: &str) -> bool {
        introspection_has_member(
            self.facts.introspection.get(&(bus, service.to_string())),
            interface,
            "property",
            property,
        )
    }

    fn portal_available(&self, portal: &str) -> bool {
        self.facts.portals.contains(portal)
    }

    fn confirmed_desktop_family(&self) -> DesktopFamily {
        self.facts.desktop_family
    }

    fn confirmed_display_server(&self) -> DisplayServer {
        self.facts.display_server
    }

    fn xwayland_available(&self) -> bool {
        self.facts.xwayland
    }

    fn binary_present(&self, binary: &str) -> bool {
        self.facts.binaries.contains(binary)
    }

    fn permission_granted(&self, _permission: &str) -> bool {
        // Runtime privilege (Polkit) is decided at dispatch, not at probe time;
        // the live probe reports no hard prerequisite permission by default.
        false
    }

    fn domain_timed_out(&self, _domain: &crate::os_control::capability::Domain) -> bool {
        // Per-call deadlines are applied during `probe`; a fact set that was
        // gathered is not itself "timed out".
        false
    }
}

/// Presence-only member check tolerant of unknown additive members and enum
/// values (OSC-031.3). We look for the interface block and a `<member name="…">`
/// entry within the bounded introspection XML without a full parser.
fn introspection_has_member(
    xml: Option<&String>,
    interface: &str,
    member_kind: &str,
    member: &str,
) -> bool {
    let Some(xml) = xml else {
        return false;
    };
    let iface_needle = format!("interface name=\"{interface}\"");
    let Some(iface_start) = xml.find(&iface_needle) else {
        return false;
    };
    // Bound the search to this interface block (up to the next interface tag).
    let rest = &xml[iface_start..];
    let block_end = rest[iface_needle.len()..]
        .find("interface name=\"")
        .map_or(rest.len(), |i| i + iface_needle.len());
    let block = &rest[..block_end];
    let member_needle = format!("{member_kind} name=\"{member}\"");
    block.contains(&member_needle)
}

// ─────────────────────────────────────────────────────────────────────────────
// Scripted probe matrix (os-control-test only; zero live access)
// ─────────────────────────────────────────────────────────────────────────────

/// A fully scripted [`SessionProbe`] for deny-live completion tests. It encodes
/// a deterministic fact matrix and performs no live access whatsoever.
#[cfg(feature = "os-control-test")]
#[derive(Debug)]
pub struct ScriptedProbeMatrix {
    env: EnvHints,
    display_server: DisplayServer,
    desktop_family: DesktopFamily,
    session_bus: bool,
    system_bus: bool,
    xwayland: bool,
    /// Present service owners, with interior mutability so a test can simulate a
    /// service restart (owner change) through a shared `&self`.
    owners: std::sync::Mutex<HashMap<(BusKind, String), ServiceOwner>>,
    methods: HashSet<(BusKind, String, String, String)>,
    properties: HashSet<(BusKind, String, String, String)>,
    portals: HashSet<String>,
    binaries: HashSet<String>,
    permissions: HashSet<String>,
    timed_out_domains: HashSet<String>,
}

#[cfg(feature = "os-control-test")]
impl ScriptedProbeMatrix {
    fn base(display_server: DisplayServer, desktop_family: DesktopFamily, env: EnvHints) -> Self {
        Self {
            env,
            display_server,
            desktop_family,
            session_bus: true,
            system_bus: true,
            xwayland: false,
            owners: std::sync::Mutex::new(HashMap::new()),
            methods: HashSet::new(),
            properties: HashSet::new(),
            portals: HashSet::new(),
            binaries: HashSet::new(),
            permissions: HashSet::new(),
            timed_out_domains: HashSet::new(),
        }
    }

    fn own(&mut self, bus: BusKind, service: &str, owner: &str) {
        self.owners
            .get_mut()
            .expect("matrix owners poisoned")
            .insert((bus, service.to_string()), ServiceOwner::new(owner));
    }

    fn add_method(&mut self, bus: BusKind, service: &str, iface: &str, method: &str) {
        self.methods.insert((
            bus,
            service.to_string(),
            iface.to_string(),
            method.to_string(),
        ));
    }

    fn add_property(&mut self, bus: BusKind, service: &str, iface: &str, property: &str) {
        self.properties.insert((
            bus,
            service.to_string(),
            iface.to_string(),
            property.to_string(),
        ));
    }

    /// Common freedesktop facts shared by every desktop matrix: system-bus
    /// logind + a session portal with a Settings property.
    fn with_common_services(mut self) -> Self {
        self.own(BusKind::System, "org.freedesktop.login1", ":1.10");
        self.add_method(
            BusKind::System,
            "org.freedesktop.login1",
            "org.freedesktop.login1.Manager",
            "Reboot",
        );
        self.own(BusKind::Session, "org.freedesktop.portal.Desktop", ":1.40");
        self.add_property(
            BusKind::Session,
            "org.freedesktop.portal.Desktop",
            "org.freedesktop.portal.Settings",
            "version",
        );
        self.portals
            .insert("org.freedesktop.portal.Desktop".to_string());
        self
    }

    /// GNOME on native Wayland (Mutter DisplayConfig present; XWayland present).
    #[must_use]
    pub fn gnome_wayland() -> Self {
        let env = EnvHints::from_raw(
            Some("wayland".to_string()),
            Some("wayland-0".to_string()),
            Some(":0".to_string()),
            Some("ubuntu:GNOME".to_string()),
        );
        let mut m =
            Self::base(DisplayServer::Wayland, DesktopFamily::Gnome, env).with_common_services();
        m.xwayland = true;
        m.own(BusKind::Session, "org.gnome.Mutter.DisplayConfig", ":1.60");
        m.add_method(
            BusKind::Session,
            "org.gnome.Mutter.DisplayConfig",
            "org.gnome.Mutter.DisplayConfig",
            "ApplyMonitorsConfig",
        );
        m
    }

    /// GNOME on X11 (Mutter present; XRandR binary available).
    #[must_use]
    pub fn gnome_x11() -> Self {
        let env = EnvHints::from_raw(
            Some("x11".to_string()),
            None,
            Some(":0".to_string()),
            Some("ubuntu:GNOME".to_string()),
        );
        let mut m =
            Self::base(DisplayServer::X11, DesktopFamily::Gnome, env).with_common_services();
        m.own(BusKind::Session, "org.gnome.Mutter.DisplayConfig", ":1.60");
        m.add_method(
            BusKind::Session,
            "org.gnome.Mutter.DisplayConfig",
            "org.gnome.Mutter.DisplayConfig",
            "ApplyMonitorsConfig",
        );
        m.binaries.insert("xrandr".to_string());
        m
    }

    /// KDE on native Wayland (KScreen present; Mutter absent).
    #[must_use]
    pub fn kde_wayland() -> Self {
        let env = EnvHints::from_raw(
            Some("wayland".to_string()),
            Some("wayland-0".to_string()),
            None,
            Some("KDE".to_string()),
        );
        let mut m =
            Self::base(DisplayServer::Wayland, DesktopFamily::Kde, env).with_common_services();
        m.own(BusKind::Session, "org.kde.KScreen", ":1.70");
        m
    }

    /// Override session-bus availability (absent-bus matrix).
    #[must_use]
    pub fn with_session_bus(mut self, available: bool) -> Self {
        self.session_bus = available;
        self
    }

    /// Override system-bus availability.
    #[must_use]
    pub fn with_system_bus(mut self, available: bool) -> Self {
        self.system_bus = available;
        self
    }

    /// Overlay stale Wayland env vars while leaving the confirmed session as-is
    /// (proves hints never fabricate access — OSC-003.3/OSC-032.7).
    #[must_use]
    pub fn with_stale_wayland_env(mut self) -> Self {
        self.env = EnvHints::from_raw(
            Some("wayland".to_string()),
            Some("wayland-1".to_string()),
            self.env.display.clone(),
            self.env.xdg_current_desktop.clone(),
        );
        self
    }

    /// Inject unknown additive interface members / an unknown enum-like owner to
    /// prove tolerance (OSC-031.3).
    #[must_use]
    pub fn with_unknown_future_fields(mut self) -> Self {
        self.add_method(
            BusKind::System,
            "org.freedesktop.login1",
            "org.freedesktop.login1.Manager",
            "FutureUnknownMethodV99",
        );
        self.add_property(
            BusKind::System,
            "org.freedesktop.login1",
            "org.freedesktop.login1.Manager",
            "FutureUnknownProperty",
        );
        self.own(
            BusKind::Session,
            "org.example.FutureUnknownService",
            ":1.255",
        );
        self
    }

    /// Mark a probe domain as timed out (degraded snapshot).
    #[must_use]
    pub fn with_timed_out_domain(mut self, domain: &str) -> Self {
        self.timed_out_domains.insert(domain.to_string());
        self
    }

    /// Grant a hard prerequisite permission.
    #[must_use]
    pub fn with_permission(mut self, permission: &str) -> Self {
        self.permissions.insert(permission.to_string());
        self
    }

    /// Simulate a service restart: the service stays present but its owner
    /// changes, which owner-change refresh must detect (OSC-003.5). Uses shared
    /// `&self` interior mutability so it can be called after the matrix is moved
    /// into a prober.
    pub fn restart_service(&self, bus: BusKind, service: &str) {
        let mut owners = self.owners.lock().expect("matrix owners poisoned");
        let key = (bus, service.to_string());
        // Deterministic new owner name distinct from the seeded owner.
        owners.insert(key, ServiceOwner::new(":1.900"));
    }

    /// Drop a service entirely (interface disappearance).
    pub fn drop_service(&self, bus: BusKind, service: &str) {
        let mut owners = self.owners.lock().expect("matrix owners poisoned");
        owners.remove(&(bus, service.to_string()));
    }
}

#[cfg(feature = "os-control-test")]
impl SessionProbe for ScriptedProbeMatrix {
    fn env_hints(&self) -> EnvHints {
        self.env.clone()
    }

    fn bus_status(&self, bus: BusKind) -> BusStatus {
        let up = match bus {
            BusKind::Session => self.session_bus,
            BusKind::System => self.system_bus,
        };
        if up {
            BusStatus::Available
        } else {
            BusStatus::Unavailable
        }
    }

    fn service_owner(&self, bus: BusKind, service: &str) -> Option<ServiceOwner> {
        // A service on an unavailable bus is not observable.
        if self.bus_status(bus) != BusStatus::Available {
            return None;
        }
        self.owners
            .lock()
            .expect("matrix owners poisoned")
            .get(&(bus, service.to_string()))
            .cloned()
    }

    fn has_method(&self, bus: BusKind, service: &str, interface: &str, method: &str) -> bool {
        self.methods.contains(&(
            bus,
            service.to_string(),
            interface.to_string(),
            method.to_string(),
        ))
    }

    fn has_property(&self, bus: BusKind, service: &str, interface: &str, property: &str) -> bool {
        self.properties.contains(&(
            bus,
            service.to_string(),
            interface.to_string(),
            property.to_string(),
        ))
    }

    fn portal_available(&self, portal: &str) -> bool {
        self.session_bus && self.portals.contains(portal)
    }

    fn confirmed_desktop_family(&self) -> DesktopFamily {
        self.desktop_family
    }

    fn confirmed_display_server(&self) -> DisplayServer {
        self.display_server
    }

    fn xwayland_available(&self) -> bool {
        self.xwayland
    }

    fn binary_present(&self, binary: &str) -> bool {
        self.binaries.contains(binary)
    }

    fn permission_granted(&self, permission: &str) -> bool {
        self.permissions.contains(permission)
    }

    fn domain_timed_out(&self, domain: &crate::os_control::capability::Domain) -> bool {
        self.timed_out_domains.contains(domain.as_str())
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn scripted_matrix_reports_no_service_on_absent_bus() {
        let m = ScriptedProbeMatrix::gnome_wayland().with_session_bus(false);
        assert_eq!(m.bus_status(BusKind::Session), BusStatus::Unavailable);
        assert!(m
            .service_owner(BusKind::Session, "org.freedesktop.portal.Desktop")
            .is_none());
        // System bus service still observable.
        assert!(m
            .service_owner(BusKind::System, "org.freedesktop.login1")
            .is_some());
    }

    #[test]
    fn introspection_member_check_tolerates_unknown_members() {
        let xml = r#"
            <node>
              <interface name="org.freedesktop.login1.Manager">
                <method name="Reboot"/>
                <method name="FutureUnknownMethodV99"/>
                <property name="Docked" type="b" access="read"/>
              </interface>
              <interface name="org.other.Thing">
                <method name="Nope"/>
              </interface>
            </node>"#
            .to_string();
        let facts = Some(&xml);
        assert!(introspection_has_member(
            facts,
            "org.freedesktop.login1.Manager",
            "method",
            "Reboot"
        ));
        // Unknown additive member is present but does not break lookups.
        assert!(introspection_has_member(
            facts,
            "org.freedesktop.login1.Manager",
            "method",
            "FutureUnknownMethodV99"
        ));
        // A method from another interface must not leak into this one.
        assert!(!introspection_has_member(
            facts,
            "org.freedesktop.login1.Manager",
            "method",
            "Nope"
        ));
    }

    #[test]
    fn restart_service_changes_owner_identity() {
        let m = ScriptedProbeMatrix::gnome_wayland();
        let before = m
            .service_owner(BusKind::System, "org.freedesktop.login1")
            .unwrap();
        m.restart_service(BusKind::System, "org.freedesktop.login1");
        let after = m
            .service_owner(BusKind::System, "org.freedesktop.login1")
            .unwrap();
        assert_ne!(before.as_str(), after.as_str());
    }
}
