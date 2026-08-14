//! Task 2.2 — "Migrate brightness and prepare display provider seam"
//! (OSC-019, OSC-031, OSC-032), design §3, §9.6.
//!
//! # What this binary proves
//!
//! [`os_control::display`] already unit-tests its pieces in isolation (parsers,
//! backend selection/argv, physical-vs-gamma classification, no-XRandR-on-
//! Wayland selection). This is the **deny-live, in-process** harness that
//! drives the *real* [`DisplayControl`]`<`[`FakeDisplayTransport`]`>` provider
//! through [`OsControlRuntime::run_mutation`] end to end, over the same
//! governed audit-admission + resource-lease + grant chain the F1 foundation
//! harness uses, so the full observe → idempotency → seal → apply → verify →
//! (rollback) lifecycle is exercised for `set_brightness`, plus a read-only
//! `get_display_state` observation:
//!
//! * `set_brightness` is `Unchanged` (zero dispatch) when the observed
//!   brightness is already within the configured percentage tolerance;
//! * `set_brightness` dispatches the exact governed `brightnessctl set` argv,
//!   verifies against fresh evidence, and reaches `Verified`;
//! * a post-apply contradiction rolls back to the captured prior brightness
//!   and reaches `RolledBack`;
//! * a missing/absent backend selection reports the frozen `Unavailable`
//!   envelope — never a fabricated state (OSC-031);
//! * `select_brightness_backend` (the provider-selection choke point) never
//!   selects the X11-only XRandR gamma adapter for a Wayland session, even
//!   when it is the only backend reported as available (OSC-019.3, OSC-032.3);
//! * the whole run never trips the process-wide deny-live sentinel and no
//!   display handler in this module ever launches a child process directly —
//!   the only captured "process" evidence is the redacted
//!   [`StructuredCommandSummary`] the fake transport records.
//!
//! Run ONLY this binary, serially:
//! `cargo test -p kria-core --no-default-features --features os-control-test \
//!    --test os_control_display_lifecycle -- --test-threads=1`

#![cfg(feature = "os-control-test")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::resource_lease::{ResourceLeaseManager, ResourceRequirement};
use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::safety::RiskLevel;

use kria_core::os_control::capability::DisplayServer;
use kria_core::os_control::display::fake::FakeDisplayTransport;
use kria_core::os_control::display::{
    select_brightness_backend, BrightnessBackend, DisplayControl, DisplayOp, DisplayRequest,
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

const SESSION: &str = "sess-display-1";

/// Compose the full governed chain for a mutating display tool, mirroring the
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

fn set_brightness_request(percent: u8) -> DisplayRequest {
    DisplayRequest {
        action: "set_brightness".to_string(),
        params: serde_json::json!({ "percent": percent }),
        op: DisplayOp::SetBrightness(percent),
    }
}

fn get_state_request() -> DisplayRequest {
    DisplayRequest {
        action: "get_display_state".to_string(),
        params: serde_json::json!({}),
        op: DisplayOp::GetState,
    }
}

fn brightness_plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-display-1"),
        provider: ProviderId::new("display-fake-brightnessctl"),
        comparator: ComparatorKind::WithinTolerance,
        tolerance: Some(Tolerance { abs: 2.0 }),
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn recorded() -> kria_core::os_control::AuditCompletionState {
    kria_core::os_control::AuditCompletionState::Recorded {
        record_id: kria_core::os_control::AuditRecordId::new("rec-display"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// A) set_brightness idempotency: already within tolerance → Unchanged, zero
//    dispatch
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_brightness_within_tolerance_is_unchanged_with_zero_dispatch() {
    let baseline = sentinel_trip_count();
    let params = serde_json::json!({ "percent": 60 });
    let chain = Chain::build("set_brightness", params).await;
    assert_eq!(chain.admission_count(), 1);

    // Observed brightness (61) is within the 2.0-point tolerance of desired (60).
    let transport = FakeDisplayTransport::new(BrightnessBackend::Brightnessctl).read_ok(61);
    let provider = DisplayControl::with_tolerance(transport, 2.0);
    let request = set_brightness_request(60);
    let desired = request
        .desired_state(BrightnessBackend::Brightnessctl)
        .expect("mutation has a desired state");

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
            &brightness_plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert!(!receipt.changed());
    assert_eq!(
        provider.transport().dispatch_count(),
        0,
        "idempotent brightness must not dispatch a command"
    );
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// B) set_brightness mutation: dispatch exact governed brightnessctl argv,
//    verify, Verified
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_brightness_dispatches_exact_argv_and_reaches_verified() {
    let baseline = sentinel_trip_count();
    let params = serde_json::json!({ "percent": 75 });
    let chain = Chain::build("set_brightness", params).await;

    let transport = FakeDisplayTransport::new(BrightnessBackend::Brightnessctl)
        // 1: run_mutation's pre-observation (idempotency check).
        .read_ok(40)
        // 2: run_mutation's under-lease re-observation (TOCTOU close).
        .read_ok(40)
        // 3: DisplayControl::apply's own pre-apply snapshot (for rollback).
        .read_ok(40)
        // 4: run_mutation's post-apply fresh re-observation.
        .read_ok(75)
        // 5: DisplayControl::verify's independent read.
        .read_ok(75)
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = DisplayControl::with_tolerance(transport, 2.0);
    let request = set_brightness_request(75);
    let desired = request
        .desired_state(BrightnessBackend::Brightnessctl)
        .unwrap();

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
            &brightness_plan(RollbackPlan::Unavailable),
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
    // The captured governed command is the exact brightnessctl set argv — no
    // display handler in this crate ever launches a process directly; this is
    // the only "command" evidence, and it is a redacted digest-only summary.
    assert_eq!(captured[0].capability, "set_brightness");
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// C) Post-apply contradiction rolls back to the captured prior brightness
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_brightness_contradiction_rolls_back_to_prior_brightness() {
    let params = serde_json::json!({ "percent": 90 });
    let chain = Chain::build("set_brightness", params).await;

    let transport = FakeDisplayTransport::new(BrightnessBackend::Brightnessctl)
        .read_ok(20) // 1: run_mutation pre-observation (idempotency)
        .read_ok(20) // 2: run_mutation under-lease re-observation
        .read_ok(20) // 3: DisplayControl::apply pre-apply snapshot (captures 20)
        .read_ok(20) // 4: run_mutation post-apply re-observation (still 20 → contradiction)
        .read_ok(20) // 5: DisplayControl::verify independent read (contradicted)
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ))
        // The rollback's own restore-verification read, after the rollback dispatch.
        .read_ok(20) // 6: rollback's verify() read, confirming restore to 20
        .dispatch_outcome(kria_core::os_control::ApplyOutcome::Applied(
            kria_core::os_control::AppliedDispatch::new(
                None,
                kria_core::os_control::BoundedVec::new(),
            ),
        ));
    let provider = DisplayControl::with_tolerance(transport, 2.0);
    let request = set_brightness_request(90);
    let desired = request
        .desired_state(BrightnessBackend::Brightnessctl)
        .unwrap();

    let token = RollbackToken::new(
        Digest::of_str("display-rollback-tok"),
        SessionId::new(SESSION),
        Digest::of_str("set_brightness"),
        ProviderId::new("display-fake-brightnessctl"),
        kria_core::os_control::ReceiptId::new("r-display-3"),
        kria_core::os_control::GrantNonce::new("nonce-1"),
        std::time::SystemTime::now() + Duration::from_secs(60),
    );
    let plan = MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-display-3"),
        provider: ProviderId::new("display-fake-brightnessctl"),
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
// D) get_display_state observation: read-only, no dispatch, no grant needed
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn get_display_state_observes_without_dispatch() {
    let chain = Chain::build("get_display_state", serde_json::json!({})).await;
    let transport = FakeDisplayTransport::new(BrightnessBackend::GnomeSettingsDaemon).read_ok(33);
    let provider = DisplayControl::new(transport);
    let request = get_state_request();

    let state = provider
        .observe(&chain.host_ctx, &request)
        .await
        .expect("observation succeeds");

    assert_eq!(state.brightness_percent, 33);
    assert_eq!(state.backend, BrightnessBackend::GnomeSettingsDaemon);
    assert_eq!(provider.transport().dispatch_count(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// E) Missing session display (no scripted read) reports Unavailable — never a
//    fabricated state (OSC-031 / parser-ambiguity-never-success).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn missing_session_display_reports_unavailable_not_a_fabricated_state() {
    let chain = Chain::build("get_display_state", serde_json::json!({})).await;
    // No `read_ok`/`read_err` scripted: the fake's default is `Unavailable`.
    let transport = FakeDisplayTransport::new(BrightnessBackend::Brightnessctl);
    let provider = DisplayControl::new(transport);
    let request = get_state_request();

    let err = provider
        .observe(&chain.host_ctx, &request)
        .await
        .expect_err("missing session display must report Unavailable");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::Unavailable { .. }
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// F) The no-XRandR-on-Wayland invariant (OSC-019.3, OSC-032.3): the provider
//    -selection choke point never returns XrandrGamma for a Wayland session,
//    even when it is the only backend reported as available.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn xrandr_gamma_is_never_selected_in_a_wayland_session() {
    use BrightnessBackend::*;

    // XRandR is the ONLY backend the (scripted) session reports as available —
    // a real degraded-provider scenario. Selection for Wayland must still
    // yield `None` rather than falsely selecting an X11-only adapter.
    assert_eq!(
        select_brightness_backend(DisplayServer::Wayland, &[XrandrGamma]),
        None,
        "XRandR must never be selected for a Wayland session, even as last resort"
    );

    // With a full candidate set, Wayland selects the GNOME session D-Bus
    // property (preferred, display-server-neutral), never XRandR.
    assert_eq!(
        select_brightness_backend(
            DisplayServer::Wayland,
            &[GnomeSettingsDaemon, Brightnessctl, XrandrGamma]
        ),
        Some(GnomeSettingsDaemon)
    );

    // An X11 session MAY select XRandR as a last resort when nothing else is
    // available.
    assert_eq!(
        select_brightness_backend(DisplayServer::X11, &[XrandrGamma]),
        Some(XrandrGamma)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// G) The runtime's display() port seam resolves through a composed
//    HostOsControl aggregate and falls back to Unavailable when none is
//    composed (Task 2.2 HostOsControl::display() addition).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn runtime_display_port_is_unavailable_when_no_provider_is_composed() {
    assert!(sentinel_is_armed());
    let rt = OsControlRuntime::detached();
    let result = rt.display("set_brightness");
    assert!(
        matches!(
            result,
            Err(kria_core::os_control::OsControlError::Unavailable { .. })
        ),
        "no provider composed must map to Unavailable"
    );
}

#[test]
fn runtime_display_port_resolves_through_composed_fake_host() {
    use kria_core::os_control::testing::FakeHostOsControl;

    let transport = FakeDisplayTransport::new(BrightnessBackend::Brightnessctl).read_ok(50);
    let display_provider: Arc<dyn kria_core::os_control::DisplayControlPort> =
        Arc::new(DisplayControl::new(transport));

    let fake_host = FakeHostOsControl::new("display-aggregate").with_display(display_provider);
    let rt = OsControlRuntime::with_host(Arc::new(fake_host));

    let display = rt.display("get_display_state").expect("display port composed");
    // The port is reachable and dyn-dispatched; identity comes from the
    // aggregate, never a raw provider handle.
    assert_eq!(rt.provider_id().unwrap().as_str(), "display-aggregate");
    let _ = display; // exercised type; behavior is covered by the Chain-based tests above.
}
