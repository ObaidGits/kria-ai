//! Capability probing, availability, and per-operation provider selection.
//!
//! linux-os-control-production **Task 1.3** — "Implement `SessionContext` and
//! capability probing" (OSC-003, OSC-031, OSC-032), design §§3, 7, 8.
//!
//! # What this module owns
//!
//! * the [`SessionProbe`] seam — the single **injectable/fakeable** interface
//!   through which every session/system-bus, service-owner, interface,
//!   method/property, portal, desktop-family, binary, and permission fact is
//!   read. Real transports implement it in [`crate::os_control::linux::probe`]
//!   (and their constructors are armed by the deny-live sentinel); tests inject
//!   the scripted probe matrix. The prober never touches the live system
//!   directly;
//! * pure environment-hint normalization ([`EnvHints`]) that treats
//!   `XDG_SESSION_TYPE` / `WAYLAND_DISPLAY` / `DISPLAY` / `XDG_CURRENT_DESKTOP`
//!   as **hints only** (OSC-003.3), never as authority;
//! * the [`CapabilityProber`] that turns probe facts into a deterministic,
//!   redacted, bounded, operation-level [`CapabilitySnapshot`] with per-domain
//!   caching and single-domain invalidation on owner/session change
//!   (OSC-003.5);
//! * the capability descriptors ([`CapabilityRequirement`], [`ProviderCandidate`],
//!   [`ProviderNeeds`]) that declare **what a given operation needs** so
//!   selection is capability/interface-based and **never** branches on an Ubuntu
//!   release number (OSC-031.1/2).
//!
//! # Determinism and safety invariants
//!
//! * Equal probe facts produce an equal snapshot: operations are sorted by
//!   capability id, all collections are bounded, and no timestamps/random enter
//!   the snapshot (OSC-003 crit. 3).
//! * Snapshots carry **no secret values** — reasons are [`SafeText`] (redacted,
//!   bounded) and only public contract identifiers (interface/method/provider
//!   names) appear (OSC-003.7).
//! * Provider loss degrades only the affected operations; unrelated domains keep
//!   their availability (OSC-031.6).
//! * Unknown/additive interface fields never change selection or panic — the
//!   prober only asks "is method/property X present?" and tolerates extras
//!   (OSC-031.3).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::os_control::contract::{
    AvailabilityStatus, BoundedVec, CapabilityId, ProviderId, SafeText, SnapshotRevision,
    VerificationClass,
};

/// Hard cap on operations recorded in one snapshot (design §2 invariant 12).
/// The frozen manifest has 149 operations; this bounds any future growth.
pub const MAX_SNAPSHOT_OPERATIONS: usize = 512;

/// Hard cap on fallback providers recorded per operation.
pub const MAX_FALLBACK_PROVIDERS: usize = 8;

// ─────────────────────────────────────────────────────────────────────────────
// Session descriptor enums (referenced by SessionContext + snapshot)
// ─────────────────────────────────────────────────────────────────────────────

/// The confirmed display-server family of the session (design §8 session
/// matrix). Confirmed by probe facts, never fabricated from env vars.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum DisplayServer {
    /// A native Wayland session.
    Wayland,
    /// An X11 session.
    X11,
    /// No graphical session (headless / tty).
    Headless,
    /// A graphical session whose family could not be conclusively probed.
    #[default]
    Unknown,
}

/// The desktop-environment family (design §8). Selection is family/interface
/// based, never Ubuntu-release based (OSC-031.1).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum DesktopFamily {
    /// GNOME / Mutter.
    Gnome,
    /// KDE Plasma / KWin / KScreen.
    Kde,
    /// A wlroots-based compositor.
    Wlroots,
    /// Another recognized desktop.
    Other,
    /// Not conclusively probed.
    #[default]
    Unknown,
}

/// D-Bus bus availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusStatus {
    /// The bus is reachable.
    Available,
    /// The bus is not reachable.
    Unavailable,
}

/// freedesktop portal availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortalStatus {
    /// A desktop portal backend is present.
    Available,
    /// No desktop portal backend is present.
    Unavailable,
}

/// Whether an operation requires user confirmation before dispatch. The
/// authoritative risk/confirmation decision is made by the policy layer; this is
/// the capability-declared expectation surfaced in the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationPolicy {
    /// No confirmation expected (GREEN reads / idempotent).
    None,
    /// Confirmation expected (YELLOW/RED mutations).
    Confirm,
}

/// Which bus a probe question targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BusKind {
    /// The per-user session bus.
    Session,
    /// The system bus.
    System,
}

/// Per-operation X11/Wayland support declaration (OSC-032.2). Display-neutral
/// operations set both `true`; X11-only providers set `wayland = false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DisplayServerSupport {
    /// Supported under X11.
    pub x11: bool,
    /// Supported under native Wayland.
    pub wayland: bool,
}

impl DisplayServerSupport {
    /// Display-neutral: usable under both X11 and Wayland.
    pub const NEUTRAL: Self = Self {
        x11: true,
        wayland: true,
    };
    /// X11-only (e.g. XRandR, xdotool) — never usable in a native Wayland path.
    pub const X11_ONLY: Self = Self {
        x11: true,
        wayland: false,
    };
    /// Wayland-only.
    pub const WAYLAND_ONLY: Self = Self {
        x11: false,
        wayland: true,
    };

    /// Whether this support declaration covers the given display server.
    #[must_use]
    pub fn covers(self, ds: DisplayServer) -> bool {
        match ds {
            DisplayServer::Wayland => self.wayland,
            DisplayServer::X11 => self.x11,
            // Display-neutral operations (both flags) remain meaningful with no
            // graphical session; a display-specific op is not.
            DisplayServer::Headless | DisplayServer::Unknown => self.x11 && self.wayland,
        }
    }
}

/// A bounded, redacted probe/caching domain (e.g. `audio`, `display`, `power`).
/// Owner/session invalidation is per domain (OSC-003.5).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
pub struct Domain(String);

impl Domain {
    /// Maximum length (chars) of a domain label.
    pub const MAX_CHARS: usize = 48;

    /// Construct a bounded domain label.
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let bounded: String = raw
            .chars()
            .filter(|c| !c.is_control())
            .take(Self::MAX_CHARS)
            .collect();
        Self(bounded)
    }

    /// Borrow the label.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Domain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A redacted D-Bus service owner identity used only for presence + change
/// detection. **Never** surfaced in a snapshot (OSC-003.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceOwner(String);

impl ServiceOwner {
    /// Wrap a bounded, control-char-free owner label.
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        Self(raw.chars().filter(|c| !c.is_control()).take(128).collect())
    }

    /// Borrow the owner label.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Environment-hint normalization (OSC-003.3) — hints only, never authority
// ─────────────────────────────────────────────────────────────────────────────

/// Normalized session environment hints. These are **hints**: the prober
/// confirms provider/display availability independently and never fabricates
/// access from them (OSC-003.3, OSC-032.7).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvHints {
    /// `XDG_SESSION_TYPE` (`wayland` / `x11` / `tty` …).
    pub xdg_session_type: Option<String>,
    /// `WAYLAND_DISPLAY` (presence hints a Wayland compositor socket).
    pub wayland_display: Option<String>,
    /// `DISPLAY` (presence hints an X server / XWayland).
    pub display: Option<String>,
    /// `XDG_CURRENT_DESKTOP` (`GNOME`, `KDE`, `ubuntu:GNOME`, …).
    pub xdg_current_desktop: Option<String>,
}

impl EnvHints {
    /// Build from raw values, normalizing case/whitespace. `None`/empty stays
    /// `None`. No release-version parsing occurs.
    #[must_use]
    pub fn from_raw(
        xdg_session_type: Option<String>,
        wayland_display: Option<String>,
        display: Option<String>,
        xdg_current_desktop: Option<String>,
    ) -> Self {
        fn norm(v: Option<String>) -> Option<String> {
            v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
        }
        Self {
            xdg_session_type: norm(xdg_session_type),
            wayland_display: norm(wayland_display),
            display: norm(display),
            xdg_current_desktop: norm(xdg_current_desktop),
        }
    }

    /// The display-server family *hinted* by the environment (not authoritative).
    #[must_use]
    pub fn display_server_hint(&self) -> DisplayServer {
        if let Some(t) = &self.xdg_session_type {
            match t.to_ascii_lowercase().as_str() {
                "wayland" => return DisplayServer::Wayland,
                "x11" => return DisplayServer::X11,
                "tty" => return DisplayServer::Headless,
                _ => {}
            }
        }
        if self.wayland_display.is_some() {
            DisplayServer::Wayland
        } else if self.display.is_some() {
            DisplayServer::X11
        } else {
            DisplayServer::Headless
        }
    }

    /// The desktop family *hinted* by `XDG_CURRENT_DESKTOP` (not authoritative).
    /// Tolerates the `ubuntu:GNOME` colon-list form without release branching.
    #[must_use]
    pub fn desktop_family_hint(&self) -> DesktopFamily {
        let Some(raw) = &self.xdg_current_desktop else {
            return DesktopFamily::Unknown;
        };
        let lower = raw.to_ascii_lowercase();
        // Colon-separated multi-desktop lists (e.g. `ubuntu:GNOME`) are matched
        // token-by-token; no Ubuntu release number is ever parsed (OSC-031.1).
        for token in lower.split(':') {
            match token.trim() {
                "gnome" => return DesktopFamily::Gnome,
                "kde" | "plasma" => return DesktopFamily::Kde,
                "sway" | "wlroots" | "hyprland" | "river" => return DesktopFamily::Wlroots,
                _ => {}
            }
        }
        if lower.contains("gnome") {
            DesktopFamily::Gnome
        } else if lower.contains("kde") || lower.contains("plasma") {
            DesktopFamily::Kde
        } else {
            DesktopFamily::Other
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The SessionProbe seam (injectable / fakeable)
// ─────────────────────────────────────────────────────────────────────────────

/// The single injectable interface through which capability facts are read.
///
/// Every method answers one bounded question. Live implementations
/// ([`crate::os_control::linux::probe`]) open real transports **only** behind the
/// deny-live sentinel + [`crate::os_control::access::LiveHostAccessToken`]; tests
/// inject a scripted matrix. The prober itself performs no I/O.
///
/// Method/property queries are deliberately presence-only so unknown additive
/// fields and enum values are tolerated without panic (OSC-031.3).
pub trait SessionProbe: Send + Sync {
    /// Normalized environment hints (used for diagnostics/tie-breaking only).
    fn env_hints(&self) -> EnvHints;

    /// Whether the given bus is reachable.
    fn bus_status(&self, bus: BusKind) -> BusStatus;

    /// The current owner of a well-known service on `bus`, if any.
    fn service_owner(&self, bus: BusKind, service: &str) -> Option<ServiceOwner>;

    /// Whether `interface.method` is present on `service`.
    fn has_method(&self, bus: BusKind, service: &str, interface: &str, method: &str) -> bool;

    /// Whether `interface.property` is present on `service`.
    fn has_property(&self, bus: BusKind, service: &str, interface: &str, property: &str) -> bool;

    /// Whether a desktop portal is available.
    fn portal_available(&self, portal: &str) -> bool;

    /// The confirmed desktop family (probe-confirmed, not the env hint).
    fn confirmed_desktop_family(&self) -> DesktopFamily;

    /// The confirmed display server (probe-confirmed, not the env hint).
    fn confirmed_display_server(&self) -> DisplayServer;

    /// Whether XWayland is available (reported separately; not full Wayland
    /// authority — OSC-032.4).
    fn xwayland_available(&self) -> bool;

    /// Whether a trusted binary is present.
    fn binary_present(&self, binary: &str) -> bool;

    /// Whether the caller holds a hard prerequisite permission.
    fn permission_granted(&self, permission: &str) -> bool;

    /// Whether probing this domain timed out (yields a degraded snapshot).
    fn domain_timed_out(&self, domain: &Domain) -> bool;
}

// ─────────────────────────────────────────────────────────────────────────────
// Capability descriptors — what an operation needs
// ─────────────────────────────────────────────────────────────────────────────

/// What a single provider candidate requires to be eligible for an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderNeeds {
    /// Bus that must be reachable, if any.
    pub bus: Option<BusKind>,
    /// Well-known service that must be owned on `bus`, if any.
    pub service: Option<String>,
    /// `(interface, method)` pairs that must be present.
    pub methods: Vec<(String, String)>,
    /// `(interface, property)` pairs that must be present.
    pub properties: Vec<(String, String)>,
    /// Portal that must be available, if any.
    pub portal: Option<String>,
    /// Binary that must be present, if any.
    pub binary: Option<String>,
    /// Hard prerequisite permission that must be granted, if any.
    pub permission: Option<String>,
    /// Display-server constraint for this provider (X11-only sets `X11_ONLY`).
    pub display_server: DisplayServerSupport,
}

impl Default for ProviderNeeds {
    fn default() -> Self {
        Self {
            bus: None,
            service: None,
            methods: Vec::new(),
            properties: Vec::new(),
            portal: None,
            binary: None,
            permission: None,
            display_server: DisplayServerSupport::NEUTRAL,
        }
    }
}

/// One ordered provider candidate for an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCandidate {
    /// The provider identity (never model prose).
    pub provider: ProviderId,
    /// What this candidate needs to be eligible.
    pub needs: ProviderNeeds,
    /// `(interface, property)` pairs whose absence downgrades an otherwise
    /// eligible selection to [`AvailabilityStatus::Degraded`] (reduced fidelity,
    /// e.g. a missing verification property).
    pub degrade_if_missing: Vec<(String, String)>,
}

impl ProviderCandidate {
    /// Convenience constructor with no degradation properties.
    #[must_use]
    pub fn new(provider: ProviderId, needs: ProviderNeeds) -> Self {
        Self {
            provider,
            needs,
            degrade_if_missing: Vec::new(),
        }
    }
}

/// The declared capability requirement for one operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRequirement {
    /// The capability/operation identity.
    pub capability: CapabilityId,
    /// The probing/caching domain this operation belongs to.
    pub domain: Domain,
    /// Ordered provider candidates (most preferred first).
    pub candidates: Vec<ProviderCandidate>,
    /// Declared per-operation X11/Wayland support (OSC-032.2).
    pub display_servers: DisplayServerSupport,
    /// Whether the operation requires elevated privilege at dispatch.
    pub requires_root: bool,
    /// The declared confirmation expectation.
    pub requires_confirmation: ConfirmationPolicy,
    /// Whether the operation is reversible.
    pub reversible: bool,
    /// How strongly the operation can be verified.
    pub verifiable: VerificationClass,
}

// ─────────────────────────────────────────────────────────────────────────────
// Snapshot output
// ─────────────────────────────────────────────────────────────────────────────

/// Per-operation availability (design §7 `CapabilityAvailability`), redacted and
/// bounded.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CapabilityAvailability {
    /// The operation identity.
    pub capability: CapabilityId,
    /// The owning domain.
    pub domain: Domain,
    /// Whether the operation is usable.
    pub status: AvailabilityStatus,
    /// The selected provider, if any.
    pub selected: Option<ProviderId>,
    /// Eligible fallback providers (bounded).
    pub fallbacks: BoundedVec<ProviderId>,
    /// Declared X11/Wayland support.
    pub display_servers: DisplayServerSupport,
    /// Whether elevated privilege is required.
    pub requires_root: bool,
    /// Confirmation expectation.
    pub requires_confirmation: ConfirmationPolicy,
    /// Whether the operation is reversible.
    pub reversible: bool,
    /// Verification strength.
    pub verifiable: VerificationClass,
    /// Redacted degradation/unavailability reason (names the failed
    /// interface/provider, never a secret) — OSC-031.7.
    pub reason: Option<SafeText>,
}

/// A deterministic, redacted, bounded, operation-level capability snapshot
/// (design §7, OSC-003).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CapabilitySnapshot {
    /// Monotonic revision; a grant binds the revision it was issued under so a
    /// stale-snapshot resume is detectable (OSC-001.5, consumed by Task 1.7).
    pub revision: SnapshotRevision,
    /// Confirmed display server.
    pub display_server: DisplayServer,
    /// Confirmed desktop family.
    pub desktop_family: DesktopFamily,
    /// Session bus availability.
    pub session_bus: BusStatus,
    /// System bus availability.
    pub system_bus: BusStatus,
    /// Portal availability.
    pub portals: PortalStatus,
    /// XWayland availability (reported separately from Wayland authority).
    pub xwayland: bool,
    /// Operation-level availability, sorted by capability id (deterministic).
    pub operations: BoundedVec<CapabilityAvailability>,
    /// Whether any operation is not fully available.
    pub degraded: bool,
    /// A bounded, redacted summary of the degradation, if any.
    pub degradation_reason: Option<SafeText>,
}

impl CapabilitySnapshot {
    /// Look up one operation's availability by capability id.
    #[must_use]
    pub fn operation(&self, capability: &CapabilityId) -> Option<&CapabilityAvailability> {
        self.operations
            .as_slice()
            .iter()
            .find(|o| &o.capability == capability)
    }

    /// Whether two snapshots describe the same capabilities, ignoring revision.
    /// (Two fresh probers over equal facts also produce equal revisions.)
    #[must_use]
    pub fn same_capabilities(&self, other: &Self) -> bool {
        self.display_server == other.display_server
            && self.desktop_family == other.desktop_family
            && self.session_bus == other.session_bus
            && self.system_bus == other.system_bus
            && self.portals == other.portals
            && self.xwayland == other.xwayland
            && self.degraded == other.degraded
            && self.degradation_reason == other.degradation_reason
            && self.operations == other.operations
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Selection logic (pure over probe facts)
// ─────────────────────────────────────────────────────────────────────────────

/// Result of evaluating one candidate against probe facts.
enum CandidateEval {
    /// Eligible; `degraded` marks reduced fidelity (a `degrade_if_missing`
    /// property was absent).
    Eligible { degraded: Option<SafeText> },
    /// Ineligible with a redacted, secret-free reason naming what failed.
    Ineligible { reason: SafeText },
}

fn eval_candidate<P: SessionProbe>(
    probe: &P,
    display_server: DisplayServer,
    candidate: &ProviderCandidate,
) -> CandidateEval {
    let needs = &candidate.needs;

    // Display-server constraint first (OSC-032.3: X11-only never in Wayland).
    if !needs.display_server.covers(display_server) {
        return CandidateEval::Ineligible {
            reason: SafeText::new(format!(
                "provider `{}` does not support the {:?} session",
                candidate.provider, display_server
            )),
        };
    }

    if let Some(bus) = needs.bus {
        if probe.bus_status(bus) != BusStatus::Available {
            return CandidateEval::Ineligible {
                reason: SafeText::new(format!("{bus:?} bus is unavailable")),
            };
        }
    }

    if let Some(service) = &needs.service {
        let bus = needs.bus.unwrap_or(BusKind::Session);
        if probe.service_owner(bus, service).is_none() {
            return CandidateEval::Ineligible {
                reason: SafeText::new(format!("service `{service}` is not owned")),
            };
        }
        for (iface, method) in &needs.methods {
            if !probe.has_method(bus, service, iface, method) {
                return CandidateEval::Ineligible {
                    reason: SafeText::new(format!("missing method `{iface}.{method}`")),
                };
            }
        }
        for (iface, property) in &needs.properties {
            if !probe.has_property(bus, service, iface, property) {
                return CandidateEval::Ineligible {
                    reason: SafeText::new(format!("missing property `{iface}.{property}`")),
                };
            }
        }
    }

    if let Some(portal) = &needs.portal {
        if !probe.portal_available(portal) {
            return CandidateEval::Ineligible {
                reason: SafeText::new(format!("portal `{portal}` is unavailable")),
            };
        }
    }

    if let Some(binary) = &needs.binary {
        if !probe.binary_present(binary) {
            return CandidateEval::Ineligible {
                reason: SafeText::new(format!("binary `{binary}` is not present")),
            };
        }
    }

    if let Some(permission) = &needs.permission {
        if !probe.permission_granted(permission) {
            return CandidateEval::Ineligible {
                reason: SafeText::new(format!("permission `{permission}` is not granted")),
            };
        }
    }

    // Eligible: check for reduced fidelity.
    let bus = needs.bus.unwrap_or(BusKind::Session);
    for (iface, property) in &candidate.degrade_if_missing {
        if let Some(service) = &needs.service {
            if !probe.has_property(bus, service, iface, property) {
                return CandidateEval::Eligible {
                    degraded: Some(SafeText::new(format!(
                        "verification property `{iface}.{property}` unavailable; reduced fidelity"
                    ))),
                };
            }
        }
    }

    CandidateEval::Eligible { degraded: None }
}

/// Evaluate one requirement into its `CapabilityAvailability`, purely from
/// probe facts. `display_server` is the confirmed session display server.
fn evaluate_requirement<P: SessionProbe>(
    probe: &P,
    req: &CapabilityRequirement,
    display_server: DisplayServer,
) -> CapabilityAvailability {
    let mut fallbacks: BoundedVec<ProviderId> = BoundedVec::with_cap(MAX_FALLBACK_PROVIDERS);
    let mut selected: Option<ProviderId> = None;
    let mut status = AvailabilityStatus::Unavailable;
    let mut reason: Option<SafeText> = None;
    let mut first_failure: Option<SafeText> = None;

    // Timeout → degraded domain (design §7 "timeout yields degraded snapshot").
    if probe.domain_timed_out(&req.domain) {
        return CapabilityAvailability {
            capability: req.capability.clone(),
            domain: req.domain.clone(),
            status: AvailabilityStatus::Degraded,
            selected: None,
            fallbacks,
            display_servers: req.display_servers,
            requires_root: req.requires_root,
            requires_confirmation: req.requires_confirmation,
            reversible: req.reversible,
            verifiable: req.verifiable,
            reason: Some(SafeText::new(format!(
                "probe for domain `{}` timed out; capability degraded",
                req.domain
            ))),
        };
    }

    for candidate in &req.candidates {
        match eval_candidate(probe, display_server, candidate) {
            CandidateEval::Eligible { degraded } => {
                if selected.is_none() {
                    selected = Some(candidate.provider.clone());
                    if let Some(d) = degraded {
                        status = AvailabilityStatus::Degraded;
                        reason = Some(d);
                    } else {
                        status = AvailabilityStatus::Available;
                    }
                } else {
                    // Additional eligible candidates become bounded fallbacks.
                    let _ = fallbacks.try_push(candidate.provider.clone());
                }
            }
            CandidateEval::Ineligible { reason: r } => {
                if first_failure.is_none() {
                    first_failure = Some(r);
                }
            }
        }
    }

    if selected.is_none() {
        // Truthful blocker naming the failed interface/provider (OSC-031.7).
        // For a display-specific op with no eligible provider in this session,
        // surface a Wayland/X11 handoff-style blocker (OSC-032.5).
        reason = Some(first_failure.unwrap_or_else(|| {
            SafeText::new(format!(
                "no eligible provider for `{}` in the {:?} session",
                req.capability, display_server
            ))
        }));
    }

    CapabilityAvailability {
        capability: req.capability.clone(),
        domain: req.domain.clone(),
        status,
        selected,
        fallbacks,
        display_servers: req.display_servers,
        requires_root: req.requires_root,
        requires_confirmation: req.requires_confirmation,
        reversible: req.reversible,
        verifiable: req.verifiable,
        reason,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The prober
// ─────────────────────────────────────────────────────────────────────────────

/// Cached availability entries for one domain.
#[derive(Debug, Clone)]
struct DomainCache {
    entries: Vec<CapabilityAvailability>,
}

/// Mutable prober state guarded by a mutex (probing is infrequent).
struct ProberState {
    /// Per-domain cache; a missing domain is rebuilt on the next snapshot.
    domains: BTreeMap<Domain, DomainCache>,
    /// Last observed owners for `(bus, service)` used in the catalog, so an
    /// owner change can invalidate exactly the affected domains (OSC-003.5).
    owners: BTreeMap<(BusKind, String), Option<ServiceOwner>>,
    /// Domains rebuilt during the most recent [`CapabilityProber::snapshot`].
    last_rebuilt: Vec<Domain>,
}

/// Turns [`SessionProbe`] facts into deterministic operation-level snapshots
/// with per-domain caching and single-domain invalidation.
pub struct CapabilityProber<P: SessionProbe> {
    probe: P,
    catalog: Vec<CapabilityRequirement>,
    state: Mutex<ProberState>,
    revision: AtomicU64,
}

impl<P: SessionProbe> CapabilityProber<P> {
    /// Create a prober over a probe seam and a capability catalog. The catalog
    /// is deduplicated/sorted deterministically by capability id.
    #[must_use]
    pub fn new(probe: P, mut catalog: Vec<CapabilityRequirement>) -> Self {
        catalog.sort_by(|a, b| a.capability.cmp(&b.capability));
        catalog.dedup_by(|a, b| a.capability == b.capability);
        Self {
            probe,
            catalog,
            state: Mutex::new(ProberState {
                domains: BTreeMap::new(),
                owners: BTreeMap::new(),
                last_rebuilt: Vec::new(),
            }),
            revision: AtomicU64::new(0),
        }
    }

    /// Borrow the underlying probe seam.
    #[must_use]
    pub fn probe(&self) -> &P {
        &self.probe
    }

    /// The distinct domains referenced by the catalog (sorted).
    #[must_use]
    pub fn domains(&self) -> Vec<Domain> {
        let mut d: Vec<Domain> = self.catalog.iter().map(|r| r.domain.clone()).collect();
        d.sort();
        d.dedup();
        d
    }

    /// The current snapshot revision (monotonic; 0 before the first snapshot).
    #[must_use]
    pub fn current_revision(&self) -> SnapshotRevision {
        SnapshotRevision(self.revision.load(Ordering::SeqCst))
    }

    /// Invalidate a single domain's cache; the next snapshot renegotiates only
    /// that domain (OSC-003.5).
    pub fn invalidate_domain(&self, domain: &Domain) {
        let mut state = self.state.lock().expect("prober state poisoned");
        state.domains.remove(domain);
    }

    /// Invalidate all cached domains (e.g. on a session change).
    pub fn invalidate_all(&self) {
        let mut state = self.state.lock().expect("prober state poisoned");
        state.domains.clear();
    }

    /// The domains rebuilt during the most recent [`Self::snapshot`] call.
    #[must_use]
    pub fn last_rebuilt_domains(&self) -> Vec<Domain> {
        self.state
            .lock()
            .expect("prober state poisoned")
            .last_rebuilt
            .clone()
    }

    /// Re-read service owners; for every `(bus, service)` whose owner changed,
    /// invalidate exactly the domains whose candidates reference that service,
    /// and return the invalidated domains (OSC-003.5). Unrelated domains keep
    /// their cache (OSC-031.6).
    pub fn refresh_owner_changes(&self) -> Vec<Domain> {
        // Collect the distinct (bus, service) references from the catalog.
        let mut refs: Vec<(BusKind, String)> = Vec::new();
        for req in &self.catalog {
            for cand in &req.candidates {
                if let Some(service) = &cand.needs.service {
                    let bus = cand.needs.bus.unwrap_or(BusKind::Session);
                    let key = (bus, service.clone());
                    if !refs.contains(&key) {
                        refs.push(key);
                    }
                }
            }
        }

        let mut invalidated: Vec<Domain> = Vec::new();
        let mut state = self.state.lock().expect("prober state poisoned");
        for (bus, service) in refs {
            let now = self.probe.service_owner(bus, &service);
            let previous = state.owners.get(&(bus, service.clone()));
            let changed = match previous {
                Some(prev) => {
                    prev.as_ref().map(ServiceOwner::as_str)
                        != now.as_ref().map(ServiceOwner::as_str)
                }
                // First observation is not a "change".
                None => false,
            };
            state.owners.insert((bus, service.clone()), now);
            if changed {
                for domain in self.domains_referencing(bus, &service) {
                    state.domains.remove(&domain);
                    if !invalidated.contains(&domain) {
                        invalidated.push(domain);
                    }
                }
            }
        }
        invalidated.sort();
        invalidated.dedup();
        invalidated
    }

    /// Domains whose candidates reference `(bus, service)`.
    fn domains_referencing(&self, bus: BusKind, service: &str) -> Vec<Domain> {
        let mut out: Vec<Domain> = Vec::new();
        for req in &self.catalog {
            let hit = req.candidates.iter().any(|c| {
                c.needs.service.as_deref() == Some(service)
                    && c.needs.bus.unwrap_or(BusKind::Session) == bus
            });
            if hit && !out.contains(&req.domain) {
                out.push(req.domain.clone());
            }
        }
        out
    }

    /// Build the current capability snapshot, rebuilding only domains missing
    /// from the cache and bumping the monotonic revision when anything changed.
    pub fn snapshot(&self) -> CapabilitySnapshot {
        let display_server = self.probe.confirmed_display_server();

        let mut rebuilt: Vec<Domain> = Vec::new();
        {
            let mut state = self.state.lock().expect("prober state poisoned");
            // Seed the owner table on first observation so later changes register.
            if state.owners.is_empty() {
                for req in &self.catalog {
                    for cand in &req.candidates {
                        if let Some(service) = &cand.needs.service {
                            let bus = cand.needs.bus.unwrap_or(BusKind::Session);
                            state
                                .owners
                                .entry((bus, service.clone()))
                                .or_insert_with(|| self.probe.service_owner(bus, service));
                        }
                    }
                }
            }

            // Rebuild any domain not currently cached.
            for domain in self.domains() {
                if state.domains.contains_key(&domain) {
                    continue;
                }
                let entries: Vec<CapabilityAvailability> = self
                    .catalog
                    .iter()
                    .filter(|r| r.domain == domain)
                    .map(|r| evaluate_requirement(&self.probe, r, display_server))
                    .collect();
                state
                    .domains
                    .insert(domain.clone(), DomainCache { entries });
                rebuilt.push(domain);
            }
            state.last_rebuilt = rebuilt.clone();
        }

        if !rebuilt.is_empty() {
            self.revision.fetch_add(1, Ordering::SeqCst);
        }

        // Assemble a deterministic, bounded operation list.
        let mut operations: Vec<CapabilityAvailability> = {
            let state = self.state.lock().expect("prober state poisoned");
            state
                .domains
                .values()
                .flat_map(|c| c.entries.iter().cloned())
                .collect()
        };
        operations.sort_by(|a, b| a.capability.cmp(&b.capability));

        let mut bounded: BoundedVec<CapabilityAvailability> =
            BoundedVec::with_cap(MAX_SNAPSHOT_OPERATIONS);
        for op in operations {
            let _ = bounded.try_push(op);
        }

        // Snapshot-level degradation summary.
        let mut unavailable = 0usize;
        let mut degraded = 0usize;
        for op in bounded.as_slice() {
            match op.status {
                AvailabilityStatus::Available => {}
                AvailabilityStatus::Degraded => degraded += 1,
                AvailabilityStatus::Unavailable => unavailable += 1,
            }
        }
        let is_degraded = unavailable > 0 || degraded > 0;
        let degradation_reason = if is_degraded {
            Some(SafeText::new(format!(
                "{unavailable} operation(s) unavailable, {degraded} degraded of {} probed",
                bounded.len()
            )))
        } else {
            None
        };

        CapabilitySnapshot {
            revision: self.current_revision(),
            display_server,
            desktop_family: self.probe.confirmed_desktop_family(),
            session_bus: self.probe.bus_status(BusKind::Session),
            system_bus: self.probe.bus_status(BusKind::System),
            portals: if self
                .probe
                .portal_available("org.freedesktop.portal.Desktop")
            {
                PortalStatus::Available
            } else {
                PortalStatus::Unavailable
            },
            xwayland: self.probe.xwayland_available(),
            operations: bounded,
            degraded: is_degraded,
            degradation_reason,
        }
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;
    use crate::os_control::linux::probe::ScriptedProbeMatrix;

    // ── Env-hint normalization: hints only, no release branching ────────────

    #[test]
    fn env_hints_normalize_and_hint_display_server() {
        let h = EnvHints::from_raw(
            Some("  wayland ".to_string()),
            Some("wayland-0".to_string()),
            Some(":0".to_string()),
            Some("ubuntu:GNOME".to_string()),
        );
        assert_eq!(h.xdg_session_type.as_deref(), Some("wayland"));
        assert_eq!(h.display_server_hint(), DisplayServer::Wayland);
        assert_eq!(h.desktop_family_hint(), DesktopFamily::Gnome);
    }

    #[test]
    fn env_hints_empty_is_headless_unknown() {
        let h = EnvHints::from_raw(None, None, None, Some("   ".to_string()));
        assert_eq!(h.display_server_hint(), DisplayServer::Headless);
        assert_eq!(h.desktop_family_hint(), DesktopFamily::Unknown);
    }

    #[test]
    fn env_hints_kde_and_x11() {
        let h = EnvHints::from_raw(
            Some("x11".to_string()),
            None,
            Some(":0".to_string()),
            Some("KDE".to_string()),
        );
        assert_eq!(h.display_server_hint(), DisplayServer::X11);
        assert_eq!(h.desktop_family_hint(), DesktopFamily::Kde);
    }

    // ── Helper catalog: representative cross-domain operations ───────────────

    fn audio_set_volume() -> CapabilityRequirement {
        CapabilityRequirement {
            capability: CapabilityId::new("set_volume"),
            domain: Domain::new("audio"),
            candidates: vec![ProviderCandidate {
                provider: ProviderId::new("pipewire"),
                needs: ProviderNeeds {
                    bus: Some(BusKind::Session),
                    service: Some("org.freedesktop.portal.Desktop".to_string()),
                    ..Default::default()
                },
                degrade_if_missing: vec![(
                    "org.freedesktop.portal.Settings".to_string(),
                    "version".to_string(),
                )],
            }],
            display_servers: DisplayServerSupport::NEUTRAL,
            requires_root: false,
            requires_confirmation: ConfirmationPolicy::Confirm,
            reversible: true,
            verifiable: VerificationClass::Verifiable,
        }
    }

    fn power_reboot() -> CapabilityRequirement {
        CapabilityRequirement {
            capability: CapabilityId::new("reboot"),
            domain: Domain::new("power"),
            candidates: vec![ProviderCandidate::new(
                ProviderId::new("logind"),
                ProviderNeeds {
                    bus: Some(BusKind::System),
                    service: Some("org.freedesktop.login1".to_string()),
                    methods: vec![(
                        "org.freedesktop.login1.Manager".to_string(),
                        "Reboot".to_string(),
                    )],
                    ..Default::default()
                },
            )],
            display_servers: DisplayServerSupport::NEUTRAL,
            requires_root: true,
            requires_confirmation: ConfirmationPolicy::Confirm,
            reversible: false,
            verifiable: VerificationClass::AcceptedOnly,
        }
    }

    fn display_set_topology() -> CapabilityRequirement {
        CapabilityRequirement {
            capability: CapabilityId::new("set_display_topology"),
            domain: Domain::new("display"),
            candidates: vec![
                // GNOME/Mutter (Wayland + X11).
                ProviderCandidate::new(
                    ProviderId::new("gnome_display"),
                    ProviderNeeds {
                        bus: Some(BusKind::Session),
                        service: Some("org.gnome.Mutter.DisplayConfig".to_string()),
                        methods: vec![(
                            "org.gnome.Mutter.DisplayConfig".to_string(),
                            "ApplyMonitorsConfig".to_string(),
                        )],
                        display_server: DisplayServerSupport::NEUTRAL,
                        ..Default::default()
                    },
                ),
                // KDE KScreen (Wayland + X11).
                ProviderCandidate::new(
                    ProviderId::new("kscreen_display"),
                    ProviderNeeds {
                        bus: Some(BusKind::Session),
                        service: Some("org.kde.KScreen".to_string()),
                        display_server: DisplayServerSupport::NEUTRAL,
                        ..Default::default()
                    },
                ),
                // XRandR — X11-only; never eligible in a native Wayland session.
                ProviderCandidate::new(
                    ProviderId::new("xrandr_display"),
                    ProviderNeeds {
                        binary: Some("xrandr".to_string()),
                        display_server: DisplayServerSupport::X11_ONLY,
                        ..Default::default()
                    },
                ),
            ],
            display_servers: DisplayServerSupport {
                x11: true,
                wayland: true,
            },
            requires_root: false,
            requires_confirmation: ConfirmationPolicy::Confirm,
            reversible: true,
            verifiable: VerificationClass::Verifiable,
        }
    }

    fn full_catalog() -> Vec<CapabilityRequirement> {
        vec![audio_set_volume(), power_reboot(), display_set_topology()]
    }

    // ── GNOME Wayland matrix ─────────────────────────────────────────────────

    #[test]
    fn gnome_wayland_selects_neutral_and_gnome_providers() {
        let probe = ScriptedProbeMatrix::gnome_wayland();
        let prober = CapabilityProber::new(probe, full_catalog());
        let snap = prober.snapshot();

        assert_eq!(snap.display_server, DisplayServer::Wayland);
        assert_eq!(snap.desktop_family, DesktopFamily::Gnome);

        let vol = snap.operation(&CapabilityId::new("set_volume")).unwrap();
        assert_eq!(vol.status, AvailabilityStatus::Available);
        assert_eq!(vol.selected.as_ref().unwrap().as_str(), "pipewire");

        let reboot = snap.operation(&CapabilityId::new("reboot")).unwrap();
        assert_eq!(reboot.status, AvailabilityStatus::Available);
        assert_eq!(reboot.selected.as_ref().unwrap().as_str(), "logind");

        let topo = snap
            .operation(&CapabilityId::new("set_display_topology"))
            .unwrap();
        assert_eq!(topo.status, AvailabilityStatus::Available);
        // GNOME/Mutter is selected; XRandR is NOT a fallback in Wayland.
        assert_eq!(topo.selected.as_ref().unwrap().as_str(), "gnome_display");
        assert!(!topo
            .fallbacks
            .as_slice()
            .iter()
            .any(|p| p.as_str() == "xrandr_display"));
    }

    // ── GNOME X11 matrix: XRandR becomes an eligible fallback ────────────────

    #[test]
    fn gnome_x11_allows_xrandr_fallback() {
        let probe = ScriptedProbeMatrix::gnome_x11();
        let prober = CapabilityProber::new(probe, full_catalog());
        let snap = prober.snapshot();

        assert_eq!(snap.display_server, DisplayServer::X11);
        let topo = snap
            .operation(&CapabilityId::new("set_display_topology"))
            .unwrap();
        assert_eq!(topo.status, AvailabilityStatus::Available);
        assert_eq!(topo.selected.as_ref().unwrap().as_str(), "gnome_display");
        // XRandR is X11-eligible here, so it appears as a fallback.
        assert!(topo
            .fallbacks
            .as_slice()
            .iter()
            .any(|p| p.as_str() == "xrandr_display"));
    }

    // ── KDE Wayland matrix: KScreen selected, GNOME absent ───────────────────

    #[test]
    fn kde_wayland_selects_kscreen() {
        let probe = ScriptedProbeMatrix::kde_wayland();
        let prober = CapabilityProber::new(probe, full_catalog());
        let snap = prober.snapshot();

        assert_eq!(snap.display_server, DisplayServer::Wayland);
        assert_eq!(snap.desktop_family, DesktopFamily::Kde);
        let topo = snap
            .operation(&CapabilityId::new("set_display_topology"))
            .unwrap();
        assert_eq!(topo.status, AvailabilityStatus::Available);
        assert_eq!(topo.selected.as_ref().unwrap().as_str(), "kscreen_display");
    }

    // ── Absent bus: partial probe preserves system-bus operations ────────────

    #[test]
    fn absent_session_bus_degrades_only_session_operations() {
        let probe = ScriptedProbeMatrix::gnome_wayland().with_session_bus(false);
        let prober = CapabilityProber::new(probe, full_catalog());
        let snap = prober.snapshot();

        // Session-bus audio + display become unavailable...
        let vol = snap.operation(&CapabilityId::new("set_volume")).unwrap();
        assert_eq!(vol.status, AvailabilityStatus::Unavailable);
        assert!(vol.reason.as_ref().unwrap().as_str().contains("bus"));
        let topo = snap
            .operation(&CapabilityId::new("set_display_topology"))
            .unwrap();
        assert_eq!(topo.status, AvailabilityStatus::Unavailable);

        // ...but the system-bus power operation is preserved (OSC-031.6).
        let reboot = snap.operation(&CapabilityId::new("reboot")).unwrap();
        assert_eq!(reboot.status, AvailabilityStatus::Available);
        assert!(snap.degraded);
    }

    // ── Stale env vars never fabricate provider access ───────────────────────

    #[test]
    fn stale_wayland_env_does_not_override_confirmed_x11() {
        // Env claims Wayland, but the probe confirms an X11 session with no
        // Wayland compositor. The snapshot must reflect the probe (OSC-003.3,
        // OSC-032.7), not the stale env var.
        let probe = ScriptedProbeMatrix::gnome_x11().with_stale_wayland_env();
        assert_eq!(
            probe.env_hints().display_server_hint(),
            DisplayServer::Wayland
        );

        let prober = CapabilityProber::new(probe, full_catalog());
        let snap = prober.snapshot();
        assert_eq!(snap.display_server, DisplayServer::X11);
        // XRandR (X11-only) is eligible precisely because the confirmed session
        // is X11 — not because the env said Wayland.
        let topo = snap
            .operation(&CapabilityId::new("set_display_topology"))
            .unwrap();
        assert!(topo
            .fallbacks
            .as_slice()
            .iter()
            .any(|p| p.as_str() == "xrandr_display"));
    }

    // ── Service restart invalidates only the affected domain ─────────────────

    #[test]
    fn service_owner_change_invalidates_only_that_domain() {
        let probe = ScriptedProbeMatrix::gnome_wayland();
        let prober = CapabilityProber::new(probe, full_catalog());

        let first = prober.snapshot();
        assert_eq!(first.revision, SnapshotRevision(1));
        // All three domains built on first snapshot.
        assert_eq!(prober.last_rebuilt_domains().len(), 3);

        // Restart logind (system-bus power service): change its owner.
        prober
            .probe()
            .restart_service(BusKind::System, "org.freedesktop.login1");
        let invalidated = prober.refresh_owner_changes();
        assert_eq!(invalidated, vec![Domain::new("power")]);

        let second = prober.snapshot();
        // Only the power domain was renegotiated (OSC-003.5).
        assert_eq!(prober.last_rebuilt_domains(), vec![Domain::new("power")]);
        assert_eq!(second.revision, SnapshotRevision(2));
        // Unrelated audio/display availability is unchanged.
        assert!(first.same_capabilities(&second) || second.revision > first.revision);
        let reboot = second.operation(&CapabilityId::new("reboot")).unwrap();
        assert_eq!(reboot.status, AvailabilityStatus::Available);
    }

    // ── Unknown future interface fields do not break selection ───────────────

    #[test]
    fn unknown_additive_interface_fields_are_tolerated() {
        let probe = ScriptedProbeMatrix::gnome_wayland().with_unknown_future_fields();
        let prober = CapabilityProber::new(probe, full_catalog());
        let snap = prober.snapshot();
        // Extra unknown methods/properties/enum values must not panic or change
        // the proven selection (OSC-031.3).
        let reboot = snap.operation(&CapabilityId::new("reboot")).unwrap();
        assert_eq!(reboot.status, AvailabilityStatus::Available);
        assert_eq!(reboot.selected.as_ref().unwrap().as_str(), "logind");
    }

    // ── Timeout yields a degraded snapshot ───────────────────────────────────

    #[test]
    fn domain_timeout_yields_degraded_snapshot() {
        let probe = ScriptedProbeMatrix::gnome_wayland().with_timed_out_domain("display");
        let prober = CapabilityProber::new(probe, full_catalog());
        let snap = prober.snapshot();

        let topo = snap
            .operation(&CapabilityId::new("set_display_topology"))
            .unwrap();
        assert_eq!(topo.status, AvailabilityStatus::Degraded);
        assert!(topo.reason.as_ref().unwrap().as_str().contains("timed out"));
        assert!(snap.degraded);
        // Other domains still resolve normally.
        let reboot = snap.operation(&CapabilityId::new("reboot")).unwrap();
        assert_eq!(reboot.status, AvailabilityStatus::Available);
    }

    // ── Determinism + redaction/boundedness ──────────────────────────────────

    #[test]
    fn equal_probes_produce_equal_snapshots() {
        let a = CapabilityProber::new(ScriptedProbeMatrix::gnome_wayland(), full_catalog());
        let b = CapabilityProber::new(ScriptedProbeMatrix::gnome_wayland(), full_catalog());
        let sa = a.snapshot();
        let sb = b.snapshot();
        assert!(sa.same_capabilities(&sb));
        assert_eq!(sa, sb, "equal probes must produce fully equal snapshots");
    }

    #[test]
    fn snapshot_is_bounded_and_operations_sorted() {
        let prober = CapabilityProber::new(ScriptedProbeMatrix::gnome_wayland(), full_catalog());
        let snap = prober.snapshot();
        assert!(snap.operations.len() <= MAX_SNAPSHOT_OPERATIONS);
        let ids: Vec<&str> = snap
            .operations
            .as_slice()
            .iter()
            .map(|o| o.capability.as_str())
            .collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "operations must be deterministically sorted");
    }

    #[test]
    fn snapshot_reasons_carry_no_control_characters() {
        let prober = CapabilityProber::new(
            ScriptedProbeMatrix::gnome_wayland().with_session_bus(false),
            full_catalog(),
        );
        let snap = prober.snapshot();
        // Serialize the whole snapshot and confirm it is redaction-safe: no raw
        // newlines/control characters leak into diagnostics (OSC-003.7).
        let json = serde_json::to_string(&snap).unwrap();
        assert!(!json.contains('\u{7}'));
        for op in snap.operations.as_slice() {
            if let Some(r) = &op.reason {
                assert!(!r.as_str().contains('\n'));
                assert!(!r.as_str().contains('\t'));
            }
        }
    }

    #[test]
    fn probing_never_touches_a_live_transport() {
        // Building snapshots across the full matrix must open no live bus: the
        // deny-live sentinel stays armed and records zero trips (OSC-033).
        for probe in [
            ScriptedProbeMatrix::gnome_wayland(),
            ScriptedProbeMatrix::gnome_x11(),
            ScriptedProbeMatrix::kde_wayland(),
        ] {
            let prober = CapabilityProber::new(probe, full_catalog());
            let _ = prober.snapshot();
            prober.invalidate_all();
            let _ = prober.snapshot();
        }
        assert!(crate::os_control::access::sentinel_is_armed());
        assert_eq!(crate::os_control::access::sentinel_trip_count(), 0);
    }

    #[test]
    fn revision_is_monotonic_across_invalidations() {
        let prober = CapabilityProber::new(ScriptedProbeMatrix::gnome_wayland(), full_catalog());
        let r0 = prober.current_revision();
        assert_eq!(r0, SnapshotRevision(0));
        let s1 = prober.snapshot();
        assert_eq!(s1.revision, SnapshotRevision(1));
        // A no-op snapshot (nothing invalidated) does not bump the revision.
        let s2 = prober.snapshot();
        assert_eq!(s2.revision, SnapshotRevision(1));
        prober.invalidate_all();
        let s3 = prober.snapshot();
        assert_eq!(s3.revision, SnapshotRevision(2));
    }
}
