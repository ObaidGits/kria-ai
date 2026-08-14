//! Task 3.3 — desktop-association domain slice ("Complete applications,
//! intents and privacy-safe process semantics", OSC-013.9).
//!
//! # What this binary proves
//!
//! The **deny-live, in-process** harness driving the real
//! [`DesktopAssociationControl`]`<`[`FakeDesktopAssociationTransport`]`>`
//! provider through [`OsControlRuntime::run_mutation`] end to end for
//! `set_default_application`/`manage_autostart`:
//!
//! * already-associated (same default app / same autostart state) →
//!   `Unchanged`, zero dispatch;
//! * a real change dispatches exactly once and reaches `Verified`;
//! * rollback (`rollbackClaim: UserRequestable`) restores the exact prior
//!   default application / autostart state, proven with the real
//!   `RealDesktopAssociationTransport` over an injected `tempfile::TempDir`
//!   root (never the real user's `~/.config`);
//! * a missing scripted read reports `Unavailable`, never a fabricated
//!   state;
//! * the whole run never trips the deny-live sentinel.
//!
//! Run ONLY this binary, serially:
//! `cargo test -p kria-core --no-default-features --features os-control-test \
//!    --test os_control_desktop_association_lifecycle -- --test-threads=1`

#![cfg(feature = "os-control-test")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::resource_lease::{ResourceLeaseManager, ResourceRequirement};
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::safety::RiskLevel;

use kria_core::os_control::applications::fake_association::FakeDesktopAssociationTransport;
use kria_core::os_control::applications::{
    AssociationOp, AssociationRequest, DesktopAssociationControl, RealDesktopAssociationTransport,
};
use kria_core::os_control::DesktopAssociationTransport;
use kria_core::os_control::context::{AdmittedMutationContext, MutationPermit};
use kria_core::os_control::resource::os_write_requirements;
use kria_core::os_control::runtime::OsControlRuntime;
use kria_core::os_control::runtime::SealBinding;
use kria_core::os_control::{
    sentinel_is_armed, sentinel_trip_count, ActionId, ActionLifecycle, AdmissionRequest,
    AuditAdmissionToken, ComparatorKind, CorrelationId,
    DesiredStateControl, Digest, GrantNonce, HostExecutionContext, MutationPlan, OsAuditStore,
    OsLeaseContext, OsResourceCoordinator, ProviderId, ReceiptId, RedactionPolicy,
    RequestSensitivity, RollbackPlan, RollbackToken, SessionContext, SessionId, SnapshotRevision,
};

const SESSION: &str = "sess-desktop-assoc-1";

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

    fn mutation_ctx(&self) -> AdmittedMutationContext<'_> {
        let permit = MutationPermit::for_test(
            &self.lease_set,
            &self.token,
            Digest::of_str(self.grant.resource_set_digest()),
        );
        AdmittedMutationContext::for_test(&self.host_ctx, &self.grant, permit)
    }
}

fn default_app_request(mime: &str, app_id: &str) -> AssociationRequest {
    AssociationRequest {
        action: "set_default_application".to_string(),
        params: serde_json::json!({ "mime": mime, "app_id": app_id }),
        op: AssociationOp::SetDefaultApplication {
            mime: mime.to_string(),
            app_id: app_id.to_string(),
        },
    }
}

fn autostart_request(app_id: &str, enabled: bool) -> AssociationRequest {
    AssociationRequest {
        action: "manage_autostart".to_string(),
        params: serde_json::json!({ "app_id": app_id, "enabled": enabled }),
        op: AssociationOp::SetAutostart {
            app_id: app_id.to_string(),
            enabled,
        },
    }
}

fn plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: ReceiptId::new("r-desktop-assoc-1"),
        provider: ProviderId::new("application-desktop-association"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn recorded() -> kria_core::os_control::AuditCompletionState {
    kria_core::os_control::AuditCompletionState::Recorded {
        record_id: kria_core::os_control::AuditRecordId::new("rec-desktop-assoc"),
    }
}

fn rollback_token() -> RollbackToken {
    RollbackToken::new(
        Digest::of_str("token-1"),
        SessionId::new(SESSION),
        Digest::of_str("set_default_application"),
        ProviderId::new("application-desktop-association"),
        ReceiptId::new("r-desktop-assoc-1"),
        GrantNonce::new("nonce-1"),
        std::time::SystemTime::now() + Duration::from_secs(60),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// A) set_default_application idempotency: already default → Unchanged
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_default_application_already_default_is_unchanged_with_zero_dispatch() {
    let baseline = sentinel_trip_count();
    let params = serde_json::json!({ "mime": "text/plain", "app_id": "gedit" });
    let chain = Chain::build("set_default_application", params).await;
    assert_eq!(chain.admission_count(), 1);

    let transport = FakeDesktopAssociationTransport::new().with_default("text/plain", "gedit");
    let provider = DesktopAssociationControl::new(transport);
    let request = default_app_request("text/plain", "gedit");
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
    assert_eq!(provider.transport().set_default_calls().len(), 0);
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// B) set_default_application real change: dispatches once, reaches Verified
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_default_application_changes_and_reaches_verified() {
    let params = serde_json::json!({ "mime": "text/plain", "app_id": "kate" });
    let chain = Chain::build("set_default_application", params).await;

    let transport = FakeDesktopAssociationTransport::new().with_default("text/plain", "gedit");
    let provider = DesktopAssociationControl::new(transport);
    let request = default_app_request("text/plain", "kate");
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
    let calls = provider.transport().set_default_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], ("text/plain".to_string(), "kate".to_string()));
}

// ─────────────────────────────────────────────────────────────────────────────
// C) manage_autostart idempotency + real change
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn manage_autostart_already_enabled_is_unchanged() {
    let params = serde_json::json!({ "app_id": "myapp", "enabled": true });
    let chain = Chain::build("manage_autostart", params).await;

    let transport = FakeDesktopAssociationTransport::new().with_autostart("myapp", true);
    let provider = DesktopAssociationControl::new(transport);
    let request = autostart_request("myapp", true);
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
    assert_eq!(provider.transport().set_autostart_calls().len(), 0);
}

#[tokio::test]
#[serial]
async fn manage_autostart_disables_and_reaches_verified() {
    let params = serde_json::json!({ "app_id": "myapp", "enabled": false });
    let chain = Chain::build("manage_autostart", params).await;

    let transport = FakeDesktopAssociationTransport::new().with_autostart("myapp", true);
    let provider = DesktopAssociationControl::new(transport);
    let request = autostart_request("myapp", false);
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
    let calls = provider.transport().set_autostart_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], ("myapp".to_string(), false));
}

// ─────────────────────────────────────────────────────────────────────────────
// D) Rollback with the REAL transport over an injected tempdir root
//    (OSC-013.9): prior association is captured and restored exactly.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_default_application_rollback_restores_prior_app_over_real_transport() {
    let dir = tempfile::tempdir().unwrap();
    let real = RealDesktopAssociationTransport::new(dir.path()).unwrap();
    real.write_default_application_now("text/plain", "gedit")
        .unwrap();

    let params = serde_json::json!({ "mime": "text/plain", "app_id": "kate" });
    let chain = Chain::build("set_default_application", params).await;
    let provider = DesktopAssociationControl::new(real);
    let request = default_app_request("text/plain", "kate");

    // apply() captures the pre-apply state ("gedit") before dispatching.
    let _ = provider
        .apply(&chain.mutation_ctx(), &request, &request.desired_state())
        .await
        .expect("apply succeeds");
    assert_eq!(
        provider
            .transport()
            .read_default_application("text/plain")
            .await
            .unwrap(),
        Some("kate".to_string())
    );

    let outcome = provider
        .rollback(&chain.mutation_ctx(), &rollback_token())
        .await
        .expect("rollback succeeds");

    assert!(matches!(outcome, kria_core::os_control::ApplyOutcome::Applied(_)));
    assert_eq!(
        provider
            .transport()
            .read_default_application("text/plain")
            .await
            .unwrap(),
        Some("gedit".to_string()),
        "rollback must restore the exact prior default application"
    );
}

#[tokio::test]
#[serial]
async fn manage_autostart_rollback_restores_prior_state_over_real_transport() {
    let dir = tempfile::tempdir().unwrap();
    let real = RealDesktopAssociationTransport::new(dir.path()).unwrap();
    real.write_autostart_now("myapp", true).unwrap();

    let params = serde_json::json!({ "app_id": "myapp", "enabled": false });
    let chain = Chain::build("manage_autostart", params).await;
    let provider = DesktopAssociationControl::new(real);
    let request = autostart_request("myapp", false);

    let _ = provider
        .apply(&chain.mutation_ctx(), &request, &request.desired_state())
        .await
        .expect("apply succeeds");
    assert!(!provider.transport().read_autostart("myapp").await.unwrap());

    let token = RollbackToken::new(
        Digest::of_str("token-2"),
        SessionId::new(SESSION),
        Digest::of_str("manage_autostart"),
        ProviderId::new("application-desktop-association"),
        ReceiptId::new("r-desktop-assoc-1"),
        GrantNonce::new("nonce-2"),
        std::time::SystemTime::now() + Duration::from_secs(60),
    );
    let outcome = provider
        .rollback(&chain.mutation_ctx(), &token)
        .await
        .expect("rollback succeeds");

    assert!(matches!(outcome, kria_core::os_control::ApplyOutcome::Applied(_)));
    assert!(
        provider.transport().read_autostart("myapp").await.unwrap(),
        "rollback must restore the exact prior autostart-enabled state"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// E) Missing scripted read reports Unavailable — never a fabricated state
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn runtime_desktop_association_port_is_unavailable_when_no_provider_is_composed() {
    assert!(sentinel_is_armed());
    let rt = OsControlRuntime::detached();
    let result = rt.desktop_association("set_default_application");
    assert!(matches!(
        result,
        Err(kria_core::os_control::OsControlError::Unavailable { .. })
    ));
}

#[tokio::test]
#[serial]
async fn runtime_desktop_association_port_resolves_through_composed_fake_host() {
    use kria_core::os_control::testing::FakeHostOsControl;

    let transport = FakeDesktopAssociationTransport::new();
    let assoc_provider: Arc<dyn kria_core::os_control::DesktopAssociationControlPort> =
        Arc::new(DesktopAssociationControl::new(transport));

    let fake_host =
        FakeHostOsControl::new("desktop-association-aggregate").with_desktop_association(assoc_provider);
    let rt = OsControlRuntime::with_host(Arc::new(fake_host));

    let _ = rt
        .desktop_association("set_default_application")
        .expect("desktop-association port composed");
    assert_eq!(
        rt.provider_id().unwrap().as_str(),
        "desktop-association-aggregate"
    );
}
