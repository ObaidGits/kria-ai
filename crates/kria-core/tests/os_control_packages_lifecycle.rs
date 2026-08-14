//! Task 3.4 — "Complete package planning, install/remove and update
//! assessment" (OSC-014), design §3, §9.3, §10.1, §12.
//!
//! # What this binary proves
//!
//! [`os_control::packages`] already unit-tests
//! [`PackagePlan::classify_desired_state`] in isolation (the install-vs-
//! update-vs-remove-vs-no-change fix). This is the **deny-live,
//! in-process** harness that drives the *real*
//! [`PackageControl`]`<`[`FakePackageTransport`]`>` provider through
//! [`OsControlRuntime::run_mutation`] end to end, over the same governed
//! audit-admission + resource-lease + grant chain the other domain
//! lifecycle harnesses use, proving:
//!
//! * `install_package`/`uninstall_package` applying an already-applied
//!   plan digest is `Unchanged` (zero dispatch) — the idempotency half of
//!   the plan-apply contract;
//! * applying a not-yet-applied plan dispatches exactly once and reaches
//!   `Verified` once the transaction is confirmed through a **fresh**
//!   re-observation (OSC-014.7: verification never reuses the apply-time
//!   observation, and never claims automatic rollback);
//! * a denied/partial apply surfaces the proven-no-effect
//!   [`OsControlError::PermissionDenied`]/uncertain outcome rather than a
//!   silently "successful" receipt;
//! * `rollback()` is never actually invoked by any completion path here and,
//!   if called directly, reports the truthful "no inverse" fact — never an
//!   automatic downgrade/reinstall claim (OSC-014.7);
//! * the runtime's `packages()` port resolves `Unavailable` with no
//!   provider composed and resolves through a composed `FakeHostOsControl`
//!   otherwise;
//! * the whole run never trips the process-wide deny-live sentinel — every
//!   effect is the scripted [`FakePackageTransport`], never a live
//!   PackageKit D-Bus call or `apt`/`dnf`/`pacman`/`zypper`/`snap`/
//!   `flatpak`/`pkexec`/`sudo` subprocess.
//!
//! Run ONLY this binary, serially:
//! `cargo test -p kria-core --no-default-features --features os-control-test \
//!    --test os_control_packages_lifecycle -- --test-threads=1`

#![cfg(feature = "os-control-test")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::resource_lease::{ResourceLeaseManager, ResourceRequirement};
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::safety::RiskLevel;

use kria_core::os_control::packages::fake::FakePackageTransport;
use kria_core::os_control::packages::{
    PackageChange, PackageControl, PackageObservation, PackageOperation, PackagePage, PackagePlan,
    PackageProviderId, PackageRef, PackageRequest, PackageTransactionState, UpdateAssessment,
};
use kria_core::os_control::resource::os_write_requirements;
use kria_core::os_control::runtime::SealBinding;
use kria_core::os_control::{
    sentinel_is_armed, sentinel_trip_count, ActionId, ActionLifecycle, AdmissionRequest,
    AppliedDispatch, ApplyOutcome, AuditAdmissionToken, ComparatorKind, CorrelationId, Digest,
    HostExecutionContext, MutationPlan, OsAuditStore, OsControlError, OsControlRuntime,
    OsLeaseContext, OsResourceCoordinator, ProviderId, RedactionPolicy, RequestSensitivity,
    RollbackPlan, SessionContext, SessionId, SnapshotRevision,
};

const SESSION: &str = "sess-packages-1";

/// Compose the full governed chain for a mutating packages tool, mirroring
/// the F1 prompt-contract harness's `Chain` (see
/// `os_control_storage_lifecycle.rs`).
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

fn htop_ref() -> PackageRef {
    PackageRef::new(PackageProviderId::Apt, "htop")
}

fn install_plan() -> PackagePlan {
    PackagePlan {
        operation: PackageOperation::Install,
        provider: PackageProviderId::Apt,
        requested: vec![htop_ref()],
        installs: vec![PackageChange {
            package: htop_ref(),
            from_version: None,
            to_version: Some("3.0.5".to_string()),
        }],
        upgrades: vec![],
        removals: vec![],
        download_bytes: Some(512_000),
        disk_delta_bytes: Some(1_024_000),
        security_relevant: Some(false),
        reboot_required: Some(false),
    }
}

fn install_request() -> PackageRequest {
    let plan = install_plan();
    PackageRequest {
        action: "install_package".to_string(),
        params: serde_json::json!({ "plan_digest": plan.digest().as_hex() }),
        plan,
    }
}

fn uninstall_request() -> PackageRequest {
    let plan = PackagePlan {
        operation: PackageOperation::Remove,
        provider: PackageProviderId::Apt,
        requested: vec![htop_ref()],
        installs: vec![],
        upgrades: vec![],
        removals: vec![PackageChange {
            package: htop_ref(),
            from_version: Some("3.0.5".to_string()),
            to_version: None,
        }],
        download_bytes: None,
        disk_delta_bytes: Some(-1_024_000),
        security_relevant: Some(false),
        reboot_required: Some(false),
    };
    PackageRequest {
        action: "uninstall_package".to_string(),
        params: serde_json::json!({ "plan_digest": plan.digest().as_hex() }),
        plan,
    }
}

fn plan_state(plan: &PackagePlan, applied: bool) -> PackageTransactionState {
    PackageTransactionState::new(plan.provider, plan.digest(), applied, Some(false))
}

fn plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-packages-1"),
        provider: ProviderId::new("packages-fake-packagekit"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn recorded() -> kria_core::os_control::AuditCompletionState {
    kria_core::os_control::AuditCompletionState::Recorded {
        record_id: kria_core::os_control::AuditRecordId::new("rec-packages"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// A) install_package idempotency: plan already applied → Unchanged, zero
//    dispatch.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn install_package_already_applied_is_unchanged_with_zero_dispatch() {
    let baseline = sentinel_trip_count();
    let request = install_request();
    let params = request.params.clone();
    let chain = Chain::build("install_package", params).await;
    assert_eq!(chain.admission_count(), 1);

    let transport =
        FakePackageTransport::new().transaction_state_ok(plan_state(&request.plan, true));
    let provider = PackageControl::new(transport);
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
        "already-applied plan must not dispatch a transaction"
    );
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// B) install_package on a not-yet-applied plan: dispatches once, verifies
//    through a FRESH re-observation (OSC-014.7), reaches Verified.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn install_package_dispatches_once_and_verifies_via_fresh_observation() {
    let baseline = sentinel_trip_count();
    let request = install_request();
    let params = request.params.clone();
    let chain = Chain::build("install_package", params).await;

    let transport = FakePackageTransport::new()
        // 1: run_mutation pre-observation (idempotency check) — not yet applied.
        .transaction_state_ok(plan_state(&request.plan, false))
        // 2: run_mutation under-lease re-observation (TOCTOU close).
        .transaction_state_ok(plan_state(&request.plan, false))
        // 3: run_mutation post-apply fresh re-observation.
        .transaction_state_ok(plan_state(&request.plan, true))
        // 4: PackageControl::verify's own fresh, independent re-read
        //    (OSC-014.7 — never reuses the apply-time observation).
        .transaction_state_ok(plan_state(&request.plan, true))
        .apply_outcome(Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            None,
            kria_core::os_control::BoundedVec::new(),
        ))));
    let provider = PackageControl::new(transport);
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
    assert_eq!(
        provider.transport().dispatch_count(),
        1,
        "apply exactly once"
    );
    assert!(
        provider
            .transport()
            .labels()
            .contains(&"apply_transaction".to_string()),
        "install must dispatch through the package transport's apply_transaction \
         (which itself routes exclusively through BrokerOperation::ApplyPackagePlan)"
    );
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// C) uninstall_package on an already-removed plan: Unchanged, zero dispatch.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn uninstall_package_already_removed_is_unchanged_with_zero_dispatch() {
    let request = uninstall_request();
    let params = request.params.clone();
    let chain = Chain::build("uninstall_package", params).await;

    let transport =
        FakePackageTransport::new().transaction_state_ok(plan_state(&request.plan, true));
    let provider = PackageControl::new(transport);
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
// D) Denied apply: PermissionDenied surfaces as a proven-no-effect error, not
//    a "successful" receipt claiming partial rollback.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn install_package_denial_surfaces_permission_denied_never_a_receipt() {
    let request = install_request();
    let params = request.params.clone();
    let chain = Chain::build("install_package", params).await;

    let transport = FakePackageTransport::new()
        // pre-observation — not yet applied (real attempt).
        .transaction_state_ok(plan_state(&request.plan, false))
        .transaction_state_ok(plan_state(&request.plan, false))
        .apply_outcome(Err(OsControlError::PermissionDenied {
            authority: kria_core::os_control::contract::SafeText::new("polkit"),
            remediation: kria_core::os_control::contract::SafeText::new("authenticate"),
        }));
    let provider = PackageControl::new(transport);
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
        result.expect_err("denied apply must surface PermissionDenied, not a receipt"),
        OsControlError::PermissionDenied { .. }
    ));
    assert_eq!(provider.transport().dispatch_count(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// E) rollback() never claims an automatic inverse (OSC-014.7, design §9.3).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn rollback_reports_truthful_no_inverse_never_automatic_downgrade() {
    let request = install_request();
    let chain = Chain::build("install_package", request.params.clone()).await;
    let transport = FakePackageTransport::new();
    let provider = PackageControl::new(transport);

    let permit = kria_core::os_control::context::MutationPermit::for_test(
        &chain.lease_set,
        &chain.token,
        Digest::from_hex(chain.grant.resource_set_digest()),
    );
    let ctx = kria_core::os_control::context::AdmittedMutationContext::for_test(
        &chain.host_ctx,
        &chain.grant,
        permit,
    );
    let token = kria_core::os_control::receipt::RollbackToken::new(
        Digest::of_str("token"),
        SessionId::new(SESSION),
        Digest::of_str("install_package"),
        ProviderId::new("packages-fake-packagekit"),
        kria_core::os_control::ReceiptId::new("r-packages-1"),
        kria_core::os_control::GrantNonce::new("nonce-1"),
        std::time::SystemTime::now() + Duration::from_secs(60),
    );

    let outcome =
        kria_core::os_control::contract::DesiredStateControl::rollback(&provider, &ctx, &token)
            .await
            .expect("rollback call itself never errors — it reports the truthful fact");

    match outcome {
        ApplyOutcome::Uncertain(dispatch) => {
            assert_eq!(
                dispatch.cause(),
                kria_core::os_control::UncertainEffectCause::Unobservable,
                "rollback must report the truthful 'no inverse' fact, never claim an \
                 automatic downgrade/reinstall"
            );
        }
        other => panic!("expected Uncertain(Unobservable), got {other:?}"),
    }
    // No package transaction was actually dispatched by rollback.
    assert_eq!(provider.transport().dispatch_count(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// F) Runtime port seam: Unavailable with no provider composed; resolves
//    through a composed FakeHostOsControl otherwise.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn runtime_packages_port_is_unavailable_when_no_provider_is_composed() {
    assert!(sentinel_is_armed());
    let rt = OsControlRuntime::detached();
    let result = rt.packages("install_package");
    assert!(matches!(
        result,
        Err(kria_core::os_control::OsControlError::Unavailable { .. })
    ));
}

#[test]
fn runtime_packages_port_resolves_through_composed_fake_host() {
    use kria_core::os_control::testing::FakeHostOsControl;

    let transport = FakePackageTransport::new();
    let packages_provider: Arc<dyn kria_core::os_control::PackageControlPort> =
        Arc::new(PackageControl::new(transport));

    let fake_host = FakeHostOsControl::new("packages-aggregate").with_packages(packages_provider);
    let rt = OsControlRuntime::with_host(Arc::new(fake_host));

    let _ = rt
        .packages("install_package")
        .expect("packages port composed");
    assert_eq!(rt.provider_id().unwrap().as_str(), "packages-aggregate");
}

// ─────────────────────────────────────────────────────────────────────────────
// G) Provider coexistence: apt + snap + flatpak resolve as distinct,
//    explicit provider identities within the same transport (design §9.3
//    "On Ubuntu, APT, Snap, and Flatpak may coexist and provider identity
//    remains explicit").
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn search_reports_distinct_explicit_provider_identity_per_result() {
    let chain = Chain::build("search_package", serde_json::json!({ "query": "code" })).await;
    let page = PackagePage {
        items: vec![
            kria_core::os_control::PackageEntry {
                package: PackageRef::new(PackageProviderId::Apt, "code"),
                provider: PackageProviderId::Apt,
                installed_version: None,
                candidate_version: Some("1.0".to_string()),
                origin: Some("jammy".to_string()),
                size_bytes: Some(1000),
            },
            kria_core::os_control::PackageEntry {
                package: PackageRef::new(PackageProviderId::Snap, "code"),
                provider: PackageProviderId::Snap,
                installed_version: None,
                candidate_version: Some("1.85".to_string()),
                origin: Some("snap-store".to_string()),
                size_bytes: Some(200_000),
            },
            kria_core::os_control::PackageEntry {
                package: PackageRef::new(PackageProviderId::Flatpak, "com.visualstudio.code"),
                provider: PackageProviderId::Flatpak,
                installed_version: None,
                candidate_version: Some("1.85.0".to_string()),
                origin: Some("flathub".to_string()),
                size_bytes: Some(250_000),
            },
        ],
        truncated: false,
    };
    let transport = FakePackageTransport::new().search_ok(page);
    let provider = PackageControl::new(transport);

    let result = provider
        .search(&chain.host_ctx, "code", None, 0, 32)
        .await
        .expect("search succeeds");

    let providers: Vec<PackageProviderId> = result.items.iter().map(|i| i.provider).collect();
    assert_eq!(
        providers,
        vec![
            PackageProviderId::Apt,
            PackageProviderId::Snap,
            PackageProviderId::Flatpak
        ],
        "each result must retain its own explicit provider identity, never merged"
    );
    assert_eq!(
        provider.transport().dispatch_count(),
        0,
        "search is a pure read"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// H) check_system_updates / get_reboot_required: pure reads never dispatch.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn check_system_updates_is_a_pure_read_with_honest_unknown_metadata() {
    let chain = Chain::build("check_system_updates", serde_json::json!({})).await;
    let transport = FakePackageTransport::new().update_assessment_ok(UpdateAssessment {
        provider: PackageProviderId::Apt,
        update_count: 5,
        security_update_count: None,
        download_bytes: Some(10_000_000),
        reboot_likely: None,
    });
    let provider = PackageControl::new(transport);

    let assessment = provider
        .assess_updates(&chain.host_ctx, None)
        .await
        .expect("assessment succeeds");

    assert_eq!(assessment.update_count, 5);
    assert!(
        assessment.security_update_count.is_none(),
        "unknown security relevance must be honest None, never fabricated"
    );
    assert!(
        assessment.reboot_likely.is_none(),
        "unknown reboot likelihood must be honest None, never fabricated"
    );
    assert_eq!(provider.transport().dispatch_count(), 0);
}

#[tokio::test]
#[serial]
async fn get_package_info_never_fabricates_reboot_implication() {
    let chain = Chain::build("get_package_info", serde_json::json!({})).await;
    let transport = FakePackageTransport::new().info_ok(PackageObservation {
        package: htop_ref(),
        provider: PackageProviderId::Apt,
        installed_version: Some("3.0.5".to_string()),
        candidate_version: Some("3.0.5".to_string()),
        origin: Some("jammy".to_string()),
        size_bytes: Some(200_000),
        dependency_count: Some(3),
        reboot_implication: None,
    });
    let provider = PackageControl::new(transport);

    let observation = provider
        .get_info(&chain.host_ctx, &htop_ref())
        .await
        .expect("info read succeeds");

    assert!(observation.reboot_implication.is_none());
    assert_eq!(provider.transport().dispatch_count(), 0);
}
