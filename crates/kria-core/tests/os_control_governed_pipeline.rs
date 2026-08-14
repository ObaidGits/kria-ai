//! Code-level proof of the governed OS pipeline (linux-os-control-production).
//!
//! Covers the handoff this spec was missing: durable admission → held write leases
//! → sealed mutation permit → apply-once → verification → durable terminal audit.
//!
//! Deny-live only. Every provider is a fake, so no process, bus, or device is ever
//! touched; each test asserts the deny-live sentinel never tripped.

use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::resource_lease::ResourceLeaseManager;
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::os_control::access::sentinel_trip_count;
use kria_core::os_control::audio::selection::AudioBackend;
use kria_core::os_control::audio::{fake::FakeAudioTransport, AudioControl};
use kria_core::os_control::context::RedactionPolicy;
use kria_core::os_control::contract::{ActionId, CorrelationId, SnapshotRevision};
use kria_core::os_control::governed::{
    execute_governed_mutation, OsCallRequest, OsGovernedCall,
};
use kria_core::os_control::resource::{os_write_requirements, OsResourceCoordinator};
use kria_core::os_control::runtime::{MutationPlan, OsControlRuntime, RollbackPlan};
use kria_core::os_control::{
    ActionLifecycle, ApplyOutcome, AppliedDispatch, AuditCompletionState, BoundedVec,
    ComparatorKind, OsAuditStore, ReceiptId, Tolerance,
};
use kria_core::safety::RiskLevel;

const SESSION: &str = "governed-pipeline-session";
const TOOL: &str = "set_volume";

fn params(percent: u8) -> serde_json::Value {
    serde_json::json!({ "percent": percent })
}

fn call_request<'a>(
    session: &'a str,
    params: &'a serde_json::Value,
) -> OsCallRequest<'a> {
    OsCallRequest {
        session_id: session,
        correlation_id: CorrelationId::new("corr-governed-1"),
        action_id: ActionId::new("act-governed-1"),
        action: TOOL,
        params,
        target: ExecutionTarget::Host,
        risk: RiskLevel::Yellow,
        requirements: os_write_requirements(TOOL, params),
        snapshot_revision: SnapshotRevision(1),
        cancellation: CancellationToken::new(),
        deadline: Instant::now() + Duration::from_secs(30),
        redaction: RedactionPolicy::default(),
        // No probe in a deny-live test: the context keeps environment hints rather
        // than claiming probe-confirmed facts.
        snapshot: None,
    }
}

fn grant_for(params: &serde_json::Value) -> OsActionGrant {
    OsActionGrant::for_test(
        SESSION,
        TOOL,
        params,
        ExecutionTarget::Host,
        &os_write_requirements(TOOL, params),
        RiskLevel::Yellow,
    )
}

fn volume_plan() -> MutationPlan {
    MutationPlan {
        receipt_id: ReceiptId::new("r-governed-1"),
        provider: kria_core::os_control::ProviderId::new("fake-audio"),
        comparator: ComparatorKind::WithinTolerance,
        tolerance: Some(Tolerance { abs: 2.0 }),
        deadline_ms: 500,
        rollback: RollbackPlan::Unavailable,
        latency_ms: 1,
    }
}

/// The whole chain runs and the terminal audit record lands durably.
#[serial]
#[tokio::test]
async fn governed_mutation_records_a_durable_terminal() {
    let baseline = sentinel_trip_count();
    let audit = OsAuditStore::open_in_memory();
    let coordinator = OsResourceCoordinator::new(ResourceLeaseManager::new());
    let p = params(75);

    let call = OsGovernedCall::admit(&audit, &coordinator, grant_for(&p), call_request(SESSION, &p))
        .await
        .expect("admission and lease acquisition must succeed on a healthy store");
    assert!(call.is_mutation(), "a granted call carries a mutation permit");

    // Five scripted reads: pre-observation, under-lease re-observation, pre-apply
    // snapshot, post-apply re-observation, verify.
    let transport = FakeAudioTransport::new(AudioBackend::Wpctl)
        .read_ok(40, false)
        .read_ok(40, false)
        .read_ok(40, false)
        .read_ok(75, false)
        .read_ok(75, false)
        .dispatch_outcome(ApplyOutcome::Applied(AppliedDispatch::new(
            None,
            BoundedVec::new(),
        )));
    let provider = AudioControl::with_tolerance(transport, 2.0);

    let request = kria_core::os_control::audio::AudioRequest {
        action: TOOL.to_string(),
        params: p.clone(),
        op: kria_core::os_control::audio::AudioOp::SetOutputLevel(75),
        endpoint: kria_core::os_control::audio::AudioEndpointKind::Output,
    };
    let desired = request.desired_state().expect("mutation has a desired state");
    let plan = volume_plan();

    let outcome = execute_governed_mutation(
        &OsControlRuntime::detached(),
        &provider,
        &call,
        &audit,
        &request,
        &desired,
        &plan,
    )
    .await
    .expect("the governed chain completes");

    assert_eq!(outcome.receipt.lifecycle(), ActionLifecycle::Verified);
    assert!(outcome.receipt.changed());
    assert!(
        outcome.durably_recorded(),
        "a successful terminal append must upgrade the completion state to Recorded"
    );
    assert!(matches!(
        outcome.completion,
        AuditCompletionState::Recorded { .. }
    ));
    assert_eq!(
        provider.transport().dispatch_count(),
        1,
        "apply exactly once"
    );
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

/// An already-satisfied request is Unchanged and dispatches nothing, while still
/// closing its audit record — idempotency must not skip the ledger.
#[serial]
#[tokio::test]
async fn idempotent_mutation_is_unchanged_and_still_recorded() {
    let baseline = sentinel_trip_count();
    let audit = OsAuditStore::open_in_memory();
    let coordinator = OsResourceCoordinator::new(ResourceLeaseManager::new());
    let p = params(60);

    let call = OsGovernedCall::admit(&audit, &coordinator, grant_for(&p), call_request(SESSION, &p))
        .await
        .expect("admission succeeds");

    // Observed 61 is within the 2.0 tolerance of the desired 60.
    let transport = FakeAudioTransport::new(AudioBackend::Wpctl).read_ok(61, false);
    let provider = AudioControl::with_tolerance(transport, 2.0);

    let request = kria_core::os_control::audio::AudioRequest {
        action: TOOL.to_string(),
        params: p.clone(),
        op: kria_core::os_control::audio::AudioOp::SetOutputLevel(60),
        endpoint: kria_core::os_control::audio::AudioEndpointKind::Output,
    };
    let desired = request.desired_state().expect("mutation has a desired state");

    let outcome = execute_governed_mutation(
        &OsControlRuntime::detached(),
        &provider,
        &call,
        &audit,
        &request,
        &desired,
        &volume_plan(),
    )
    .await
    .expect("the governed chain completes");

    assert_eq!(outcome.receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert!(!outcome.receipt.changed());
    assert!(outcome.durably_recorded());
    assert_eq!(
        provider.transport().dispatch_count(),
        0,
        "an already-satisfied request must not dispatch"
    );
    assert_eq!(sentinel_trip_count(), baseline);
}

/// A read-admitted call carries no permit, so attempting a mutation with it is
/// refused before any provider contact.
#[serial]
#[tokio::test]
async fn read_admitted_call_cannot_mutate() {
    let baseline = sentinel_trip_count();
    let audit = OsAuditStore::open_in_memory();
    let p = params(30);

    let call = OsGovernedCall::admit_read(&audit, call_request(SESSION, &p), false)
        .expect("a plain read is admitted");
    assert!(!call.is_mutation(), "a read call carries no permit");
    assert!(call.grant().is_none());
    assert!(call.leases().is_none());

    let transport = FakeAudioTransport::new(AudioBackend::Wpctl).read_ok(30, false);
    let provider = AudioControl::with_tolerance(transport, 2.0);
    let request = kria_core::os_control::audio::AudioRequest {
        action: TOOL.to_string(),
        params: p.clone(),
        op: kria_core::os_control::audio::AudioOp::SetOutputLevel(90),
        endpoint: kria_core::os_control::audio::AudioEndpointKind::Output,
    };
    let desired = request.desired_state().expect("mutation has a desired state");

    // `GovernedOutcome` is intentionally not `Debug` (it carries redacted state),
    // so match rather than `expect_err`.
    let error = match execute_governed_mutation(
        &OsControlRuntime::detached(),
        &provider,
        &call,
        &audit,
        &request,
        &desired,
        &volume_plan(),
    )
    .await
    {
        Ok(_) => panic!("a read-admitted call must not be able to mutate"),
        Err(error) => error,
    };

    assert_eq!(error.code(), "os_control.policy_denied");
    assert_eq!(
        provider.transport().dispatch_count(),
        0,
        "refusal happens before any dispatch"
    );
    assert_eq!(sentinel_trip_count(), baseline);
}

/// Two calls contending for the same resource: the second fails closed rather than
/// interleaving with the first.
#[serial]
#[tokio::test]
async fn contended_resource_fails_closed() {
    let audit = OsAuditStore::open_in_memory();
    // ONE coordinator, so both calls are arbitrated by the same authority.
    let coordinator = OsResourceCoordinator::new(ResourceLeaseManager::new());
    let p = params(50);

    let _first =
        OsGovernedCall::admit(&audit, &coordinator, grant_for(&p), call_request(SESSION, &p))
            .await
            .expect("the first call acquires its write leases");

    let second = OsGovernedCall::admit(
        &audit,
        &coordinator,
        grant_for(&p),
        call_request("other-session", &p),
    )
    .await;

    let error = match second {
        Ok(_) => panic!("the second call must not acquire a held resource"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "os_control.resource_busy");
}

/// The audit ledger's integrity chain verifies after a governed mutation.
#[serial]
#[tokio::test]
async fn audit_chain_verifies_after_a_governed_mutation() {
    let audit = OsAuditStore::open_in_memory();
    let coordinator = OsResourceCoordinator::new(ResourceLeaseManager::new());
    let p = params(20);

    let call = OsGovernedCall::admit(&audit, &coordinator, grant_for(&p), call_request(SESSION, &p))
        .await
        .expect("admission succeeds");

    let transport = FakeAudioTransport::new(AudioBackend::Wpctl).read_ok(20, false);
    let provider = AudioControl::with_tolerance(transport, 2.0);
    let request = kria_core::os_control::audio::AudioRequest {
        action: TOOL.to_string(),
        params: p.clone(),
        op: kria_core::os_control::audio::AudioOp::SetOutputLevel(20),
        endpoint: kria_core::os_control::audio::AudioEndpointKind::Output,
    };
    let desired = request.desired_state().expect("mutation has a desired state");

    let _ = execute_governed_mutation(
        &OsControlRuntime::detached(),
        &provider,
        &call,
        &audit,
        &request,
        &desired,
        &volume_plan(),
    )
    .await
    .expect("the governed chain completes");

    audit
        .verify_chain()
        .expect("the append-only audit chain must verify");
    assert_eq!(
        audit.incomplete_admission_count(),
        0,
        "a closed action leaves no incomplete admission"
    );
}
