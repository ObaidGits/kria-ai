//! Code-level proof that every canonical OS tool handler is wired to the governed
//! runtime and fails closed (linux-os-control-production).
//!
//! These are the guarantees that must hold for all 46 OS handlers, so they are
//! asserted over the whole frozen manifest rather than tool by tool:
//!
//! 1. With no provider composed, a handler returns the FROZEN `Unavailable`
//!    envelope — never a panic, never a silent success, never a direct subprocess.
//! 2. With a provider composed but no admitted governed call, a mutation still
//!    refuses: the permit, not the provider, is what authorises a host effect.
//! 3. No handler opens a raw transport — the deny-live sentinel never trips.
//!
//! Deny-live only; every provider is a fake.

use std::sync::Arc;

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::os_control::access::sentinel_trip_count;
use kria_core::os_control::audio::selection::AudioBackend;
use kria_core::os_control::audio::{fake::FakeAudioTransport, AudioControl, AudioControlPort};
use kria_core::os_control::runtime::OsControlRuntime;
use kria_core::os_control::testing::FakeHostOsControl;
use kria_core::tools::registry::build_default_registry;

/// Every canonical OS tool that the frozen manifest declares.
fn frozen_os_tools() -> Vec<String> {
    kria_core::os_control::manifest::frozen_tool_names()
}

/// A registry whose runtime has NO composed provider (the default posture).
fn detached_registry() -> Arc<kria_core::tools::registry::ToolRegistry> {
    let registry = Arc::new(build_default_registry());
    registry.set_os_runtime(Arc::new(OsControlRuntime::detached()));
    registry
}

/// Frozen tools that legitimately answer without an OS provider, with the reason.
///
/// Kept explicit and tiny: anything added here is a claim that the tool touches
/// no host transport at all. A new ungoverned bypass fails the test below unless
/// someone deliberately justifies it here.
const IN_PROCESS_READS: &[(&str, &str)] = &[
    (
        "list_installed_apps",
        "serves the already-populated in-process InstalledAppRegistry; design §9.2 \
         forbids re-parsing .desktop files, and no host transport is opened",
    ),
    (
        "list_running_apps",
        "in-process window/app registry snapshot; no host transport",
    ),
    // The `sysinfo`-backed telemetry reads below sample /proc and /sys in-process.
    // They are reads, they mutate nothing, and they predate the os_control
    // surface. They are listed explicitly so a future WRITE can never hide here.
    ("get_battery_status", "sysinfo in-process sample; read-only"),
    ("get_cpu_usage", "sysinfo in-process sample; read-only"),
    ("get_memory_info", "sysinfo in-process sample; read-only"),
    ("get_disk_space", "sysinfo in-process sample; read-only"),
    ("get_system_uptime", "sysinfo in-process sample; read-only"),
    ("get_gpu_info", "sysinfo/nvml in-process sample; read-only"),
    (
        "check_system_health",
        "aggregates the sysinfo reads above; read-only",
    ),
];

/// How many frozen OS tools still have **no registered handler**.
///
/// These are the F3–F5 surface (Bluetooth, firewall, VPN, hotspot, proxy, audio
/// devices, media, display topology, battery/sensors, logs/recovery, clipboard
/// history, DND, desktop search, secrets, printing, scanning, backup, privacy,
/// workflows). Each needs a provider, a transport and a handler.
///
/// This is a **ratchet**: it must only ever shrink. If you register a handler,
/// lower it. If this assertion fails upward, a tool was added to the manifest
/// without a handler.
const UNIMPLEMENTED_TOOL_BUDGET: usize = 0;

#[serial]
#[tokio::test]
async fn every_os_handler_fails_closed_without_a_provider() {
    let baseline = sentinel_trip_count();
    let registry = detached_registry();
    let tools = frozen_os_tools();
    assert!(
        tools.len() >= 40,
        "the frozen manifest should declare the full OS tool surface, got {}",
        tools.len()
    );

    let mut checked = 0;
    let mut exempt = 0;
    let mut missing_handler = Vec::new();
    let mut unexpected_success = Vec::new();
    for tool in &tools {
        let Some(handler) = registry.get_handler(tool) else {
            missing_handler.push(tool.clone());
            continue;
        };
        if IN_PROCESS_READS.iter().any(|(name, _)| name == tool) {
            exempt += 1;
            continue;
        }
        let ctx = registry.make_tool_context(CancellationToken::new());
        // Empty params on purpose: a handler must reject or report unavailable
        // rather than panic on absent input.
        let result = handler
            .execute_with_context(serde_json::json!({}), ctx)
            .await;
        if result.success {
            unexpected_success.push(tool.clone());
        }
        checked += 1;
    }

    // The invariant that matters for every IMPLEMENTED handler.
    assert!(
        unexpected_success.is_empty(),
        "these tools reported success with NO provider composed — each is either an \
         ungoverned bypass or an in-process read that belongs in IN_PROCESS_READS: \
         {unexpected_success:?}"
    );
    assert!(
        checked > 0,
        "at least the implemented OS handlers must have been exercised"
    );
    // The ratchet on the not-yet-implemented surface.
    assert!(
        missing_handler.len() <= UNIMPLEMENTED_TOOL_BUDGET,
        "frozen OS tools without a handler grew to {} (budget {}): {:?}",
        missing_handler.len(),
        UNIMPLEMENTED_TOOL_BUDGET,
        missing_handler
    );
    assert!(
        exempt <= IN_PROCESS_READS.len(),
        "the in-process-read exemption list must stay small; it has grown to {exempt}"
    );
    assert_eq!(
        sentinel_trip_count(),
        baseline,
        "no handler may open a raw live transport"
    );
}

#[serial]
#[tokio::test]
async fn composed_provider_without_a_permit_still_refuses_a_mutation() {
    let baseline = sentinel_trip_count();

    // A real (fake-backed) audio provider IS composed…
    let transport = FakeAudioTransport::new(AudioBackend::Wpctl).read_ok(40, false);
    let port: Arc<dyn AudioControlPort> = Arc::new(AudioControl::new(transport));
    let host = FakeHostOsControl::new("test-aggregate").with_audio(port);

    let registry = Arc::new(build_default_registry());
    registry.set_os_runtime(Arc::new(OsControlRuntime::with_host(Arc::new(host))));

    let handler = registry
        .get_handler("set_volume")
        .expect("set_volume is a frozen OS tool");
    // …but the tool context carries no governed call, so no permit exists.
    let ctx = registry.make_tool_context(CancellationToken::new());
    assert!(
        ctx.os_call().is_none(),
        "this context intentionally carries no admitted call"
    );

    let result = handler
        .execute_with_context(serde_json::json!({ "level": 80 }), ctx)
        .await;

    assert!(
        !result.success,
        "a composed provider must not be enough: the mutation permit authorises the effect"
    );
    assert_eq!(sentinel_trip_count(), baseline);
}

#[serial]
#[tokio::test]
async fn read_handlers_report_unavailable_rather_than_fabricating_state() {
    let baseline = sentinel_trip_count();
    let registry = detached_registry();

    // Reads must be as fail-closed as mutations when nothing is composed: an
    // invented "volume 0" or "no networks" answer would be a fabricated state.
    for tool in [
        "get_audio_state",
        "get_display_state",
        "get_network_state",
        "get_wifi_networks",
        "list_processes",
        "list_storage_devices",
        "get_clipboard",
    ] {
        let handler = registry
            .get_handler(tool)
            .unwrap_or_else(|| panic!("{tool} must have a registered handler"));
        let ctx = registry.make_tool_context(CancellationToken::new());
        let result = handler
            .execute_with_context(serde_json::json!({}), ctx)
            .await;
        assert!(
            !result.success,
            "{tool} must report unavailable rather than fabricate a state"
        );
    }
    assert_eq!(sentinel_trip_count(), baseline);
}

#[serial]
#[tokio::test]
async fn the_registry_always_carries_a_runtime_seam() {
    // A registry built the standard way always injects at least a detached
    // runtime, so an OS handler can never find the seam absent and fall back to
    // some ungoverned path.
    let registry = build_default_registry();
    let ctx = registry.make_tool_context(CancellationToken::new());
    assert!(
        ctx.os_runtime.is_some(),
        "the standard builder must inject the OS-control runtime seam"
    );
    assert!(
        !ctx.os_runtime
            .as_ref()
            .expect("seam present")
            .provider_present(),
        "the default seam is detached: no provider until a live root composes one"
    );
}
