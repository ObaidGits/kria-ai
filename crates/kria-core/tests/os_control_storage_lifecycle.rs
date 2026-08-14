//! Task 3.2 — "Complete storage and removable-media lifecycle" (OSC-012,
//! OSC-030), design §3, §9.1, §10.1, §12.
//!
//! # What this binary proves
//!
//! [`os_control::storage`] already unit-tests its pieces in isolation (digest
//! binding, desired-state mapping, id sanitization). This is the
//! **deny-live, in-process** harness that drives the *real*
//! [`StorageControl`]`<`[`FakeStorageTransport`]`>` provider through
//! [`OsControlRuntime::run_mutation`] end to end, over the same governed
//! audit-admission + resource-lease + grant chain the other domain lifecycle
//! harnesses use, proving:
//!
//! * `mount_device` on an already-mounted device is `Unchanged` (zero
//!   dispatch) — the idempotency half of OSC-012's mount contract;
//! * `mount_device` on an unmounted device dispatches exactly once and
//!   reaches `Verified` once the mount is confirmed through a **fresh**
//!   re-observation (OSC-012.7: verification never reuses the apply-time
//!   observation);
//! * `unmount_device`/`eject_device` on an already-unmounted device are
//!   `Unchanged` with zero dispatch;
//! * a busy `unmount_device`/`eject_device` (open file handle) surfaces the
//!   distinct [`OsControlError::ResourceBusy`] blocking state — never a
//!   forced retry (OSC-012.3, OSC-012.4);
//! * `get_storage_health` reports a degraded/unavailable evidence state
//!   honestly rather than fabricating a healthy/unhealthy status
//!   (OSC-012.5, OSC-031);
//! * the runtime's `storage()` port resolves `Unavailable` with no provider
//!   composed and resolves through a composed `FakeHostOsControl` otherwise;
//! * the whole run never trips the process-wide deny-live sentinel and no
//!   storage handler in this module ever opens a system-bus connection —
//!   every effect is the scripted [`FakeStorageTransport`].
//!
//! Run ONLY this binary, serially:
//! `cargo test -p kria-core --no-default-features --features os-control-test \
//!    --test os_control_storage_lifecycle -- --test-threads=1`

#![cfg(feature = "os-control-test")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::resource_lease::{ResourceLeaseManager, ResourceRequirement};
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::safety::RiskLevel;

use kria_core::os_control::resource::os_write_requirements;
use kria_core::os_control::runtime::SealBinding;
use kria_core::os_control::storage::fake::FakeStorageTransport;
use kria_core::os_control::storage::{
    HealthAvailability, StorageControl, StorageDeviceId, StorageHealthReport, StorageMountState,
    StorageOp, StorageRequest,
};
use kria_core::os_control::{
    device_busy_error, sentinel_is_armed, sentinel_trip_count, ActionId, ActionLifecycle,
    AdmissionRequest, AppliedDispatch, ApplyOutcome, AuditAdmissionToken, ComparatorKind,
    CorrelationId, Digest, HostExecutionContext, MutationPlan, OsAuditStore,
    OsControlError, OsControlRuntime, OsLeaseContext, OsResourceCoordinator, ProviderId,
    RedactionPolicy, RequestSensitivity, RollbackPlan, SessionContext, SessionId,
    SnapshotRevision,
};

const SESSION: &str = "sess-storage-1";

/// Compose the full governed chain for a mutating storage tool, mirroring the
/// F1 prompt-contract harness's `Chain` (see `os_control_files_lifecycle.rs`).
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
                risk: RiskLevel::Red,
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
            RiskLevel::Red,
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

fn mount_request(device: &str) -> StorageRequest {
    StorageRequest {
        action: "mount_device".to_string(),
        params: serde_json::json!({ "device": device }),
        op: StorageOp::Mount {
            device: StorageDeviceId::new(device),
            filesystem: None,
        },
    }
}

fn unmount_request(device: &str) -> StorageRequest {
    StorageRequest {
        action: "unmount_device".to_string(),
        params: serde_json::json!({ "device": device }),
        op: StorageOp::Unmount {
            device: StorageDeviceId::new(device),
        },
    }
}

fn eject_request(device: &str) -> StorageRequest {
    StorageRequest {
        action: "eject_device".to_string(),
        params: serde_json::json!({ "device": device }),
        op: StorageOp::Eject {
            device: StorageDeviceId::new(device),
        },
    }
}

fn plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-storage-1"),
        provider: ProviderId::new("storage-fake-udisks2"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn recorded() -> kria_core::os_control::AuditCompletionState {
    kria_core::os_control::AuditCompletionState::Recorded {
        record_id: kria_core::os_control::AuditRecordId::new("rec-storage"),
    }
}

fn mounted(device: &str, mount_point: &str) -> StorageMountState {
    StorageMountState::new(
        StorageDeviceId::new(device),
        None,
        true,
        Some(mount_point.to_string()),
    )
}

fn unmounted(device: &str) -> StorageMountState {
    StorageMountState::new(StorageDeviceId::new(device), None, false, None)
}

// ─────────────────────────────────────────────────────────────────────────────
// A) mount_device idempotency: already mounted → Unchanged, zero dispatch
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn mount_device_already_mounted_is_unchanged_with_zero_dispatch() {
    let baseline = sentinel_trip_count();
    let params = serde_json::json!({ "device": "usb-1" });
    let chain = Chain::build("mount_device", params).await;
    assert_eq!(chain.admission_count(), 1);

    let transport = FakeStorageTransport::new().mount_state_ok(mounted("usb-1", "/media/usb-1"));
    let provider = StorageControl::new(transport);
    let request = mount_request("usb-1");
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
            &plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert!(!receipt.changed());
    assert_eq!(
        provider.transport().dispatch_count(),
        0,
        "already-mounted device must not dispatch a mount"
    );
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// B) mount_device on an unmounted device: dispatches once, verifies through a
//    FRESH re-observation (OSC-012.7), reaches Verified.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn mount_device_dispatches_once_and_verifies_via_fresh_observation() {
    let baseline = sentinel_trip_count();
    let params = serde_json::json!({ "device": "usb-1" });
    let chain = Chain::build("mount_device", params).await;

    let transport = FakeStorageTransport::new()
        // 1: run_mutation pre-observation (idempotency check) — unmounted.
        .mount_state_ok(unmounted("usb-1"))
        // 2: run_mutation under-lease re-observation (TOCTOU close).
        .mount_state_ok(unmounted("usb-1"))
        // 3: run_mutation post-apply fresh re-observation.
        .mount_state_ok(mounted("usb-1", "/media/usb-1"))
        // 4: StorageControl::verify's own fresh, independent re-read
        //    (OSC-012.7 — never reuses the apply-time observation).
        .mount_state_ok(mounted("usb-1", "/media/usb-1"))
        .mount_outcome(Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            None,
            kria_core::os_control::BoundedVec::new(),
        ))));
    let provider = StorageControl::new(transport);
    let request = mount_request("usb-1");
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
            &plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("verified receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    assert!(receipt.verification().is_some());
    assert_eq!(provider.transport().dispatch_count(), 1, "apply exactly once");
    assert!(
        provider.transport().labels().contains(&"mount".to_string()),
        "mount must dispatch directly to the storage transport (not the broker)"
    );
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// C) unmount_device / eject_device idempotency: already unmounted →
//    Unchanged, zero dispatch.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn unmount_device_already_unmounted_is_unchanged_with_zero_dispatch() {
    let params = serde_json::json!({ "device": "usb-2" });
    let chain = Chain::build("unmount_device", params).await;

    let transport = FakeStorageTransport::new().mount_state_ok(unmounted("usb-2"));
    let provider = StorageControl::new(transport);
    let request = unmount_request("usb-2");
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
            &plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert_eq!(provider.transport().dispatch_count(), 0);
}

#[tokio::test]
#[serial]
async fn eject_device_already_unmounted_is_unchanged_with_zero_dispatch() {
    let params = serde_json::json!({ "device": "usb-3" });
    let chain = Chain::build("eject_device", params).await;

    let transport = FakeStorageTransport::new().mount_state_ok(unmounted("usb-3"));
    let provider = StorageControl::new(transport);
    let request = eject_request("usb-3");
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
            &plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert_eq!(provider.transport().dispatch_count(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// D) Busy unmount/eject: a device with an open file handle reports
//    ResourceBusy — never a forced retry (OSC-012.3, OSC-012.4).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn unmount_device_busy_reports_resource_busy_never_force() {
    let params = serde_json::json!({ "device": "usb-4" });
    let chain = Chain::build("unmount_device", params).await;

    let device = StorageDeviceId::new("usb-4");
    let transport = FakeStorageTransport::new()
        // 1: pre-observation — mounted (so this is a real, non-idempotent attempt).
        .mount_state_ok(mounted("usb-4", "/media/usb-4"))
        // 2: under-lease re-observation.
        .mount_state_ok(mounted("usb-4", "/media/usb-4"))
        // 3: apply dispatch fails with the busy signal — proven no effect
        //    started (an OsControlError, not an ApplyOutcome).
        .unmount_outcome(Err(device_busy_error(&device)));
    let provider = StorageControl::new(transport);
    let request = unmount_request("usb-4");
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
            &plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await;

    let err = result.expect_err("a busy device must surface ResourceBusy, not a receipt");
    match err {
        OsControlError::ResourceBusy { resource, .. } => {
            assert!(resource.as_str().contains("usb-4"));
        }
        other => panic!("expected ResourceBusy, got {other:?}"),
    }
    // Never a second, forced dispatch attempt — there is no force parameter
    // anywhere in this module to even express one.
    assert_eq!(provider.transport().dispatch_count(), 1);
}

#[tokio::test]
#[serial]
async fn eject_device_busy_reports_resource_busy_never_force() {
    let params = serde_json::json!({ "device": "usb-5" });
    let chain = Chain::build("eject_device", params).await;

    let device = StorageDeviceId::new("usb-5");
    let transport = FakeStorageTransport::new()
        .mount_state_ok(mounted("usb-5", "/media/usb-5"))
        .mount_state_ok(mounted("usb-5", "/media/usb-5"))
        .eject_outcome(Err(device_busy_error(&device)));
    let provider = StorageControl::new(transport);
    let request = eject_request("usb-5");
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
            &plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await;

    assert!(matches!(
        result.expect_err("busy eject must surface ResourceBusy"),
        OsControlError::ResourceBusy { .. }
    ));
    assert_eq!(provider.transport().dispatch_count(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// E) get_storage_health: degraded/unavailable evidence is reported honestly,
//    never fabricated (OSC-012.5, OSC-031).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn get_storage_health_reports_unavailable_evidence_honestly() {
    let chain = Chain::build("get_storage_health", serde_json::json!({})).await;
    let device = StorageDeviceId::new("nvme-1");
    let transport = FakeStorageTransport::new().health_ok(StorageHealthReport {
        device_id: device.clone(),
        availability: HealthAvailability::Unavailable,
        health_state: None,
        temperature_millikelvin: None,
    });
    let provider = StorageControl::new(transport);

    let report = provider
        .read_health(&chain.host_ctx, Some(&device))
        .await
        .expect("health read succeeds even when evidence is unavailable");

    assert_eq!(report.availability, HealthAvailability::Unavailable);
    assert!(report.health_state.is_none(), "no fabricated health state");
    assert_eq!(provider.transport().dispatch_count(), 0, "health is a pure read");
}

#[tokio::test]
#[serial]
async fn get_storage_health_reports_degraded_evidence_distinctly() {
    let chain = Chain::build("get_storage_health", serde_json::json!({})).await;
    let device = StorageDeviceId::new("nvme-2");
    let transport = FakeStorageTransport::new().health_ok(StorageHealthReport {
        device_id: device.clone(),
        availability: HealthAvailability::Degraded,
        health_state: Some("ok".to_string()),
        temperature_millikelvin: Some(305_150),
    });
    let provider = StorageControl::new(transport);

    let report = provider
        .read_health(&chain.host_ctx, Some(&device))
        .await
        .expect("health read succeeds");

    assert_eq!(report.availability, HealthAvailability::Degraded);
    assert_eq!(report.health_state.as_deref(), Some("ok"));
}

// ─────────────────────────────────────────────────────────────────────────────
// F) Runtime port seam: Unavailable with no provider composed; resolves
//    through a composed FakeHostOsControl otherwise.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn runtime_storage_port_is_unavailable_when_no_provider_is_composed() {
    assert!(sentinel_is_armed());
    let rt = OsControlRuntime::detached();
    let result = rt.storage("mount_device");
    assert!(matches!(
        result,
        Err(kria_core::os_control::OsControlError::Unavailable { .. })
    ));
}

#[test]
fn runtime_storage_port_resolves_through_composed_fake_host() {
    use kria_core::os_control::testing::FakeHostOsControl;

    let transport = FakeStorageTransport::new();
    let storage_provider: Arc<dyn kria_core::os_control::StorageControlPort> =
        Arc::new(StorageControl::new(transport));

    let fake_host = FakeHostOsControl::new("storage-aggregate").with_storage(storage_provider);
    let rt = OsControlRuntime::with_host(Arc::new(fake_host));

    let _ = rt.storage("mount_device").expect("storage port composed");
    assert_eq!(rt.provider_id().unwrap().as_str(), "storage-aggregate");
}

// ─────────────────────────────────────────────────────────────────────────────
// G) Completion proof: the closed storage tool set has exactly the five
//    frozen operations, none accepts a raw device-command/force parameter,
//    and no format/partition/resize/secure-erase/encryption-provisioning
//    tool exists in the frozen manifest at all (OSC-012.4, OSC-012.6,
//    OSC-030).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn storage_tool_set_is_closed_and_accepts_no_raw_device_or_force_parameter() {
    use kria_core::os_control::frozen_contract;

    let expected_storage_tools = [
        "list_storage_devices",
        "mount_device",
        "unmount_device",
        "eject_device",
        "get_storage_health",
    ];

    for tool in expected_storage_tools {
        let contract = frozen_contract(tool)
            .unwrap_or_else(|| panic!("`{tool}` must be a frozen canonical operation"));

        // No raw device-command parameter (e.g. a bare `/dev/sdX` string
        // field) and no force flag anywhere in the closed input schema.
        let schema_text = serde_json::to_string(&contract.input_schema).unwrap();
        assert!(
            !schema_text.contains("\"force\""),
            "`{tool}` input schema must not accept a force parameter: {schema_text}"
        );
        assert!(
            !schema_text.to_lowercase().contains("device_node")
                && !schema_text.to_lowercase().contains("raw_device"),
            "`{tool}` input schema must never expose a raw device-node field: {schema_text}"
        );

        // Every field name in the schema is drawn from the closed
        // OSC-012-scoped vocabulary (device/filesystem identity, paging,
        // never a raw shell/command string).
        assert!(
            kria_core::os_control::manifest::schema_is_closed(&contract.input_schema).is_ok(),
            "`{tool}` input schema must be closed (additionalProperties:false)"
        );
    }

    // No format/partition/resize/secure-erase/encryption-provisioning tool
    // exists anywhere in the frozen manifest under any name.
    let destructive_name_fragments = [
        "format_", "partition_", "resize_disk", "secure_erase", "encrypt_disk",
        "luks", "wipe_device",
    ];
    for tool in kria_core::os_control::frozen_tool_names() {
        for fragment in destructive_name_fragments {
            assert!(
                !tool.contains(fragment),
                "frozen manifest must not contain a destructive disk-administration \
                 tool; found `{tool}` matching `{fragment}`"
            );
        }
    }
}
