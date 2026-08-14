//! Code-level proof that the production capability catalog drives real
//! availability decisions (Task 1.3, OSC-003, OSC-031, OSC-032).
//!
//! The catalog says what each canonical operation *needs*; the prober answers
//! whether this host has it. These tests drive the real catalog through a
//! **scripted** probe, so they assert the wiring without touching a bus.
//!
//! Deny-live only: `ScriptedProbeMatrix` is the test seam, so no D-Bus connection
//! is ever opened.

use serial_test::serial;

use kria_core::os_control::access::sentinel_trip_count;
use kria_core::os_control::capability::{CapabilityProber, DisplayServer};
use kria_core::os_control::catalog::{capability_catalog, probe_plan};
use kria_core::os_control::contract::CapabilityId;
use kria_core::os_control::linux::probe::ScriptedProbeMatrix;
use kria_core::os_control::AvailabilityStatus;

fn prober_on(matrix: ScriptedProbeMatrix) -> CapabilityProber<ScriptedProbeMatrix> {
    CapabilityProber::new(matrix, capability_catalog())
}

fn status_of(
    prober: &CapabilityProber<ScriptedProbeMatrix>,
    tool: &str,
) -> Option<AvailabilityStatus> {
    prober
        .snapshot()
        .operation(&CapabilityId::new(tool))
        .map(|op| op.status)
}

#[serial]
#[test]
fn catalog_operations_all_appear_in_the_snapshot() {
    let baseline = sentinel_trip_count();
    let prober = prober_on(ScriptedProbeMatrix::gnome_wayland());
    let snapshot = prober.snapshot();

    for req in capability_catalog() {
        assert!(
            snapshot.operation(&req.capability).is_some(),
            "{} is catalogued but missing from the snapshot",
            req.capability.as_str()
        );
    }
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

#[serial]
#[test]
fn a_bus_backed_domain_is_unavailable_when_its_bus_is_down() {
    // With no system bus there is no way to reach logind, NetworkManager,
    // UDisks2 or PackageKit — those operations must report unavailable rather
    // than selecting a provider that cannot be contacted.
    let prober = prober_on(ScriptedProbeMatrix::gnome_wayland().with_system_bus(false));

    for tool in [
        "lock_screen",
        "toggle_wifi",
        "mount_device",
        "install_package",
    ] {
        let status = status_of(&prober, tool).expect("catalogued");
        assert_ne!(
            status,
            AvailabilityStatus::Available,
            "{tool} must not be Available with the system bus down"
        );
    }
}

#[serial]
#[test]
fn an_intrinsic_domain_stays_available_without_any_bus() {
    // Process control is kernel-backed: it needs neither bus nor binary, so a
    // bus outage must not take it down.
    let prober = prober_on(
        ScriptedProbeMatrix::gnome_wayland()
            .with_system_bus(false)
            .with_session_bus(false),
    );
    assert_eq!(
        status_of(&prober, "list_processes").expect("catalogued"),
        AvailabilityStatus::Available,
        "kernel-backed process reads do not depend on D-Bus"
    );
}

#[serial]
#[test]
fn wayland_never_selects_the_xrandr_gamma_fallback() {
    // OSC-019.3 / OSC-032.3: XRandR scales the gamma ramp rather than the real
    // backlight, so it must never be the selected brightness provider on Wayland
    // even when it is the only candidate present.
    let prober = prober_on(ScriptedProbeMatrix::gnome_wayland());
    let snapshot = prober.snapshot();
    assert_eq!(snapshot.display_server, DisplayServer::Wayland);

    if let Some(op) = snapshot.operation(&CapabilityId::new("set_brightness")) {
        if let Some(selected) = &op.selected {
            assert_ne!(
                selected.as_str(),
                "xrandr-gamma",
                "XRandR gamma must never be selected in a Wayland session"
            );
        }
    }
}

#[serial]
#[test]
fn repeated_snapshots_of_an_unchanged_host_keep_the_revision() {
    let prober = prober_on(ScriptedProbeMatrix::gnome_wayland());

    let first = prober.snapshot();
    let second = prober.snapshot();
    assert_eq!(
        first.revision, second.revision,
        "an unchanged host must not churn the capability revision"
    );
    assert!(
        first.same_capabilities(&second),
        "an unchanged host must produce the same capability set"
    );
    // Revision-bump-on-change is covered by the prober's own unit tests, which can
    // script an owner change for a service they control. The scripted matrix here
    // owns no service this catalog depends on, so dropping one would prove nothing.
}

#[serial]
#[test]
fn the_probe_plan_is_derived_from_the_catalog() {
    // Every bus service and binary any catalogued candidate needs must be in the
    // plan, otherwise the probe would never check it and the capability would
    // look unavailable for the wrong reason.
    let plan = probe_plan();
    for req in capability_catalog() {
        for cand in req.candidates {
            if let Some(service) = cand.needs.service {
                assert!(
                    plan.services.iter().any(|(_, s, _)| *s == service),
                    "probe plan is missing service {service}"
                );
            }
            if let Some(binary) = cand.needs.binary {
                assert!(
                    plan.binaries.iter().any(|b| *b == binary),
                    "probe plan is missing binary {binary}"
                );
            }
        }
    }
}
