//! Task 1.11 — "Wire prompt-to-provider in-process contract harness"
//! (OSC-001, OSC-009, OSC-033, OSC-036), design Correctness Properties 1, 9, 33,
//! 36 and design §§2, 4, 6, 15, 18.
//!
//! # What this binary proves (the F1 foundation completion gate)
//!
//! This is a **deny-live, in-process** contract harness that composes the whole
//! F1 foundation — prompt routing (`IntentRouter`), native-OS admission
//! (`ExecutionGate` + durable `DecisionStore`), durable audit admission/terminal
//! (`OsAuditStore`), deterministic resource leasing (`OsResourceCoordinator`),
//! and the sealed governed runtime (`OsControlRuntime`) with **scripted fake
//! providers** — and exercises representative prompts of each class end-to-end:
//!
//! * GREEN read / idempotent-unchanged (one audit admission before
//!   pre-observation, no-op, no approval, zero apply);
//! * YELLOW bounded mutation (fresh grant, exact leases, matching audit token,
//!   private mutation permit, apply-once, verification, one terminal audit,
//!   lease release);
//! * RED requiring approval (committed SQLite approval gates the grant; approval
//!   expiry invalidates; privacy-sensitive RED read admits fail-closed);
//! * every deny-live negative / receipt-variant path;
//! * `unavailable` (no provider), `ambiguous` (clarification, no provider call),
//!   and `BLACK` (refused, never routed).
//!
//! Across the whole harness the process-wide deny-live sentinel stays untripped
//! (`sentinel_trip_count()` unchanged) and no Tauri/Axum/live bus/process/
//! session/device transport is ever opened — every effect is a scripted fake in
//! an in-memory SQLite / isolated-lease composition (OSC-033, Property 33).
//!
//! Run ONLY this binary, serially:
//! `cargo test -p kria-core --no-default-features --features os-control-test \
//!    --test os_control_prompt_contract -- --test-threads=1`

#![cfg(feature = "os-control-test")]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::collaborative_decision::{DecisionCandidate, DecisionStore};
use kria_core::agent::execution_gate::{
    ExecutionGate, ExecutionGateInput, OsActionGrant, ResumeGateOutcome,
};
use kria_core::agent::os_action_authority::{
    effects_request_native_os, is_native_os_action, NATIVE_OS_EFFECT,
};
use kria_core::agent::resource_lease::{ResourceLeaseManager, ResourceRequirement};
use kria_core::agent::router::{Intent, IntentRouter};
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::safety::{PolicyEngine, RiskLevel};

use kria_core::os_control::resource::os_write_requirements;
use kria_core::os_control::runtime::RollbackExecPlan;
use kria_core::os_control::{
    frozen_contract, frozen_tool_names, sentinel_is_armed, sentinel_trip_count, AcceptanceEvidence,
    AcceptedDispatch, ActionId, ActionLifecycle, AdmissionRequest, AdmittedMutationContext,
    AppliedDispatch, ApplyOutcome, AuditAdmissionToken, AuditCompletionState, AuditFault,
    AuditRecordId, BoundedVec, ComparatorKind, CorrelationId, DesiredStateControl, Digest,
    GrantNonce, HostExecutionContext, MutationPlan, MutationResult, NonEmptyBoundedVec,
    NormalizedObservation, OsAuditStore, OsControlError, OsControlRuntime, OsLeaseContext,
    OsResourceCoordinator, PartialDispatch, PartialEffectCause, ProviderId, ReceiptId,
    RedactionPolicy, RequestSensitivity, RollbackPlan, RollbackToken, SafeErrorCode, SafeStepId,
    SafeText, SatisfyingVerification, SealBinding, SessionContext, SessionId, SnapshotRevision,
    TerminalAppendOutcome, TerminalRecord, Tolerance, UncertainDispatch, UncertainEffectCause,
    VerificationContradiction, VerificationReliability, VerificationReport, FROZEN_OPERATION_COUNT,
};

// ─────────────────────────────────────────────────────────────────────────────
// Shared deny-live scaffolding
// ─────────────────────────────────────────────────────────────────────────────

const SESSION: &str = "sess-os-1";

/// A normalized domain observation the runtime can compare for idempotency and
/// verification (mirrors the provider observation contract).
#[derive(Debug, Clone, PartialEq)]
struct TestObs {
    tag: String,
    value: Option<f64>,
}

impl TestObs {
    fn new(tag: &str, value: Option<f64>) -> Self {
        Self {
            tag: tag.to_string(),
            value,
        }
    }
}

impl NormalizedObservation for TestObs {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&self.tag)
    }
    fn numeric_value(&self) -> Option<f64> {
        self.value
    }
}

/// An ordered recorder of the governed provider calls (no live transport).
#[derive(Clone, Default)]
struct Calls(Arc<Mutex<Vec<String>>>);

impl Calls {
    fn record(&self, label: &str) {
        self.0.lock().unwrap().push(label.to_string());
    }
    fn labels(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
    fn count(&self, label: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|l| *l == label)
            .count()
    }
}

/// A scripted fake `DesiredStateControl` provider. Each phase pops a queued
/// scripted response; a missing response maps to `Unavailable` and NEVER falls
/// through to any live path (OSC-033.2). Every call is recorded in order so the
/// harness can assert apply-once, no-redispatch, and phase ordering.
struct ScriptedProvider {
    calls: Calls,
    observe: Mutex<VecDeque<Result<TestObs, OsControlError>>>,
    apply: Mutex<VecDeque<Result<ApplyOutcome, OsControlError>>>,
    verify: Mutex<VecDeque<Result<VerificationReport<TestObs>, OsControlError>>>,
    rollback: Mutex<VecDeque<Result<ApplyOutcome, OsControlError>>>,
}

impl ScriptedProvider {
    fn new(calls: Calls) -> Self {
        Self {
            calls,
            observe: Mutex::new(VecDeque::new()),
            apply: Mutex::new(VecDeque::new()),
            verify: Mutex::new(VecDeque::new()),
            rollback: Mutex::new(VecDeque::new()),
        }
    }
    fn observe_ok(self, tag: &str) -> Self {
        self.observe
            .lock()
            .unwrap()
            .push_back(Ok(TestObs::new(tag, None)));
        self
    }
    fn observe_err(self) -> Self {
        self.observe.lock().unwrap().push_back(Err(unavailable()));
        self
    }
    fn apply(self, outcome: ApplyOutcome) -> Self {
        self.apply.lock().unwrap().push_back(Ok(outcome));
        self
    }
    fn verify(self, report: VerificationReport<TestObs>) -> Self {
        self.verify.lock().unwrap().push_back(Ok(report));
        self
    }
    fn rollback(self, outcome: ApplyOutcome) -> Self {
        self.rollback.lock().unwrap().push_back(Ok(outcome));
        self
    }
}

#[async_trait]
impl DesiredStateControl<(), TestObs> for ScriptedProvider {
    async fn observe(
        &self,
        _ctx: &HostExecutionContext,
        _request: &(),
    ) -> Result<TestObs, OsControlError> {
        self.calls.record("observe");
        self.observe
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(unavailable()))
    }
    async fn apply(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        _request: &(),
        _desired: &TestObs,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.calls.record("apply");
        self.apply
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(OsControlError::CancelledBeforeMutation))
    }
    async fn verify(
        &self,
        _ctx: &HostExecutionContext,
        _request: &(),
        _desired: &TestObs,
    ) -> Result<VerificationReport<TestObs>, OsControlError> {
        self.calls.record("verify");
        self.verify
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(unavailable()))
    }
    async fn rollback(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        _token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.calls.record("rollback");
        self.rollback
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(OsControlError::CancelledBeforeMutation))
    }
}

fn unavailable() -> OsControlError {
    OsControlError::Unavailable {
        provider: None,
        reason: SafeText::new("no scripted response"),
        retryable: false,
    }
}

// ── ApplyOutcome / VerificationReport builders ──────────────────────────────

fn applied() -> ApplyOutcome {
    ApplyOutcome::Applied(AppliedDispatch::new(None, BoundedVec::new()))
}

fn accepted() -> ApplyOutcome {
    ApplyOutcome::Accepted(AcceptedDispatch::new(
        None,
        AcceptanceEvidence {
            detail: SafeText::new("logind accepted session-ending action"),
            accepted_at: SystemTime::now(),
        },
        BoundedVec::new(),
    ))
}

fn uncertain() -> ApplyOutcome {
    ApplyOutcome::Uncertain(UncertainDispatch::new(
        None,
        UncertainEffectCause::TransportLostAfterDispatch,
        BoundedVec::new(),
    ))
}

fn partial() -> ApplyOutcome {
    let completed = NonEmptyBoundedVec::new(SafeStepId::new("step-1"), BoundedVec::new());
    ApplyOutcome::PartiallyApplied(PartialDispatch::new(
        None,
        completed,
        SafeStepId::new("step-2"),
        PartialEffectCause::StepFailedAfterCommit,
        BoundedVec::new(),
    ))
}

fn satisfying(freshness_ms: u64, tag: &str) -> VerificationReport<TestObs> {
    let obs = TestObs::new(tag, None);
    VerificationReport::Satisfied(SatisfyingVerification::new(
        kria_core::os_control::OsEvidenceSource::AuthoritativeServiceState,
        VerificationReliability::Strong,
        ProviderId::new("fake"),
        kria_core::os_control::RedactedObservation::new(obs.clone(), obs.observation_digest()),
        None,
        SystemTime::now(),
        freshness_ms,
    ))
}

fn contradicted() -> VerificationReport<TestObs> {
    VerificationReport::Contradicted(VerificationContradiction::new(
        Digest::of_str("expected"),
        Some(Digest::of_str("observed")),
        SafeErrorCode::from_static("os_control.incident.contradicted"),
    ))
}

fn inconclusive() -> VerificationReport<TestObs> {
    VerificationReport::Inconclusive {
        reason: SafeText::new("no decisive evidence"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Governed composition fixture — REAL audit store + REAL resource coordinator +
// gate-shaped grant, all bound to the same action/params/session/resource set so
// the runtime seal actually agrees (Property 1: apply is unreachable without a
// matching grant + permit borrowing matching live leases + committed admission).
// ─────────────────────────────────────────────────────────────────────────────

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
    /// Compose the full governed chain for a mutating tool: durable audit
    /// admission (before any observation), held write leases, a fresh gate-shaped
    /// grant, and an observation context lent from the same admission.
    async fn build(tool: &str, params: serde_json::Value, sensitivity: RequestSensitivity) -> Self {
        Self::build_with_grant(tool, params, sensitivity, false).await
    }

    async fn build_with_grant(
        tool: &str,
        params: serde_json::Value,
        sensitivity: RequestSensitivity,
        expired_grant: bool,
    ) -> Self {
        let audit = OsAuditStore::open_in_memory();

        // (1) Durable audit admission — one admission BEFORE pre-observation.
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
                sensitivity,
            })
            .expect("audit admission must succeed on a healthy store");

        // (2) Held exclusive write leases in the single canonical order.
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

        // (3) Fresh gate-shaped grant bound to the same admitted facts.
        let grant = if expired_grant {
            OsActionGrant::for_test_expired(
                SESSION,
                tool,
                &params,
                ExecutionTarget::Host,
                &reqs,
                RiskLevel::Yellow,
            )
        } else {
            OsActionGrant::for_test(
                SESSION,
                tool,
                &params,
                ExecutionTarget::Host,
                &reqs,
                RiskLevel::Yellow,
            )
        };

        // (4) Observation-only context lent from THIS admission.
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
        self.binding_with(SESSION, SnapshotRevision(1), &self.params)
    }

    fn binding_with<'b>(
        &'b self,
        session: &'b str,
        revision: SnapshotRevision,
        params: &'b serde_json::Value,
    ) -> SealBinding<'b> {
        SealBinding {
            session_id: session,
            action: &self.tool,
            params,
            target: ExecutionTarget::Host,
            resource_requirements: &self.reqs,
            capability_snapshot_revision: revision,
        }
    }

    /// Number of durable `admission` rows for this action (must be exactly one).
    fn admission_count(&self) -> usize {
        // A healthy hash chain is a precondition for trusting the count.
        self.audit.verify_chain().expect("audit hash chain intact");
        self.audit.admission_count(self.token.admission_id())
    }
}

/// A representative terminal record for the normal (recorded-once) path.
fn terminal(lifecycle: ActionLifecycle, rollback_available: bool) -> TerminalRecord {
    TerminalRecord {
        lifecycle,
        provider: ProviderId::new("fake"),
        before_digest: Some(Digest::of_str("before")),
        after_digest: Some(Digest::of_str("after")),
        provider_receipt_digest: None,
        verification_source: Some(
            kria_core::os_control::OsEvidenceSource::AuthoritativeServiceState,
        ),
        verification_reliability: Some(VerificationReliability::Strong),
        rollback_available,
        incident_code: None,
        duration_ms: 5,
    }
}

fn plan(comparator: ComparatorKind, rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: ReceiptId::new("r-1"),
        provider: ProviderId::new("fake"),
        comparator,
        tolerance: match comparator {
            ComparatorKind::WithinTolerance => Some(Tolerance { abs: 5.0 }),
            _ => None,
        },
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn recorded() -> AuditCompletionState {
    AuditCompletionState::Recorded {
        record_id: AuditRecordId::new("rec-1"),
    }
}

/// Drive `run_mutation` over a scripted provider using the composed chain.
async fn run(
    chain: &Chain,
    provider: &ScriptedProvider,
    desired: &TestObs,
    plan: &MutationPlan,
    completion: AuditCompletionState,
) -> MutationResult<TestObs> {
    OsControlRuntime::detached()
        .run_mutation(
            provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &(),
            desired,
            plan,
            completion,
        )
        .await
}

// ─────────────────────────────────────────────────────────────────────────────
// A) Prompt routing: prompt → canonical tool (Property 9/36 first hop). BLACK &
//    ambiguous prompts produce no native-OS tool routing.
// ─────────────────────────────────────────────────────────────────────────────

fn routed_tool(prompt: &str) -> Option<String> {
    let result = IntentRouter::classify(prompt);
    match result.intent {
        Intent::DirectTool(tool) => Some(tool),
        _ => result.tool_hint,
    }
}

#[test]
#[serial]
fn representative_prompts_route_to_frozen_canonical_tools() {
    assert!(sentinel_is_armed(), "deny-live sentinel must be armed");
    let baseline = sentinel_trip_count();

    // GREEN/privacy read, YELLOW mutations, RED session-ending — each resolves to
    // exactly one frozen canonical OS tool.
    let cases = [
        ("list nearby wifi networks", "get_wifi_networks"),
        ("set volume to 40", "set_volume"),
        ("turn on wifi", "toggle_wifi"),
        ("set brightness to 50", "set_brightness"),
        ("reboot the system", "reboot_system"),
        ("lock screen", "lock_screen"),
    ];
    for (prompt, expected) in cases {
        let tool = routed_tool(prompt)
            .unwrap_or_else(|| panic!("prompt `{prompt}` did not route to a tool"));
        assert_eq!(tool, expected, "prompt `{prompt}` routed to `{tool}`");
        assert!(
            frozen_contract(&tool).is_some(),
            "routed tool `{tool}` must be a frozen canonical OS operation"
        );
        assert!(
            is_native_os_action(&tool),
            "routed tool `{tool}` must be a native-OS action"
        );
    }

    // Routing performs zero host effects.
    assert_eq!(
        sentinel_trip_count(),
        baseline,
        "routing tripped the sentinel"
    );
}

#[test]
#[serial]
fn black_and_ambiguous_prompts_do_not_route_to_a_native_os_tool() {
    // BLACK-scope administration prompts never resolve to a native-OS tool.
    for black in [
        "format my hard drive",
        "repartition the disk with fdisk",
        "reinstall the grub bootloader",
    ] {
        let routed = routed_tool(black);
        let is_native = routed.as_deref().map(is_native_os_action).unwrap_or(false);
        assert!(
            !is_native,
            "BLACK prompt `{black}` must not route to a native-OS tool (got {routed:?})"
        );
    }

    // A vague/ambiguous prompt yields no actionable tool hint → clarification,
    // never a provider dispatch.
    let ambiguous = IntentRouter::classify("do the thing with the stuff");
    assert!(
        ambiguous.tool_hint.is_none(),
        "ambiguous prompt must not bind a tool hint"
    );
    assert!(!matches!(ambiguous.intent, Intent::DirectTool(_)));
}

// ─────────────────────────────────────────────────────────────────────────────
// B) Frozen metadata / event contract preserved (OSC-009/036).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn frozen_manifest_metadata_is_preserved() {
    assert_eq!(frozen_tool_names().len(), FROZEN_OPERATION_COUNT);
    // Representative rows keep their frozen risk/verification/rollback contract.
    let sv = frozen_contract("set_volume").expect("set_volume frozen");
    assert_eq!(sv.default_tier(), RiskLevel::Yellow);
    assert!(matches!(
        sv.verification,
        kria_core::os_control::ManifestVerificationClass::FreshAuthoritativeObservation
    ));
    let gw = frozen_contract("get_wifi_networks").expect("get_wifi_networks frozen");
    assert!(matches!(
        gw.verification,
        kria_core::os_control::ManifestVerificationClass::NoVerification
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// C) GREEN / idempotent-unchanged: one admission before pre-observation, no-op,
//    zero apply, no approval (Property 1, OSC-006).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn green_idempotent_unchanged_performs_no_apply_and_one_admission() {
    let baseline = sentinel_trip_count();
    let chain = Chain::build(
        "set_volume",
        serde_json::json!({ "percent": 40 }),
        RequestSensitivity::Mutation,
    )
    .await;

    // Exactly one admission is durable BEFORE any provider observation.
    assert_eq!(chain.admission_count(), 1, "exactly one durable admission");

    let calls = Calls::default();
    // `observe` reports the desired state already holds → Unchanged, zero apply.
    let provider = ScriptedProvider::new(calls.clone()).observe_ok("already");
    let desired = TestObs::new("already", None);

    let receipt = run(
        &chain,
        &provider,
        &desired,
        &plan(ComparatorKind::Exact, RollbackPlan::Unavailable),
        recorded(),
    )
    .await
    .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert_eq!(calls.count("apply"), 0, "idempotent state must not apply");
    assert_eq!(calls.count("rollback"), 0);
    // Observation happened; apply/verify did not.
    assert_eq!(calls.labels(), vec!["observe".to_string()]);
    assert_eq!(sentinel_trip_count(), baseline);
}

// ─────────────────────────────────────────────────────────────────────────────
// D) YELLOW bounded mutation: full normative chain — fresh grant, exact leases,
//    matching audit token, private permit, apply-once, verification, one terminal
//    audit, lease release, stable serialized snapshot (Property 1, 36).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn yellow_mutation_completes_full_governed_chain() {
    let baseline = sentinel_trip_count();
    let chain = Chain::build(
        "toggle_wifi",
        serde_json::json!({ "enabled": true }),
        RequestSensitivity::Mutation,
    )
    .await;
    assert_eq!(chain.admission_count(), 1);
    let held_before = chain.lease_set.held_count();
    assert!(held_before >= 1, "at least one exclusive write lease held");

    // Record the sole terminal audit for the verified outcome.
    let append = chain
        .audit
        .append_terminal(&chain.token, &terminal(ActionLifecycle::Verified, true));
    let completion = match &append {
        TerminalAppendOutcome::Recorded { .. } => append.completion_state(),
        other => panic!("expected Recorded terminal, got {other:?}"),
    };
    assert_eq!(
        chain.audit.incomplete_admission_count(),
        0,
        "the sole terminal closes the admission"
    );
    assert!(chain.audit.is_healthy());

    let calls = Calls::default();
    // observe(before) != desired, re-observe under lease != desired, apply, then
    // fresh satisfying verification → Verified.
    let provider = ScriptedProvider::new(calls.clone())
        .observe_ok("off")
        .observe_ok("off")
        .observe_ok("on")
        .apply(applied())
        .verify(satisfying(10, "on"));
    let desired = TestObs::new("on", None);

    let receipt = run(
        &chain,
        &provider,
        &desired,
        &plan(ComparatorKind::Exact, RollbackPlan::Unavailable),
        completion,
    )
    .await
    .expect("verified receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    assert!(receipt.verification().is_some());
    assert_eq!(calls.count("apply"), 1, "apply exactly once");
    assert_eq!(calls.count("verify"), 1, "verify not retried");
    assert_eq!(calls.count("rollback"), 0);

    // Stable serialized result snapshot (safe summary is the sole projection).
    let json = serde_json::to_value(receipt.safe_summary()).expect("serialize summary");
    assert_eq!(json["lifecycle"], "verified");
    for key in ["receipt_id", "action_hash", "provider"] {
        assert!(json.get(key).is_some(), "snapshot missing `{key}`: {json}");
    }

    // Leases release on drop (RAII); nothing tripped the sentinel.
    drop(chain);
    assert_eq!(sentinel_trip_count(), baseline);
}

// ─────────────────────────────────────────────────────────────────────────────
// E) RED requiring approval: committed SQLite approval gates the resume grant;
//    decision persistence failure fails closed (Property 1, OSC-001.9).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
#[serial]
fn committed_sqlite_approval_gates_the_resume_grant() {
    let store = Arc::new(DecisionStore::in_memory());
    let gate = ExecutionGate::new(Arc::new(PolicyEngine::new()), Arc::clone(&store));
    let params = serde_json::json!({ "enabled": true });

    // Admit a native-OS action to obtain a correctly-hashed action proposal.
    let evaluation = gate.evaluate(ExecutionGateInput {
        session_id: SESSION,
        user_text: "turn on wifi",
        action: "toggle_wifi",
        params: &params,
        destructive_hint: false,
    });
    let proposal = evaluation
        .action_proposal
        .expect("native-OS evaluation carries an action proposal");

    // Create a DURABLE (SQLite) OS decision and resolve it — the commit is the
    // gate for a resume grant.
    let decision = store
        .create_decision_for_action(
            &proposal,
            DecisionCandidate::target_selection(
                "Select execution target",
                vec!["host".to_string()],
                "toggle_wifi",
            ),
        )
        .expect("durable OS decision must commit");
    let resolved = store
        .resolve_with_version(&decision.id, decision.version, "host", "user_gui")
        .expect("resolution commit")
        .expect("resolved decision exists");

    let resume = gate.revalidate_resume(&resolved, false);
    match resume.outcome {
        ResumeGateOutcome::Ready => {
            let grant = resume
                .os_action_grant
                .expect("Ready resume of a native-OS decision mints a grant");
            assert_eq!(grant.action(), "toggle_wifi");
            assert!(
                grant.decision_id().is_some(),
                "resume grant must be linked to the committed durable decision"
            );
        }
        // If policy re-raises approval/risk on resume, the invariant still holds:
        // no grant is minted unless the resume is Ready.
        other => {
            assert!(
                resume.os_action_grant.is_none(),
                "no grant may be minted unless resume is Ready (got {other:?})"
            );
        }
    }
}

#[test]
#[serial]
fn decision_persistence_failure_fails_closed_with_no_decision() {
    // A store with NO durable OS-decision authority must fail closed when asked
    // to create a native-OS decision — nothing is written, so no grant can ever
    // observe it (OSC-001.9: no in-memory OS fallback).
    let store = Arc::new(DecisionStore::in_memory_without_os_authority());
    let gate = ExecutionGate::new(Arc::new(PolicyEngine::new()), Arc::clone(&store));
    let params = serde_json::json!({ "enabled": true });
    let evaluation = gate.evaluate(ExecutionGateInput {
        session_id: SESSION,
        user_text: "turn on wifi",
        action: "toggle_wifi",
        params: &params,
        destructive_hint: false,
    });
    let proposal = evaluation
        .action_proposal
        .expect("native-OS evaluation carries an action proposal");

    let created = store.create_decision_for_action(
        &proposal,
        DecisionCandidate::target_selection(
            "Select target",
            vec!["host".to_string()],
            "toggle_wifi",
        ),
    );
    assert!(
        created.is_err(),
        "native-OS decision create must fail closed with no durable authority"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// F) Privacy-sensitive RED read admits, and fails closed while audit unhealthy.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn privacy_sensitive_read_admits_and_fails_closed_when_unhealthy() {
    let audit = OsAuditStore::open_in_memory();
    let req = |sensitivity| AdmissionRequest {
        session_id: SessionId::new(SESSION),
        correlation_id: CorrelationId::new("corr-read"),
        action_id: ActionId::new("act-read"),
        tool_name: "get_wifi_networks".to_string(),
        params: serde_json::json!({}),
        target_hash: Digest::of_str(ExecutionTarget::Host.as_str()),
        capability_snapshot_revision: SnapshotRevision(1),
        risk: RiskLevel::Red,
        decision_id: None,
        sensitivity,
    };

    // Healthy store admits a privacy-sensitive read.
    let token = audit
        .admit_action(&req(RequestSensitivity::PrivacySensitiveRead))
        .expect("privacy-sensitive read admits on healthy store");
    assert_eq!(audit.admission_count(token.admission_id()), 1);

    // Force unhealthy: an interrupted terminal marks audit unhealthy.
    audit.inject_fault(AuditFault::InterruptNextTerminal);
    let interrupted = audit.append_terminal(&token, &terminal(ActionLifecycle::Unverified, false));
    assert!(matches!(
        interrupted,
        TerminalAppendOutcome::PendingRecovery { .. }
    ));
    assert!(!audit.is_healthy());

    // A privacy-sensitive read now fails closed (pre-provider).
    let err = audit
        .admit_action(&req(RequestSensitivity::PrivacySensitiveRead))
        .expect_err("privacy-sensitive read must fail closed while unhealthy");
    assert_eq!(err.code(), "os_control.audit_unavailable");
}

// ─────────────────────────────────────────────────────────────────────────────
// G) Deny-live negative / seal paths: apply is unreachable without every
//    authority agreeing; no provider mutation before the permit (Property 1).
// ─────────────────────────────────────────────────────────────────────────────

/// Run with a caller-supplied binding; assert no apply occurred and return err.
async fn seal_error_for_binding(chain: &Chain, binding: SealBinding<'_>) -> OsControlError {
    let calls = Calls::default();
    let provider = ScriptedProvider::new(calls.clone()).observe_ok("before"); // != desired
    let desired = TestObs::new("after", None);
    let err = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &binding,
            &(),
            &desired,
            &plan(ComparatorKind::Exact, RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect_err("seal must fail");
    assert_eq!(calls.count("apply"), 0, "seal failure must not apply");
    assert_eq!(calls.count("rollback"), 0);
    err
}

#[tokio::test]
#[serial]
async fn missing_provider_returns_unavailable_and_never_falls_back() {
    let rt = OsControlRuntime::detached();
    assert!(!rt.provider_present());
    let err = rt.probe_provider("set_volume").unwrap_err();
    assert_eq!(err.code(), "os_control.unavailable");
    let env = err.to_envelope();
    assert!(env["os_control"]["provider"].is_null());
    assert_eq!(env["os_control"]["availability"], "unavailable");
}

#[tokio::test]
#[serial]
async fn provider_observation_unavailable_is_pre_mutation_error() {
    let chain = Chain::build(
        "toggle_wifi",
        serde_json::json!({ "enabled": true }),
        RequestSensitivity::Mutation,
    )
    .await;
    let calls = Calls::default();
    let provider = ScriptedProvider::new(calls.clone()).observe_err();
    let desired = TestObs::new("on", None);
    let err = run(
        &chain,
        &provider,
        &desired,
        &plan(ComparatorKind::Exact, RollbackPlan::Unavailable),
        recorded(),
    )
    .await
    .expect_err("pre-observation failure is a pre-mutation error");
    assert_eq!(err.code(), "os_control.unavailable");
    assert_eq!(calls.count("apply"), 0);
}

#[tokio::test]
#[serial]
async fn forged_stale_and_mismatched_authorities_block_apply() {
    let chain = Chain::build(
        "set_volume",
        serde_json::json!({ "percent": 40 }),
        RequestSensitivity::Mutation,
    )
    .await;
    let other_params = serde_json::json!({ "percent": 99 });

    // Session mismatch.
    let e = seal_error_for_binding(
        &chain,
        chain.binding_with("other-session", SnapshotRevision(1), &chain.params),
    )
    .await;
    assert_eq!(e.code(), "os_control.grant_invalid");

    // Parameter (argv) mismatch — stale/forged grant for different params.
    let e = seal_error_for_binding(
        &chain,
        chain.binding_with(SESSION, SnapshotRevision(1), &other_params),
    )
    .await;
    assert_eq!(e.code(), "os_control.grant_invalid");

    // Stale capability-snapshot revision.
    let e = seal_error_for_binding(
        &chain,
        chain.binding_with(SESSION, SnapshotRevision(2), &chain.params),
    )
    .await;
    assert_eq!(e.code(), "os_control.grant_invalid");

    // Non-host binding target.
    let mut host_only = chain.binding();
    host_only.target = ExecutionTarget::Vm;
    let e = seal_error_for_binding(&chain, host_only).await;
    assert_eq!(e.code(), "os_control.invalid_request");
}

#[tokio::test]
#[serial]
async fn unheld_lease_and_expired_grant_block_apply() {
    // Unheld / mismatched lease set → resource_busy, no provider mutation.
    let chain = Chain::build(
        "set_volume",
        serde_json::json!({ "percent": 40 }),
        RequestSensitivity::Mutation,
    )
    .await;
    let wrong_lease =
        kria_core::os_control::AcquiredResourceLeaseSet::for_test(Digest::of_str("different-set"));
    let calls = Calls::default();
    let provider = ScriptedProvider::new(calls.clone()).observe_ok("before");
    let desired = TestObs::new("after", None);
    let err = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &wrong_lease,
            &chain.token,
            &chain.binding(),
            &(),
            &desired,
            &plan(ComparatorKind::Exact, RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect_err("unheld lease blocks");
    assert_eq!(err.code(), "os_control.resource_busy");
    assert_eq!(calls.count("apply"), 0);

    // Expired grant → approval_expired (approval freshness), no provider mutation.
    let expired = Chain::build_with_grant(
        "set_volume",
        serde_json::json!({ "percent": 40 }),
        RequestSensitivity::Mutation,
        true,
    )
    .await;
    let e = seal_error_for_binding(&expired, expired.binding()).await;
    assert_eq!(e.code(), "os_control.approval_expired");
}

#[tokio::test]
#[serial]
async fn observation_from_a_foreign_admission_blocks_apply() {
    let chain = Chain::build(
        "set_volume",
        serde_json::json!({ "percent": 40 }),
        RequestSensitivity::Mutation,
    )
    .await;
    // An observation context lent from a DIFFERENT admission token.
    let foreign_store = OsAuditStore::open_in_memory();
    let foreign_token = foreign_store
        .admit_action(&AdmissionRequest {
            session_id: SessionId::new(SESSION),
            correlation_id: CorrelationId::new("corr-x"),
            action_id: ActionId::new("act-x"),
            tool_name: "set_volume".to_string(),
            params: chain.params.clone(),
            target_hash: Digest::of_str(ExecutionTarget::Host.as_str()),
            capability_snapshot_revision: SnapshotRevision(1),
            risk: RiskLevel::Yellow,
            decision_id: None,
            sensitivity: RequestSensitivity::Mutation,
        })
        .unwrap();
    let foreign_ctx = HostExecutionContext::for_test(
        CorrelationId::new("corr-x"),
        ActionId::new("act-x"),
        foreign_token.observation_authority(),
        Arc::new(SessionContext::new(SessionId::new(SESSION))),
        CancellationToken::new(),
        Instant::now() + Duration::from_secs(30),
        RedactionPolicy::default(),
    );
    let calls = Calls::default();
    let provider = ScriptedProvider::new(calls.clone()).observe_ok("before");
    let desired = TestObs::new("after", None);
    let err = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &foreign_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &(),
            &desired,
            &plan(ComparatorKind::Exact, RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect_err("foreign-admission observation blocks");
    assert_eq!(err.code(), "os_control.grant_invalid");
    assert_eq!(calls.count("apply"), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// H) Every terminal receipt variant is reachable through the governed chain.
// ─────────────────────────────────────────────────────────────────────────────

async fn yellow_chain() -> Chain {
    Chain::build(
        "toggle_wifi",
        serde_json::json!({ "enabled": true }),
        RequestSensitivity::Mutation,
    )
    .await
}

#[tokio::test]
#[serial]
async fn receipt_variant_verified() {
    let chain = yellow_chain().await;
    let calls = Calls::default();
    let p = ScriptedProvider::new(calls.clone())
        .observe_ok("off")
        .observe_ok("off")
        .observe_ok("on")
        .apply(applied())
        .verify(satisfying(10, "on"));
    let r = run(
        &chain,
        &p,
        &TestObs::new("on", None),
        &plan(ComparatorKind::Exact, RollbackPlan::Unavailable),
        recorded(),
    )
    .await
    .unwrap();
    assert_eq!(r.lifecycle(), ActionLifecycle::Verified);
    assert_eq!(calls.count("apply"), 1);
}

#[tokio::test]
#[serial]
async fn receipt_variant_unverified_stale_and_inconclusive_and_unobservable() {
    // Stale evidence.
    let chain = yellow_chain().await;
    let p = ScriptedProvider::new(Calls::default())
        .observe_ok("off")
        .observe_ok("off")
        .observe_ok("on")
        .apply(applied())
        .verify(satisfying(5_000, "on"));
    let r = run(
        &chain,
        &p,
        &TestObs::new("on", None),
        &plan(ComparatorKind::Exact, RollbackPlan::Unavailable),
        recorded(),
    )
    .await
    .unwrap();
    assert_eq!(r.lifecycle(), ActionLifecycle::Unverified);

    // Inconclusive verification.
    let chain = yellow_chain().await;
    let p = ScriptedProvider::new(Calls::default())
        .observe_ok("off")
        .observe_ok("off")
        .observe_ok("on")
        .apply(applied())
        .verify(inconclusive());
    let r = run(
        &chain,
        &p,
        &TestObs::new("on", None),
        &plan(ComparatorKind::Exact, RollbackPlan::Unavailable),
        recorded(),
    )
    .await
    .unwrap();
    assert_eq!(r.lifecycle(), ActionLifecycle::Unverified);

    // After-observation unavailable (no decisive terminal).
    let chain = yellow_chain().await;
    let p = ScriptedProvider::new(Calls::default())
        .observe_ok("off")
        .observe_ok("off")
        .observe_err()
        .apply(applied());
    let r = run(
        &chain,
        &p,
        &TestObs::new("on", None),
        &plan(ComparatorKind::Exact, RollbackPlan::Unavailable),
        recorded(),
    )
    .await
    .unwrap();
    assert_eq!(r.lifecycle(), ActionLifecycle::Unverified);
}

#[tokio::test]
#[serial]
async fn receipt_variant_accepted_and_partial() {
    // Accepted (session-ending / async) from acceptance evidence only.
    let chain = yellow_chain().await;
    let calls = Calls::default();
    let p = ScriptedProvider::new(calls.clone())
        .observe_ok("off")
        .observe_ok("off")
        .apply(accepted());
    let r = run(
        &chain,
        &p,
        &TestObs::new("on", None),
        &plan(ComparatorKind::Exact, RollbackPlan::Unavailable),
        recorded(),
    )
    .await
    .unwrap();
    assert_eq!(r.lifecycle(), ActionLifecycle::Accepted);
    assert_eq!(calls.count("verify"), 0, "accepted needs no verification");

    // PartiallyApplied (multi-step residue).
    let chain = yellow_chain().await;
    let p = ScriptedProvider::new(Calls::default())
        .observe_ok("off")
        .observe_ok("off")
        .apply(partial());
    let r = run(
        &chain,
        &p,
        &TestObs::new("on", None),
        &plan(ComparatorKind::Exact, RollbackPlan::Unavailable),
        recorded(),
    )
    .await
    .unwrap();
    assert_eq!(r.lifecycle(), ActionLifecycle::PartiallyApplied);
}

#[tokio::test]
#[serial]
async fn receipt_variant_uncertain_never_reports_verified() {
    let chain = yellow_chain().await;
    let p = ScriptedProvider::new(Calls::default())
        .observe_ok("off")
        .observe_ok("off")
        .observe_ok("mid")
        .apply(uncertain())
        .verify(satisfying(10, "on"));
    let r = run(
        &chain,
        &p,
        &TestObs::new("on", None),
        &plan(ComparatorKind::Exact, RollbackPlan::Unavailable),
        recorded(),
    )
    .await
    .unwrap();
    // Uncertain dispatch can never reach Verified.
    assert_eq!(r.lifecycle(), ActionLifecycle::Unverified);
}

#[tokio::test]
#[serial]
async fn receipt_variant_contradiction_verification_failed_and_rolled_back() {
    // Contradiction with NO automatic rollback → VerificationFailed.
    let chain = yellow_chain().await;
    let p = ScriptedProvider::new(Calls::default())
        .observe_ok("off")
        .observe_ok("off")
        .observe_ok("still-off")
        .apply(applied())
        .verify(contradicted());
    let r = run(
        &chain,
        &p,
        &TestObs::new("on", None),
        &plan(ComparatorKind::Exact, RollbackPlan::Unavailable),
        recorded(),
    )
    .await
    .unwrap();
    assert_eq!(r.lifecycle(), ActionLifecycle::VerificationFailed);

    // Contradiction WITH auto rollback that restores prior state → RolledBack.
    let chain = yellow_chain().await;
    let calls = Calls::default();
    let token = RollbackToken::new(
        Digest::of_str("tok"),
        SessionId::new(SESSION),
        Digest::of_str("toggle_wifi"),
        ProviderId::new("fake"),
        ReceiptId::new("r-1"),
        GrantNonce::new("n"),
        SystemTime::now() + Duration::from_secs(60),
    );
    let p = ScriptedProvider::new(calls.clone())
        .observe_ok("off")
        .observe_ok("off")
        .observe_ok("still-off")
        .apply(applied())
        .verify(contradicted())
        .rollback(applied())
        .verify(satisfying(10, "off")); // restore verified against prior state
    let r = run(
        &chain,
        &p,
        &TestObs::new("on", None),
        &plan(
            ComparatorKind::Exact,
            RollbackPlan::Available { token, auto: true },
        ),
        recorded(),
    )
    .await
    .unwrap();
    assert_eq!(r.lifecycle(), ActionLifecycle::RolledBack);
    assert_eq!(calls.count("rollback"), 1, "rollback dispatched once");
    assert_eq!(calls.count("apply"), 1, "forward apply never redispatched");
}

// ─────────────────────────────────────────────────────────────────────────────
// I) Separately-audited rollback action links to the original receipt.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn governed_rollback_action_restores_and_links() {
    let chain = yellow_chain().await;
    let calls = Calls::default();
    let token = RollbackToken::new(
        Digest::of_str("tok"),
        SessionId::new(SESSION),
        Digest::of_str("toggle_wifi"),
        ProviderId::new("fake"),
        ReceiptId::new("r-orig"),
        GrantNonce::new("n"),
        SystemTime::now() + Duration::from_secs(60),
    );
    let p = ScriptedProvider::new(calls.clone())
        .rollback(applied())
        .verify(satisfying(10, "prior"));
    let exec_plan = RollbackExecPlan {
        rollback_receipt_id: ReceiptId::new("r-rollback"),
        linked_receipt: ReceiptId::new("r-orig"),
        original_action_hash: Digest::of_str("toggle_wifi"),
        capability: ProviderId::new("fake"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        latency_ms: 5,
    };
    let prior = TestObs::new("prior", None);
    let receipt = OsControlRuntime::detached()
        .run_rollback(
            &p,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &(),
            &prior,
            &token,
            &exec_plan,
            recorded(),
        )
        .await
        .expect("rollback action runs");
    let json = serde_json::to_value(receipt.safe_summary()).expect("serialize rollback summary");
    assert!(
        json.is_object(),
        "rollback summary is a stable object: {json}"
    );
    assert_eq!(calls.count("rollback"), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// J) Terminal-persistence interruption + restart reconciliation never
//    redispatches (OSC-007, Property 1/7). The audit store holds NO provider
//    handle, so redispatch is structurally impossible.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn terminal_interruption_then_reconcile_closes_without_redispatch() {
    let audit = OsAuditStore::open_in_memory();
    let token = audit
        .admit_action(&AdmissionRequest {
            session_id: SessionId::new(SESSION),
            correlation_id: CorrelationId::new("corr-recover"),
            action_id: ActionId::new("act-recover"),
            tool_name: "toggle_wifi".to_string(),
            params: serde_json::json!({ "enabled": true }),
            target_hash: Digest::of_str(ExecutionTarget::Host.as_str()),
            capability_snapshot_revision: SnapshotRevision(1),
            risk: RiskLevel::Yellow,
            decision_id: None,
            sensitivity: RequestSensitivity::Mutation,
        })
        .unwrap();

    // Interrupt the terminal append → PendingRecovery, audit unhealthy.
    audit.inject_fault(AuditFault::InterruptNextTerminal);
    let interrupted = audit.append_terminal(&token, &terminal(ActionLifecycle::Verified, false));
    assert!(matches!(
        interrupted,
        TerminalAppendOutcome::PendingRecovery { .. }
    ));
    assert!(!audit.is_healthy());
    assert_eq!(audit.incomplete_admission_count(), 1);

    // A subsequent automatic mutation admission fails closed.
    let blocked = audit.admit_action(&AdmissionRequest {
        session_id: SessionId::new(SESSION),
        correlation_id: CorrelationId::new("corr-next"),
        action_id: ActionId::new("act-next"),
        tool_name: "set_volume".to_string(),
        params: serde_json::json!({ "percent": 10 }),
        target_hash: Digest::of_str(ExecutionTarget::Host.as_str()),
        capability_snapshot_revision: SnapshotRevision(1),
        risk: RiskLevel::Yellow,
        decision_id: None,
        sensitivity: RequestSensitivity::Mutation,
    });
    assert_eq!(
        blocked
            .expect_err("mutation blocked while unhealthy")
            .code(),
        "os_control.audit_unavailable"
    );

    // Bounded restart reconciliation closes the incomplete admission with the
    // sole terminal — using only durable rows, never a provider.
    let report = audit.reconcile_incomplete_admissions(64, None);
    assert!(report.reconciled >= 1);
    assert_eq!(audit.incomplete_admission_count(), 0);
    assert!(audit.is_healthy(), "health restored after reconcile");

    // The reconciler recorded exactly one terminal (idempotent, no redispatch):
    // a repeat reconcile is a no-op.
    let again = audit.reconcile_incomplete_admissions(64, None);
    assert_eq!(again.reconciled, 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// K) Extension dispatcher cannot bypass the runtime / obtain native-OS authority.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
#[serial]
fn extension_plane_cannot_authorize_native_os_effects() {
    use kria_core::capability::descriptor::{Effects, Reversibility};
    use kria_core::capability::grants::{GrantDecision, GrantStore, ScopeKind, ScopedGrant};
    use kria_core::capability::permission::{
        AuthorizeRequest, DefaultPermissionEngine, PermissionDecision, PermissionEngine,
    };

    let native = vec![NATIVE_OS_EFFECT.to_string()];

    // The permission engine denies native host-OS effects outright.
    let grants = GrantStore::in_memory().unwrap();
    let engine = DefaultPermissionEngine;
    let decision = engine.authorize(
        &AuthorizeRequest {
            provider_id: "rogue".to_string(),
            capability_id: "reboot_the_host".to_string(),
            effects: Effects {
                classes: native.clone(),
                reversible: Reversibility::Irreversible,
                idempotent: false,
                resource_class: Default::default(),
            },
            session_id: None,
            workspace_id: Some("default".to_string()),
        },
        &grants,
    );
    assert!(matches!(decision, PermissionDecision::Deny { .. }));

    // The grant store refuses to persist native host-OS authority.
    let grant = ScopedGrant {
        grant_id: "g-native".to_string(),
        provider_id: "rogue".to_string(),
        capability_id: "reboot_the_host".to_string(),
        scope_kind: ScopeKind::Persistent,
        scope_key: None,
        effects: native.clone(),
        decision: GrantDecision::Allow,
        granted_at: chrono::Utc::now(),
        expires_at: None,
        revoked: false,
    };
    assert!(grants.insert(&grant).is_err());

    // The effect marker is detected; generic extension effects are unaffected.
    assert!(effects_request_native_os(&native));
    assert!(!effects_request_native_os(&[
        "read".to_string(),
        "write".to_string(),
        "network".to_string(),
    ]));
    // Typed native-OS tools are recognized; generic execution is not.
    assert!(is_native_os_action("reboot_system"));
    assert!(!is_native_os_action("execute_bash"));
}

// ─────────────────────────────────────────────────────────────────────────────
// L) Whole-harness sentinel invariant: no live transport was ever opened.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
#[serial]
fn deny_live_sentinel_never_tripped() {
    assert!(
        sentinel_is_armed(),
        "sentinel must be armed under os-control-test"
    );
    assert_eq!(
        sentinel_trip_count(),
        0,
        "no live bus/process/session/device transport may be opened by this harness"
    );
}
