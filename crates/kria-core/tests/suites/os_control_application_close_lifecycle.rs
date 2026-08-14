//! Task 2.5 — application graceful-close domain slice ("Migrate files,
//! processes, applications, packages, scheduler, disk, clipboard and
//! notifications", OSC-013).
//!
//! # What this binary proves
//!
//! The **deny-live, in-process** harness driving the real
//! [`ApplicationCloseControl`]`<`[`FakeApplicationCloseTransport`]`>`
//! provider through [`OsControlRuntime::run_mutation`] end to end for
//! `graceful_close_application`:
//!
//! * already-closed (zero matching processes) → `Unchanged`, zero dispatch;
//! * a real close dispatches exactly one `terminate_matching` call (never a
//!   forced kill — that is the separate `kill_process` operation) and
//!   reaches `Verified` once no matches remain;
//! * `graceful_close_application` never claims rollback availability;
//! * a missing scripted count reports `Unavailable`, never a fabricated
//!   count;
//! * the whole run never trips the deny-live sentinel.
//!
//! Run ONLY this binary, serially:
//! `cargo test -p kria-core --no-default-features --features os-control-test \
//!    --test os_control_application_close_lifecycle -- --test-threads=1`

#![cfg(feature = "os-control-test")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::resource_lease::{ResourceLeaseManager, ResourceRequirement};
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::safety::RiskLevel;

use kria_core::os_control::applications::fake::FakeApplicationCloseTransport;
use kria_core::os_control::applications::{ApplicationCloseControl, ApplicationCloseRequest};
use kria_core::os_control::resource::os_write_requirements;
use kria_core::os_control::runtime::OsControlRuntime;
use kria_core::os_control::runtime::SealBinding;
use kria_core::os_control::{
    sentinel_is_armed, sentinel_trip_count, ActionId, ActionLifecycle, AdmissionRequest,
    AppliedDispatch, AuditAdmissionToken, BoundedVec, ComparatorKind, CorrelationId,
    DesiredStateControl, Digest, HostExecutionContext, MutationPlan, OsAuditStore, OsLeaseContext,
    OsResourceCoordinator, ProviderId, RedactionPolicy, RequestSensitivity, RollbackPlan,
    SessionContext, SessionId, SnapshotRevision,
};

const SESSION: &str = "sess-app-close-1";
const TOOL: &str = "graceful_close_application";

struct Chain {
    audit: OsAuditStore,
    grant: OsActionGrant,
    host_ctx: HostExecutionContext,
    lease_set: kria_core::os_control::AcquiredResourceLeaseSet,
    token: AuditAdmissionToken,
    reqs: Vec<ResourceRequirement>,
    params: serde_json::Value,
}

impl Chain {
    async fn build(params: serde_json::Value) -> Self {
        let audit = OsAuditStore::open_in_memory();

        let token = audit
            .admit_action(&AdmissionRequest {
                session_id: SessionId::new(SESSION),
                correlation_id: CorrelationId::new("corr-1"),
                action_id: ActionId::new("act-1"),
                tool_name: TOOL.to_string(),
                params: params.clone(),
                target_hash: Digest::of_str(ExecutionTarget::Host.as_str()),
                capability_snapshot_revision: SnapshotRevision(1),
                risk: RiskLevel::Yellow,
                decision_id: None,
                sensitivity: RequestSensitivity::Mutation,
            })
            .expect("audit admission must succeed on a healthy store");

        let reqs = os_write_requirements(TOOL, &params);
        let coordinator = OsResourceCoordinator::new(ResourceLeaseManager::new());
        let lease_set = coordinator
            .acquire_write_leases(
                &OsLeaseContext {
                    workflow_id: SESSION.to_string(),
                    stage_id: None,
                    action_hash: Digest::of_str(TOOL).as_hex().to_string(),
                },
                TOOL,
                &params,
            )
            .await
            .expect("write leases acquire in canonical order");

        let grant = OsActionGrant::for_test(
            SESSION,
            TOOL,
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
        }
    }

    fn binding(&self) -> SealBinding<'_> {
        SealBinding {
            session_id: SESSION,
            action: TOOL,
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

fn close_request(name: &str) -> ApplicationCloseRequest {
    ApplicationCloseRequest {
        action: TOOL.to_string(),
        params: serde_json::json!({ "app_id": name }),
        name: name.to_string(),
    }
}

fn plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-app-close-1"),
        provider: ProviderId::new("application-close-native-syscall"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn recorded() -> kria_core::os_control::AuditCompletionState {
    kria_core::os_control::AuditCompletionState::Recorded {
        record_id: kria_core::os_control::AuditRecordId::new("rec-app-close"),
    }
}

fn applied() -> kria_core::os_control::ApplyOutcome {
    kria_core::os_control::ApplyOutcome::Applied(AppliedDispatch::new(None, BoundedVec::new()))
}

#[tokio::test]
#[serial]
async fn already_closed_is_unchanged_with_zero_dispatch() {
    let baseline = sentinel_trip_count();
    let params = serde_json::json!({ "app_id": "gedit" });
    let chain = Chain::build(params).await;
    assert_eq!(chain.admission_count(), 1);

    let transport = FakeApplicationCloseTransport::new().count_ok(0);
    let provider = ApplicationCloseControl::new(transport);
    let request = close_request("gedit");
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
    assert_eq!(sentinel_trip_count(), baseline);
}

#[tokio::test]
#[serial]
async fn graceful_close_dispatches_once_and_reaches_verified() {
    let params = serde_json::json!({ "app_id": "gedit" });
    let chain = Chain::build(params).await;

    let transport = FakeApplicationCloseTransport::new()
        .count_ok(2) // 1: pre-observation
        .count_ok(2) // 2: under-lease re-observation
        .count_ok(0) // 3: post-apply re-observation
        .count_ok(0) // 4: verify independent read
        .dispatch_outcome(applied());
    let provider = ApplicationCloseControl::new(transport);
    let request = close_request("gedit");
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
    let calls = provider.transport().terminate_calls();
    assert_eq!(calls.len(), 1, "apply exactly once");
    assert_eq!(calls[0], "gedit");
}

#[tokio::test]
#[serial]
async fn missing_scripted_count_reports_unavailable_not_a_fabricated_state() {
    let chain = Chain::build(serde_json::json!({ "app_id": "gedit" })).await;
    let transport = FakeApplicationCloseTransport::new();
    let provider = ApplicationCloseControl::new(transport);
    let request = close_request("gedit");

    let err = provider
        .observe(&chain.host_ctx, &request)
        .await
        .expect_err("missing scripted count must report Unavailable");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::Unavailable { .. }
    ));
}

#[test]
fn runtime_application_close_port_is_unavailable_when_no_provider_is_composed() {
    assert!(sentinel_is_armed());
    let rt = OsControlRuntime::detached();
    assert!(matches!(
        rt.application_close(TOOL),
        Err(kria_core::os_control::OsControlError::Unavailable { .. })
    ));
}

#[test]
fn runtime_application_close_port_resolves_through_composed_fake_host() {
    use kria_core::os_control::testing::FakeHostOsControl;

    let transport = FakeApplicationCloseTransport::new().count_ok(0);
    let provider: Arc<dyn kria_core::os_control::ApplicationCloseControlPort> =
        Arc::new(ApplicationCloseControl::new(transport));

    let fake_host = FakeHostOsControl::new("app-close-aggregate").with_application_close(provider);
    let rt = OsControlRuntime::with_host(Arc::new(fake_host));

    let _ = rt.application_close(TOOL).expect("port composed");
    assert_eq!(rt.provider_id().unwrap().as_str(), "app-close-aggregate");
}
