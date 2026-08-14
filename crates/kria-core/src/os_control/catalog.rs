//! The production capability catalog (Task 1.3, OSC-003, OSC-031, OSC-032).
//!
//! [`CapabilityProber`](crate::os_control::capability::CapabilityProber) needs a
//! catalog describing, per canonical operation, which providers could serve it
//! and what each one *needs* to be eligible: a bus name, a trusted executable, a
//! portal, a display server. The probe then answers "is this actually available
//! on THIS host" from observed facts instead of assumption.
//!
//! # Why this lives here and not in the manifest
//!
//! The frozen manifest declares each tool's port operations, risk, resources and
//! verification class — but deliberately not *how* a provider reaches the host.
//! That knowledge belongs to each domain's selection module (which backend, which
//! executable, which service name), so the catalog is assembled from those rather
//! than duplicating bus names into the manifest.
//!
//! # Ordering is meaningful
//!
//! Candidates are listed most-preferred first, matching each domain's
//! `PREFERENCE` constant. The prober selects the first eligible candidate, so a
//! native bus path always wins over a CLI fallback when both are present.

use crate::os_control::capability::{
    BusKind, CapabilityRequirement, ConfirmationPolicy, DisplayServerSupport, Domain,
    ProviderCandidate, ProviderNeeds,
};
use crate::os_control::contract::{CapabilityId, ProviderId, VerificationClass};

/// X11 and Wayland both supported.
const ANY_DISPLAY: DisplayServerSupport = DisplayServerSupport::NEUTRAL;
/// Wayland-ineligible (X11-only), e.g. XRandR gamma scaling.
const X11_ONLY: DisplayServerSupport = DisplayServerSupport {
    x11: true,
    wayland: false,
};

fn bus_candidate(provider: &str, bus: BusKind, service: &str) -> ProviderCandidate {
    ProviderCandidate {
        provider: ProviderId::new(provider),
        needs: ProviderNeeds {
            bus: Some(bus),
            service: Some(service.to_string()),
            ..ProviderNeeds::default()
        },
        degrade_if_missing: Vec::new(),
    }
}

fn binary_candidate(provider: &str, binary: &str) -> ProviderCandidate {
    ProviderCandidate {
        provider: ProviderId::new(provider),
        needs: ProviderNeeds {
            binary: Some(binary.to_string()),
            ..ProviderNeeds::default()
        },
        degrade_if_missing: Vec::new(),
    }
}

fn binary_candidate_x11(provider: &str, binary: &str) -> ProviderCandidate {
    ProviderCandidate {
        provider: ProviderId::new(provider),
        needs: ProviderNeeds {
            binary: Some(binary.to_string()),
            display_server: X11_ONLY,
            ..ProviderNeeds::default()
        },
        degrade_if_missing: Vec::new(),
    }
}

/// No external dependency: the kernel or in-process filesystem access.
fn intrinsic_candidate(provider: &str) -> ProviderCandidate {
    ProviderCandidate {
        provider: ProviderId::new(provider),
        needs: ProviderNeeds::default(),
        degrade_if_missing: Vec::new(),
    }
}

// ── Per-domain candidate sets, in preference order ──────────────────────────

fn audio_candidates() -> Vec<ProviderCandidate> {
    vec![
        binary_candidate("wpctl", "/usr/bin/wpctl"),
        binary_candidate("pactl", "/usr/bin/pactl"),
        binary_candidate("amixer", "/usr/bin/amixer"),
    ]
}

fn display_candidates() -> Vec<ProviderCandidate> {
    vec![
        bus_candidate(
            "gnome-settings-daemon",
            BusKind::Session,
            "org.gnome.SettingsDaemon.Power",
        ),
        binary_candidate("brightnessctl", "/usr/bin/brightnessctl"),
        // XRandR scales the gamma ramp, not the physical backlight, and is never
        // eligible on Wayland (OSC-019.3 / OSC-032.3).
        binary_candidate_x11("xrandr-gamma", "/usr/bin/xrandr"),
    ]
}

fn connectivity_candidates() -> Vec<ProviderCandidate> {
    vec![
        bus_candidate(
            "network-manager",
            BusKind::System,
            "org.freedesktop.NetworkManager",
        ),
        binary_candidate("nmcli", "/usr/bin/nmcli"),
    ]
}

fn power_profile_candidates() -> Vec<ProviderCandidate> {
    vec![
        bus_candidate(
            "power-profiles-daemon",
            BusKind::System,
            "org.freedesktop.UPower.PowerProfiles",
        ),
        binary_candidate("powerprofilesctl", "/usr/bin/powerprofilesctl"),
    ]
}

fn power_session_candidates() -> Vec<ProviderCandidate> {
    vec![
        bus_candidate("logind", BusKind::System, "org.freedesktop.login1"),
        binary_candidate("loginctl", "/usr/bin/loginctl"),
    ]
}

fn packages_candidates() -> Vec<ProviderCandidate> {
    vec![bus_candidate(
        "packagekit",
        BusKind::System,
        "org.freedesktop.PackageKit",
    )]
}

fn storage_candidates() -> Vec<ProviderCandidate> {
    vec![bus_candidate(
        "udisks2",
        BusKind::System,
        "org.freedesktop.UDisks2",
    )]
}

fn notification_candidates() -> Vec<ProviderCandidate> {
    vec![bus_candidate(
        "freedesktop-notifications",
        BusKind::Session,
        "org.freedesktop.Notifications",
    )]
}

fn clipboard_candidates() -> Vec<ProviderCandidate> {
    // The clipboard is a display-server selection, not a bus name: the provider
    // owns the Wayland/X11 split internally.
    vec![intrinsic_candidate("desktop-clipboard")]
}

fn process_candidates() -> Vec<ProviderCandidate> {
    vec![intrinsic_candidate("kernel-process")]
}

fn filesystem_candidates() -> Vec<ProviderCandidate> {
    vec![intrinsic_candidate("filesystem")]
}

fn bluetooth_candidates() -> Vec<ProviderCandidate> {
    vec![
        bus_candidate("bluez", BusKind::System, "org.bluez"),
        binary_candidate("bluetoothctl", "/usr/bin/bluetoothctl"),
    ]
}

fn application_candidates() -> Vec<ProviderCandidate> {
    vec![intrinsic_candidate("desktop-entries")]
}

/// One catalog row.
struct Row {
    tool: &'static str,
    domain: &'static str,
    candidates: fn() -> Vec<ProviderCandidate>,
    displays: DisplayServerSupport,
    requires_root: bool,
    confirm: ConfirmationPolicy,
    reversible: bool,
    verifiable: VerificationClass,
}

/// Every canonical operation currently served by a composed provider.
///
/// Tools absent from this table are not yet implemented (their handlers report
/// the frozen `Unavailable` envelope), so probing them would assert availability
/// for something that cannot run.
fn rows() -> Vec<Row> {
    use ConfirmationPolicy::{Confirm, None as NoConfirm};
    use VerificationClass::{AcceptedOnly, Verifiable};

    vec![
        // ── audio ──────────────────────────────────────────────────────────
        Row { tool: "set_volume", domain: "audio", candidates: audio_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "set_audio_mute", domain: "audio", candidates: audio_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "get_audio_state", domain: "audio", candidates: audio_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        // ── display ────────────────────────────────────────────────────────
        Row { tool: "set_brightness", domain: "display", candidates: display_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "get_display_state", domain: "display", candidates: display_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        // ── connectivity ───────────────────────────────────────────────────
        Row { tool: "toggle_wifi", domain: "connectivity", candidates: connectivity_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: true, verifiable: Verifiable },
        Row { tool: "connect_wifi", domain: "connectivity", candidates: connectivity_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: true, verifiable: Verifiable },
        Row { tool: "disconnect_wifi", domain: "connectivity", candidates: connectivity_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: false, verifiable: Verifiable },
        Row { tool: "forget_wifi", domain: "connectivity", candidates: connectivity_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: false, verifiable: Verifiable },
        Row { tool: "activate_network_profile", domain: "connectivity", candidates: connectivity_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: true, verifiable: Verifiable },
        Row { tool: "get_wifi_networks", domain: "connectivity", candidates: connectivity_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "get_network_state", domain: "connectivity", candidates: connectivity_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        // ── power profile ──────────────────────────────────────────────────
        Row { tool: "set_power_plan", domain: "power", candidates: power_profile_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "get_power_plan", domain: "power", candidates: power_profile_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        // ── power session ──────────────────────────────────────────────────
        // Session-ending operations can only ever be Accepted: nothing survives to
        // observe the post-state, so they must never claim Verified.
        Row { tool: "lock_screen", domain: "power-session", candidates: power_session_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: true, verifiable: Verifiable },
        Row { tool: "sleep", domain: "power-session", candidates: power_session_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: false, verifiable: AcceptedOnly },
        Row { tool: "hibernate", domain: "power-session", candidates: power_session_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: false, verifiable: AcceptedOnly },
        Row { tool: "shutdown_system", domain: "power-session", candidates: power_session_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: false, verifiable: AcceptedOnly },
        Row { tool: "reboot_system", domain: "power-session", candidates: power_session_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: false, verifiable: AcceptedOnly },
        // ── packages ───────────────────────────────────────────────────────
        Row { tool: "install_package", domain: "packages", candidates: packages_candidates, displays: ANY_DISPLAY, requires_root: true, confirm: Confirm, reversible: true, verifiable: Verifiable },
        Row { tool: "uninstall_package", domain: "packages", candidates: packages_candidates, displays: ANY_DISPLAY, requires_root: true, confirm: Confirm, reversible: true, verifiable: Verifiable },
        Row { tool: "search_package", domain: "packages", candidates: packages_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "get_package_info", domain: "packages", candidates: packages_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "list_installed_packages", domain: "packages", candidates: packages_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "plan_package_changes", domain: "packages", candidates: packages_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "check_system_updates", domain: "packages", candidates: packages_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "get_reboot_required", domain: "packages", candidates: packages_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        // ── storage ────────────────────────────────────────────────────────
        Row { tool: "mount_device", domain: "storage", candidates: storage_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: true, verifiable: Verifiable },
        Row { tool: "unmount_device", domain: "storage", candidates: storage_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: true, verifiable: Verifiable },
        Row { tool: "eject_device", domain: "storage", candidates: storage_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: false, verifiable: Verifiable },
        Row { tool: "list_storage_devices", domain: "storage", candidates: storage_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "get_storage_health", domain: "storage", candidates: storage_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        // ── notifications ──────────────────────────────────────────────────
        // A notification is delivered, not observable afterwards.
        Row { tool: "send_notification", domain: "notifications", candidates: notification_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: false, verifiable: AcceptedOnly },
        // ── clipboard ──────────────────────────────────────────────────────
        Row { tool: "set_clipboard", domain: "clipboard", candidates: clipboard_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "get_clipboard", domain: "clipboard", candidates: clipboard_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        // `transform_clipboard` is deliberately absent: it is a KRIA convenience
        // composed over get+set clipboard, not a canonical manifest capability, so
        // it has no independent availability to probe.
        // ── processes ──────────────────────────────────────────────────────
        Row { tool: "kill_process", domain: "processes", candidates: process_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: false, verifiable: Verifiable },
        Row { tool: "set_process_priority", domain: "processes", candidates: process_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "list_processes", domain: "processes", candidates: process_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "get_process_info", domain: "processes", candidates: process_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "get_process_command_metadata", domain: "processes", candidates: process_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        // ── bluetooth (Task 3.7) ───────────────────────────────────────────
        // Discovery reads are RED: enumerating nearby hardware is privacy-
        // sensitive. Pair / trust / remove are RED mutations requiring approval.
        Row { tool: "get_bluetooth_state", domain: "bluetooth", candidates: bluetooth_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: true, verifiable: Verifiable },
        Row { tool: "scan_bluetooth", domain: "bluetooth", candidates: bluetooth_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: true, verifiable: Verifiable },
        Row { tool: "set_bluetooth_enabled", domain: "bluetooth", candidates: bluetooth_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "pair_bluetooth_device", domain: "bluetooth", candidates: bluetooth_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: false, verifiable: Verifiable },
        Row { tool: "connect_bluetooth_device", domain: "bluetooth", candidates: bluetooth_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: true, verifiable: Verifiable },
        Row { tool: "disconnect_bluetooth_device", domain: "bluetooth", candidates: bluetooth_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: false, verifiable: Verifiable },
        Row { tool: "set_bluetooth_trust", domain: "bluetooth", candidates: bluetooth_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: true, verifiable: Verifiable },
        // Irreversible: the pairing keys are destroyed.
        Row { tool: "remove_bluetooth_device", domain: "bluetooth", candidates: bluetooth_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: false, verifiable: Verifiable },
        // ── applications ───────────────────────────────────────────────────
        // NOTE: the canonical capability is `graceful_close_application`; the
        // currently-registered handler uses the legacy name `close_application`.
        // Registering the canonical name is part of the F3 surface.
        Row { tool: "graceful_close_application", domain: "applications", candidates: application_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: Confirm, reversible: false, verifiable: Verifiable },
        Row { tool: "set_default_application", domain: "applications", candidates: application_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        Row { tool: "manage_autostart", domain: "applications", candidates: application_candidates, displays: ANY_DISPLAY, requires_root: false, confirm: NoConfirm, reversible: true, verifiable: Verifiable },
        // ── files ──────────────────────────────────────────────────────────
        // Canonical name is `set_file_ownership`; the registered handler uses the
        // legacy `set_file_owner`. Both route to the same governed provider.
        Row { tool: "set_file_ownership", domain: "files", candidates: filesystem_candidates, displays: ANY_DISPLAY, requires_root: true, confirm: Confirm, reversible: true, verifiable: Verifiable },
    ]
}

/// The production capability catalog.
///
/// Deterministic: the prober sorts and dedups by capability id, so the returned
/// order does not matter to callers, but the per-row candidate order does.
#[must_use]
pub fn capability_catalog() -> Vec<CapabilityRequirement> {
    rows()
        .into_iter()
        .map(|row| CapabilityRequirement {
            capability: CapabilityId::new(row.tool),
            domain: Domain::new(row.domain),
            candidates: (row.candidates)(),
            display_servers: row.displays,
            requires_root: row.requires_root,
            requires_confirmation: row.confirm,
            reversible: row.reversible,
            verifiable: row.verifiable,
        })
        .collect()
}

/// The probe plan implied by the catalog: every distinct bus service, portal and
/// binary any candidate needs.
///
/// Derived rather than hand-listed so a new catalog row cannot be forgotten in
/// the probe plan — which would silently make its capability look unavailable.
#[must_use]
pub fn probe_plan() -> crate::os_control::linux::probe::LiveProbePlan {
    let mut services: Vec<(BusKind, String, String)> = Vec::new();
    let mut portals: Vec<String> = Vec::new();
    let mut binaries: Vec<String> = Vec::new();

    for req in capability_catalog() {
        for cand in req.candidates {
            if let (Some(bus), Some(service)) = (cand.needs.bus, cand.needs.service.clone()) {
                // The well-known object path for a service is conventionally its
                // name with dots as slashes; the probe only needs a path to
                // introspect, and an own-check does not depend on it.
                let path = format!("/{}", service.replace('.', "/"));
                let entry = (bus, service, path);
                if !services.contains(&entry) {
                    services.push(entry);
                }
            }
            if let Some(portal) = cand.needs.portal.clone() {
                if !portals.contains(&portal) {
                    portals.push(portal);
                }
            }
            if let Some(binary) = cand.needs.binary.clone() {
                if !binaries.contains(&binary) {
                    binaries.push(binary);
                }
            }
        }
    }

    crate::os_control::linux::probe::LiveProbePlan {
        services,
        portals,
        binaries,
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_non_empty_and_uniquely_keyed() {
        let catalog = capability_catalog();
        assert!(catalog.len() >= 40, "catalog covers the wired tool surface");
        let mut ids: Vec<String> = catalog
            .iter()
            .map(|r| r.capability.as_str().to_string())
            .collect();
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "no duplicate capability ids");
    }

    #[test]
    fn every_catalog_tool_is_a_frozen_manifest_tool() {
        let frozen = crate::os_control::manifest::frozen_tool_names();
        let unknown: Vec<&str> = capability_catalog()
            .iter()
            .map(|r| r.capability.as_str())
            .filter(|name| !frozen.iter().any(|t| t == name))
            .map(|name| {
                // Leak-free: the catalog rows are &'static str literals.
                rows()
                    .into_iter()
                    .find(|r| r.tool == name)
                    .map(|r| r.tool)
                    .unwrap_or("<unknown>")
            })
            .collect();
        assert!(
            unknown.is_empty(),
            "catalog names that are NOT canonical frozen tools: {unknown:?}"
        );
    }

    #[test]
    fn probe_plan_covers_every_candidate_need() {
        let plan = probe_plan();
        // Bus services the composed domains genuinely depend on.
        for service in [
            "org.freedesktop.login1",
            "org.freedesktop.NetworkManager",
            "org.freedesktop.UDisks2",
            "org.freedesktop.PackageKit",
            "org.freedesktop.Notifications",
        ] {
            assert!(
                plan.services.iter().any(|(_, s, _)| s == service),
                "probe plan must check {service}"
            );
        }
        for binary in ["/usr/bin/wpctl", "/usr/bin/nmcli", "/usr/bin/loginctl"] {
            assert!(
                plan.binaries.iter().any(|b| b == binary),
                "probe plan must check {binary}"
            );
        }
    }

    #[test]
    fn session_ending_operations_never_claim_verifiable() {
        for req in capability_catalog() {
            if matches!(
                req.capability.as_str(),
                "sleep" | "hibernate" | "shutdown_system" | "reboot_system"
            ) {
                assert_eq!(
                    req.verifiable,
                    VerificationClass::AcceptedOnly,
                    "{} ends the session; nothing survives to observe the post-state",
                    req.capability.as_str()
                );
            }
        }
    }

    #[test]
    fn xrandr_gamma_is_never_wayland_eligible() {
        let display = capability_catalog()
            .into_iter()
            .find(|r| r.capability.as_str() == "set_brightness")
            .expect("set_brightness is catalogued");
        let xrandr = display
            .candidates
            .iter()
            .find(|c| c.provider.as_str() == "xrandr-gamma")
            .expect("xrandr candidate present");
        assert!(!xrandr.needs.display_server.wayland);
        assert!(xrandr.needs.display_server.x11);
    }
}
