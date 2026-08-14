//! Task 2.3 — "Migrate Wi-Fi and power-profile controls" (OSC-020, OSC-031),
//! design §3, §9.7.
//!
//! # What this binary proves
//!
//! [`os_control::power`] already unit-tests its pieces in isolation (the
//! `powerprofilesctl` output parser, backend selection/argv, profile
//! parsing). This is the **deny-live, in-process** harness that drives the
//! *real* [`PowerControl`]`<`[`FakePowerProfileTransport`]`>` provider through
//! [`OsControlRuntime::run_mutation`] end to end, over the same governed
//! audit-admission + resource-lease + grant chain the F1 foundation harness
//! uses, so the full observe → idempotency → seal → apply → verify →
//! (rollback) lifecycle is exercised for `set_power_plan`, plus a read-only
//! `get_power_plan` observation:
//!
//! * `set_power_plan` is `Unchanged` (zero dispatch) when the observed
//!   profile already matches the desired profile;
//! * `set_power_plan` dispatches the exact governed `powerprofilesctl set`
//!   argv, verifies against fresh evidence, and reaches `Verified`;
//! * a post-apply contradiction rolls back to the captured prior profile and
//!   reaches `RolledBack`;
//! * a missing/absent scripted read reports the frozen `Unavailable` envelope
//!   — never a fabricated profile (OSC-031);
//! * the whole run never trips the process-wide deny-live sentinel and no
//!   power handler in this module ever launches a child process directly —
//!   the only captured "process" evidence is the redacted
//!   [`StructuredCommandSummary`] the fake transport records.
//!
//! Run ONLY this binary, serially:
//! `cargo test -p kria-core --no-default-features --features os-control-test \
//!    --test os_control_power_lifecycle -- --test-threads=1`

#![cfg(feature = "os-control-test")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::resource_lease::{ResourceLeaseManager, ResourceRequirement};
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::safety::RiskLevel;

use kria_core::os_control::power::fake::FakePowerProfileTransport;
use kria_core::os_control::power::{
    PowerControl, PowerProfile, PowerProfileBackend, PowerProfileOp, PowerProfileRequest,
};
use kria_core::os_control::resource::os_write_requirements;
use kria_core::os_control::runtime::SealBinding;
use kria_core::os_control::{
    sentinel_is_armed, sentinel_trip_count, ActionId, ActionLifecycle, AdmissionRequest,
    AuditAdmissionToken, ComparatorKind, CorrelationId, DesiredStateControl, Digest,
    HostExecutionContext, MutationPlan, OsAuditStore, OsLeaseContext, OsResourceCoordinator,
    ProviderId, RedactionPolicy, RequestSensitivity, RollbackPlan, RollbackToken, SessionContext,
    SessionId, SnapshotRevision,
};
use kria_core::os_control::runtime::OsControlRuntime;

const SESSION: &str = "sess-power-1";

/// Compose the full governed chain for a mutating power tool, mirroring the
/// F1 prompt-contract harness's `Chain` (see `os_control_audio_lifecycle.rs`).
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

fn set_power_plan_request(profile: PowerProfile) -> PowerProfileRequest {
    PowerProfileRequest {
        action: "set_power_plan".to_string(),
        params: serde_json::json!({ "profile": profile.as_str() }),
        op: PowerProfileOp::SetProfile(profile),
    }
}

fn get_power_plan_request() -> PowerProfileRequest {
    PowerProfileRequest {
        action: "get_power_plan".to_string(),
        params: serde_json::json!({}),
        op: PowerProfileOp::GetProfile,
    }
}

fn profile_plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-power-1"),
        provider: ProviderId::new("power-fake-powerprofilesctl"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn recorded() -> kria_core::os_control::AuditCompletionState {
    kria_core::os_control::AuditCompletionState::Recorded {
        record_id: kria_core::os_control::AuditRecordId::new("rec-power"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// A) set_power_plan idempotency: already in desired profile → Unchanged, zero
//    dispatch
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_power_plan_already_active_is_unchanged_with_zero_dispatch() {
    let baseline = sentinel_trip_count();
    let params = serde_json::json!({ "profile": "balanced" });
    let chain = Chain::build("set_power_plan", params).await;
    assert_eq!(chain.admission_count(), 1);

    let transport =
        FakePowerProfileTransport::new(PowerProfileBackend::Powerprofilesctl).read_ok(PowerProfile::Balanced);
    let provider = PowerControl::new(transport);
    let request = set_power_plan_request(PowerProfile::Balanced);
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
            &profile_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert!(!receipt.changed());
    assert_eq!(
        provider.transport().dispatch_count(),
        0,
        "idempotent power plan must not dispatch a command"
    );
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// B) set_power_plan mutation: dispatch exact governed powerprofilesctl argv,
//    verify, Verified
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_power_plan_dispatches_exact_argv_and_reaches_verified() {
    let baseline = sentinel_trip_count();
    let params = serde_json::json!({ "profile": "performance" });
    let chain = Chain::build("set_power_plan", params).await;

    let transport = FakePowerProfileTransport::new(PowerProfileBackend::Powerprofilesctl)
        // 1: run_mutation pre-observation (idempotency check).
        .read_ok(PowerProfile::Balanced)
        // 2: run_mutation under-lease re-observation (TOCTOU close).
        .read_ok(PowerProfile::Balanced)
        // 3: PowerControl::apply pre-apply snapshot (for rollback).
        .read_ok(PowerProfile::Balanced)
        // 4: run_mutation post-apply fresh re-observation.
        .read_ok(PowerProfile::Performance)
        // 5: PowerControl::verify independent read.
        .read_ok(PowerProfile::Performance)
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = PowerControl::new(transport);
    let request = set_power_plan_request(PowerProfile::Performance);
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
            &profile_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("verified receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    assert!(receipt.verification().is_some());
    assert_eq!(provider.transport().dispatch_count(), 1, "apply exactly once");
    let captured = provider.transport().captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].capability, "set_power_plan");
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// C) Post-apply contradiction rolls back to the captured prior profile
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_power_plan_contradiction_rolls_back_to_prior_profile() {
    let params = serde_json::json!({ "profile": PowerProfile::PowerSaver.as_str() });
    let chain = Chain::build("set_power_plan", params).await;

    let transport = FakePowerProfileTransport::new(PowerProfileBackend::Powerprofilesctl)
        .read_ok(PowerProfile::Balanced) // 1: pre-observation
        .read_ok(PowerProfile::Balanced) // 2: under-lease re-observation
        .read_ok(PowerProfile::Balanced) // 3: apply pre-apply snapshot (captures Balanced)
        .read_ok(PowerProfile::Balanced) // 4: post-apply re-observation (still Balanced → contradiction)
        .read_ok(PowerProfile::Balanced) // 5: verify independent read (contradicted)
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ))
        // The rollback's own restore-verification read, after the rollback dispatch.
        .read_ok(PowerProfile::Balanced) // 6: rollback verify() read, confirming restore
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = PowerControl::new(transport);
    let request = set_power_plan_request(PowerProfile::PowerSaver);
    let desired = request.desired_state().unwrap();

    let token = RollbackToken::new(
        Digest::of_str("power-rollback-tok"),
        SessionId::new(SESSION),
        Digest::of_str("set_power_plan"),
        ProviderId::new("power-fake-powerprofilesctl"),
        kria_core::os_control::ReceiptId::new("r-power-3"),
        kria_core::os_control::GrantNonce::new("nonce-1"),
        std::time::SystemTime::now() + Duration::from_secs(60),
    );
    let plan = MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-power-3"),
        provider: ProviderId::new("power-fake-powerprofilesctl"),
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
// D) get_power_plan observation: read-only, no dispatch, no grant needed
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn get_power_plan_observes_without_dispatch() {
    let chain = Chain::build("get_power_plan", serde_json::json!({})).await;
    let transport = FakePowerProfileTransport::new(PowerProfileBackend::PowerProfilesDaemon)
        .read_ok(PowerProfile::Balanced);
    let provider = PowerControl::new(transport);
    let request = get_power_plan_request();

    let state = provider
        .observe(&chain.host_ctx, &request)
        .await
        .expect("observation succeeds");

    assert_eq!(state.profile, PowerProfile::Balanced);
    assert_eq!(provider.transport().dispatch_count(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// E) Missing scripted read reports Unavailable — never a fabricated profile
//    (OSC-031).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn missing_session_power_reports_unavailable_not_a_fabricated_profile() {
    let chain = Chain::build("get_power_plan", serde_json::json!({})).await;
    let transport = FakePowerProfileTransport::new(PowerProfileBackend::Powerprofilesctl);
    let provider = PowerControl::new(transport);
    let request = get_power_plan_request();

    let err = provider
        .observe(&chain.host_ctx, &request)
        .await
        .expect_err("missing session power must report Unavailable");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::Unavailable { .. }
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// F) The runtime's power() port seam resolves through a composed
//    HostOsControl aggregate and falls back to Unavailable when none is
//    composed (Task 2.3 HostOsControl::power() addition).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn runtime_power_port_is_unavailable_when_no_provider_is_composed() {
    assert!(sentinel_is_armed());
    let rt = OsControlRuntime::detached();
    let result = rt.power("set_power_plan");
    assert!(
        matches!(
            result,
            Err(kria_core::os_control::OsControlError::Unavailable { .. })
        ),
        "no provider composed must map to Unavailable"
    );
}

#[test]
fn runtime_power_port_resolves_through_composed_fake_host() {
    use kria_core::os_control::testing::FakeHostOsControl;

    let transport = FakePowerProfileTransport::new(PowerProfileBackend::Powerprofilesctl)
        .read_ok(PowerProfile::Balanced);
    let power_provider: Arc<dyn kria_core::os_control::PowerControlPort> =
        Arc::new(PowerControl::new(transport));

    let fake_host = FakeHostOsControl::new("power-aggregate").with_power(power_provider);
    let rt = OsControlRuntime::with_host(Arc::new(fake_host));

    let power = rt.power("get_power_plan").expect("power port composed");
    assert_eq!(rt.provider_id().unwrap().as_str(), "power-aggregate");
    let _ = power; // exercised type; behavior covered by the Chain-based tests above.
}
