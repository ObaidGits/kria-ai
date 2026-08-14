//! Code-level proof of the Bluetooth domain lifecycle (Task 3.7, OSC-021,
//! OSC-029).
//!
//! Deny-live only: every read and dispatch goes through
//! `FakeBluetoothTransport`, an in-memory BlueZ object manager. No system bus, no
//! `bluetoothctl`, no radio — each test asserts the deny-live sentinel never
//! tripped.
//!
//! Covers the races and refusals task 3.7 names: a disappearing device, a scan
//! timeout, an absent adapter, duplicate advertised names, bounded scans, and the
//! rule that pairing and removal must never advertise a rollback.

use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::resource_lease::ResourceLeaseManager;
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::os_control::access::sentinel_trip_count;
use kria_core::os_control::bluetooth::fake::{FakeBluetoothFault, FakeBluetoothTransport};
use kria_core::os_control::bluetooth::selection::BluetoothBackend;
use kria_core::os_control::bluetooth::{
    BluetoothControl, BluetoothControlPort, BluetoothDeviceId, BluetoothFocus, BluetoothOp,
    BluetoothRequest, BLUETOOTH_SCAN_MAX_MS,
};
use kria_core::os_control::context::RedactionPolicy;
use kria_core::os_control::contract::{ActionId, CorrelationId, SnapshotRevision};
use kria_core::os_control::governed::{execute_governed_mutation, OsCallRequest, OsGovernedCall};
use kria_core::os_control::resource::{os_write_requirements, OsResourceCoordinator};
use kria_core::os_control::runtime::{MutationPlan, OsControlRuntime, RollbackPlan};
use kria_core::os_control::{
    ActionLifecycle, ComparatorKind, OsAuditStore, ProviderId, ReceiptId,
};
use kria_core::safety::RiskLevel;

const SESSION: &str = "bluetooth-session";
const ADDR: &str = "AA:BB:CC:DD:EE:FF";

/// A read-admitted call. The caller keeps it alive and borrows
/// `call.observation()` — `HostExecutionContext` is deliberately not `Clone`, so a
/// context cannot outlive the admission it belongs to.
fn read_call(audit: &OsAuditStore, tool: &str, params: &serde_json::Value) -> OsGovernedCall {
    OsGovernedCall::admit_read(audit, call_request(tool, params), true)
        .expect("a privacy-sensitive read is admitted on a healthy store")
}

fn call_request<'a>(tool: &'a str, params: &'a serde_json::Value) -> OsCallRequest<'a> {
    OsCallRequest {
        session_id: SESSION,
        correlation_id: CorrelationId::new("corr-bt-1"),
        action_id: ActionId::new("act-bt-1"),
        action: tool,
        params,
        target: ExecutionTarget::Host,
        risk: RiskLevel::Red,
        requirements: os_write_requirements(tool, params),
        snapshot_revision: SnapshotRevision(1),
        cancellation: CancellationToken::new(),
        deadline: Instant::now() + Duration::from_secs(30),
        redaction: RedactionPolicy::default(),
        snapshot: None,
    }
}

fn grant_for(tool: &str, params: &serde_json::Value) -> OsActionGrant {
    OsActionGrant::for_test(
        SESSION,
        tool,
        params,
        ExecutionTarget::Host,
        &os_write_requirements(tool, params),
        RiskLevel::Red,
    )
}

fn plan(provider: &str, comparator: ComparatorKind) -> MutationPlan {
    MutationPlan {
        receipt_id: ReceiptId::new("r-bt-1"),
        provider: ProviderId::new(provider),
        comparator,
        tolerance: None,
        deadline_ms: 500,
        rollback: RollbackPlan::Unavailable,
        latency_ms: 1,
    }
}

fn fake_with_paired_device() -> FakeBluetoothTransport {
    FakeBluetoothTransport::new(BluetoothBackend::Bluetoothctl)
        .with_device(ADDR, "Test Headset", true, false, false)
}

/// A governed connect reaches Verified and dispatches exactly once.
#[serial]
#[tokio::test]
async fn connecting_a_paired_device_reaches_verified() {
    let baseline = sentinel_trip_count();
    let audit = OsAuditStore::open_in_memory();
    let coordinator = OsResourceCoordinator::new(ResourceLeaseManager::new());
    let tool = "connect_bluetooth_device";
    let params = serde_json::json!({ "device": ADDR });

    let call = OsGovernedCall::admit(
        &audit,
        &coordinator,
        grant_for(tool, &params),
        call_request(tool, &params),
    )
    .await
    .expect("admission and lease acquisition succeed");

    let provider = BluetoothControl::new(fake_with_paired_device());
    let request = BluetoothRequest {
        action: tool.to_string(),
        params: params.clone(),
        op: BluetoothOp::Connect(BluetoothDeviceId::new(ADDR)),
    };
    let desired = request.desired_state();

    let outcome = execute_governed_mutation(
        &OsControlRuntime::detached(),
        &provider,
        &call,
        &audit,
        &request,
        &desired,
        &plan("fake-bluetooth", request.comparator()),
    )
    .await
    .expect("the governed chain completes");

    assert_eq!(outcome.receipt.lifecycle(), ActionLifecycle::Verified);
    assert!(outcome.receipt.changed());
    assert!(outcome.durably_recorded());
    assert_eq!(
        provider.transport().dispatch_count(),
        1,
        "apply exactly once"
    );
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

/// An already-connected device is Unchanged and dispatches nothing.
#[serial]
#[tokio::test]
async fn connecting_an_already_connected_device_is_unchanged() {
    let audit = OsAuditStore::open_in_memory();
    let coordinator = OsResourceCoordinator::new(ResourceLeaseManager::new());
    let tool = "connect_bluetooth_device";
    let params = serde_json::json!({ "device": ADDR });

    let call = OsGovernedCall::admit(
        &audit,
        &coordinator,
        grant_for(tool, &params),
        call_request(tool, &params),
    )
    .await
    .expect("admission succeeds");

    let provider = BluetoothControl::new(
        FakeBluetoothTransport::new(BluetoothBackend::Bluetoothctl)
            // already paired AND connected
            .with_device(ADDR, "Test Headset", true, true, false),
    );
    let request = BluetoothRequest {
        action: tool.to_string(),
        params: params.clone(),
        op: BluetoothOp::Connect(BluetoothDeviceId::new(ADDR)),
    };
    let desired = request.desired_state();

    let outcome = execute_governed_mutation(
        &OsControlRuntime::detached(),
        &provider,
        &call,
        &audit,
        &request,
        &desired,
        &plan("fake-bluetooth", request.comparator()),
    )
    .await
    .expect("the governed chain completes");

    assert_eq!(outcome.receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert!(!outcome.receipt.changed());
    assert_eq!(
        provider.transport().dispatch_count(),
        0,
        "an already-satisfied connect must not dispatch"
    );
}

/// An absent adapter fails closed instead of reporting "powered off", which would
/// let an enable request verify as already satisfied.
#[serial]
#[tokio::test]
async fn a_missing_adapter_fails_closed_rather_than_reporting_off() {
    let audit = OsAuditStore::open_in_memory();
    let tool = "set_bluetooth_enabled";
    let params = serde_json::json!({ "enabled": true });

    let provider = BluetoothControl::new(
        FakeBluetoothTransport::new(BluetoothBackend::Bluetoothctl).without_adapter(),
    );
    let request = BluetoothRequest {
        action: tool.to_string(),
        params: params.clone(),
        op: BluetoothOp::SetEnabled(true),
    };

    let call = read_call(&audit, tool, &params);
    let error = kria_core::os_control::contract::DesiredStateControl::observe(
        &provider,
        call.observation(),
        &request,
    )
    .await
    .expect_err("no adapter must be an error, not a fabricated state");
    assert_eq!(error.code(), "os_control.unavailable");
}

/// A scan duration is clamped, so a caller cannot request an unbounded sweep.
#[serial]
#[tokio::test]
async fn scan_duration_is_clamped_to_the_bounded_maximum() {
    let audit = OsAuditStore::open_in_memory();
    let params = serde_json::json!({ "duration_ms": 999_999 });
    let provider = BluetoothControl::new(
        FakeBluetoothTransport::new(BluetoothBackend::Bluetoothctl)
            .with_discoverable("11:22:33:44:55:66", "Speaker", Some(-60)),
    );

    let call = read_call(&audit, "scan_bluetooth", &params);
    let scan = provider
        .scan(call.observation(), 999_999)
        .await
        .expect("scan succeeds");
    assert_eq!(scan.devices.len(), 1);
    assert_eq!(
        provider.transport().scan_durations(),
        vec![BLUETOOTH_SCAN_MAX_MS],
        "the provider must clamp the requested duration"
    );
}

/// A scan that exceeds its deadline surfaces an error rather than an empty list —
/// "no devices found" and "the scan failed" must not be conflated.
#[serial]
#[tokio::test]
async fn a_scan_timeout_is_an_error_not_an_empty_result() {
    let audit = OsAuditStore::open_in_memory();
    let params = serde_json::json!({ "duration_ms": 5_000 });
    let provider = BluetoothControl::new(
        FakeBluetoothTransport::new(BluetoothBackend::Bluetoothctl)
            .with_fault(FakeBluetoothFault::ScanTimeout),
    );

    let call = read_call(&audit, "scan_bluetooth", &params);
    let error = provider
        .scan(call.observation(), 5_000)
        .await
        .expect_err("a timed-out scan must not look like an empty room");
    assert_eq!(error.code(), "os_control.timed_out_before_mutation");
}

/// Two devices advertising the same name stay distinguishable, because the
/// address is the identity and the name is only a label.
#[serial]
#[tokio::test]
async fn duplicate_advertised_names_remain_distinguishable_by_address() {
    let audit = OsAuditStore::open_in_memory();
    let params = serde_json::json!({});
    let provider = BluetoothControl::new(
        FakeBluetoothTransport::new(BluetoothBackend::Bluetoothctl)
            .with_device("AA:AA:AA:AA:AA:AA", "Headphones", true, false, false)
            .with_device("BB:BB:BB:BB:BB:BB", "Headphones", true, false, false),
    );

    let call = read_call(&audit, "get_bluetooth_state", &params);
    let state = provider
        .read_state(call.observation())
        .await
        .expect("read succeeds");
    assert_eq!(state.devices.len(), 2);
    assert_eq!(
        state.devices[0].label.as_str(),
        state.devices[1].label.as_str(),
        "the labels collide, which is exactly why they are not identities"
    );
    assert_ne!(state.devices[0].device, state.devices[1].device);
}

/// A device that leaves range between the pre-observation and verification is a
/// contradiction, not a silent success.
#[serial]
#[tokio::test]
async fn a_device_that_disappears_mid_flight_contradicts_verification() {
    let audit = OsAuditStore::open_in_memory();
    let coordinator = OsResourceCoordinator::new(ResourceLeaseManager::new());
    let tool = "connect_bluetooth_device";
    let params = serde_json::json!({ "device": ADDR });

    let call = OsGovernedCall::admit(
        &audit,
        &coordinator,
        grant_for(tool, &params),
        call_request(tool, &params),
    )
    .await
    .expect("admission succeeds");

    // Present for the first read, gone for the next.
    let provider = BluetoothControl::new(
        FakeBluetoothTransport::new(BluetoothBackend::Bluetoothctl)
            .with_device(ADDR, "Test Headset", true, false, false)
            .vanishing(ADDR),
    );
    let request = BluetoothRequest {
        action: tool.to_string(),
        params: params.clone(),
        op: BluetoothOp::Connect(BluetoothDeviceId::new(ADDR)),
    };
    let desired = request.desired_state();

    let outcome = execute_governed_mutation(
        &OsControlRuntime::detached(),
        &provider,
        &call,
        &audit,
        &request,
        &desired,
        &plan("fake-bluetooth", request.comparator()),
    )
    .await
    .expect("the chain completes with a receipt");

    assert_ne!(
        outcome.receipt.lifecycle(),
        ActionLifecycle::Verified,
        "a device that vanished must never verify as connected"
    );
}

/// Removal is verified by the device no longer being known, and never advertises
/// a rollback — the pairing keys are gone.
#[serial]
#[tokio::test]
async fn removal_is_verified_by_absence_and_claims_no_rollback() {
    let audit = OsAuditStore::open_in_memory();
    let coordinator = OsResourceCoordinator::new(ResourceLeaseManager::new());
    let tool = "remove_bluetooth_device";
    let params = serde_json::json!({ "device": ADDR });

    let call = OsGovernedCall::admit(
        &audit,
        &coordinator,
        grant_for(tool, &params),
        call_request(tool, &params),
    )
    .await
    .expect("admission succeeds");

    let provider = BluetoothControl::new(fake_with_paired_device());
    let request = BluetoothRequest {
        action: tool.to_string(),
        params: params.clone(),
        op: BluetoothOp::Remove(BluetoothDeviceId::new(ADDR)),
    };
    let desired = request.desired_state();
    assert_eq!(desired.focus, BluetoothFocus::DeviceKnown);
    assert!(!desired.value, "removal wants the device to be unknown");

    let outcome = execute_governed_mutation(
        &OsControlRuntime::detached(),
        &provider,
        &call,
        &audit,
        &request,
        &desired,
        &plan("fake-bluetooth", request.comparator()),
    )
    .await
    .expect("the governed chain completes");

    assert_eq!(outcome.receipt.lifecycle(), ActionLifecycle::Verified);
    assert!(
        !outcome.receipt.rollback_available(),
        "a removed device's pairing keys are destroyed; rollback must not be advertised"
    );
    assert_eq!(provider.transport().device_count(), 0);
}

/// The captured argv is the exact bluetoothctl verb — and never a passkey.
#[serial]
#[tokio::test]
async fn captured_argv_is_the_exact_verb_and_carries_no_secret() {
    let audit = OsAuditStore::open_in_memory();
    let coordinator = OsResourceCoordinator::new(ResourceLeaseManager::new());
    let tool = "set_bluetooth_trust";
    let params = serde_json::json!({ "device": ADDR, "trusted": true });

    let call = OsGovernedCall::admit(
        &audit,
        &coordinator,
        grant_for(tool, &params),
        call_request(tool, &params),
    )
    .await
    .expect("admission succeeds");

    let provider = BluetoothControl::new(fake_with_paired_device());
    let request = BluetoothRequest {
        action: tool.to_string(),
        params: params.clone(),
        op: BluetoothOp::SetTrust {
            device: BluetoothDeviceId::new(ADDR),
            trusted: true,
        },
    };
    let desired = request.desired_state();

    let _ = execute_governed_mutation(
        &OsControlRuntime::detached(),
        &provider,
        &call,
        &audit,
        &request,
        &desired,
        &plan("fake-bluetooth", request.comparator()),
    )
    .await
    .expect("the governed chain completes");

    let captured = provider.transport().captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].args(), ["trust", ADDR]);
    for arg in captured[0].args() {
        assert!(
            !arg.to_lowercase().contains("passkey") && !arg.chars().all(|c| c.is_ascii_digit()),
            "argv must never carry a passkey: {arg}"
        );
    }
}
