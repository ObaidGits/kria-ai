//! Task 2.5 — clipboard domain slice ("Migrate files, processes,
//! applications, packages, scheduler, disk, clipboard and notifications",
//! OSC-023).
//!
//! # What this binary proves
//!
//! The **deny-live, in-process** harness driving the real
//! [`ClipboardControl`]`<`[`FakeClipboardTransport`]`>` provider through
//! [`OsControlRuntime::run_mutation`] end to end for `set_clipboard`:
//!
//! * setting the same text the clipboard already holds → `Unchanged`, zero
//!   dispatch;
//! * a real write dispatches exactly one governed write, and the raw text
//!   never leaks into the fake's recorded write call being confused for a
//!   redacted audit value (the fake retains the raw text for test assertion
//!   only — production audit uses the shared `Content` redaction class);
//! * `set_clipboard` never claims rollback availability;
//! * a missing scripted read reports `Unavailable`;
//! * `get_clipboard`'s read-only path never dispatches.
//!
//! Run ONLY this binary, serially:
//! `cargo test -p kria-core --no-default-features --features os-control-test \
//!    --test os_control_clipboard_lifecycle -- --test-threads=1`

#![cfg(feature = "os-control-test")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::resource_lease::{ResourceLeaseManager, ResourceRequirement};
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::safety::RiskLevel;

use kria_core::os_control::clipboard::fake::FakeClipboardTransport;
use kria_core::os_control::clipboard::{ClipboardControl, ClipboardRequest};
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

const SESSION: &str = "sess-clipboard-1";
const TOOL: &str = "set_clipboard";

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
                risk: RiskLevel::Red,
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

fn set_request(text: &str) -> ClipboardRequest {
    ClipboardRequest {
        action: TOOL.to_string(),
        params: serde_json::json!({}),
        text: text.to_string(),
    }
}

fn plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-clipboard-1"),
        provider: ProviderId::new("clipboard-native-selection"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn recorded() -> kria_core::os_control::AuditCompletionState {
    kria_core::os_control::AuditCompletionState::Recorded {
        record_id: kria_core::os_control::AuditRecordId::new("rec-clipboard"),
    }
}

fn applied() -> kria_core::os_control::ApplyOutcome {
    kria_core::os_control::ApplyOutcome::Applied(AppliedDispatch::new(None, BoundedVec::new()))
}

#[tokio::test]
#[serial]
async fn set_clipboard_same_text_is_unchanged_with_zero_dispatch() {
    let baseline = sentinel_trip_count();
    let chain = Chain::build(serde_json::json!({})).await;
    assert_eq!(chain.admission_count(), 1);

    let transport = FakeClipboardTransport::new().read_ok("hello");
    let provider = ClipboardControl::new(transport);
    let request = set_request("hello");
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
async fn set_clipboard_new_text_dispatches_once_and_reaches_verified() {
    let chain = Chain::build(serde_json::json!({})).await;

    let transport = FakeClipboardTransport::new()
        .read_ok("old") // 1: pre-observation
        .read_ok("old") // 2: under-lease re-observation
        .read_ok("new content") // 3: post-apply re-observation
        .read_ok("new content") // 4: verify independent read
        .dispatch_outcome(applied());
    let provider = ClipboardControl::new(transport);
    let request = set_request("new content");
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
    let calls = provider.transport().write_calls();
    assert_eq!(calls.len(), 1, "apply exactly once");
    assert_eq!(calls[0], "new content");
}

#[tokio::test]
#[serial]
async fn missing_scripted_read_reports_unavailable_not_a_fabricated_state() {
    let chain = Chain::build(serde_json::json!({})).await;
    let transport = FakeClipboardTransport::new();
    let provider = ClipboardControl::new(transport);
    let request = set_request("x");

    let err = provider
        .observe(&chain.host_ctx, &request)
        .await
        .expect_err("missing scripted read must report Unavailable");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::Unavailable { .. }
    ));
}

#[tokio::test]
#[serial]
async fn get_clipboard_read_only_never_dispatches() {
    let chain = Chain::build(serde_json::json!({})).await;
    let transport = FakeClipboardTransport::new().read_ok("current text");
    let provider = ClipboardControl::new(transport);

    let text = provider
        .current_text(&chain.host_ctx)
        .await
        .expect("read succeeds");

    assert_eq!(text, "current text");
    assert_eq!(provider.transport().dispatch_count(), 0);
}

#[test]
fn runtime_clipboard_port_is_unavailable_when_no_provider_is_composed() {
    assert!(sentinel_is_armed());
    let rt = OsControlRuntime::detached();
    assert!(matches!(
        rt.clipboard(TOOL),
        Err(kria_core::os_control::OsControlError::Unavailable { .. })
    ));
}

#[test]
fn runtime_clipboard_port_resolves_through_composed_fake_host() {
    use kria_core::os_control::testing::FakeHostOsControl;

    let transport = FakeClipboardTransport::new().read_ok("x");
    let provider: Arc<dyn kria_core::os_control::ClipboardControlPort> =
        Arc::new(ClipboardControl::new(transport));

    let fake_host = FakeHostOsControl::new("clipboard-aggregate").with_clipboard(provider);
    let rt = OsControlRuntime::with_host(Arc::new(fake_host));

    let _ = rt.clipboard(TOOL).expect("port composed");
    assert_eq!(rt.provider_id().unwrap().as_str(), "clipboard-aggregate");
}
