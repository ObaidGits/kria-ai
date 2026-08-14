//! Dispatch facts, terminal receipt states, and safe summaries.
//!
//! linux-os-control-production **Task 1.1**, design §4 (OSC-005, OSC-006).
//!
//! This module encodes the single most important safety property of the
//! OS-control runtime: **every base pre/post-dispatch fact has exactly one
//! representable contract state, and forbidden combinations are unrepresentable
//! in safe Rust.**
//!
//! The type architecture that enforces it:
//!
//! * Providers may only produce one of four narrow *dispatch facts*
//!   ([`ApplyOutcome`] → [`AppliedDispatch`] / [`AcceptedDispatch`] /
//!   [`UncertainDispatch`] / [`PartialDispatch`]). Each has **private fields**
//!   and a validated constructor, so a provider can build a fact but can neither
//!   relabel it nor construct a runtime receipt state.
//! * The runtime receipt state ([`RuntimeReceiptState`]) is **private to this
//!   module** and its narrow constructors accept only the one dispatch fact that
//!   each terminal state may contain. Thus `Verified + Uncertain`,
//!   `Accepted + Applied`, a partial without steps, and rollback failure in a
//!   non-failure state have no constructible type.
//! * [`SafeReceiptSummary::from_receipt`] is the *sole* constructor of a
//!   summary; its fields are private and derived from a validated state, so no
//!   recovery/adapter code can forge independent lifecycle/changed flags.
//!
//! Construction of a [`MutationReceipt`] is `pub(crate)` for Task 1.1 and is
//! tightened to a runtime-only authority witness in Task 1.7
//! ([`crate::os_control::runtime`]); until then no provider/adapter module
//! exists that could call it.

use std::time::SystemTime;

use crate::os_control::contract::{
    BoundedVec, Digest, GrantNonce, NonEmptyBoundedVec, OsEvidenceSource, ProviderId, ReceiptId,
    SafeErrorCode, SafeRevision, SafeStepId, SafeText, SafeWarning, SessionId,
    VerificationReliability,
};
// Task 1.7 tightens the narrow receipt constructors to a runtime-only witness.
// `RuntimeSealAuthority` has a module-private field in
// [`crate::os_control::runtime`], so only `OsControlRuntime` can construct one;
// every terminal-state constructor now borrows it, making
// "only runtime can construct a terminal receipt" a compile-time guarantee.
use crate::os_control::runtime::RuntimeSealAuthority;

// ─────────────────────────────────────────────────────────────────────────────
// Narrow dispatch facts (provider outputs) — design §4
// ─────────────────────────────────────────────────────────────────────────────

/// Evidence that the OS accepted an asynchronous / session-ending action.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AcceptanceEvidence {
    /// Redacted description of the acceptance signal (e.g. "logind accepted").
    pub detail: SafeText,
    /// When acceptance was observed.
    #[serde(skip)]
    pub accepted_at: SystemTime,
}

/// Why a dispatched effect is uncertain (design §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UncertainEffectCause {
    /// Provider reported failure *after* dispatch may have started.
    ProviderReportedFailureAfterDispatch,
    /// Transport was lost after dispatch (result unknown).
    TransportLostAfterDispatch,
    /// A deadline elapsed after dispatch.
    TimedOutAfterDispatch,
    /// Cancellation arrived after dispatch.
    CancelledAfterDispatch,
    /// The outcome is inherently unobservable.
    Unobservable,
}

impl UncertainEffectCause {
    /// Closed incident code.
    #[must_use]
    pub fn code(self) -> SafeErrorCode {
        SafeErrorCode::from_static(match self {
            Self::ProviderReportedFailureAfterDispatch => {
                "os_control.incident.provider_reported_failure_after_dispatch"
            }
            Self::TransportLostAfterDispatch => "os_control.incident.transport_lost_after_dispatch",
            Self::TimedOutAfterDispatch => "os_control.incident.timed_out_after_dispatch",
            Self::CancelledAfterDispatch => "os_control.incident.cancelled_after_dispatch",
            Self::Unobservable => "os_control.incident.unobservable",
        })
    }
}

/// Why a multi-step effect is partial (design §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartialEffectCause {
    /// A step reported failure after earlier steps committed.
    StepFailedAfterCommit,
    /// A deadline elapsed mid-sequence.
    TimedOutMidSequence,
    /// Cancellation arrived mid-sequence.
    CancelledMidSequence,
}

impl PartialEffectCause {
    /// Closed incident code.
    #[must_use]
    pub fn code(self) -> SafeErrorCode {
        SafeErrorCode::from_static(match self {
            Self::StepFailedAfterCommit => "os_control.incident.step_failed_after_commit",
            Self::TimedOutMidSequence => "os_control.incident.timed_out_mid_sequence",
            Self::CancelledMidSequence => "os_control.incident.cancelled_mid_sequence",
        })
    }
}

/// The effect was applied and is expected to be verifiable (design §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedDispatch {
    provider_receipt_digest: Option<Digest>,
    warnings: BoundedVec<SafeWarning>,
}

impl AppliedDispatch {
    /// Construct an applied dispatch fact.
    #[must_use]
    pub fn new(provider_receipt_digest: Option<Digest>, warnings: BoundedVec<SafeWarning>) -> Self {
        Self {
            provider_receipt_digest,
            warnings,
        }
    }

    /// The optional opaque provider-receipt digest.
    #[must_use]
    pub fn provider_receipt_digest(&self) -> Option<&Digest> {
        self.provider_receipt_digest.as_ref()
    }

    /// Redacted warnings.
    #[must_use]
    pub fn warnings(&self) -> &[SafeWarning] {
        self.warnings.as_slice()
    }
}

/// The OS accepted an action that terminates or suspends observability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedDispatch {
    provider_receipt_digest: Option<Digest>,
    acceptance: AcceptanceEvidence,
    warnings: BoundedVec<SafeWarning>,
}

impl AcceptedDispatch {
    /// Construct an accepted dispatch fact. Requires acceptance evidence, so an
    /// `Accepted` outcome can never be fabricated without an acceptance signal.
    #[must_use]
    pub fn new(
        provider_receipt_digest: Option<Digest>,
        acceptance: AcceptanceEvidence,
        warnings: BoundedVec<SafeWarning>,
    ) -> Self {
        Self {
            provider_receipt_digest,
            acceptance,
            warnings,
        }
    }

    /// The acceptance evidence.
    #[must_use]
    pub fn acceptance(&self) -> &AcceptanceEvidence {
        &self.acceptance
    }
}

/// The effect may or may not have taken hold (design §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UncertainDispatch {
    provider_receipt_digest: Option<Digest>,
    cause: UncertainEffectCause,
    warnings: BoundedVec<SafeWarning>,
}

impl UncertainDispatch {
    /// Construct an uncertain dispatch fact.
    #[must_use]
    pub fn new(
        provider_receipt_digest: Option<Digest>,
        cause: UncertainEffectCause,
        warnings: BoundedVec<SafeWarning>,
    ) -> Self {
        Self {
            provider_receipt_digest,
            cause,
            warnings,
        }
    }

    /// Why the effect is uncertain.
    #[must_use]
    pub fn cause(&self) -> UncertainEffectCause {
        self.cause
    }
}

/// A multi-step effect completed some steps and failed one (design §4). The
/// completed steps are a [`NonEmptyBoundedVec`], so "partial without steps" is
/// unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialDispatch {
    provider_receipt_digest: Option<Digest>,
    completed_steps: NonEmptyBoundedVec<SafeStepId>,
    failed_step: SafeStepId,
    cause: PartialEffectCause,
    warnings: BoundedVec<SafeWarning>,
}

impl PartialDispatch {
    /// Construct a partial dispatch fact. The completed-steps type guarantees at
    /// least one committed step.
    #[must_use]
    pub fn new(
        provider_receipt_digest: Option<Digest>,
        completed_steps: NonEmptyBoundedVec<SafeStepId>,
        failed_step: SafeStepId,
        cause: PartialEffectCause,
        warnings: BoundedVec<SafeWarning>,
    ) -> Self {
        Self {
            provider_receipt_digest,
            completed_steps,
            failed_step,
            cause,
            warnings,
        }
    }

    /// The committed steps (guaranteed non-empty).
    #[must_use]
    pub fn completed_steps(&self) -> &NonEmptyBoundedVec<SafeStepId> {
        &self.completed_steps
    }

    /// The step that failed.
    #[must_use]
    pub fn failed_step(&self) -> &SafeStepId {
        &self.failed_step
    }

    /// Why the effect is partial.
    #[must_use]
    pub fn cause(&self) -> PartialEffectCause {
        self.cause
    }
}

/// The four — and only four — dispatch facts a provider may return (design §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Applied; expected verifiable.
    Applied(AppliedDispatch),
    /// Accepted; observability terminates.
    Accepted(AcceptedDispatch),
    /// Uncertain effect.
    Uncertain(UncertainDispatch),
    /// Partially applied multi-step effect.
    PartiallyApplied(PartialDispatch),
}

// ─────────────────────────────────────────────────────────────────────────────
// Narrow dispatch wrappers used by terminal states — design §4
// ─────────────────────────────────────────────────────────────────────────────

/// The only dispatch facts that can precede an `Unverified` state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnverifiedDispatch {
    /// Applied but no decisive observation.
    Applied(AppliedDispatch),
    /// Uncertain and no decisive observation.
    Uncertain(UncertainDispatch),
}

/// The only dispatch facts that can precede a `VerificationFailed` state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContradictedDispatch {
    /// Applied but contradicted by fresh evidence.
    Applied(AppliedDispatch),
    /// Uncertain and contradicted by fresh evidence.
    Uncertain(UncertainDispatch),
}

// ─────────────────────────────────────────────────────────────────────────────
// Verification & rollback evidence — design §4, §13
// ─────────────────────────────────────────────────────────────────────────────

/// A redacted, digest-bound normalized observation of domain state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedObservation<O> {
    value: O,
    digest: Digest,
}

impl<O> RedactedObservation<O> {
    /// Wrap an already-redacted observation with its binding digest.
    #[must_use]
    pub fn new(value: O, digest: Digest) -> Self {
        Self { value, digest }
    }

    /// Borrow the redacted value.
    #[must_use]
    pub fn value(&self) -> &O {
        &self.value
    }

    /// The observation digest.
    #[must_use]
    pub fn digest(&self) -> &Digest {
        &self.digest
    }
}

/// Fresh, satisfying postcondition evidence (design §5, §13). A `Verified` or
/// `RolledBack` state can only be built from this, so success without evidence
/// is unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SatisfyingVerification<O> {
    source: OsEvidenceSource,
    reliability: VerificationReliability,
    provider: ProviderId,
    observation: RedactedObservation<O>,
    provider_revision: Option<SafeRevision>,
    observed_at: SystemTime,
    freshness_ms: u64,
}

impl<O> SatisfyingVerification<O> {
    /// Construct satisfying verification evidence.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: OsEvidenceSource,
        reliability: VerificationReliability,
        provider: ProviderId,
        observation: RedactedObservation<O>,
        provider_revision: Option<SafeRevision>,
        observed_at: SystemTime,
        freshness_ms: u64,
    ) -> Self {
        Self {
            source,
            reliability,
            provider,
            observation,
            provider_revision,
            observed_at,
            freshness_ms,
        }
    }

    /// Evidence source rank.
    #[must_use]
    pub fn source(&self) -> OsEvidenceSource {
        self.source
    }

    /// Evidence reliability.
    #[must_use]
    pub fn reliability(&self) -> VerificationReliability {
        self.reliability
    }

    /// Verifying provider.
    #[must_use]
    pub fn provider(&self) -> &ProviderId {
        &self.provider
    }

    /// The satisfying observation.
    #[must_use]
    pub fn observation(&self) -> &RedactedObservation<O> {
        &self.observation
    }

    /// Evidence freshness in milliseconds.
    #[must_use]
    pub fn freshness_ms(&self) -> u64 {
        self.freshness_ms
    }
}

/// Fresh evidence contradicting the desired state (design §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationContradiction {
    expected: Digest,
    observed: Option<Digest>,
    code: SafeErrorCode,
}

impl VerificationContradiction {
    /// Construct a contradiction record.
    #[must_use]
    pub fn new(expected: Digest, observed: Option<Digest>, code: SafeErrorCode) -> Self {
        Self {
            expected,
            observed,
            code,
        }
    }

    /// The incident code.
    #[must_use]
    pub fn code(&self) -> &SafeErrorCode {
        &self.code
    }
}

/// What the provider verify() step concluded (design §4 `VerificationReport`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationReport<O> {
    /// Fresh satisfying evidence.
    Satisfied(SatisfyingVerification<O>),
    /// Fresh contradicting evidence.
    Contradicted(VerificationContradiction),
    /// No decisive evidence available.
    Inconclusive {
        /// Redacted reason.
        reason: SafeText,
    },
}

/// A failed rollback attempt (design §4). Only ever attached to a truthful
/// failure state via [`FailureRollbackState::Failed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackFailure {
    code: SafeErrorCode,
    observed_digest: Option<Digest>,
}

impl RollbackFailure {
    /// Construct a rollback failure.
    #[must_use]
    pub fn new(code: SafeErrorCode, observed_digest: Option<Digest>) -> Self {
        Self {
            code,
            observed_digest,
        }
    }

    /// The incident code.
    #[must_use]
    pub fn code(&self) -> &SafeErrorCode {
        &self.code
    }
}

/// Whether rollback is available for a receipt (design §4). A `Verified` state
/// can only carry this, never a [`RollbackFailure`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollbackAvailability {
    /// Rollback is available; carries the opaque token.
    Available(RollbackToken),
    /// Rollback is not available (no reliable inverse or insufficient prior).
    Unavailable,
}

impl RollbackAvailability {
    /// Whether rollback is advertised.
    #[must_use]
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }
}

/// Rollback disposition for a *failure* state (design §4). Only failure states
/// can express an attempted-and-failed rollback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureRollbackState {
    /// Rollback was not attempted; carries whether it was available.
    NotAttempted(RollbackAvailability),
    /// Rollback was attempted and failed.
    Failed(RollbackFailure),
}

/// The original failure that made a successful rollback eligible (design §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackEligibleFailure {
    dispatch: ContradictedDispatch,
    contradiction: VerificationContradiction,
}

impl RollbackEligibleFailure {
    /// Construct from the contradicted dispatch and its contradiction.
    #[must_use]
    pub fn new(dispatch: ContradictedDispatch, contradiction: VerificationContradiction) -> Self {
        Self {
            dispatch,
            contradiction,
        }
    }
}

/// An opaque, bounded, session-scoped, expiring rollback token (OSC-006 crit.
/// 4). It is excluded from model-visible prose: it carries only opaque digests
/// and identifiers.
///
/// Beyond expiry and session scope (Task 1.1), the token is **action-linked**
/// and **capability-owned** (Task 1.9, OSC-006.4/OSC-006.5): it names the exact
/// canonical action-name digest it can undo and the provider capability that
/// owns that reversible operation, plus the original [`ReceiptId`] it is linked
/// to. The rollback coordinator validates all four bindings *before* invoking
/// any provider rollback, so a mismatched-action, wrong-capability, foreign-
/// session, or expired token performs no compensation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackToken {
    token_id: Digest,
    session_id: SessionId,
    /// Action linkage: the canonical action-name digest this token can undo.
    action_hash: Digest,
    /// Capability ownership: the provider that owns the reversible operation.
    capability: ProviderId,
    /// The original receipt this token is linked to (OSC-006.5).
    linked_receipt: ReceiptId,
    nonce: GrantNonce,
    expires_at: SystemTime,
}

impl RollbackToken {
    /// Construct an opaque, action-linked, capability-owned rollback token.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        token_id: Digest,
        session_id: SessionId,
        action_hash: Digest,
        capability: ProviderId,
        linked_receipt: ReceiptId,
        nonce: GrantNonce,
        expires_at: SystemTime,
    ) -> Self {
        Self {
            token_id,
            session_id,
            action_hash,
            capability,
            linked_receipt,
            nonce,
            expires_at,
        }
    }

    /// The opaque token identity.
    #[must_use]
    pub fn token_id(&self) -> &Digest {
        &self.token_id
    }

    /// The bound session identity.
    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// The action-name digest this token is linked to (action linkage).
    #[must_use]
    pub fn action_hash(&self) -> &Digest {
        &self.action_hash
    }

    /// The provider capability that owns the reversible operation.
    #[must_use]
    pub fn capability(&self) -> &ProviderId {
        &self.capability
    }

    /// The original receipt this token can undo (OSC-006.5 linkage).
    #[must_use]
    pub fn linked_receipt(&self) -> &ReceiptId {
        &self.linked_receipt
    }

    /// Whether the token has expired relative to `now`.
    #[must_use]
    pub fn is_expired(&self, now: SystemTime) -> bool {
        now >= self.expires_at
    }

    /// Validate the token's bindings **before** any provider rollback is
    /// invoked (OSC-006.4/.5). Checks, in order: expiry, session scope, action
    /// linkage, then capability ownership. Any failure is a *pre-rollback*
    /// rejection that must perform no compensation.
    pub fn validate(
        &self,
        now: SystemTime,
        session_id: &str,
        action_hash: &Digest,
        capability: &ProviderId,
    ) -> Result<(), RollbackTokenRejection> {
        if self.is_expired(now) {
            return Err(RollbackTokenRejection::Expired);
        }
        if self.session_id.as_str() != session_id {
            return Err(RollbackTokenRejection::SessionMismatch);
        }
        if &self.action_hash != action_hash {
            return Err(RollbackTokenRejection::ActionMismatch);
        }
        if &self.capability != capability {
            return Err(RollbackTokenRejection::CapabilityMismatch);
        }
        Ok(())
    }
}

/// Why a [`RollbackToken`] was rejected before any provider rollback ran
/// (Task 1.9, OSC-006.4/.5). Each variant is a proven-no-compensation
/// pre-rollback error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackTokenRejection {
    /// The token's bounded lifetime elapsed.
    Expired,
    /// The token belongs to a different login session.
    SessionMismatch,
    /// The token is not linked to the action being rolled back.
    ActionMismatch,
    /// The token is owned by a different provider capability.
    CapabilityMismatch,
}

impl RollbackTokenRejection {
    /// Stable, redaction-safe code for traces and errors.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Expired => "os_control.rollback.token_expired",
            Self::SessionMismatch => "os_control.rollback.token_session_mismatch",
            Self::ActionMismatch => "os_control.rollback.token_action_mismatch",
            Self::CapabilityMismatch => "os_control.rollback.token_capability_mismatch",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Audit completion state — design §4, §14
// ─────────────────────────────────────────────────────────────────────────────

/// Terminal-audit completion state (design §4, §14). A terminal-audit append
/// interruption is representable *only* as [`Self::PendingRecovery`]; no durable
/// incident id is invented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditCompletionState {
    /// The terminal record was durably appended.
    Recorded {
        /// Durable terminal record id.
        record_id: crate::os_control::contract::AuditRecordId,
    },
    /// Terminal append was interrupted; recovery is pending (fail closed).
    PendingRecovery {
        /// The detectably-incomplete admission id.
        admission_id: crate::os_control::contract::AuditAdmissionId,
        /// The idempotent recovery key committed before dispatch.
        recovery_key: crate::os_control::contract::AuditRecoveryKey,
    },
}

impl AuditCompletionState {
    /// Stable token for the additive result envelope (design §4).
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Recorded { .. } => "recorded",
            Self::PendingRecovery { .. } => "pending_recovery",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Terminal receipt states — design §4
// ─────────────────────────────────────────────────────────────────────────────

/// The seven terminal lifecycle labels (design §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionLifecycle {
    /// Desired state already held; no mutation.
    Unchanged,
    /// Applied and verified by fresh evidence.
    Verified,
    /// Accepted (session-ending / async).
    Accepted,
    /// Applied/uncertain but not decisively observed.
    Unverified,
    /// Fresh evidence contradicted the desired state.
    VerificationFailed,
    /// Failure was rolled back successfully.
    RolledBack,
    /// Multi-step residue left in place.
    PartiallyApplied,
}

impl ActionLifecycle {
    /// Stable token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::Verified => "verified",
            Self::Accepted => "accepted",
            Self::Unverified => "unverified",
            Self::VerificationFailed => "verification_failed",
            Self::RolledBack => "rolled_back",
            Self::PartiallyApplied => "partially_applied",
        }
    }
}

/// Common receipt metadata shared by every terminal state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptCommon {
    receipt_id: ReceiptId,
    action_hash: Digest,
    target_hash: Digest,
    provider: ProviderId,
    latency_ms: u64,
}

impl ReceiptCommon {
    /// Construct receipt-common metadata.
    #[must_use]
    pub fn new(
        receipt_id: ReceiptId,
        action_hash: Digest,
        target_hash: Digest,
        provider: ProviderId,
        latency_ms: u64,
    ) -> Self {
        Self {
            receipt_id,
            action_hash,
            target_hash,
            provider,
            latency_ms,
        }
    }
}

/// The private terminal receipt state. **Private to `os_control::receipt`** —
/// provider and adapter modules cannot name or construct it. Each variant holds
/// exactly the narrow evidence its lifecycle allows, so forbidden combinations
/// (`Verified + Uncertain`, `Accepted + Applied`, partial without steps,
/// rollback failure in a non-failure state) have no constructible type.
// The narrow constructors below are the runtime's (Task 1.7) sole path to a
// receipt; under a plain `cargo check` (no test target, no runtime yet) they
// have no caller, which is expected until 1.7 wires the sealing runtime.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeReceiptState<O> {
    Unchanged {
        observation: RedactedObservation<O>,
    },
    Verified {
        before: RedactedObservation<O>,
        after: RedactedObservation<O>,
        apply: AppliedDispatch,
        verification: SatisfyingVerification<O>,
        rollback: RollbackAvailability,
    },
    Accepted {
        before: Option<RedactedObservation<O>>,
        apply: AcceptedDispatch,
    },
    Unverified {
        before: Option<RedactedObservation<O>>,
        after: Option<RedactedObservation<O>>,
        dispatch: UnverifiedDispatch,
        cause: UnverifiedCause,
        rollback: FailureRollbackState,
    },
    VerificationFailed {
        before: RedactedObservation<O>,
        after: RedactedObservation<O>,
        dispatch: ContradictedDispatch,
        contradiction: VerificationContradiction,
        rollback: FailureRollbackState,
    },
    RolledBack {
        before: RedactedObservation<O>,
        failed_after: Option<RedactedObservation<O>>,
        original: RollbackEligibleFailure,
        rollback_verification: SatisfyingVerification<O>,
    },
    PartiallyApplied {
        before: Option<RedactedObservation<O>>,
        after: Option<RedactedObservation<O>>,
        apply: PartialDispatch,
        rollback: FailureRollbackState,
    },
}

/// Why an `Unverified` state could not reach a decisive observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnverifiedCause {
    /// No decisive observation was available in the deadline.
    NoDecisiveObservation,
    /// Verification observation source was unavailable.
    ObservationUnavailable,
}

impl UnverifiedCause {
    /// Closed incident code.
    #[must_use]
    pub fn code(self) -> SafeErrorCode {
        SafeErrorCode::from_static(match self {
            Self::NoDecisiveObservation => "os_control.incident.no_decisive_observation",
            Self::ObservationUnavailable => "os_control.incident.observation_unavailable",
        })
    }
}

/// A terminal OS-action receipt: common metadata + a private validated state +
/// its audit completion state (design §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationReceipt<O> {
    common: ReceiptCommon,
    state: RuntimeReceiptState<O>,
    audit_completion: AuditCompletionState,
}

/// `Result` alias for runtime/tool-facade mutation outputs (design §4).
pub type MutationResult<O> = Result<MutationReceipt<O>, crate::os_control::error::OsControlError>;

#[allow(dead_code)] // narrow constructors are runtime-only (Task 1.7); see note above.
impl<O> MutationReceipt<O> {
    // ── Narrow constructors (runtime-only authority; `pub(crate)` for 1.1,
    //    tightened to a runtime witness in Task 1.7). Each accepts only the one
    //    dispatch fact its terminal state may contain. ──────────────────────

    /// Build an `Unchanged` receipt (desired state already held; no mutation).
    #[must_use]
    pub(crate) fn unchanged(
        _authority: &RuntimeSealAuthority,
        common: ReceiptCommon,
        observation: RedactedObservation<O>,
        audit_completion: AuditCompletionState,
    ) -> Self {
        Self {
            common,
            state: RuntimeReceiptState::Unchanged { observation },
            audit_completion,
        }
    }

    /// Build a `Verified` receipt. Accepts only [`AppliedDispatch`] and only a
    /// [`RollbackAvailability`] (never a rollback *failure*).
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn verified(
        _authority: &RuntimeSealAuthority,
        common: ReceiptCommon,
        before: RedactedObservation<O>,
        after: RedactedObservation<O>,
        apply: AppliedDispatch,
        verification: SatisfyingVerification<O>,
        rollback: RollbackAvailability,
        audit_completion: AuditCompletionState,
    ) -> Self {
        Self {
            common,
            state: RuntimeReceiptState::Verified {
                before,
                after,
                apply,
                verification,
                rollback,
            },
            audit_completion,
        }
    }

    /// Build an `Accepted` receipt. Accepts only [`AcceptedDispatch`].
    #[must_use]
    pub(crate) fn accepted(
        _authority: &RuntimeSealAuthority,
        common: ReceiptCommon,
        before: Option<RedactedObservation<O>>,
        apply: AcceptedDispatch,
        audit_completion: AuditCompletionState,
    ) -> Self {
        Self {
            common,
            state: RuntimeReceiptState::Accepted { before, apply },
            audit_completion,
        }
    }

    /// Build an `Unverified` receipt from an [`UnverifiedDispatch`].
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn unverified(
        _authority: &RuntimeSealAuthority,
        common: ReceiptCommon,
        before: Option<RedactedObservation<O>>,
        after: Option<RedactedObservation<O>>,
        dispatch: UnverifiedDispatch,
        cause: UnverifiedCause,
        rollback: FailureRollbackState,
        audit_completion: AuditCompletionState,
    ) -> Self {
        Self {
            common,
            state: RuntimeReceiptState::Unverified {
                before,
                after,
                dispatch,
                cause,
                rollback,
            },
            audit_completion,
        }
    }

    /// Build a `VerificationFailed` receipt from a [`ContradictedDispatch`].
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn verification_failed(
        _authority: &RuntimeSealAuthority,
        common: ReceiptCommon,
        before: RedactedObservation<O>,
        after: RedactedObservation<O>,
        dispatch: ContradictedDispatch,
        contradiction: VerificationContradiction,
        rollback: FailureRollbackState,
        audit_completion: AuditCompletionState,
    ) -> Self {
        Self {
            common,
            state: RuntimeReceiptState::VerificationFailed {
                before,
                after,
                dispatch,
                contradiction,
                rollback,
            },
            audit_completion,
        }
    }

    /// Build a `RolledBack` receipt. Requires satisfying rollback verification,
    /// so a rollback cannot claim success without fresh evidence.
    #[must_use]
    pub(crate) fn rolled_back(
        _authority: &RuntimeSealAuthority,
        common: ReceiptCommon,
        before: RedactedObservation<O>,
        failed_after: Option<RedactedObservation<O>>,
        original: RollbackEligibleFailure,
        rollback_verification: SatisfyingVerification<O>,
        audit_completion: AuditCompletionState,
    ) -> Self {
        Self {
            common,
            state: RuntimeReceiptState::RolledBack {
                before,
                failed_after,
                original,
                rollback_verification,
            },
            audit_completion,
        }
    }

    /// Build a `PartiallyApplied` receipt. Accepts only [`PartialDispatch`],
    /// whose completed steps are guaranteed non-empty.
    #[must_use]
    pub(crate) fn partially_applied(
        _authority: &RuntimeSealAuthority,
        common: ReceiptCommon,
        before: Option<RedactedObservation<O>>,
        after: Option<RedactedObservation<O>>,
        apply: PartialDispatch,
        rollback: FailureRollbackState,
        audit_completion: AuditCompletionState,
    ) -> Self {
        Self {
            common,
            state: RuntimeReceiptState::PartiallyApplied {
                before,
                after,
                apply,
                rollback,
            },
            audit_completion,
        }
    }

    // ── Derived read-only accessors (design §4) ─────────────────────────────

    /// The terminal lifecycle label, derived from the validated state.
    #[must_use]
    pub fn lifecycle(&self) -> ActionLifecycle {
        match &self.state {
            RuntimeReceiptState::Unchanged { .. } => ActionLifecycle::Unchanged,
            RuntimeReceiptState::Verified { .. } => ActionLifecycle::Verified,
            RuntimeReceiptState::Accepted { .. } => ActionLifecycle::Accepted,
            RuntimeReceiptState::Unverified { .. } => ActionLifecycle::Unverified,
            RuntimeReceiptState::VerificationFailed { .. } => ActionLifecycle::VerificationFailed,
            RuntimeReceiptState::RolledBack { .. } => ActionLifecycle::RolledBack,
            RuntimeReceiptState::PartiallyApplied { .. } => ActionLifecycle::PartiallyApplied,
        }
    }

    /// Whether the host state changed relative to before. `Unchanged` and a
    /// successful `RolledBack` are net-unchanged.
    #[must_use]
    pub fn changed(&self) -> bool {
        !matches!(
            self.state,
            RuntimeReceiptState::Unchanged { .. } | RuntimeReceiptState::RolledBack { .. }
        )
    }

    /// The satisfying verification evidence, when the state carries it.
    #[must_use]
    pub fn verification(&self) -> Option<&SatisfyingVerification<O>> {
        match &self.state {
            RuntimeReceiptState::Verified { verification, .. } => Some(verification),
            RuntimeReceiptState::RolledBack {
                rollback_verification,
                ..
            } => Some(rollback_verification),
            _ => None,
        }
    }

    /// Whether rollback is currently advertised as available.
    #[must_use]
    pub fn rollback_available(&self) -> bool {
        match &self.state {
            RuntimeReceiptState::Verified { rollback, .. } => rollback.is_available(),
            RuntimeReceiptState::Unverified { rollback, .. }
            | RuntimeReceiptState::VerificationFailed { rollback, .. }
            | RuntimeReceiptState::PartiallyApplied { rollback, .. } => {
                matches!(rollback, FailureRollbackState::NotAttempted(a) if a.is_available())
            }
            _ => false,
        }
    }

    /// The audit completion state.
    #[must_use]
    pub fn audit_completion(&self) -> &AuditCompletionState {
        &self.audit_completion
    }

    /// The receipt identity.
    #[must_use]
    pub fn receipt_id(&self) -> &ReceiptId {
        &self.common.receipt_id
    }

    /// The before-state digest, when the state carries one.
    #[must_use]
    fn before_digest(&self) -> Option<Digest> {
        match &self.state {
            RuntimeReceiptState::Unchanged { observation } => Some(observation.digest().clone()),
            RuntimeReceiptState::Verified { before, .. }
            | RuntimeReceiptState::VerificationFailed { before, .. }
            | RuntimeReceiptState::RolledBack { before, .. } => Some(before.digest().clone()),
            RuntimeReceiptState::Accepted { before, .. }
            | RuntimeReceiptState::Unverified { before, .. }
            | RuntimeReceiptState::PartiallyApplied { before, .. } => {
                before.as_ref().map(|o| o.digest().clone())
            }
        }
    }

    /// The after-state digest, when the state carries one.
    #[must_use]
    fn after_digest(&self) -> Option<Digest> {
        match &self.state {
            RuntimeReceiptState::Unchanged { observation } => Some(observation.digest().clone()),
            RuntimeReceiptState::Verified { after, .. }
            | RuntimeReceiptState::VerificationFailed { after, .. } => Some(after.digest().clone()),
            RuntimeReceiptState::Unverified { after, .. }
            | RuntimeReceiptState::PartiallyApplied { after, .. } => {
                after.as_ref().map(|o| o.digest().clone())
            }
            RuntimeReceiptState::RolledBack { failed_after, .. } => {
                failed_after.as_ref().map(|o| o.digest().clone())
            }
            RuntimeReceiptState::Accepted { .. } => None,
        }
    }

    /// Collect the closed incident codes carried by the state.
    fn incident_codes(&self) -> BoundedVec<SafeErrorCode> {
        let mut codes: BoundedVec<SafeErrorCode> = BoundedVec::with_cap(8);
        match &self.state {
            RuntimeReceiptState::Unverified {
                dispatch, cause, ..
            } => {
                if let UnverifiedDispatch::Uncertain(u) = dispatch {
                    codes.try_push(u.cause().code());
                }
                codes.try_push(cause.code());
            }
            RuntimeReceiptState::VerificationFailed {
                dispatch,
                contradiction,
                rollback,
                ..
            } => {
                if let ContradictedDispatch::Uncertain(u) = dispatch {
                    codes.try_push(u.cause().code());
                }
                codes.try_push(contradiction.code().clone());
                if let FailureRollbackState::Failed(f) = rollback {
                    codes.try_push(f.code().clone());
                }
            }
            RuntimeReceiptState::PartiallyApplied {
                apply, rollback, ..
            } => {
                codes.try_push(apply.cause().code());
                if let FailureRollbackState::Failed(f) = rollback {
                    codes.try_push(f.code().clone());
                }
            }
            RuntimeReceiptState::RolledBack { original, .. } => {
                codes.try_push(original.contradiction.code().clone());
            }
            _ => {}
        }
        codes
    }

    /// Derive the safe, serializable summary (sole path to a summary).
    #[must_use]
    pub fn safe_summary(&self) -> SafeReceiptSummary {
        SafeReceiptSummary::from_receipt(self)
    }
}

/// A safe, serializable projection of a receipt (design §4). Every field is
/// private and derived by [`Self::from_receipt`] from a validated state, so no
/// caller can forge independent lifecycle/changed/digest flags.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SafeReceiptSummary {
    receipt_id: ReceiptId,
    action_hash: Digest,
    target_hash: Digest,
    provider: ProviderId,
    lifecycle: ActionLifecycle,
    changed: bool,
    before_digest: Option<Digest>,
    after_digest: Option<Digest>,
    incident_codes: BoundedVec<SafeErrorCode>,
}

impl SafeReceiptSummary {
    /// The **sole** constructor: derive every field from a validated receipt.
    #[must_use]
    pub fn from_receipt<O>(receipt: &MutationReceipt<O>) -> Self {
        Self {
            receipt_id: receipt.common.receipt_id.clone(),
            action_hash: receipt.common.action_hash.clone(),
            target_hash: receipt.common.target_hash.clone(),
            provider: receipt.common.provider.clone(),
            lifecycle: receipt.lifecycle(),
            changed: receipt.changed(),
            before_digest: receipt.before_digest(),
            after_digest: receipt.after_digest(),
            incident_codes: receipt.incident_codes(),
        }
    }

    /// The lifecycle label.
    #[must_use]
    pub fn lifecycle(&self) -> ActionLifecycle {
        self.lifecycle
    }

    /// Whether host state changed.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// The closed incident codes.
    #[must_use]
    /// The redacted before-state digest, when the receipt carries one.
    ///
    /// Exposed so the governed-call layer can build the durable terminal audit
    /// record (design §14, record field `before_digest`) without reaching into
    /// the receipt's private state.
    pub fn before_digest(&self) -> Option<Digest> {
        self.before_digest.clone()
    }

    /// The redacted after-state digest, when the receipt carries one.
    #[must_use]
    pub fn after_digest(&self) -> Option<Digest> {
        self.after_digest.clone()
    }

    /// The provider that produced the outcome.
    #[must_use]
    pub fn provider(&self) -> &ProviderId {
        &self.provider
    }

    /// The closed incident/error codes, if the terminal is an incident.
    pub fn incident_codes(&self) -> &[SafeErrorCode] {
        self.incident_codes.as_slice()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rollback coordinator outcomes — design §4, §13.1, §14.5 (Task 1.9)
// ─────────────────────────────────────────────────────────────────────────────

/// The outcome of an explicit, separately-audited rollback logical action
/// (OSC-006.5, OSC-028.7). A rollback is its own action linked to the original
/// receipt; its result never overwrites the original receipt's outcome, so the
/// original failure and any rollback failure are preserved separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollbackOutcome<O> {
    /// The prior state was restored and verified with fresh satisfying evidence.
    Restored {
        /// The satisfying observation of the restored prior state.
        observation: RedactedObservation<O>,
        /// Fresh satisfying verification of the restore.
        verification: SatisfyingVerification<O>,
    },
    /// The rollback dispatched but the restore could not be decisively verified.
    Unverified {
        /// Why no decisive restore observation was reached.
        cause: UnverifiedCause,
    },
    /// The rollback attempt itself failed, was uncertain, or was contradicted;
    /// no restore is claimed.
    Failed(RollbackFailure),
}

impl<O> RollbackOutcome<O> {
    /// Whether the prior state was verifiably restored.
    #[must_use]
    pub fn restored(&self) -> bool {
        matches!(self, Self::Restored { .. })
    }
}

/// A terminal receipt for a rollback logical action (Task 1.9, OSC-006.5). It
/// carries its own opaque receipt id and the [`ReceiptId`] of the original
/// action it undoes, so audit and results link the two without merging them.
/// Constructed only by the runtime (via a [`RuntimeSealAuthority`] witness).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackReceipt<O> {
    rollback_receipt_id: ReceiptId,
    linked_receipt: ReceiptId,
    provider: ProviderId,
    outcome: RollbackOutcome<O>,
    audit_completion: AuditCompletionState,
    latency_ms: u64,
}

impl<O> RollbackReceipt<O> {
    /// Build a rollback receipt from a classified outcome. Runtime-only: the
    /// borrowed [`RuntimeSealAuthority`] witness cannot be forged outside
    /// [`crate::os_control::runtime`].
    #[must_use]
    pub(crate) fn new(
        _authority: &RuntimeSealAuthority,
        rollback_receipt_id: ReceiptId,
        linked_receipt: ReceiptId,
        provider: ProviderId,
        outcome: RollbackOutcome<O>,
        audit_completion: AuditCompletionState,
        latency_ms: u64,
    ) -> Self {
        Self {
            rollback_receipt_id,
            linked_receipt,
            provider,
            outcome,
            audit_completion,
            latency_ms,
        }
    }

    /// The rollback action's own receipt identity.
    #[must_use]
    pub fn rollback_receipt_id(&self) -> &ReceiptId {
        &self.rollback_receipt_id
    }

    /// The original receipt this rollback is linked to (OSC-006.5).
    #[must_use]
    pub fn linked_receipt(&self) -> &ReceiptId {
        &self.linked_receipt
    }

    /// The classified rollback outcome.
    #[must_use]
    pub fn outcome(&self) -> &RollbackOutcome<O> {
        &self.outcome
    }

    /// Whether the prior state was verifiably restored.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.outcome.restored()
    }

    /// The audit completion state of the rollback action.
    #[must_use]
    pub fn audit_completion(&self) -> &AuditCompletionState {
        &self.audit_completion
    }

    /// The satisfying verification evidence, when the restore was verified.
    #[must_use]
    pub fn verification(&self) -> Option<&SatisfyingVerification<O>> {
        match &self.outcome {
            RollbackOutcome::Restored { verification, .. } => Some(verification),
            _ => None,
        }
    }

    /// A safe, serializable projection of the rollback receipt.
    #[must_use]
    pub fn safe_summary(&self) -> SafeRollbackSummary {
        let (status, incident): (&'static str, Option<SafeErrorCode>) = match &self.outcome {
            RollbackOutcome::Restored { .. } => ("restored", None),
            RollbackOutcome::Unverified { cause } => ("unverified", Some(cause.code())),
            RollbackOutcome::Failed(f) => ("failed", Some(f.code().clone())),
        };
        SafeRollbackSummary {
            rollback_receipt_id: self.rollback_receipt_id.clone(),
            linked_receipt: self.linked_receipt.clone(),
            provider: self.provider.clone(),
            status,
            restored: self.succeeded(),
            incident_code: incident,
        }
    }
}

/// A safe, serializable projection of a [`RollbackReceipt`] (Task 1.9). Every
/// field is private and derived from a validated outcome, so no caller can forge
/// a `restored` flag independent of the real outcome.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SafeRollbackSummary {
    rollback_receipt_id: ReceiptId,
    linked_receipt: ReceiptId,
    provider: ProviderId,
    status: &'static str,
    restored: bool,
    incident_code: Option<SafeErrorCode>,
}

impl SafeRollbackSummary {
    /// The stable rollback status token.
    #[must_use]
    pub fn status(&self) -> &'static str {
        self.status
    }

    /// Whether the prior state was restored.
    #[must_use]
    pub fn restored(&self) -> bool {
        self.restored
    }

    /// The original receipt this rollback is linked to.
    #[must_use]
    pub fn linked_receipt(&self) -> &ReceiptId {
        &self.linked_receipt
    }

    /// The rollback incident code, when the rollback did not restore.
    #[must_use]
    pub fn incident_code(&self) -> Option<&SafeErrorCode> {
        self.incident_code.as_ref()
    }
}

/// A precise report of reverse-order multi-step compensation (Task 1.9,
/// OSC-006.7/OSC-028). It records which completed steps were compensated (in the
/// reverse order they were applied), which were left in place because they are
/// not declared reversible, and — if compensation stopped early — the first step
/// whose compensation failed. This is how partial completion is "reported
/// precisely".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CompensationReport {
    compensated: BoundedVec<SafeStepId>,
    skipped_irreversible: BoundedVec<SafeStepId>,
    failed_step: Option<SafeStepId>,
    failure_code: Option<SafeErrorCode>,
}

impl CompensationReport {
    /// Construct an empty report sized for the completed-step bound.
    #[must_use]
    pub(crate) fn with_cap(cap: usize) -> Self {
        Self {
            compensated: BoundedVec::with_cap(cap),
            skipped_irreversible: BoundedVec::with_cap(cap),
            failed_step: None,
            failure_code: None,
        }
    }

    pub(crate) fn record_compensated(&mut self, step: SafeStepId) {
        self.compensated.try_push(step);
    }

    pub(crate) fn record_skipped(&mut self, step: SafeStepId) {
        self.skipped_irreversible.try_push(step);
    }

    pub(crate) fn record_failure(&mut self, step: SafeStepId, code: SafeErrorCode) {
        self.failed_step = Some(step);
        self.failure_code = Some(code);
    }

    /// The steps compensated, in the reverse order they were applied.
    #[must_use]
    pub fn compensated(&self) -> &[SafeStepId] {
        self.compensated.as_slice()
    }

    /// The completed steps left in place because they are not declared reversible.
    #[must_use]
    pub fn skipped_irreversible(&self) -> &[SafeStepId] {
        self.skipped_irreversible.as_slice()
    }

    /// The first step whose compensation failed, if compensation stopped early.
    #[must_use]
    pub fn failed_step(&self) -> Option<&SafeStepId> {
        self.failed_step.as_ref()
    }

    /// Whether every reversible step was compensated with no failure.
    #[must_use]
    pub fn fully_compensated(&self) -> bool {
        self.failed_step.is_none()
    }
}

// Compile-time proof of thread-safety across the receipt surface (design §18).
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ApplyOutcome>();
    assert_send_sync::<SafeReceiptSummary>();
    assert_send_sync::<AuditCompletionState>();
    assert_send_sync::<MutationReceipt<u32>>();
    assert_send_sync::<RollbackToken>();
    assert_send_sync::<RollbackReceipt<u32>>();
    assert_send_sync::<SafeRollbackSummary>();
    assert_send_sync::<CompensationReport>();
};

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;
    use crate::os_control::contract::{AuditRecordId, SessionId};
    use std::time::{Duration, SystemTime};

    fn auth() -> RuntimeSealAuthority {
        RuntimeSealAuthority::for_test()
    }

    fn obs(tag: &str) -> RedactedObservation<u32> {
        RedactedObservation::new(0u32, Digest::of_str(tag))
    }

    fn common() -> ReceiptCommon {
        ReceiptCommon::new(
            ReceiptId::new("r1"),
            Digest::of_str("action"),
            Digest::of_str("target"),
            ProviderId::new("fake"),
            12,
        )
    }

    fn recorded() -> AuditCompletionState {
        AuditCompletionState::Recorded {
            record_id: AuditRecordId::new("rec-1"),
        }
    }

    fn satisfying() -> SatisfyingVerification<u32> {
        SatisfyingVerification::new(
            OsEvidenceSource::AuthoritativeServiceState,
            VerificationReliability::Strong,
            ProviderId::new("fake"),
            obs("after"),
            None,
            SystemTime::now(),
            10,
        )
    }

    #[test]
    fn unchanged_receipt_reports_no_change() {
        let r = MutationReceipt::unchanged(&auth(), common(), obs("state"), recorded());
        assert_eq!(r.lifecycle(), ActionLifecycle::Unchanged);
        assert!(!r.changed());
        assert!(!r.rollback_available());
        let s = r.safe_summary();
        assert_eq!(s.lifecycle(), ActionLifecycle::Unchanged);
        assert!(s.before_digest.is_some());
    }

    #[test]
    fn verified_requires_applied_and_carries_only_availability() {
        let r = MutationReceipt::verified(
            &auth(),
            common(),
            obs("before"),
            obs("after"),
            AppliedDispatch::new(None, BoundedVec::new()),
            satisfying(),
            RollbackAvailability::Unavailable,
            recorded(),
        );
        assert_eq!(r.lifecycle(), ActionLifecycle::Verified);
        assert!(r.changed());
        assert!(r.verification().is_some());
        assert!(!r.rollback_available());
    }

    #[test]
    fn verified_advertises_rollback_when_available() {
        let token = RollbackToken::new(
            Digest::of_str("tok"),
            SessionId::new("s"),
            Digest::of_str("set_volume"),
            ProviderId::new("pipewire"),
            ReceiptId::new("r-orig"),
            GrantNonce::new("n"),
            SystemTime::now() + Duration::from_secs(60),
        );
        let r = MutationReceipt::verified(
            &auth(),
            common(),
            obs("before"),
            obs("after"),
            AppliedDispatch::new(None, BoundedVec::new()),
            satisfying(),
            RollbackAvailability::Available(token),
            recorded(),
        );
        assert!(r.rollback_available());
    }

    #[test]
    fn accepted_requires_acceptance_evidence() {
        let accepted = AcceptedDispatch::new(
            None,
            AcceptanceEvidence {
                detail: SafeText::new("logind accepted"),
                accepted_at: SystemTime::now(),
            },
            BoundedVec::new(),
        );
        let r: MutationReceipt<u32> =
            MutationReceipt::accepted(&auth(), common(), None, accepted, recorded());
        assert_eq!(r.lifecycle(), ActionLifecycle::Accepted);
        assert!(r.changed());
        assert!(r.after_digest().is_none());
        assert!(r.verification().is_none());
    }

    #[test]
    fn unverified_collects_uncertain_incident_code() {
        let dispatch = UnverifiedDispatch::Uncertain(UncertainDispatch::new(
            None,
            UncertainEffectCause::TransportLostAfterDispatch,
            BoundedVec::new(),
        ));
        let r = MutationReceipt::unverified(
            &auth(),
            common(),
            Some(obs("before")),
            None,
            dispatch,
            UnverifiedCause::NoDecisiveObservation,
            FailureRollbackState::NotAttempted(RollbackAvailability::Unavailable),
            recorded(),
        );
        assert_eq!(r.lifecycle(), ActionLifecycle::Unverified);
        let s = r.safe_summary();
        let codes: Vec<&str> = s.incident_codes().iter().map(|c| c.as_str()).collect();
        assert!(codes.contains(&"os_control.incident.transport_lost_after_dispatch"));
        assert!(codes.contains(&"os_control.incident.no_decisive_observation"));
    }

    #[test]
    fn verification_failed_carries_contradiction_and_rollback_failure() {
        let dispatch = ContradictedDispatch::Applied(AppliedDispatch::new(None, BoundedVec::new()));
        let contradiction = VerificationContradiction::new(
            Digest::of_str("expected"),
            Some(Digest::of_str("observed")),
            SafeErrorCode::from_static("os_control.incident.contradicted"),
        );
        let r = MutationReceipt::verification_failed(
            &auth(),
            common(),
            obs("before"),
            obs("after"),
            dispatch,
            contradiction,
            FailureRollbackState::Failed(RollbackFailure::new(
                SafeErrorCode::from_static("os_control.incident.rollback_failed"),
                None,
            )),
            recorded(),
        );
        assert_eq!(r.lifecycle(), ActionLifecycle::VerificationFailed);
        assert!(!r.rollback_available());
        let codes: Vec<String> = r
            .incident_codes()
            .as_slice()
            .iter()
            .map(|c| c.as_str().to_string())
            .collect();
        assert!(codes.iter().any(|c| c.contains("contradicted")));
        assert!(codes.iter().any(|c| c.contains("rollback_failed")));
    }

    #[test]
    fn rolled_back_is_net_unchanged() {
        let original = RollbackEligibleFailure::new(
            ContradictedDispatch::Applied(AppliedDispatch::new(None, BoundedVec::new())),
            VerificationContradiction::new(
                Digest::of_str("e"),
                None,
                SafeErrorCode::from_static("os_control.incident.contradicted"),
            ),
        );
        let r = MutationReceipt::rolled_back(
            &auth(),
            common(),
            obs("before"),
            None,
            original,
            satisfying(),
            recorded(),
        );
        assert_eq!(r.lifecycle(), ActionLifecycle::RolledBack);
        assert!(!r.changed());
        assert!(r.verification().is_some());
    }

    #[test]
    fn partial_dispatch_requires_at_least_one_completed_step() {
        let partial = PartialDispatch::new(
            None,
            NonEmptyBoundedVec::single(SafeStepId::new("step-1")),
            SafeStepId::new("step-2"),
            PartialEffectCause::StepFailedAfterCommit,
            BoundedVec::new(),
        );
        assert_eq!(partial.completed_steps().len(), 1);
        let r: MutationReceipt<u32> = MutationReceipt::partially_applied(
            &auth(),
            common(),
            None,
            None,
            partial,
            FailureRollbackState::NotAttempted(RollbackAvailability::Unavailable),
            recorded(),
        );
        assert_eq!(r.lifecycle(), ActionLifecycle::PartiallyApplied);
    }

    #[test]
    fn audit_completion_pending_recovery_token() {
        let state = AuditCompletionState::PendingRecovery {
            admission_id: crate::os_control::contract::AuditAdmissionId::new("adm-1"),
            recovery_key: crate::os_control::contract::AuditRecoveryKey::new("rk-1"),
        };
        assert_eq!(state.as_str(), "pending_recovery");
        let r = MutationReceipt::unchanged(&auth(), common(), obs("state"), state);
        assert_eq!(r.audit_completion().as_str(), "pending_recovery");
    }

    #[test]
    fn every_lifecycle_has_distinct_token() {
        use std::collections::BTreeSet;
        let all = [
            ActionLifecycle::Unchanged,
            ActionLifecycle::Verified,
            ActionLifecycle::Accepted,
            ActionLifecycle::Unverified,
            ActionLifecycle::VerificationFailed,
            ActionLifecycle::RolledBack,
            ActionLifecycle::PartiallyApplied,
        ];
        let set: BTreeSet<&str> = all.iter().map(|l| l.as_str()).collect();
        assert_eq!(set.len(), 7);
    }

    #[test]
    fn safe_summary_serializes_to_stable_shape() {
        let r = MutationReceipt::unchanged(&auth(), common(), obs("state"), recorded());
        let json = serde_json::to_value(r.safe_summary()).expect("serialize summary");
        for key in [
            "receipt_id",
            "action_hash",
            "target_hash",
            "provider",
            "lifecycle",
            "changed",
            "before_digest",
            "after_digest",
            "incident_codes",
        ] {
            assert!(json.get(key).is_some(), "summary must expose {key}");
        }
        assert_eq!(json["lifecycle"], "unchanged");
        assert_eq!(json["changed"], false);
    }

    #[test]
    fn rollback_token_expiry() {
        let past = SystemTime::now() - Duration::from_secs(10);
        let token = RollbackToken::new(
            Digest::of_str("t"),
            SessionId::new("s"),
            Digest::of_str("set_volume"),
            ProviderId::new("pipewire"),
            ReceiptId::new("r-orig"),
            GrantNonce::new("n"),
            past,
        );
        assert!(token.is_expired(SystemTime::now()));
    }

    #[test]
    fn rollback_token_validate_checks_every_binding_in_order() {
        let now = SystemTime::now();
        let future = now + Duration::from_secs(60);
        let action = Digest::of_str("set_volume");
        let cap = ProviderId::new("pipewire");
        let token = RollbackToken::new(
            Digest::of_str("tok"),
            SessionId::new("sess"),
            action.clone(),
            cap.clone(),
            ReceiptId::new("r-orig"),
            GrantNonce::new("n"),
            future,
        );
        // Full agreement validates.
        assert!(token.validate(now, "sess", &action, &cap).is_ok());
        // Expiry is checked first.
        let expired = RollbackToken::new(
            Digest::of_str("tok"),
            SessionId::new("sess"),
            action.clone(),
            cap.clone(),
            ReceiptId::new("r-orig"),
            GrantNonce::new("n"),
            now - Duration::from_secs(1),
        );
        assert_eq!(
            expired.validate(now, "sess", &action, &cap),
            Err(RollbackTokenRejection::Expired)
        );
        // Session, action, and capability mismatches are each distinct.
        assert_eq!(
            token.validate(now, "other", &action, &cap),
            Err(RollbackTokenRejection::SessionMismatch)
        );
        assert_eq!(
            token.validate(now, "sess", &Digest::of_str("kill_process"), &cap),
            Err(RollbackTokenRejection::ActionMismatch)
        );
        assert_eq!(
            token.validate(now, "sess", &action, &ProviderId::new("logind")),
            Err(RollbackTokenRejection::CapabilityMismatch)
        );
    }
}
