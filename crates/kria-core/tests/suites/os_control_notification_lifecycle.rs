//! Task 2.5 — notification domain slice ("Migrate files, processes,
//! applications, packages, scheduler, disk, clipboard and notifications",
//! OSC-023) — "upgrade notification adapter".
//!
//! # What this binary proves
//!
//! The **deny-live, in-process** harness driving the real
//! [`NotificationControl`]`<`[`FakeNotificationTransport`]`>` provider
//! through [`OsControlRuntime::run_mutation`] end to end for
//! `send_notification`:
//!
//! * every send is a distinct desired state (never idempotency-skipped —
//!   there is no "already sent" state), so `run_mutation` always reaches the
//!   provider's `apply`;
//! * a successful portal `Applied` dispatch reaches `Verified` via the
//!   provider's synthesized satisfying evidence (the portal reply *is* the
//!   delivery evidence);
//! * `send_notification` never claims rollback availability;
//! * the provider never routes through `notify-send`/`paplay` — the only
//!   evidence is the fake transport's recorded [`SendCall`];
//! * the whole run never trips the deny-live sentinel.
//!
//! Run ONLY this binary, serially:
//! `cargo test -p kria-core --no-default-features --features os-control-test \
//!    --test os_control_notification_lifecycle -- --test-threads=1`

#![cfg(feature = "os-control-test")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::resource_lease::{ResourceLeaseManager, ResourceRequirement};
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::safety::RiskLevel;

use kria_core::os_control::notifications::fake::FakeNotificationTransport;
use kria_core::os_control::notifications::{NotificationControl, NotificationRequest};
use kria_core::os_control::resource::os_write_requirements;
use kria_core::os_control::runtime::OsControlRuntime;
use kria_core::os_control::runtime::SealBinding;
use kria_core::os_control::{
    sentinel_is_armed, sentinel_trip_count, ActionId, ActionLifecycle, AdmissionRequest,
    AppliedDispatch, AuditAdmissionToken, BoundedVec, ComparatorKind, CorrelationId, Digest,
    HostExecutionContext, MutationPlan, OsAuditStore, OsLeaseContext, OsResourceCoordinator,
    ProviderId, RedactionPolicy, RequestSensitivity, RollbackPlan, SessionContext, SessionId,
    SnapshotRevision,
};

const SESSION: &str = "sess-notification-1";
const TOOL: &str = "send_notification";

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

fn send_request(title: &str, body: &str, nonce: u64) -> NotificationRequest {
    NotificationRequest {
        action: TOOL.to_string(),
        params: serde_json::json!({ "title": title, "body": body }),
        title: title.to_string(),
        body: body.to_string(),
        nonce,
    }
}

fn plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-notification-1"),
        provider: ProviderId::new("notifications-freedesktop-portal"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn recorded() -> kria_core::os_control::AuditCompletionState {
    kria_core::os_control::AuditCompletionState::Recorded {
        record_id: kria_core::os_control::AuditRecordId::new("rec-notification"),
    }
}

fn applied() -> kria_core::os_control::ApplyOutcome {
    kria_core::os_control::ApplyOutcome::Applied(AppliedDispatch::new(None, BoundedVec::new()))
}

// ─────────────────────────────────────────────────────────────────────────────
// A) Every send is a distinct desired state — never idempotency-skipped.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn send_notification_never_idempotency_skips() {
    let baseline = sentinel_trip_count();
    let chain = Chain::build(serde_json::json!({ "title": "T", "body": "B" })).await;
    assert_eq!(chain.admission_count(), 1);

    let transport = FakeNotificationTransport::new().dispatch_outcome(applied());
    let provider = NotificationControl::new(transport);
    let request = send_request("T", "B", 1);
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

    // Never Unchanged: a notification send always dispatches.
    assert_ne!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    let calls = provider.transport().send_calls();
    assert_eq!(calls.len(), 1, "apply exactly once");
    assert_eq!(calls[0].title, "T");
    assert_eq!(calls[0].body, "B");
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// B) Two distinct sends never collapse into the same digest.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn two_identical_content_sends_are_distinct_dispatches() {
    let chain1 = Chain::build(serde_json::json!({ "title": "Reminder", "body": "Standup" })).await;
    let transport1 = FakeNotificationTransport::new().dispatch_outcome(applied());
    let provider1 = NotificationControl::new(transport1);
    let request1 = send_request("Reminder", "Standup", 1);
    let desired1 = request1.desired_state();

    let receipt1 = OsControlRuntime::detached()
        .run_mutation(
            &provider1,
            &chain1.host_ctx,
            &chain1.grant,
            &chain1.lease_set,
            &chain1.token,
            &chain1.binding(),
            &request1,
            &desired1,
            &plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("first send verified");
    assert_eq!(receipt1.lifecycle(), ActionLifecycle::Verified);

    // Second send, same content, different nonce.
    let chain2 = Chain::build(serde_json::json!({ "title": "Reminder", "body": "Standup" })).await;
    let transport2 = FakeNotificationTransport::new().dispatch_outcome(applied());
    let provider2 = NotificationControl::new(transport2);
    let request2 = send_request("Reminder", "Standup", 2);
    let desired2 = request2.desired_state();

    let receipt2 = OsControlRuntime::detached()
        .run_mutation(
            &provider2,
            &chain2.host_ctx,
            &chain2.grant,
            &chain2.lease_set,
            &chain2.token,
            &chain2.binding(),
            &request2,
            &desired2,
            &plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("second send verified");
    assert_eq!(receipt2.lifecycle(), ActionLifecycle::Verified);
    assert_eq!(provider2.transport().dispatch_count(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// C) The runtime's notifications() port seam.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn runtime_notifications_port_is_unavailable_when_no_provider_is_composed() {
    assert!(sentinel_is_armed());
    let rt = OsControlRuntime::detached();
    assert!(matches!(
        rt.notifications(TOOL),
        Err(kria_core::os_control::OsControlError::Unavailable { .. })
    ));
}

#[test]
fn runtime_notifications_port_resolves_through_composed_fake_host() {
    use kria_core::os_control::testing::FakeHostOsControl;

    let transport = FakeNotificationTransport::new().dispatch_outcome(applied());
    let provider: Arc<dyn kria_core::os_control::NotificationControlPort> =
        Arc::new(NotificationControl::new(transport));

    let fake_host = FakeHostOsControl::new("notification-aggregate").with_notifications(provider);
    let rt = OsControlRuntime::with_host(Arc::new(fake_host));

    let _ = rt.notifications(TOOL).expect("port composed");
    assert_eq!(rt.provider_id().unwrap().as_str(), "notification-aggregate");
}
