//! Task 2.4 — "Migrate lock, suspend, hibernate, shutdown and reboot"
//! (OSC-004, OSC-005, OSC-020), design §3, §9.7.
//!
//! # What this binary proves
//!
//! [`os_control::power::session`] already unit-tests its pieces in isolation
//! (the `loginctl` argv builder, backend selection, digest binding). This is
//! the **deny-live, in-process** harness that drives the *real*
//! [`PowerSessionControl`]`<`[`FakePowerSessionTransport`]`>` provider through
//! [`OsControlRuntime::run_mutation`] end to end, over the same governed
//! audit-admission + resource-lease + grant chain the F1 foundation harness
//! uses, so the full observe → seal → apply → verify lifecycle is exercised
//! for all five migrated operations:
//!
//! * `lock_screen` reaches `Verified` from a fresh `LockedHint` observation;
//! * `sleep`/`hibernate` reach `Accepted` — never `Verified`/`Completed` —
//!   backed by real acceptance evidence;
//! * hibernate-unavailable reports `Unsupported` **before** any dispatch —
//!   never a fabricated acceptance (OSC-020);
//! * `shutdown_system`/`reboot_system` reach `Accepted` and never advertise
//!   rollback (`rollbackClaim: None` in the frozen manifest);
//! * a missing/absent scripted lock-state read reports the frozen
//!   `Unavailable` envelope — never a fabricated lock state;
//! * the runtime's `power_session()` port resolves through a composed
//!   `HostOsControl` aggregate and falls back to `Unavailable` when none is
//!   composed;
//! * the whole run never trips the process-wide deny-live sentinel and no
//!   power-session handler in this module ever launches a child process or
//!   opens a live logind session directly — the only captured "process"
//!   evidence is the redacted [`StructuredCommandSummary`] the fake transport
//!   records.
//!
//! Run ONLY this binary, serially:
//! `cargo test -p kria-core --no-default-features --features os-control-test \
//!    --test os_control_session_lifecycle -- --test-threads=1`

#![cfg(feature = "os-control-test")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::resource_lease::{ResourceLeaseManager, ResourceRequirement};
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::safety::RiskLevel;

use kria_core::os_control::power::session::fake::FakePowerSessionTransport;
use kria_core::os_control::power::session::{
    PowerSessionBackend, PowerSessionControl, PowerSessionOp, PowerSessionRequest,
};
use kria_core::os_control::resource::os_write_requirements;
use kria_core::os_control::runtime::SealBinding;
use kria_core::os_control::{
    sentinel_is_armed, sentinel_trip_count, ActionId, ActionLifecycle, AdmissionRequest,
    AuditAdmissionToken, ComparatorKind, CorrelationId, DesiredStateControl, Digest,
    HostExecutionContext, MutationPlan, OsAuditStore, OsLeaseContext, OsResourceCoordinator,
    ProviderId, RedactionPolicy, RequestSensitivity, RollbackPlan, SessionContext, SessionId,
    SnapshotRevision,
};
use kria_core::os_control::runtime::OsControlRuntime;

const SESSION: &str = "sess-power-session-1";

/// Compose the full governed chain for a mutating power-session tool,
/// mirroring the F1 prompt-contract harness's `Chain` (see
/// `os_control_power_lifecycle.rs`).
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

fn lock_request() -> PowerSessionRequest {
    PowerSessionRequest {
        action: "lock_screen".to_string(),
        params: serde_json::json!({}),
        op: PowerSessionOp::Lock,
    }
}

fn suspend_request() -> PowerSessionRequest {
    PowerSessionRequest {
        action: "sleep".to_string(),
        params: serde_json::json!({}),
        op: PowerSessionOp::Suspend,
    }
}

fn hibernate_request() -> PowerSessionRequest {
    PowerSessionRequest {
        action: "hibernate".to_string(),
        params: serde_json::json!({}),
        op: PowerSessionOp::Hibernate,
    }
}

fn shutdown_request(delay_minutes: u64) -> PowerSessionRequest {
    PowerSessionRequest {
        action: "shutdown_system".to_string(),
        params: serde_json::json!({ "delay_minutes": delay_minutes }),
        op: PowerSessionOp::Shutdown { delay_minutes },
    }
}

fn reboot_request() -> PowerSessionRequest {
    PowerSessionRequest {
        action: "reboot_system".to_string(),
        params: serde_json::json!({}),
        op: PowerSessionOp::Reboot,
    }
}

fn session_plan(receipt_id: &str) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new(receipt_id),
        provider: ProviderId::new("power-session-fake-logind"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        // The frozen manifest declares `rollbackClaim: None` for every
        // operation in this slice.
        rollback: RollbackPlan::Unavailable,
        latency_ms: 5,
    }
}

fn recorded() -> kria_core::os_control::AuditCompletionState {
    kria_core::os_control::AuditCompletionState::Recorded {
        record_id: kria_core::os_control::AuditRecordId::new("rec-power-session"),
    }
}

fn accepted_outcome() -> kria_core::os_control::ApplyOutcome {
    kria_core::os_control::ApplyOutcome::Accepted(kria_core::os_control::AcceptedDispatch::new(
        None,
        kria_core::os_control::AcceptanceEvidence {
            detail: kria_core::os_control::contract::SafeText::new("logind accepted"),
            accepted_at: std::time::SystemTime::now(),
        },
        kria_core::os_control::BoundedVec::new(),
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// A) lock_screen: verified via LockedHint observation
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn lock_screen_dispatches_and_reaches_verified_via_locked_hint() {
    let baseline = sentinel_trip_count();
    let chain = Chain::build("lock_screen", serde_json::json!({})).await;
    assert_eq!(chain.admission_count(), 1);

    let transport = FakePowerSessionTransport::new(PowerSessionBackend::Loginctl)
        // 1: run_mutation pre-observation (idempotency check) — not yet locked.
        .locked_ok(false)
        // 2: run_mutation under-lease re-observation (TOCTOU close).
        .locked_ok(false)
        // 3: run_mutation post-apply fresh re-observation.
        .locked_ok(true)
        // 4: PowerSessionControl::verify independent read.
        .locked_ok(true)
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = PowerSessionControl::new(transport);
    let request = lock_request();
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
            &session_plan("r-lock-1"),
            recorded(),
        )
        .await
        .expect("verified receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    assert!(receipt.verification().is_some());
    assert_eq!(provider.transport().dispatch_count(), 1, "apply exactly once");
    let captured = provider.transport().captured();
    assert_eq!(captured[0].capability, "lock_screen");
    assert_eq!(captured[0].redacted_args, vec!["lock-session".to_string()]);
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

#[tokio::test]
#[serial]
async fn lock_screen_already_locked_is_unchanged_with_zero_dispatch() {
    let chain = Chain::build("lock_screen", serde_json::json!({})).await;
    let transport = FakePowerSessionTransport::new(PowerSessionBackend::Loginctl).locked_ok(true);
    let provider = PowerSessionControl::new(transport);
    let request = lock_request();
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
            &session_plan("r-lock-2"),
            recorded(),
        )
        .await
        .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert!(!receipt.changed());
    assert_eq!(
        provider.transport().dispatch_count(),
        0,
        "idempotent lock must not dispatch a command"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// B) sleep / hibernate: Accepted, never Verified/Completed
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn sleep_dispatches_exact_argv_and_reaches_accepted_never_verified() {
    let baseline = sentinel_trip_count();
    let chain = Chain::build("sleep", serde_json::json!({})).await;

    let transport = FakePowerSessionTransport::new(PowerSessionBackend::LogindDbus)
        .dispatch_outcome(accepted_outcome());
    let provider = PowerSessionControl::new(transport);
    let request = suspend_request();
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
            &session_plan("r-sleep-1"),
            recorded(),
        )
        .await
        .expect("accepted receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Accepted);
    assert_ne!(
        receipt.lifecycle(),
        ActionLifecycle::Verified,
        "session-ending action must never be Verified"
    );
    assert!(receipt.verification().is_none());
    assert_eq!(provider.transport().dispatch_count(), 1);
    let captured = provider.transport().captured();
    assert_eq!(captured[0].capability, "sleep");
    assert_eq!(captured[0].redacted_args, vec!["suspend".to_string()]);
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

#[tokio::test]
#[serial]
async fn hibernate_available_dispatches_and_reaches_accepted() {
    let chain = Chain::build("hibernate", serde_json::json!({})).await;

    let transport = FakePowerSessionTransport::new(PowerSessionBackend::LogindDbus)
        .hibernate_available(true)
        .dispatch_outcome(accepted_outcome());
    let provider = PowerSessionControl::new(transport);
    let request = hibernate_request();
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
            &session_plan("r-hibernate-1"),
            recorded(),
        )
        .await
        .expect("accepted receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Accepted);
    assert_eq!(provider.transport().dispatch_count(), 1);
    let captured = provider.transport().captured();
    assert_eq!(captured[0].redacted_args, vec!["hibernate".to_string()]);
}

// ─────────────────────────────────────────────────────────────────────────────
// C) hibernate-unavailable reports Unsupported/Unavailable — never fabricated
//    (OSC-020)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn hibernate_unavailable_fails_before_dispatch_never_fabricated() {
    let chain = Chain::build("hibernate", serde_json::json!({})).await;

    let transport =
        FakePowerSessionTransport::new(PowerSessionBackend::LogindDbus).hibernate_available(false);
    let provider = PowerSessionControl::new(transport);
    let request = hibernate_request();
    let desired = request.desired_state();

    let err = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &session_plan("r-hibernate-2"),
            recorded(),
        )
        .await
        .expect_err("hibernate-unavailable must fail before dispatch");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::Unsupported { .. }
    ));
    assert_eq!(
        provider.transport().dispatch_count(),
        0,
        "unsupported hibernate must never dispatch"
    );
}

#[tokio::test]
#[serial]
async fn hibernate_probe_missing_defaults_to_unsupported_never_fabricated() {
    let chain = Chain::build("hibernate", serde_json::json!({})).await;

    // No scripted probe at all — the fake defaults to "unavailable" rather
    // than fabricating support.
    let transport = FakePowerSessionTransport::new(PowerSessionBackend::LogindDbus);
    let provider = PowerSessionControl::new(transport);
    let request = hibernate_request();
    let desired = request.desired_state();

    let err = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &session_plan("r-hibernate-3"),
            recorded(),
        )
        .await
        .expect_err("missing hibernate probe must fail closed as Unsupported");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::Unsupported { .. }
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// D) shutdown_system / reboot_system: Accepted, never advertise rollback
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn shutdown_system_dispatches_immediate_poweroff_and_reaches_accepted() {
    let chain = Chain::build("shutdown_system", serde_json::json!({ "delay_minutes": 5 })).await;

    let transport = FakePowerSessionTransport::new(PowerSessionBackend::Loginctl)
        .dispatch_outcome(accepted_outcome());
    let provider = PowerSessionControl::new(transport);
    let request = shutdown_request(5);
    let desired = request.desired_state();

    let plan = session_plan("r-shutdown-1");
    assert!(
        matches!(plan.rollback, RollbackPlan::Unavailable),
        "shutdown_system must never advertise rollback"
    );

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
        .expect("accepted receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Accepted);
    let captured = provider.transport().captured();
    assert_eq!(
        captured[0].redacted_args,
        vec!["poweroff".to_string()],
        "delay scheduling is Task 3.8's scope; this dispatches an immediate poweroff"
    );
}

#[tokio::test]
#[serial]
async fn reboot_system_dispatches_and_reaches_accepted_no_rollback_claim() {
    let chain = Chain::build("reboot_system", serde_json::json!({})).await;

    let transport = FakePowerSessionTransport::new(PowerSessionBackend::Loginctl)
        .dispatch_outcome(accepted_outcome());
    let provider = PowerSessionControl::new(transport);
    let request = reboot_request();
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
            &session_plan("r-reboot-1"),
            recorded(),
        )
        .await
        .expect("accepted receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Accepted);
    let captured = provider.transport().captured();
    assert_eq!(captured[0].redacted_args, vec!["reboot".to_string()]);
}

// ─────────────────────────────────────────────────────────────────────────────
// E) Missing scripted lock-state read reports Unavailable — never a
//    fabricated lock state.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn missing_lock_state_reports_unavailable_not_a_fabricated_state() {
    let chain = Chain::build("lock_screen", serde_json::json!({})).await;
    let transport = FakePowerSessionTransport::new(PowerSessionBackend::Loginctl);
    let provider = PowerSessionControl::new(transport);
    let request = lock_request();

    let err = provider
        .observe(&chain.host_ctx, &request)
        .await
        .expect_err("missing session lock-state must report Unavailable");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::Unavailable { .. }
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// F) D-Bus/Polkit denial remains denied — no sudo/broader fallback. A denied
//    dispatch stays an `Uncertain`/pre-mutation fact, never silently retried
//    through an alternate privileged path.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn permission_denied_dispatch_has_no_fallback_and_stays_denied() {
    let chain = Chain::build("reboot_system", serde_json::json!({})).await;

    let transport = FakePowerSessionTransport::new(PowerSessionBackend::Loginctl);
    // No dispatch scripted at all → the fake's own "no scripted dispatch"
    // Unavailable stands in for a denied/absent transport; the provider must
    // propagate it verbatim with no retry, no sudo, and no second mutator.
    let provider = PowerSessionControl::new(transport);
    let request = reboot_request();
    let desired = request.desired_state();

    let err = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &session_plan("r-reboot-denied"),
            recorded(),
        )
        .await
        .expect_err("denied/absent transport must propagate as an error, not a fabricated Accepted");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::Unavailable { .. }
    ));
    assert_eq!(
        provider.transport().dispatch_count(),
        1,
        "exactly one dispatch attempt; no retry/fallback"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// G) The runtime's power_session() port seam resolves through a composed
//    HostOsControl aggregate and falls back to Unavailable when none is
//    composed (Task 2.4 HostOsControl::power_session() addition).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn runtime_power_session_port_is_unavailable_when_no_provider_is_composed() {
    assert!(sentinel_is_armed());
    let rt = OsControlRuntime::detached();
    let result = rt.power_session("lock_screen");
    assert!(
        matches!(
            result,
            Err(kria_core::os_control::OsControlError::Unavailable { .. })
        ),
        "no provider composed must map to Unavailable"
    );
}

#[test]
fn runtime_power_session_port_resolves_through_composed_fake_host() {
    use kria_core::os_control::testing::FakeHostOsControl;

    let transport = FakePowerSessionTransport::new(PowerSessionBackend::Loginctl).locked_ok(true);
    let session_provider: Arc<dyn kria_core::os_control::PowerSessionControlPort> =
        Arc::new(PowerSessionControl::new(transport));

    let fake_host =
        FakeHostOsControl::new("power-session-aggregate").with_power_session(session_provider);
    let rt = OsControlRuntime::with_host(Arc::new(fake_host));

    let session = rt
        .power_session("lock_screen")
        .expect("power_session port composed");
    assert_eq!(rt.provider_id().unwrap().as_str(), "power-session-aggregate");
    let _ = session; // exercised type; behavior covered by the Chain-based tests above.
}
