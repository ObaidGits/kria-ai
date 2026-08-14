//! The governed-call bundle: the handoff from the agent's admission decision to
//! a canonical OS tool handler (design §4; OSC-001, OSC-007, OSC-008).
//!
//! # Why this type exists
//!
//! `OsControlRuntime::run_mutation` needs five separately-earned artifacts: an
//! observation context, an [`ExecutionGrant`], the held
//! [`AcquiredResourceLeaseSet`], a durable [`AuditAdmissionToken`], and the live
//! [`SealBinding`]. Those are produced in the agent layer (policy gate → grant,
//! coordinator → leases, audit store → admission) but a tool handler used to
//! receive none of them, so no handler could perform a governed mutation.
//!
//! This bundle is that missing handoff. It is deliberately a *separate* type
//! rather than loose fields on the tool context, so the admission material stays
//! visible as one governed unit and is only ever attached for canonical native-OS
//! actions.
//!
//! # What it does not do
//!
//! It grants nothing by itself. Holding one means "this action was admitted and
//! its resources are held"; the runtime still re-validates the grant against the
//! live binding and seals its own mutation permit internally.

use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Instant;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::agent::resource_lease::ResourceRequirement;
use crate::agent::turn_memory::ExecutionTarget;
use crate::os_control::audit::{AdmissionRequest, OsAuditStore, RequestSensitivity};
use crate::os_control::context::{
    AuditAdmissionToken, ExecutionGrant, HostExecutionContext, RedactionPolicy, SessionContext,
};
use crate::os_control::contract::{ActionId, CorrelationId, Digest, SessionId, SnapshotRevision};
use crate::os_control::error::OsControlError;
use crate::os_control::resource::{
    AcquiredResourceLeaseSet, OsLeaseContext, OsResourceCoordinator,
};
use crate::os_control::runtime::SealBinding;
use crate::safety::RiskLevel;

// ── Single process-wide authorities ─────────────────────────────────────────
// One machine has one audit ledger and one resource arbiter. Making them
// process-global (rather than threading them through every executor) keeps that
// single-authority property true by construction: two stores would mean two
// disagreeing ledgers, and two arbiters would not exclude each other.

static AUDIT_STORE: OnceLock<OsAuditStore> = OnceLock::new();

/// Install the **durable** audit store for this process.
///
/// Call once from the desktop/server startup root with a file-backed connection.
/// Returns `false` if a store was already installed (the first one wins — an
/// audit ledger must never be swapped underneath in-flight actions).
pub fn init_audit_store(conn: rusqlite::Connection) -> bool {
    AUDIT_STORE.set(OsAuditStore::new(conn)).is_ok()
}

/// The process audit store.
///
/// Falls back to an in-memory store when [`init_audit_store`] was never called,
/// so tests and dev builds work — but that fallback is **not durable**: records
/// are lost on exit, so interrupted actions cannot be reconciled after a restart.
/// It warns once so an unconfigured production build is visible in the logs.
pub fn audit_store() -> &'static OsAuditStore {
    AUDIT_STORE.get_or_init(|| {
        tracing::warn!(
            target: "authority_trace",
            "no durable OS audit store installed; using a NON-durable in-memory ledger \
             (interrupted actions cannot be reconciled after restart)"
        );
        OsAuditStore::open_in_memory()
    })
}

/// The process resource coordinator is deliberately **not** a global here.
///
/// An `OsResourceCoordinator` must wrap the *same* [`ResourceLeaseManager`] the
/// calling executor uses. A second arbiter with its own manager would not exclude
/// the first, so a lease held by the agent would silently fail to block an OS
/// action over the same resource. Callers construct
/// `OsResourceCoordinator::new(their_manager.clone())` instead.
fn _coordinator_is_not_global() {}


/// Everything a canonical OS handler needs to perform one governed action.
///
/// A **mutation** carries all five artifacts. A **read** carries only the
/// admission and the observation context: `grant` and `leases` are `None`, which
/// is why [`execute_governed_mutation`] refuses it rather than silently mutating
/// without a permit.
pub struct OsGovernedCall {
    admission: AuditAdmissionToken,
    grant: Option<ExecutionGrant>,
    leases: Option<AcquiredResourceLeaseSet>,
    observation: HostExecutionContext,
    session_id: String,
    action: String,
    params: Value,
    target: ExecutionTarget,
    requirements: Vec<ResourceRequirement>,
    snapshot_revision: SnapshotRevision,
}

/// The inputs the agent layer already has when it decides to run an OS action.
pub struct OsCallRequest<'a> {
    /// Live user session id.
    pub session_id: &'a str,
    /// Correlation spanning the whole request.
    pub correlation_id: CorrelationId,
    /// This action's identity within the correlation.
    pub action_id: ActionId,
    /// The canonical tool name (must be a frozen OS capability).
    pub action: &'a str,
    /// The canonical parameters.
    pub params: &'a Value,
    /// Resolved execution target — must be `Host` for an OS effect.
    pub target: ExecutionTarget,
    /// Risk the policy engine assigned.
    pub risk: RiskLevel,
    /// Exclusive resource requirements the gate derived.
    pub requirements: Vec<ResourceRequirement>,
    /// Capability-snapshot revision the decision was made against.
    pub snapshot_revision: SnapshotRevision,
    /// Cooperative cancellation for the whole call.
    pub cancellation: CancellationToken,
    /// Observation deadline.
    pub deadline: Instant,
    /// Redaction policy for anything surfaced.
    pub redaction: RedactionPolicy,
    /// The capability snapshot the host was probed against, when the composition
    /// root probed it.
    ///
    /// When present its facts and revision become authoritative for this action's
    /// session context, so a provider decision is bound to the capability state it
    /// was actually made under. When absent the context keeps environment hints.
    pub snapshot: Option<crate::os_control::capability::CapabilitySnapshot>,
}

impl OsGovernedCall {
    /// Admit the action durably, acquire its write leases in canonical order, and
    /// assemble the observation context.
    ///
    /// Ordering is not incidental: audit admission happens **before** any
    /// observation so an interrupted action is always recoverable from the ledger
    /// (OSC-007), and leases are acquired before the provider is touched so two
    /// actions cannot race the same resource (OSC-008).
    ///
    /// # Errors
    /// * [`OsControlError::AuditUnavailable`] when the audit store is unhealthy —
    ///   a mutation is refused rather than performed unrecorded.
    /// * A lease conflict is surfaced as `ResourceBusy`, so a contended resource
    ///   fails closed instead of interleaving.
    pub async fn admit(
        audit: &OsAuditStore,
        coordinator: &OsResourceCoordinator,
        grant: ExecutionGrant,
        request: OsCallRequest<'_>,
    ) -> Result<Self, OsControlError> {
        let admission = audit.admit_action(&AdmissionRequest {
            session_id: SessionId::new(request.session_id),
            correlation_id: request.correlation_id.clone(),
            action_id: request.action_id.clone(),
            tool_name: request.action.to_string(),
            params: request.params.clone(),
            target_hash: Digest::of_str(request.target.as_str()),
            capability_snapshot_revision: request.snapshot_revision,
            risk: request.risk,
            decision_id: None,
            sensitivity: RequestSensitivity::Mutation,
        })?;

        let leases = coordinator
            .acquire_write_leases(
                &OsLeaseContext {
                    workflow_id: request.session_id.to_string(),
                    stage_id: None,
                    action_hash: Digest::of_str(request.action).as_hex().to_string(),
                },
                request.action,
                request.params,
            )
            .await
            .map_err(|error| OsControlError::ResourceBusy {
                resource: crate::os_control::contract::SafeResource::new(request.action),
                // The conflicting holder, as a redacted label — never raw detail.
                owner: Some(crate::os_control::contract::SafeText::new(
                    error.to_string(),
                )),
            })?;

        let mut session = SessionContext::new(SessionId::new(request.session_id));
        if let Some(snapshot) = request.snapshot.as_ref() {
            session = session.with_snapshot(snapshot);
        }
        let observation = HostExecutionContext::for_action(
            request.correlation_id,
            request.action_id,
            admission.observation_authority(),
            Arc::new(session),
            request.cancellation,
            request.deadline,
            request.redaction,
        );

        Ok(Self {
            admission,
            grant: Some(grant),
            leases: Some(leases),
            observation,
            session_id: request.session_id.to_string(),
            action: request.action.to_string(),
            params: request.params.clone(),
            target: request.target,
            requirements: request.requirements,
            snapshot_revision: request.snapshot_revision,
        })
    }

    /// Admit a **read** and assemble its observation context.
    ///
    /// No grant and no leases: a read cannot mutate, so it earns neither. The
    /// admission is still durable, because design §14.1 records one `Admission`
    /// per logical action before the first provider observation — including reads.
    ///
    /// # Errors
    /// [`OsControlError::AuditUnavailable`] when the store is unhealthy **and**
    /// the read is privacy-sensitive. A plain read may proceed while audit is
    /// degraded, per policy.
    pub fn admit_read(
        audit: &OsAuditStore,
        request: OsCallRequest<'_>,
        privacy_sensitive: bool,
    ) -> Result<Self, OsControlError> {
        let sensitivity = if privacy_sensitive {
            RequestSensitivity::PrivacySensitiveRead
        } else {
            RequestSensitivity::PlainRead
        };
        let admission = audit.admit_action(&AdmissionRequest {
            session_id: SessionId::new(request.session_id),
            correlation_id: request.correlation_id.clone(),
            action_id: request.action_id.clone(),
            tool_name: request.action.to_string(),
            params: request.params.clone(),
            target_hash: Digest::of_str(request.target.as_str()),
            capability_snapshot_revision: request.snapshot_revision,
            risk: request.risk,
            decision_id: None,
            sensitivity,
        })?;

        let mut session = SessionContext::new(SessionId::new(request.session_id));
        if let Some(snapshot) = request.snapshot.as_ref() {
            session = session.with_snapshot(snapshot);
        }
        let observation = HostExecutionContext::for_action(
            request.correlation_id,
            request.action_id,
            admission.observation_authority(),
            Arc::new(session),
            request.cancellation,
            request.deadline,
            request.redaction,
        );

        Ok(Self {
            admission,
            grant: None,
            leases: None,
            observation,
            session_id: request.session_id.to_string(),
            action: request.action.to_string(),
            params: request.params.clone(),
            target: request.target,
            requirements: request.requirements,
            snapshot_revision: request.snapshot_revision,
        })
    }

    /// Whether this call carries a mutation permit (grant + held leases).
    #[must_use]
    pub fn is_mutation(&self) -> bool {
        self.grant.is_some() && self.leases.is_some()
    }

    /// The live binding the runtime re-validates the grant against.
    #[must_use]
    pub fn binding(&self) -> SealBinding<'_> {
        SealBinding {
            session_id: &self.session_id,
            action: &self.action,
            params: &self.params,
            target: self.target,
            resource_requirements: &self.requirements,
            capability_snapshot_revision: self.snapshot_revision,
        }
    }

    /// The observation-only context (read authority).
    #[must_use]
    pub fn observation(&self) -> &HostExecutionContext {
        &self.observation
    }

    /// The execution grant minted by the policy gate, when this is a mutation.
    #[must_use]
    pub fn grant(&self) -> Option<&ExecutionGrant> {
        self.grant.as_ref()
    }

    /// The currently-held write leases, when this is a mutation.
    #[must_use]
    pub fn leases(&self) -> Option<&AcquiredResourceLeaseSet> {
        self.leases.as_ref()
    }

    /// The durable audit admission token.
    #[must_use]
    pub fn admission(&self) -> &AuditAdmissionToken {
        &self.admission
    }

    /// The canonical action name this call was admitted for.
    #[must_use]
    pub fn action(&self) -> &str {
        &self.action
    }

    /// The canonical parameters this call was admitted for.
    #[must_use]
    pub fn params(&self) -> &Value {
        &self.params
    }

    /// The pessimistic completion state to run the mutation with.
    ///
    /// Design §14 step 3: a receipt may only claim `Recorded` once the terminal
    /// record has actually landed. Starting pessimistic means an interrupted
    /// action is never described as durably recorded.
    #[must_use]
    pub fn pending_completion(&self) -> crate::os_control::receipt::AuditCompletionState {
        crate::os_control::receipt::AuditCompletionState::PendingRecovery {
            admission_id: self.admission.admission_id().clone(),
            recovery_key: self.admission.recovery_key().clone(),
        }
    }

    /// Append the action's sole terminal audit record and return the authoritative
    /// completion state (design §14 step 2).
    ///
    /// Call this after `run_mutation` returns. The returned state — not the state
    /// stamped inside the receipt — is authoritative: the receipt was run with
    /// [`Self::pending_completion`], and only a successful append upgrades it to
    /// `Recorded`. A failed append leaves the durable admission detectably
    /// incomplete so `reconcile_incomplete_admissions` can close it later, and the
    /// provider is never re-dispatched.
    ///
    /// The append is idempotent on the admission id, so a replay cannot create a
    /// second terminal.
    pub fn commit_terminal<O>(
        &self,
        audit: &OsAuditStore,
        receipt: &crate::os_control::receipt::MutationReceipt<O>,
        plan: &crate::os_control::runtime::MutationPlan,
    ) -> crate::os_control::receipt::AuditCompletionState {
        let summary = receipt.safe_summary();
        let terminal = crate::os_control::audit::TerminalRecord {
            lifecycle: summary.lifecycle(),
            provider: plan.provider.clone(),
            before_digest: summary.before_digest(),
            after_digest: summary.after_digest(),
            // Not yet surfaced by the receipt's safe projection; the provider
            // receipt digest is carried inside the dispatch variants. Threading it
            // out is a follow-up, and `None` is honest rather than invented.
            provider_receipt_digest: None,
            verification_source: None,
            verification_reliability: None,
            rollback_available: receipt.rollback_available(),
            incident_code: summary.incident_codes().first().cloned(),
            duration_ms: plan.latency_ms,
        };
        audit
            .append_terminal(&self.admission, &terminal)
            .completion_state()
    }
}

/// The outcome of one governed mutation: the receipt plus the **authoritative**
/// audit completion state (the receipt's own stamp is provisional).
pub struct GovernedOutcome<O> {
    /// The receipt describing what happened to host state.
    pub receipt: crate::os_control::receipt::MutationReceipt<O>,
    /// The durable audit completion state after the terminal append.
    pub completion: crate::os_control::receipt::AuditCompletionState,
}

impl<O> GovernedOutcome<O> {
    /// Whether the terminal record landed durably.
    #[must_use]
    pub fn durably_recorded(&self) -> bool {
        matches!(
            self.completion,
            crate::os_control::receipt::AuditCompletionState::Recorded { .. }
        )
    }
}

/// Run one governed mutation end to end: admitted observation → sealed permit →
/// apply-once → verify/rollback → durable terminal audit.
///
/// This is the single sequence **every** canonical OS handler performs, so it
/// lives here rather than being re-derived 46 times. A handler's own job is only
/// to parse its input, build the domain request/desired-state/plan, and render the
/// receipt — never to touch admission, leases, or the provider directly.
///
/// # Errors
/// Propagates the frozen pre-mutation errors unchanged (unavailable, policy
/// denied, resource busy, grant invalid, …). An error here means **no host effect
/// occurred**; the runtime proves that before dispatch.
pub async fn execute_governed_mutation<R, O, P>(
    runtime: &crate::os_control::runtime::OsControlRuntime,
    provider: &P,
    call: &OsGovernedCall,
    audit: &OsAuditStore,
    request: &R,
    desired: &O,
    plan: &crate::os_control::runtime::MutationPlan,
) -> Result<GovernedOutcome<O>, OsControlError>
where
    R: Send + Sync,
    O: crate::os_control::NormalizedObservation + Clone + Send + Sync,
    P: crate::os_control::contract::DesiredStateControl<R, O> + ?Sized,
{
    let (Some(grant), Some(leases)) = (call.grant(), call.leases()) else {
        // A read-admitted call has no permit. Refuse rather than mutate.
        return Err(OsControlError::PolicyDenied {
            reason: crate::os_control::contract::SafeText::new(
                "mutation attempted without an execution grant and held leases",
            ),
        });
    };
    let receipt = runtime
        .run_mutation(
            provider,
            call.observation(),
            grant,
            leases,
            call.admission(),
            &call.binding(),
            request,
            desired,
            plan,
            call.pending_completion(),
        )
        .await?;
    let completion = call.commit_terminal(audit, &receipt, plan);
    Ok(GovernedOutcome {
        receipt,
        completion,
    })
}

/// Run one governed **read**: an admitted observation through the domain provider.
///
/// Reads never seal a mutation permit and never dispatch a command, so there is
/// no receipt and no terminal audit record beyond the admission. A parse or
/// transport ambiguity surfaces as an error — never as a fabricated state.
///
/// # Errors
/// Propagates the provider's frozen error unchanged (unavailable, timed out,
/// protocol, …).
pub async fn execute_governed_read<R, O, P>(
    call: &OsGovernedCall,
    provider: &P,
    request: &R,
) -> Result<O, OsControlError>
where
    R: Send + Sync,
    O: crate::os_control::NormalizedObservation + Clone + Send + Sync,
    P: crate::os_control::contract::DesiredStateControl<R, O> + ?Sized,
{
    provider.observe(call.observation(), request).await
}
