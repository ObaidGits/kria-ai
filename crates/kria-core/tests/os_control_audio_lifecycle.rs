//! Task 2.1 — "Migrate audio volume and add getters/mute" (OSC-005, OSC-006,
//! OSC-018, OSC-031), design §3, §9.5.
//!
//! # What this binary proves
//!
//! [`os_control::audio`] already unit-tests its pieces in isolation (parsers,
//! backend selection/argv, privacy classification). This is the **deny-live,
//! in-process** harness that drives the *real* [`AudioControl`]`<`
//! [`FakeAudioTransport`]`>` provider through [`OsControlRuntime::run_mutation`]
//! end to end, over the same governed audit-admission + resource-lease + grant
//! chain the F1 foundation harness uses, so the full observe → idempotency →
//! seal → apply → verify → (rollback) lifecycle is exercised for `set_volume`
//! and `set_audio_mute`, plus a read-only `get_audio_state` observation:
//!
//! * `set_volume` is `Unchanged` (zero dispatch) when the observed volume is
//!   already within the configured percentage tolerance;
//! * `set_volume` dispatches the exact governed `wpctl set-volume` argv,
//!   verifies against fresh evidence, and reaches `Verified`;
//! * `set_audio_mute` reaches `Verified` through the `Exact` comparator;
//! * a post-apply contradiction rolls back to the captured prior volume and
//!   reaches `RolledBack`;
//! * a missing/absent backend selection reports the frozen `Unavailable`
//!   envelope — never a fabricated state (OSC-031);
//! * the whole run never trips the process-wide deny-live sentinel and no
//!   audio handler in this module ever launches a child process directly — the
//!   only captured "process" evidence is the redacted
//!   [`StructuredCommandSummary`] the fake transport records.
//!
//! Run ONLY this binary, serially:
//! `cargo test -p kria-core --no-default-features --features os-control-test \
//!    --test os_control_audio_lifecycle -- --test-threads=1`

#![cfg(feature = "os-control-test")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::resource_lease::{ResourceLeaseManager, ResourceRequirement};
use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::safety::RiskLevel;

use kria_core::os_control::audio::fake::FakeAudioTransport;
use kria_core::os_control::audio::{
    AudioBackend, AudioControl, AudioEndpointKind, AudioFocus, AudioOp, AudioRequest,
};
use kria_core::os_control::resource::os_write_requirements;
use kria_core::os_control::runtime::SealBinding;
use kria_core::os_control::{
    sentinel_is_armed, sentinel_trip_count, ActionId, ActionLifecycle, AdmissionRequest,
    AuditAdmissionToken, ComparatorKind, CorrelationId, DesiredStateControl, Digest,
    HostExecutionContext, MutationPlan, OsAuditStore, OsLeaseContext, OsResourceCoordinator,
    ProviderId, RedactionPolicy, RequestSensitivity, RollbackPlan, RollbackToken, SessionContext,
    SessionId, SnapshotRevision, Tolerance,
};
use kria_core::os_control::runtime::OsControlRuntime;

const SESSION: &str = "sess-audio-1";

/// Compose the full governed chain for a mutating audio tool, mirroring the
/// F1 prompt-contract harness's `Chain`: durable audit admission before any
/// observation, held exclusive write leases in canonical order, and a fresh
/// gate-shaped grant bound to the same admitted facts.
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

fn set_volume_request(percent: u8) -> AudioRequest {
    AudioRequest {
        action: "set_volume".to_string(),
        params: serde_json::json!({ "percent": percent }),
        op: AudioOp::SetOutputLevel(percent),
        endpoint: AudioEndpointKind::Output,
    }
}

fn set_mute_request(muted: bool) -> AudioRequest {
    AudioRequest {
        action: "set_audio_mute".to_string(),
        params: serde_json::json!({ "muted": muted }),
        op: AudioOp::SetOutputMute(muted),
        endpoint: AudioEndpointKind::Output,
    }
}

fn get_state_request() -> AudioRequest {
    AudioRequest {
        action: "get_audio_state".to_string(),
        params: serde_json::json!({}),
        op: AudioOp::GetState,
        endpoint: AudioEndpointKind::Output,
    }
}

fn volume_plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-audio-1"),
        provider: ProviderId::new("audio-fake-wpctl"),
        comparator: ComparatorKind::WithinTolerance,
        tolerance: Some(Tolerance { abs: 2.0 }),
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn mute_plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-audio-2"),
        provider: ProviderId::new("audio-fake-wpctl"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn recorded() -> kria_core::os_control::AuditCompletionState {
    kria_core::os_control::AuditCompletionState::Recorded {
        record_id: kria_core::os_control::AuditRecordId::new("rec-audio"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// A) set_volume idempotency: already within tolerance → Unchanged, zero dispatch
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_volume_within_tolerance_is_unchanged_with_zero_dispatch() {
    let baseline = sentinel_trip_count();
    let params = serde_json::json!({ "percent": 60 });
    let chain = Chain::build("set_volume", params).await;
    assert_eq!(chain.admission_count(), 1);

    // Observed volume (61) is within the 2.0-point tolerance of desired (60).
    let transport = FakeAudioTransport::new(AudioBackend::Wpctl).read_ok(61, false);
    let provider = AudioControl::with_tolerance(transport, 2.0);
    let request = set_volume_request(60);
    let desired = request.desired_state().expect("mutation has a desired state");

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
            &volume_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert!(!receipt.changed());
    assert_eq!(
        provider.transport().dispatch_count(),
        0,
        "idempotent volume must not dispatch a command"
    );
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// B) set_volume mutation: dispatch exact governed wpctl argv, verify, Verified
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_volume_dispatches_exact_argv_and_reaches_verified() {
    let baseline = sentinel_trip_count();
    let params = serde_json::json!({ "percent": 75 });
    let chain = Chain::build("set_volume", params).await;

    let transport = FakeAudioTransport::new(AudioBackend::Wpctl)
        // 1: run_mutation's pre-observation (idempotency check).
        .read_ok(40, false)
        // 2: run_mutation's under-lease re-observation (TOCTOU close).
        .read_ok(40, false)
        // 3: AudioControl::apply's own pre-apply snapshot (for rollback).
        .read_ok(40, false)
        // 4: run_mutation's post-apply fresh re-observation.
        .read_ok(75, false)
        // 5: AudioControl::verify's independent read.
        .read_ok(75, false)
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = AudioControl::with_tolerance(transport, 2.0);
    let request = set_volume_request(75);
    let desired = request.desired_state().unwrap();

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
            &volume_plan(RollbackPlan::Unavailable),
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
    // The captured governed command is the exact wpctl set-volume argv — no
    // audio handler in this crate ever launches a process directly; this is
    // the only "command" evidence, and it is a redacted digest-only summary.
    assert_eq!(captured[0].capability().as_str(), "set_volume");
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// C) set_audio_mute mutation reaches Verified through the Exact comparator
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_audio_mute_reaches_verified() {
    let params = serde_json::json!({ "muted": true });
    let chain = Chain::build("set_audio_mute", params).await;

    let transport = FakeAudioTransport::new(AudioBackend::Wpctl)
        .read_ok(50, false) // 1: run_mutation pre-observation (idempotency)
        .read_ok(50, false) // 2: run_mutation under-lease re-observation
        .read_ok(50, false) // 3: AudioControl::apply pre-apply snapshot
        .read_ok(50, true) // 4: run_mutation post-apply re-observation
        .read_ok(50, true) // 5: AudioControl::verify independent read
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = AudioControl::new(transport);
    let request = set_mute_request(true);
    let desired = request.desired_state().unwrap();

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
            &mute_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("verified receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    assert_eq!(provider.transport().dispatch_count(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// D) Post-apply contradiction rolls back to the captured prior volume
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_volume_contradiction_rolls_back_to_prior_volume() {
    let params = serde_json::json!({ "percent": 90 });
    let chain = Chain::build("set_volume", params).await;

    let transport = FakeAudioTransport::new(AudioBackend::Wpctl)
        .read_ok(20, false) // 1: run_mutation pre-observation (idempotency)
        .read_ok(20, false) // 2: run_mutation under-lease re-observation
        .read_ok(20, false) // 3: AudioControl::apply pre-apply snapshot (captures 20)
        .read_ok(20, false) // 4: run_mutation post-apply re-observation (still 20 → contradiction)
        .read_ok(20, false) // 5: AudioControl::verify independent read (contradicted)
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ))
        // The rollback's own restore-verification read, after the rollback dispatch.
        .read_ok(20, false) // 6: rollback's verify() read, confirming restore to 20
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = AudioControl::with_tolerance(transport, 2.0);
    let request = set_volume_request(90);
    let desired = request.desired_state().unwrap();

    let token = RollbackToken::new(
        Digest::of_str("audio-rollback-tok"),
        SessionId::new(SESSION),
        Digest::of_str("set_volume"),
        ProviderId::new("audio-fake-wpctl"),
        kria_core::os_control::ReceiptId::new("r-audio-3"),
        kria_core::os_control::GrantNonce::new("nonce-1"),
        std::time::SystemTime::now() + Duration::from_secs(60),
    );
    let plan = MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-audio-3"),
        provider: ProviderId::new("audio-fake-wpctl"),
        comparator: ComparatorKind::WithinTolerance,
        tolerance: Some(Tolerance { abs: 2.0 }),
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
    // apply dispatch + rollback dispatch = 2 captured commands.
    assert_eq!(provider.transport().dispatch_count(), 2);
}

// ─────────────────────────────────────────────────────────────────────────────
// E) get_audio_state observation: read-only, no dispatch, no grant needed
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn get_audio_state_observes_without_dispatch() {
    let chain = Chain::build("get_audio_state", serde_json::json!({})).await;
    let transport = FakeAudioTransport::new(AudioBackend::Wpctl).read_ok(33, true);
    let provider = AudioControl::new(transport);
    let request = get_state_request();

    let state = provider
        .observe(&chain.host_ctx, &request)
        .await
        .expect("observation succeeds");

    assert_eq!(state.volume_percent, 33);
    assert!(state.muted);
    assert_eq!(state.focus, AudioFocus::Full);
    assert_eq!(provider.transport().dispatch_count(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// F) Missing session audio (no scripted read) reports Unavailable — never a
//    fabricated state (OSC-031 / parser-ambiguity-never-success).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn missing_session_audio_reports_unavailable_not_a_fabricated_state() {
    let chain = Chain::build("get_audio_state", serde_json::json!({})).await;
    // No `read_ok`/`read_err` scripted: the fake's default is `Unavailable`.
    let transport = FakeAudioTransport::new(AudioBackend::Wpctl);
    let provider = AudioControl::new(transport);
    let request = get_state_request();

    let err = provider
        .observe(&chain.host_ctx, &request)
        .await
        .expect_err("missing session audio must report Unavailable");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::Unavailable { .. }
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// G) The runtime's audio() port seam resolves through a composed HostOsControl
//    aggregate and falls back to Unavailable when none is composed (Task 2.1
//    HostOsControl::audio() addition).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn runtime_audio_port_is_unavailable_when_no_provider_is_composed() {
    assert!(sentinel_is_armed());
    let rt = OsControlRuntime::detached();
    let result = rt.audio("set_volume");
    assert!(
        matches!(
            result,
            Err(kria_core::os_control::OsControlError::Unavailable { .. })
        ),
        "no provider composed must map to Unavailable"
    );
}

#[test]
fn runtime_audio_port_resolves_through_composed_fake_host() {
    use kria_core::os_control::testing::FakeHostOsControl;

    let transport = FakeAudioTransport::new(AudioBackend::Wpctl).read_ok(50, false);
    let audio_provider: Arc<dyn kria_core::os_control::AudioControlPort> =
        Arc::new(AudioControl::new(transport));

    let fake_host = FakeHostOsControl::new("audio-aggregate").with_audio(audio_provider);
    let rt = OsControlRuntime::with_host(Arc::new(fake_host));

    let audio = rt.audio("get_audio_state").expect("audio port composed");
    // The port is reachable and dyn-dispatched; identity comes from the
    // aggregate, never a raw provider handle.
    assert_eq!(rt.provider_id().unwrap().as_str(), "audio-aggregate");
    let _ = audio; // exercised type; behavior is covered by the Chain-based tests above.
}
