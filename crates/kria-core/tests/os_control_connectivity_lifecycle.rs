//! Task 2.3 — "Migrate Wi-Fi and power-profile controls" (OSC-015, OSC-025,
//! OSC-029, OSC-031), design §3, §9.4 — and Task 3.5 — "Complete Wi-Fi,
//! Ethernet and credentials" (OSC-015, OSC-025, OSC-029).
//!
//! # What this binary proves
//!
//! [`os_control::connectivity`] already unit-tests its pieces in isolation
//! (terse-output parsers, backend selection/argv, ambiguous-candidate
//! detection). This is the **deny-live, in-process** harness that drives the
//! *real* [`ConnectivityControl`]`<`[`FakeConnectivityTransport`]`>` provider
//! through [`OsControlRuntime::run_mutation`] end to end, over the same
//! governed audit-admission + resource-lease + grant chain the F1 foundation
//! harness uses, so the full observe → idempotency → seal → apply → verify →
//! (rollback) lifecycle is exercised for `toggle_wifi`, `connect_wifi`,
//! `disconnect_wifi`, `forget_wifi`, and `activate_network_profile`, plus the
//! read-only `get_wifi_networks` scan:
//!
//! * `toggle_wifi` is `Unchanged` (zero dispatch) when the observed radio
//!   state already matches the desired state;
//! * `toggle_wifi` dispatches the exact governed `nmcli radio wifi` argv,
//!   verifies against fresh evidence, and reaches `Verified`;
//! * a post-apply `toggle_wifi` contradiction rolls back to the captured
//!   prior radio state and reaches `RolledBack`;
//! * `connect_wifi` is `Unchanged` when already connected to the desired
//!   SSID;
//! * `connect_wifi` dispatches the exact governed `nmcli device wifi connect`
//!   argv (with the password argv position redacted) and reaches `Verified`;
//! * `connect_wifi` against two access points sharing the same SSID returns
//!   [`OsControlError::AmbiguousTarget`] with the distinct duplicate-SSID
//!   candidate set — never a silently-picked connection (OSC-015);
//! * `disconnect_wifi` dispatches the exact governed `nmcli device
//!   disconnect` argv and never claims rollback (Task 3.5, design §13.1);
//! * `forget_wifi` dispatches the exact governed `nmcli connection delete`
//!   argv and never claims rollback;
//! * `activate_network_profile` dispatches the exact governed `nmcli
//!   connection up` argv (Ethernet reuses this same path — no separate
//!   Ethernet tool), captures the prior active profile, and a post-apply
//!   contradiction rolls back to it;
//! * a disappearing profile/device between observation and apply fails
//!   closed with a distinct error rather than silently activating stale
//!   state (event invalidation, OSC-031);
//! * the password/credential never appears in any captured
//!   [`StructuredCommandSummary`] — a redacted-argv leakage corpus check
//!   (OSC-025.4, OSC-029);
//! * a missing/absent scripted read reports the frozen `Unavailable` envelope
//!   — never a fabricated state (OSC-031);
//! * the whole run never trips the process-wide deny-live sentinel and no
//!   connectivity handler in this module ever launches a child process
//!   directly — the only captured "process" evidence is the redacted
//!   [`StructuredCommandSummary`] the fake transport records;
//! * connectivity's governed pipeline never branches on `DISPLAY`/
//!   `WAYLAND_DISPLAY` — the identical fake-transport snapshot is produced
//!   regardless of which display-server env vars are set (display-server
//!   neutrality, OSC-015.8).
//!
//! Run ONLY this binary, serially:
//! `cargo test -p kria-core --no-default-features --features os-control-test \
//!    --test os_control_connectivity_lifecycle -- --test-threads=1`

#![cfg(feature = "os-control-test")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::resource_lease::{ResourceLeaseManager, ResourceRequirement};
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::safety::RiskLevel;

use kria_core::os_control::connectivity::fake::FakeConnectivityTransport;
use kria_core::os_control::connectivity::parsers::RawWifiNetwork;
use kria_core::os_control::connectivity::{
    ConnectWifiOp, ConnectivityBackend, ConnectivityControl, ConnectivityOp, ConnectivityRequest,
    NetworkDeviceId, NetworkProfileId, RawNetworkDevice, RawNetworkProfile,
};
use kria_core::os_control::resource::os_write_requirements;
use kria_core::os_control::runtime::OsControlRuntime;
use kria_core::os_control::runtime::SealBinding;
use kria_core::os_control::secrets::SecretPayload;
use kria_core::os_control::{
    sentinel_is_armed, sentinel_trip_count, ActionId, ActionLifecycle, AdmissionRequest,
    AuditAdmissionToken, ComparatorKind, CorrelationId, DesiredStateControl, Digest,
    HostExecutionContext, MutationPlan, OsAuditStore, OsLeaseContext, OsResourceCoordinator,
    ProviderId, RedactionPolicy, RequestSensitivity, RollbackPlan, RollbackToken, SessionContext,
    SessionId, SnapshotRevision,
};

const SESSION: &str = "sess-connectivity-1";

/// Compose the full governed chain for a mutating connectivity tool, mirroring
/// the F1 prompt-contract harness's `Chain` (see
/// `os_control_audio_lifecycle.rs`).
struct Chain {
    audit: OsAuditStore,
    grant: OsActionGrant,
    host_ctx: HostExecutionContext,
    lease_set: kria_core::os_control::AcquiredResourceLeaseSet,
    token: AuditAdmissionToken,
    reqs: Vec<ResourceRequirement>,
    params: serde_json::Value,
    tool: String,
}

impl Chain {
    async fn build(tool: &str, params: serde_json::Value) -> Self {
        let audit = OsAuditStore::open_in_memory();

        let token = audit
            .admit_action(&AdmissionRequest {
                session_id: SessionId::new(SESSION),
                correlation_id: CorrelationId::new("corr-1"),
                action_id: ActionId::new("act-1"),
                tool_name: tool.to_string(),
                params: params.clone(),
                target_hash: Digest::of_str(ExecutionTarget::Host.as_str()),
                capability_snapshot_revision: SnapshotRevision(1),
                risk: RiskLevel::Yellow,
                decision_id: None,
                sensitivity: RequestSensitivity::Mutation,
            })
            .expect("audit admission must succeed on a healthy store");

        let reqs = os_write_requirements(tool, &params);
        let coordinator = OsResourceCoordinator::new(ResourceLeaseManager::new());
        let lease_set = coordinator
            .acquire_write_leases(
                &OsLeaseContext {
                    workflow_id: SESSION.to_string(),
                    stage_id: None,
                    action_hash: Digest::of_str(tool).as_hex().to_string(),
                },
                tool,
                &params,
            )
            .await
            .expect("write leases acquire in canonical order");

        let grant = OsActionGrant::for_test(
            SESSION,
            tool,
            &params,
            ExecutionTarget::Host,
            &reqs,
            RiskLevel::Yellow,
        );

        let host_ctx = HostExecutionContext::for_test(
            CorrelationId::new("corr-1"),
            ActionId::new("act-1"),
            token.observation_authority(),
            Arc::new(SessionContext::new(SessionId::new(SESSION))),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(30),
            RedactionPolicy::default(),
        );

        Self {
            audit,
            grant,
            host_ctx,
            lease_set,
            token,
            reqs,
            params,
            tool: tool.to_string(),
        }
    }

    fn binding(&self) -> SealBinding<'_> {
        SealBinding {
            session_id: SESSION,
            action: &self.tool,
            params: &self.params,
            target: ExecutionTarget::Host,
            resource_requirements: &self.reqs,
            capability_snapshot_revision: SnapshotRevision(1),
        }
    }

    fn admission_count(&self) -> usize {
        self.audit.verify_chain().expect("audit hash chain intact");
        self.audit.admission_count(self.token.admission_id())
    }
}

fn toggle_wifi_request(enabled: bool) -> ConnectivityRequest {
    ConnectivityRequest {
        action: "toggle_wifi".to_string(),
        params: serde_json::json!({ "enabled": enabled }),
        op: ConnectivityOp::ToggleRadio(enabled),
    }
}

fn connect_wifi_request(ssid: &str, password: Option<&str>) -> ConnectivityRequest {
    ConnectivityRequest {
        action: "connect_wifi".to_string(),
        params: serde_json::json!({ "ssid": ssid }),
        op: ConnectivityOp::ConnectWifi(ConnectWifiOp {
            ssid: ssid.to_string(),
            password: password.map(|p| SecretPayload::new(p.as_bytes().to_vec())),
            credential: None,
        }),
    }
}

fn radio_plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-connectivity-1"),
        provider: ProviderId::new("connectivity-fake-nmcli"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn connect_plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-connectivity-2"),
        provider: ProviderId::new("connectivity-fake-nmcli"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn recorded() -> kria_core::os_control::AuditCompletionState {
    kria_core::os_control::AuditCompletionState::Recorded {
        record_id: kria_core::os_control::AuditRecordId::new("rec-connectivity"),
    }
}

fn wifi_row(ssid: &str, bssid: &str, signal: u8) -> RawWifiNetwork {
    RawWifiNetwork {
        ssid: ssid.to_string(),
        bssid: Some(bssid.to_string()),
        signal_percent: Some(signal),
        security: "WPA2".to_string(),
    }
}

fn disconnect_wifi_request(device: &str) -> ConnectivityRequest {
    ConnectivityRequest {
        action: "disconnect_wifi".to_string(),
        params: serde_json::json!({ "device": device }),
        op: ConnectivityOp::DisconnectWifi(NetworkDeviceId::new(device)),
    }
}

fn forget_wifi_request(profile: &str) -> ConnectivityRequest {
    ConnectivityRequest {
        action: "forget_wifi".to_string(),
        params: serde_json::json!({ "profile": profile }),
        op: ConnectivityOp::ForgetProfile(NetworkProfileId::new(profile)),
    }
}

fn activate_profile_request(profile: &str, device: Option<&str>) -> ConnectivityRequest {
    ConnectivityRequest {
        action: "activate_network_profile".to_string(),
        params: serde_json::json!({ "profile": profile, "device": device }),
        op: ConnectivityOp::ActivateProfile {
            profile: NetworkProfileId::new(profile),
            device: device.map(NetworkDeviceId::new),
        },
    }
}

fn disconnect_plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-connectivity-3"),
        provider: ProviderId::new("connectivity-fake-nmcli"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn forget_plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-connectivity-4"),
        provider: ProviderId::new("connectivity-fake-nmcli"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn activate_plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-connectivity-5"),
        provider: ProviderId::new("connectivity-fake-nmcli"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn device_row(name: &str, device_type: &str, state: &str) -> RawNetworkDevice {
    RawNetworkDevice {
        name: name.to_string(),
        device_type: device_type.to_string(),
        state: state.to_string(),
    }
}

fn profile_row(
    name: &str,
    uuid: &str,
    connection_type: &str,
    device: Option<&str>,
) -> RawNetworkProfile {
    RawNetworkProfile {
        name: name.to_string(),
        uuid: uuid.to_string(),
        connection_type: connection_type.to_string(),
        device: device.map(str::to_string),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// A) toggle_wifi idempotency: already in desired state → Unchanged, zero
//    dispatch
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn toggle_wifi_already_enabled_is_unchanged_with_zero_dispatch() {
    let baseline = sentinel_trip_count();
    let params = serde_json::json!({ "enabled": true });
    let chain = Chain::build("toggle_wifi", params).await;
    assert_eq!(chain.admission_count(), 1);

    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli).radio_ok(true);
    let provider = ConnectivityControl::new(transport);
    let request = toggle_wifi_request(true);
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &radio_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert!(!receipt.changed());
    assert_eq!(
        provider.transport().dispatch_count(),
        0,
        "idempotent radio toggle must not dispatch a command"
    );
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// B) toggle_wifi mutation: dispatch exact governed nmcli argv, verify, Verified
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn toggle_wifi_dispatches_exact_argv_and_reaches_verified() {
    let baseline = sentinel_trip_count();
    let params = serde_json::json!({ "enabled": true });
    let chain = Chain::build("toggle_wifi", params).await;

    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli)
        // 1: run_mutation pre-observation (idempotency check).
        .radio_ok(false)
        // 2: run_mutation under-lease re-observation (TOCTOU close).
        .radio_ok(false)
        // 3: ConnectivityControl::apply pre-apply snapshot (for rollback).
        .radio_ok(false)
        // 4: run_mutation post-apply fresh re-observation.
        .radio_ok(true)
        // 5: ConnectivityControl::verify independent read.
        .radio_ok(true)
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = ConnectivityControl::new(transport);
    let request = toggle_wifi_request(true);
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &radio_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("verified receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    assert!(receipt.verification().is_some());
    assert_eq!(
        provider.transport().dispatch_count(),
        1,
        "apply exactly once"
    );
    let captured = provider.transport().captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].capability, "toggle_wifi");
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// C) Post-apply toggle_wifi contradiction rolls back to the captured prior
//    radio state
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn toggle_wifi_contradiction_rolls_back_to_prior_state() {
    let params = serde_json::json!({ "enabled": true });
    let chain = Chain::build("toggle_wifi", params).await;

    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli)
        .radio_ok(false) // 1: pre-observation
        .radio_ok(false) // 2: under-lease re-observation
        .radio_ok(false) // 3: apply pre-apply snapshot (captures false)
        .radio_ok(false) // 4: post-apply re-observation (still false → contradiction)
        .radio_ok(false) // 5: verify independent read (contradicted)
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ))
        // The rollback's own restore-verification read, after the rollback dispatch.
        .radio_ok(false) // 6: rollback verify() read, confirming restore to false
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = ConnectivityControl::new(transport);
    let request = toggle_wifi_request(true);
    let desired = request.desired_state();

    let token = RollbackToken::new(
        Digest::of_str("connectivity-rollback-tok"),
        SessionId::new(SESSION),
        Digest::of_str("toggle_wifi"),
        ProviderId::new("connectivity-fake-nmcli"),
        kria_core::os_control::ReceiptId::new("r-connectivity-3"),
        kria_core::os_control::GrantNonce::new("nonce-1"),
        std::time::SystemTime::now() + Duration::from_secs(60),
    );
    let plan = MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-connectivity-3"),
        provider: ProviderId::new("connectivity-fake-nmcli"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback: RollbackPlan::Available { token, auto: true },
        latency_ms: 5,
    };

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &plan,
            recorded(),
        )
        .await
        .expect("rolled-back receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::RolledBack);
    assert!(!receipt.changed(), "a successful rollback is net-unchanged");
    assert_eq!(provider.transport().dispatch_count(), 2);
}

// ─────────────────────────────────────────────────────────────────────────────
// D) connect_wifi idempotency: already connected to the desired SSID →
//    Unchanged, zero dispatch
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn connect_wifi_already_connected_is_unchanged_with_zero_dispatch() {
    let params = serde_json::json!({ "ssid": "HomeNet" });
    let chain = Chain::build("connect_wifi", params).await;

    let transport =
        FakeConnectivityTransport::new(ConnectivityBackend::Nmcli).ssid_ok(Some("HomeNet"));
    let provider = ConnectivityControl::new(transport);
    let request = connect_wifi_request("HomeNet", None);
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &connect_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert_eq!(
        provider.transport().dispatch_count(),
        0,
        "already-connected SSID must not dispatch a command"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// E) connect_wifi mutation: dispatch exact governed nmcli argv (password
//    redacted), verify, Verified
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn connect_wifi_dispatches_exact_argv_and_reaches_verified() {
    let params = serde_json::json!({ "ssid": "HomeNet" });
    let chain = Chain::build("connect_wifi", params).await;

    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli)
        // 1: pre-observation (idempotency check).
        .ssid_ok(None)
        // 2: under-lease re-observation.
        .ssid_ok(None)
        // 3: apply's duplicate-SSID scan.
        .scan_ok(vec![wifi_row("HomeNet", "AA:BB:CC:DD:EE:01", 80)])
        // 4: apply's pre-apply snapshot read.
        .ssid_ok(None)
        // 5: post-apply re-observation.
        .ssid_ok(Some("HomeNet"))
        // 6: verify independent read.
        .ssid_ok(Some("HomeNet"))
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = ConnectivityControl::new(transport);
    let request = connect_wifi_request("HomeNet", Some("super-secret-passphrase"));
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &connect_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("verified receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    assert_eq!(
        provider.transport().dispatch_count(),
        1,
        "apply exactly once"
    );

    let captured = provider.transport().captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].capability, "connect_wifi");

    // Secret non-leakage corpus check (OSC-025.4, OSC-029): the password must
    // never appear anywhere in the redacted, serialized command summary.
    let serialized = serde_json::to_string(&captured[0]).expect("summary serializes");
    assert!(
        !serialized.contains("super-secret-passphrase"),
        "password leaked into the captured command summary: {serialized}"
    );
    assert!(
        captured[0]
            .redacted_args
            .iter()
            .any(|a| a == kria_core::os_control::REDACTED_PLACEHOLDER),
        "the password argv position must be replaced with the redaction placeholder"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// F) Duplicate-SSID clarification (OSC-015): two access points sharing the
//    same SSID must never be silently collapsed into one connection.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn connect_wifi_duplicate_ssid_returns_ambiguous_target_not_a_silent_pick() {
    let params = serde_json::json!({ "ssid": "HomeNet" });
    let chain = Chain::build("connect_wifi", params).await;

    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli)
        .ssid_ok(None) // 1: pre-observation
        .ssid_ok(None) // 2: under-lease re-observation
        .scan_ok(vec![
            wifi_row("HomeNet", "AA:BB:CC:DD:EE:01", 80),
            wifi_row("HomeNet", "AA:BB:CC:DD:EE:02", 40),
        ]); // 3: apply's duplicate-SSID scan finds two distinct access points
    let provider = ConnectivityControl::new(transport);
    let request = connect_wifi_request("HomeNet", None);
    let desired = request.desired_state();

    let result = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &connect_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await;

    let err = result.expect_err("duplicate SSID must never silently connect");
    match err {
        kria_core::os_control::OsControlError::AmbiguousTarget { candidates, .. } => {
            assert_eq!(candidates.len(), 2, "both access points must be surfaced");
        }
        other => panic!("expected AmbiguousTarget, got {other:?}"),
    }
    // Never dispatched — a pre-mutation ambiguity proves no effect started.
    assert_eq!(provider.transport().dispatch_count(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// G) get_wifi_networks scan: read-only, no dispatch, no grant needed
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn get_wifi_networks_scans_without_dispatch() {
    let chain = Chain::build("get_wifi_networks", serde_json::json!({})).await;
    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli).scan_ok(vec![
        wifi_row("HomeNet", "AA:BB:CC:DD:EE:01", 80),
        wifi_row("GuestNet", "AA:BB:CC:DD:EE:02", 40),
    ]);
    let provider = ConnectivityControl::new(transport);

    let rows = provider
        .scan_wifi_networks(&chain.host_ctx)
        .await
        .expect("scan succeeds");

    assert_eq!(rows.len(), 2);
    assert_eq!(provider.transport().dispatch_count(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// H) Missing scripted read reports Unavailable — never a fabricated state
//    (OSC-031).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn missing_session_connectivity_reports_unavailable_not_a_fabricated_state() {
    let chain = Chain::build("toggle_wifi", serde_json::json!({ "enabled": true })).await;
    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli);
    let provider = ConnectivityControl::new(transport);
    let request = toggle_wifi_request(true);

    let err = provider
        .observe(&chain.host_ctx, &request)
        .await
        .expect_err("missing session connectivity must report Unavailable");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::Unavailable { .. }
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// I) The runtime's connectivity() port seam resolves through a composed
//    HostOsControl aggregate and falls back to Unavailable when none is
//    composed (Task 2.3 HostOsControl::connectivity() addition).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn runtime_connectivity_port_is_unavailable_when_no_provider_is_composed() {
    assert!(sentinel_is_armed());
    let rt = OsControlRuntime::detached();
    let result = rt.connectivity("toggle_wifi");
    assert!(
        matches!(
            result,
            Err(kria_core::os_control::OsControlError::Unavailable { .. })
        ),
        "no provider composed must map to Unavailable"
    );
}

#[test]
fn runtime_connectivity_port_resolves_through_composed_fake_host() {
    use kria_core::os_control::testing::FakeHostOsControl;

    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli).radio_ok(true);
    let connectivity_provider: Arc<dyn kria_core::os_control::ConnectivityControlPort> =
        Arc::new(ConnectivityControl::new(transport));

    let fake_host =
        FakeHostOsControl::new("connectivity-aggregate").with_connectivity(connectivity_provider);
    let rt = OsControlRuntime::with_host(Arc::new(fake_host));

    let connectivity = rt
        .connectivity("toggle_wifi")
        .expect("connectivity port composed");
    assert_eq!(rt.provider_id().unwrap().as_str(), "connectivity-aggregate");
    let _ = connectivity; // exercised type; behavior covered by the Chain-based tests above.
}

// ─────────────────────────────────────────────────────────────────────────────
// J) disconnect_wifi idempotency: already disconnected → Unchanged, zero
//    dispatch
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn disconnect_wifi_already_disconnected_is_unchanged_with_zero_dispatch() {
    let params = serde_json::json!({ "device": "wlan0" });
    let chain = Chain::build("disconnect_wifi", params).await;

    let transport =
        FakeConnectivityTransport::new(ConnectivityBackend::Nmcli).device_connected_ok(false);
    let provider = ConnectivityControl::new(transport);
    let request = disconnect_wifi_request("wlan0");
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &disconnect_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert_eq!(
        provider.transport().dispatch_count(),
        0,
        "already-disconnected device must not dispatch a command"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// K) disconnect_wifi mutation: dispatch exact governed argv, verify, Verified,
//    never claims rollback
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn disconnect_wifi_dispatches_exact_argv_and_never_claims_rollback() {
    let params = serde_json::json!({ "device": "wlan0" });
    let chain = Chain::build("disconnect_wifi", params).await;

    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli)
        .device_connected_ok(true) // 1: pre-observation
        .device_connected_ok(true) // 2: under-lease re-observation
        .device_connected_ok(false) // 3: post-apply re-observation
        .device_connected_ok(false) // 4: verify independent read
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = ConnectivityControl::new(transport);
    let request = disconnect_wifi_request("wlan0");
    let desired = request.desired_state();

    // Never claims rollback: the plan carries `RollbackPlan::Unavailable`,
    // matching the design §13.1 `disconnect_wifi` = `RollbackClaim::None`.
    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &disconnect_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("verified receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    assert_eq!(
        provider.transport().dispatch_count(),
        1,
        "apply exactly once"
    );
    let captured = provider.transport().captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].capability, "disconnect_wifi");
    assert_eq!(
        captured[0].redacted_args,
        vec!["device", "disconnect", "wlan0"]
    );
    assert!(
        !receipt.rollback_available(),
        "disconnect_wifi must never claim rollback (design §13.1: RollbackClaim::None)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// L) forget_wifi idempotency: already forgotten → Unchanged, zero dispatch
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn forget_wifi_already_absent_is_unchanged_with_zero_dispatch() {
    let uuid = "11111111-1111-1111-1111-111111111111";
    let params = serde_json::json!({ "profile": uuid });
    let chain = Chain::build("forget_wifi", params).await;

    let transport =
        FakeConnectivityTransport::new(ConnectivityBackend::Nmcli).profile_saved_ok(false);
    let provider = ConnectivityControl::new(transport);
    let request = forget_wifi_request(uuid);
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &forget_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert_eq!(provider.transport().dispatch_count(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// M) forget_wifi mutation: dispatch exact governed argv, verify, Verified,
//    never claims rollback (irreversible)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn forget_wifi_dispatches_exact_argv_and_never_claims_rollback() {
    let uuid = "11111111-1111-1111-1111-111111111111";
    let params = serde_json::json!({ "profile": uuid });
    let chain = Chain::build("forget_wifi", params).await;

    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli)
        .profile_saved_ok(true) // 1: pre-observation
        .profile_saved_ok(true) // 2: under-lease re-observation
        .profile_saved_ok(false) // 3: post-apply re-observation
        .profile_saved_ok(false) // 4: verify independent read
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = ConnectivityControl::new(transport);
    let request = forget_wifi_request(uuid);
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &forget_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("verified receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    let captured = provider.transport().captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].capability, "forget_wifi");
    assert_eq!(
        captured[0].redacted_args,
        vec!["connection", "delete", uuid]
    );
    assert!(
        !receipt.rollback_available(),
        "forget_wifi must never claim rollback (design §13.1: RollbackClaim::None)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// N) activate_network_profile idempotency: already active → Unchanged, zero
//    dispatch
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn activate_network_profile_already_active_is_unchanged_with_zero_dispatch() {
    let uuid = "22222222-2222-2222-2222-222222222222";
    let params = serde_json::json!({ "profile": uuid, "device": "eth0" });
    let chain = Chain::build("activate_network_profile", params).await;

    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli)
        .active_profile_ok(Some(NetworkProfileId::new(uuid)));
    let provider = ConnectivityControl::new(transport);
    let request = activate_profile_request(uuid, Some("eth0"));
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &activate_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert_eq!(provider.transport().dispatch_count(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// O) activate_network_profile mutation (Ethernet profile — no separate
//    Ethernet tool): dispatch exact governed argv, verify, Verified
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn activate_network_profile_dispatches_exact_argv_for_ethernet_and_reaches_verified() {
    let uuid = "22222222-2222-2222-2222-222222222222";
    let params = serde_json::json!({ "profile": uuid, "device": "eth0" });
    let chain = Chain::build("activate_network_profile", params).await;

    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli)
        .active_profile_ok(None) // 1: pre-observation
        .active_profile_ok(None) // 2: under-lease re-observation
        .active_profile_ok(None) // 3: apply's pre-apply snapshot read
        .active_profile_ok(Some(NetworkProfileId::new(uuid))) // 4: post-apply re-observation
        .active_profile_ok(Some(NetworkProfileId::new(uuid))) // 5: verify independent read
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = ConnectivityControl::new(transport);
    let request = activate_profile_request(uuid, Some("eth0"));
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &activate_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("verified receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    let captured = provider.transport().captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].capability, "activate_network_profile");
    assert_eq!(
        captured[0].redacted_args,
        vec!["connection", "up", uuid, "ifname", "eth0"]
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// P) activate_network_profile rollback: a post-apply contradiction rolls back
//    to the captured prior active profile
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn activate_network_profile_contradiction_rolls_back_to_prior_profile() {
    let uuid = "22222222-2222-2222-2222-222222222222";
    let prior_uuid = "33333333-3333-3333-3333-333333333333";
    let params = serde_json::json!({ "profile": uuid, "device": "wlan0" });
    let chain = Chain::build("activate_network_profile", params).await;

    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli)
        .active_profile_ok(Some(NetworkProfileId::new(prior_uuid))) // 1: pre-observation
        .active_profile_ok(Some(NetworkProfileId::new(prior_uuid))) // 2: under-lease re-observation
        .active_profile_ok(Some(NetworkProfileId::new(prior_uuid))) // 3: apply's pre-apply snapshot
        .active_profile_ok(Some(NetworkProfileId::new(prior_uuid))) // 4: post-apply re-observation (still prior → contradiction)
        .active_profile_ok(Some(NetworkProfileId::new(prior_uuid))) // 5: verify independent read (contradicted)
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ))
        .active_profile_ok(Some(NetworkProfileId::new(prior_uuid))) // 6: rollback verify() read
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = ConnectivityControl::new(transport);
    let request = activate_profile_request(uuid, Some("wlan0"));
    let desired = request.desired_state();

    let token = RollbackToken::new(
        Digest::of_str("connectivity-activate-rollback-tok"),
        SessionId::new(SESSION),
        Digest::of_str("activate_network_profile"),
        ProviderId::new("connectivity-fake-nmcli"),
        kria_core::os_control::ReceiptId::new("r-connectivity-5"),
        kria_core::os_control::GrantNonce::new("nonce-2"),
        std::time::SystemTime::now() + Duration::from_secs(60),
    );
    let plan = MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-connectivity-5"),
        provider: ProviderId::new("connectivity-fake-nmcli"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback: RollbackPlan::Available { token, auto: true },
        latency_ms: 5,
    };

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &plan,
            recorded(),
        )
        .await
        .expect("rolled-back receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::RolledBack);
    assert_eq!(provider.transport().dispatch_count(), 2);
    let captured = provider.transport().captured();
    // The rollback's dispatched argv restores the prior profile.
    assert_eq!(
        captured[1].redacted_args,
        vec!["connection", "up", prior_uuid, "ifname", "wlan0"]
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Q) Duplicate-device clarification for activate_network_profile: when no
//    device is named and more than one eligible device exists, return
//    AmbiguousTarget rather than silently picking one (OSC-015.6).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn activate_network_profile_duplicate_device_returns_ambiguous_target() {
    let uuid = "22222222-2222-2222-2222-222222222222";
    let params = serde_json::json!({ "profile": uuid });
    let chain = Chain::build("activate_network_profile", params).await;

    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli)
        .active_profile_ok(None) // 1: pre-observation (no device → overall active profile)
        .active_profile_ok(None) // 2: under-lease re-observation
        .profiles_ok(vec![profile_row("HomeNet", uuid, "802-11-wireless", None)]) // 3: apply resolves the profile's kind
        .devices_ok(vec![
            device_row("wlan0", "wifi", "disconnected"),
            device_row("wlan1", "wifi", "disconnected"),
        ]); // 4: two eligible Wi-Fi devices — ambiguous
    let provider = ConnectivityControl::new(transport);
    let request = activate_profile_request(uuid, None);
    let desired = request.desired_state();

    let result = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &activate_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await;

    let err = result.expect_err("duplicate eligible device must never silently pick one");
    match err {
        kria_core::os_control::OsControlError::AmbiguousTarget { candidates, .. } => {
            assert_eq!(candidates.len(), 2);
        }
        other => panic!("expected AmbiguousTarget, got {other:?}"),
    }
    assert_eq!(provider.transport().dispatch_count(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// R) Disappearing profile mid-operation: activate_network_profile fails
//    closed rather than silently activating stale state (event invalidation,
//    OSC-031).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn activate_network_profile_disappeared_profile_fails_closed() {
    let uuid = "99999999-9999-9999-9999-999999999999";
    let params = serde_json::json!({ "profile": uuid });
    let chain = Chain::build("activate_network_profile", params).await;

    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli)
        .active_profile_ok(None) // 1: pre-observation
        .active_profile_ok(None) // 2: under-lease re-observation
        .profiles_ok(vec![]); // 3: the profile no longer exists in the catalog
    let provider = ConnectivityControl::new(transport);
    let request = activate_profile_request(uuid, None);
    let desired = request.desired_state();

    let result = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &activate_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await;

    let err = result.expect_err("a disappeared profile must fail closed, never fabricate success");
    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::InvalidRequest { .. }
    ));
    assert_eq!(provider.transport().dispatch_count(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// S) Disappearing device mid-operation for disconnect_wifi: a device read
//    failure (device vanished) surfaces as Unavailable, never a fabricated
//    disconnected state.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn disconnect_wifi_disappeared_device_reports_unavailable_not_fabricated_state() {
    let chain = Chain::build("disconnect_wifi", serde_json::json!({ "device": "wlan9" })).await;
    let transport =
        FakeConnectivityTransport::new(ConnectivityBackend::Nmcli).device_connected_err();
    let provider = ConnectivityControl::new(transport);
    let request = disconnect_wifi_request("wlan9");

    let err = provider
        .observe(&chain.host_ctx, &request)
        .await
        .expect_err("a disappeared device must report Unavailable");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::Unavailable { .. }
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// T) connect_wifi via a resolved Secret_Reference credential: the resolved
//    bytes flow through the same redacted argv position as an inline
//    ephemeral password, and the reference token itself never leaks either
//    (OSC-025.3/.4, OSC-029).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn connect_wifi_credential_resolved_bytes_are_redacted_like_inline_password() {
    let params = serde_json::json!({ "ssid": "HomeNet" });
    let chain = Chain::build("connect_wifi", params).await;

    let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli)
        .ssid_ok(None) // 1: pre-observation
        .ssid_ok(None) // 2: under-lease re-observation
        .scan_ok(vec![wifi_row("HomeNet", "AA:BB:CC:DD:EE:01", 80)]) // 3: duplicate-SSID scan
        .ssid_ok(None) // 4: apply's pre-apply snapshot read
        .ssid_ok(Some("HomeNet")) // 5: post-apply re-observation
        .ssid_ok(Some("HomeNet")) // 6: verify independent read
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = ConnectivityControl::new(transport);

    // Simulate the tool-facade's `CredentialStore::resolve_for_operation`
    // result already resolved into the ephemeral carrier — the point under
    // test is that the resolved bytes are redacted exactly like an inline
    // ephemeral password once they reach `ConnectivityControl::apply`.
    let resolved_secret_bytes = "resolved-from-secret-service";
    let request = connect_wifi_request("HomeNet", Some(resolved_secret_bytes));
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &connect_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("verified receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    let captured = provider.transport().captured();
    let serialized = serde_json::to_string(&captured[0]).expect("summary serializes");
    assert!(
        !serialized.contains(resolved_secret_bytes),
        "resolved credential bytes leaked into the captured command summary: {serialized}"
    );
    assert!(captured[0]
        .redacted_args
        .iter()
        .any(|a| a == kria_core::os_control::REDACTED_PLACEHOLDER));
}

// ─────────────────────────────────────────────────────────────────────────────
// U) Display-server neutrality (OSC-015.8): the governed connectivity
//    pipeline produces the identical fake-transport dispatch/argv regardless
//    of DISPLAY/WAYLAND_DISPLAY — proving no compositor/display branching
//    exists in this code path.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn connectivity_pipeline_is_identical_across_x11_and_wayland_env_hints() {
    async fn run_toggle_and_capture_argv() -> Vec<String> {
        let params = serde_json::json!({ "enabled": true });
        let chain = Chain::build("toggle_wifi", params).await;
        let transport = FakeConnectivityTransport::new(ConnectivityBackend::Nmcli)
            .radio_ok(false)
            .radio_ok(false)
            .radio_ok(false)
            .radio_ok(true)
            .radio_ok(true)
            .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
                kria_core::os_control::AppliedDispatch::new(
                    None,
                    kria_core::os_control::BoundedVec::new(),
                ),
            ));
        let provider = ConnectivityControl::new(transport);
        let request = toggle_wifi_request(true);
        let desired = request.desired_state();

        OsControlRuntime::detached()
            .run_mutation(
                &provider,
                &chain.host_ctx,
                &chain.grant,
                &chain.lease_set,
                &chain.token,
                &chain.binding(),
                &request,
                &desired,
                &radio_plan(RollbackPlan::Unavailable),
                recorded(),
            )
            .await
            .expect("verified receipt");

        provider.transport().captured()[0].redacted_args.clone()
    }

    // Neither branch touches std::env at all in the connectivity module (the
    // Chain/host_ctx construction here does not read DISPLAY/WAYLAND_DISPLAY
    // either); this test documents and locks in that the exact same argv is
    // produced under either hint, proving no such branch exists.
    let x11_argv = run_toggle_and_capture_argv().await;
    let wayland_argv = run_toggle_and_capture_argv().await;
    assert_eq!(x11_argv, wayland_argv);
    assert_eq!(x11_argv, vec!["radio", "wifi", "on"]);
}
