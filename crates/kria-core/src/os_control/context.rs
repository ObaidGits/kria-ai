//! Execution contexts: observation-only [`HostExecutionContext`] and the
//! deliberately unconstructable [`AdmittedMutationContext`].
//!
//! linux-os-control-production **Task 1.1**, design §4, §6 (OSC-001).
//!
//! Two contexts separate *observation authority* from *mutation authority*:
//!
//! * [`HostExecutionContext`] authorizes **observation only**. It carries no
//!   grant and is safe to create after read-policy + durable logical-action
//!   admission. Provider reads (`observe`, `verify`) take `&HostExecutionContext`.
//! * [`AdmittedMutationContext`] authorizes **mutation**. It borrows an
//!   [`ExecutionGrant`], the currently-held [`AcquiredResourceLeaseSet`], and the
//!   committed [`AuditAdmissionToken`] via a non-`Clone` [`MutationPermit`], so
//!   apply cannot outlive any of the three authorities.
//!
//! **Task 1.1 declares these interfaces but does NOT claim the mutation-context
//! constructor.** `MutationPermit` and `AdmittedMutationContext` have private
//! fields and **no public constructor**; the runtime sealing that constructs
//! them is owned by [`crate::os_control::runtime`] (Task 1.7), and the borrowed
//! [`AcquiredResourceLeaseSet`] / [`AuditAdmissionToken`] authorities are owned
//! by Tasks 1.6 / 1.8. Until then, mutation-capable context construction is
//! impossible in safe Rust anywhere in the crate.

use std::sync::Arc;
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::os_control::capability::{BusStatus, CapabilitySnapshot, DesktopFamily, DisplayServer};
use crate::os_control::contract::{
    ActionId, AuditAdmissionId, AuditRecoveryKey, CorrelationId, Digest, SessionId,
    SnapshotRevision,
};
// `AcquiredResourceLeaseSet` is owned by Task 1.6 ([`crate::os_control::resource`]);
// the permit only borrows it, so the borrow lifetime keeps `apply` from
// outliving the held leases.
use crate::os_control::resource::AcquiredResourceLeaseSet;
// The runtime sealing witness (Task 1.7). It has a module-private field in
// [`crate::os_control::runtime`], so only `OsControlRuntime` can construct one;
// the sealing constructors below merely *borrow* it, proving the seal is being
// performed by the runtime and by nothing else in the crate.
use crate::os_control::runtime::RuntimeSealAuthority;

/// The design's `ExecutionGrant`. Its canonical implementation is the
/// gate-minted [`crate::agent::execution_gate::OsActionGrant`]: fields are
/// private outside the execution-gate module and its constructor is private to
/// that module, so **only `ExecutionGate` constructs grants** (design §4,
/// OSC-001). The runtime seals a grant with held leases + audit admission into
/// an [`AdmittedMutationContext`]; a handler can hold a grant but cannot renew,
/// weaken, retarget, or self-seal it.
pub use crate::agent::execution_gate::OsActionGrant as ExecutionGrant;

/// Probed session context (design §4, §8). Carries a stable, redaction-safe
/// session identity plus the probe-confirmed display-server family, desktop
/// family, session/system bus availability, and the current capability-snapshot
/// revision the context was observed under.
///
/// The confirmed display/desktop/bus facts come from the capability prober
/// (Task 1.3) — never fabricated from environment variables (OSC-003.3). The
/// snapshot revision lets a later resume detect a stale snapshot (OSC-001.5),
/// coordinating with [`ExecutionGrant`]'s `capability_snapshot_revision`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContext {
    /// The bound user session identity.
    pub session_id: SessionId,
    /// The probe-confirmed display-server family.
    pub display_server: DisplayServer,
    /// The probe-confirmed desktop family.
    pub desktop_family: DesktopFamily,
    /// Session-bus availability.
    pub session_bus: BusStatus,
    /// System-bus availability.
    pub system_bus: BusStatus,
    /// The capability-snapshot revision this context was observed under.
    pub capability_snapshot_revision: SnapshotRevision,
}

impl SessionContext {
    /// Construct a session context with an unprobed capability snapshot. Used
    /// where only a stable session identity is needed before probing runs; the
    /// display/desktop/bus facts default to `Unknown`/`Unavailable` until a
    /// snapshot is applied via [`SessionContext::with_snapshot`].
    #[must_use]
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            display_server: DisplayServer::Unknown,
            desktop_family: DesktopFamily::Unknown,
            session_bus: BusStatus::Unavailable,
            system_bus: BusStatus::Unavailable,
            capability_snapshot_revision: SnapshotRevision::UNPROBED,
        }
    }

    /// Apply the probe-confirmed facts from a [`CapabilitySnapshot`], returning
    /// the updated session context. The snapshot's display/desktop/bus facts and
    /// its revision become authoritative for this session.
    #[must_use]
    pub fn with_snapshot(mut self, snapshot: &CapabilitySnapshot) -> Self {
        self.display_server = snapshot.display_server;
        self.desktop_family = snapshot.desktop_family;
        self.session_bus = snapshot.session_bus;
        self.system_bus = snapshot.system_bus;
        self.capability_snapshot_revision = snapshot.revision;
        self
    }

    /// The capability-snapshot revision this context was observed under.
    #[must_use]
    pub fn capability_snapshot_revision(&self) -> SnapshotRevision {
        self.capability_snapshot_revision
    }
}

/// Redaction policy handle carried by contexts (design §4). Task 1.8 owns the
/// full sensitivity registry; Task 1.1 carries the selected profile only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RedactionPolicy {
    /// Whether sensitive identifiers are hashed (vs truncated) in traces.
    pub hash_sensitive_identifiers: bool,
}

/// Observation-only audit authority lent from a committed audit admission token
/// (design §4, §14). It authorizes read/pre-observation audit but **cannot**
/// authorize mutation. Task 1.8 owns the durable admission that lends it; Task
/// 1.1 declares the borrow-only handle.
#[derive(Debug, Clone)]
pub struct ObservationAuditAuthority {
    admission_id: AuditAdmissionId,
}

impl ObservationAuditAuthority {
    /// Lend an observation-only authority from a committed admission id. Owned
    /// by [`crate::os_control::audit`] (Task 1.8): the durable admission is the
    /// sole producer.
    #[must_use]
    pub(crate) fn from_admission(admission_id: AuditAdmissionId) -> Self {
        Self { admission_id }
    }

    /// The admission this observation authority derives from.
    #[must_use]
    pub fn admission_id(&self) -> &AuditAdmissionId {
        &self.admission_id
    }
}

/// Observation-only execution context (design §4). Safe to create after read
/// policy and durable logical-action admission; carries **no** mutation grant.
pub struct HostExecutionContext {
    /// Correlation spanning one logical request.
    pub correlation_id: CorrelationId,
    /// This action's identity within the correlation.
    pub action_id: ActionId,
    /// Observation-only audit authority (not a mutation grant).
    observation_audit: ObservationAuditAuthority,
    /// The probed session context.
    pub session: Arc<SessionContext>,
    /// Cooperative cancellation for bounded observation.
    pub cancellation: CancellationToken,
    /// Observation deadline.
    pub deadline: Instant,
    /// Redaction policy for any surfaced detail.
    pub redaction: RedactionPolicy,
}

impl HostExecutionContext {
    /// The observation-only audit authority.
    #[must_use]
    pub fn observation_audit(&self) -> &ObservationAuditAuthority {
        &self.observation_audit
    }

    /// Assemble the **production** observation context for one admitted action.
    ///
    /// The `observation_audit` authority must come from
    /// [`AuditAdmissionToken::observation_authority`], which is the only way to
    /// obtain one — so a context cannot exist without a durable audit admission
    /// behind it. This carries read authority only: it grants no mutation permit,
    /// which still requires the grant + lease set at `run_mutation`.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn for_action(
        correlation_id: CorrelationId,
        action_id: ActionId,
        observation_audit: ObservationAuditAuthority,
        session: Arc<SessionContext>,
        cancellation: CancellationToken,
        deadline: Instant,
        redaction: RedactionPolicy,
    ) -> Self {
        Self {
            correlation_id,
            action_id,
            observation_audit,
            session,
            cancellation,
            deadline,
            redaction,
        }
    }
}

#[cfg(feature = "os-control-test")]
impl HostExecutionContext {
    /// Build an observation-only context for deny-live tests. Gated to
    /// `os-control-test`, so it can never exist in a live composition; the
    /// production observation context is assembled by the runtime.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn for_test(
        correlation_id: CorrelationId,
        action_id: ActionId,
        observation_audit: ObservationAuditAuthority,
        session: Arc<SessionContext>,
        cancellation: CancellationToken,
        deadline: Instant,
        redaction: RedactionPolicy,
    ) -> Self {
        Self {
            correlation_id,
            action_id,
            observation_audit,
            session,
            cancellation,
            deadline,
            redaction,
        }
    }
}

/// Committed durable audit-admission token (design §4, §14). **Owned by Task
/// 1.8** ([`crate::os_control::audit`]); the durable [`admit_action`] append is
/// its sole producer. Non-`Clone`, private fields; the token binds the
/// admission's session/action/parameter/target/capability/resource/recovery
/// digests but is deliberately **not** grant-bound — mutation authority arises
/// only when the runtime (Task 1.7) seals it together with the later grant +
/// held leases and verifies these bindings match the fresh grant.
///
/// [`admit_action`]: crate::os_control::audit::OsAuditStore::admit_action
#[derive(Debug)]
pub struct AuditAdmissionToken {
    admission_id: AuditAdmissionId,
    recovery_key: AuditRecoveryKey,
    session_id: SessionId,
    action_hash: Digest,
    parameter_hash: Digest,
    target_hash: Digest,
    capability_snapshot_revision: SnapshotRevision,
    resource_set_digest: Digest,
}

impl AuditAdmissionToken {
    /// Seal a committed admission into its token. Module-private producer:
    /// only [`crate::os_control::audit`] (Task 1.8), after a successful durable
    /// admission append, calls this — no provider/tool/adapter can forge one.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn seal(
        admission_id: AuditAdmissionId,
        recovery_key: AuditRecoveryKey,
        session_id: SessionId,
        action_hash: Digest,
        parameter_hash: Digest,
        target_hash: Digest,
        capability_snapshot_revision: SnapshotRevision,
        resource_set_digest: Digest,
    ) -> Self {
        Self {
            admission_id,
            recovery_key,
            session_id,
            action_hash,
            parameter_hash,
            target_hash,
            capability_snapshot_revision,
            resource_set_digest,
        }
    }

    /// Lend an observation-only audit authority from this admission. The lent
    /// authority cannot authorize mutation (design §4, §14).
    #[must_use]
    pub fn observation_authority(&self) -> ObservationAuditAuthority {
        ObservationAuditAuthority::from_admission(self.admission_id.clone())
    }

    /// The bound admission identity.
    #[must_use]
    pub fn admission_id(&self) -> &AuditAdmissionId {
        &self.admission_id
    }

    /// The idempotent recovery key committed before dispatch.
    #[must_use]
    pub fn recovery_key(&self) -> &AuditRecoveryKey {
        &self.recovery_key
    }

    /// The bound session identity.
    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// The bound action digest.
    #[must_use]
    pub fn action_hash(&self) -> &Digest {
        &self.action_hash
    }

    /// The bound canonical parameter digest.
    #[must_use]
    pub fn parameter_hash(&self) -> &Digest {
        &self.parameter_hash
    }

    /// The bound host-target digest.
    #[must_use]
    pub fn target_hash(&self) -> &Digest {
        &self.target_hash
    }

    /// The capability-snapshot revision bound at admission.
    #[must_use]
    pub fn capability_snapshot_revision(&self) -> SnapshotRevision {
        self.capability_snapshot_revision
    }

    /// The prospective canonical resource-set digest bound at admission.
    #[must_use]
    pub fn resource_set_digest(&self) -> &Digest {
        &self.resource_set_digest
    }
}

#[cfg(feature = "os-control-test")]
impl AuditAdmissionToken {
    /// Seal a committed admission token for deny-live tests. Gated to
    /// `os-control-test`; the production producer is the durable audit append
    /// (Task 1.8).
    #[must_use]
    pub fn for_test(admission_id: AuditAdmissionId, resource_set_digest: Digest) -> Self {
        Self::seal(
            admission_id,
            AuditRecoveryKey::new("test-recovery-key"),
            SessionId::new("test-session"),
            Digest::of_str("test-action"),
            Digest::of_str("test-parameter"),
            Digest::of_str("test-target"),
            SnapshotRevision(1),
            resource_set_digest,
        )
    }
}

/// The sealed mutation permit (design §4). Private fields, **non-`Clone`**, and
/// **no public constructor** in Task 1.1: it borrows all three authorities so
/// `apply` cannot outlive them. The construction that verifies the grant matches
/// the admission bindings, that the named resource set is held, and that the
/// audit admission committed is owned by [`crate::os_control::runtime`] (Task
/// 1.7).
#[derive(Debug)]
pub struct MutationPermit<'a> {
    #[allow(dead_code)]
    lease_set: &'a AcquiredResourceLeaseSet,
    #[allow(dead_code)]
    audit_admission: &'a AuditAdmissionToken,
    #[allow(dead_code)]
    resource_set_digest: Digest,
}

/// The sealed, mutation-capable execution context (design §4). Providers accept
/// `&AdmittedMutationContext<'_>` for every mutation, so invocation before
/// approval, resource acquisition, or audit admission is unrepresentable in safe
/// Rust. **No public constructor exists in Task 1.1** — the runtime sealing that
/// builds it lands in Task 1.7.
pub struct AdmittedMutationContext<'a> {
    #[allow(dead_code)]
    observation: &'a HostExecutionContext,
    #[allow(dead_code)]
    grant: &'a ExecutionGrant,
    #[allow(dead_code)]
    permit: MutationPermit<'a>,
    /// The action name the grant is bound to.
    requested_action: String,
    /// The parameters the grant's digest was taken over.
    requested_params: serde_json::Value,
}

impl std::fmt::Debug for AdmittedMutationContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately does not print the borrowed observation context (which is
        // not `Debug`); the grant identity is enough for diagnostics.
        f.debug_struct("AdmittedMutationContext")
            .field("grant", &self.grant)
            .field("permit", &self.permit)
            .finish_non_exhaustive()
    }
}

impl<'a> AdmittedMutationContext<'a> {
    /// Borrow the underlying observation context (read authority is a subset of
    /// mutation authority).
    #[must_use]
    pub fn observation(&self) -> &HostExecutionContext {
        self.observation
    }

    /// The action name this mutation was admitted for.
    ///
    /// A provider building its own command plan MUST use this, not a descriptive
    /// label of its own: the plan's action is compared against the grant, and a
    /// mismatch is rejected as a binding mismatch.
    #[must_use]
    pub fn requested_action(&self) -> &str {
        &self.requested_action
    }

    /// The parameters this mutation was admitted for.
    ///
    /// The plan's params digest is compared against the grant's, so a provider must
    /// pass these through unchanged rather than constructing its own object.
    #[must_use]
    pub fn requested_params(&self) -> &serde_json::Value {
        &self.requested_params
    }

    /// Borrow the sealed grant.
    #[must_use]
    pub fn grant(&self) -> &ExecutionGrant {
        self.grant
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Runtime-only sealing (Task 1.7, design §4)
// ─────────────────────────────────────────────────────────────────────────────
//
// These are the production constructors deferred by Task 1.1. Each requires a
// borrowed [`RuntimeSealAuthority`], whose only field is private to
// [`crate::os_control::runtime`]; therefore only `OsControlRuntime` can call
// them (no provider, handler, adapter, or other crate module can obtain the
// witness). The runtime calls these only *after* it has verified that the fresh
// grant matches the committed audit admission's bindings and that the exact
// named resource set is currently held (see
// [`crate::os_control::runtime::OsControlRuntime::seal_mutation_context`]).

impl<'a> MutationPermit<'a> {
    /// Seal a permit from the three held authorities. Runtime-only: the borrowed
    /// [`RuntimeSealAuthority`] cannot be constructed outside the runtime module.
    #[must_use]
    pub(crate) fn seal(
        _authority: &RuntimeSealAuthority,
        lease_set: &'a AcquiredResourceLeaseSet,
        audit_admission: &'a AuditAdmissionToken,
        resource_set_digest: Digest,
    ) -> Self {
        Self {
            lease_set,
            audit_admission,
            resource_set_digest,
        }
    }
}

impl<'a> AdmittedMutationContext<'a> {
    /// Seal a mutation-capable context from an observation context, the fresh
    /// grant, and a sealed [`MutationPermit`]. Runtime-only via the borrowed
    /// [`RuntimeSealAuthority`].
    #[must_use]
    pub(crate) fn seal(
        _authority: &RuntimeSealAuthority,
        observation: &'a HostExecutionContext,
        grant: &'a ExecutionGrant,
        permit: MutationPermit<'a>,
        requested_action: String,
        requested_params: serde_json::Value,
    ) -> Self {
        Self {
            observation,
            grant,
            permit,
            requested_action,
            requested_params,
        }
    }
}

#[cfg(feature = "os-control-test")]
impl<'a> MutationPermit<'a> {
    /// Seal a permit for deny-live tests from already-held authorities. Gated to
    /// `os-control-test`; the production sealing (which verifies the grant/lease/
    /// audit bindings match) is owned by the runtime (Task 1.7).
    #[must_use]
    pub fn for_test(
        lease_set: &'a AcquiredResourceLeaseSet,
        audit_admission: &'a AuditAdmissionToken,
        resource_set_digest: Digest,
    ) -> Self {
        Self {
            lease_set,
            audit_admission,
            resource_set_digest,
        }
    }
}

#[cfg(feature = "os-control-test")]
impl<'a> AdmittedMutationContext<'a> {
    /// Assemble a sealed mutation context for deny-live tests. Gated to
    /// `os-control-test`; the production constructor is the runtime seal.
    ///
    /// The requested action is taken from the grant so a test context is
    /// self-consistent by construction: a provider that dispatches through the
    /// shared helper will bind to the same action the grant carries. Params default
    /// to an empty object; tests that need a specific payload use
    /// [`Self::for_test_with_params`].
    #[must_use]
    pub fn for_test(
        observation: &'a HostExecutionContext,
        grant: &'a ExecutionGrant,
        permit: MutationPermit<'a>,
    ) -> Self {
        Self {
            requested_action: grant.action().to_string(),
            requested_params: serde_json::json!({}),
            observation,
            grant,
            permit,
        }
    }

    /// As [`Self::for_test`], with explicit requested parameters.
    ///
    /// Needed by any test that asserts the grant's params digest is honoured: the
    /// digest is taken over these, so they must be the same object the grant was
    /// minted from.
    #[must_use]
    pub fn for_test_with_params(
        observation: &'a HostExecutionContext,
        grant: &'a ExecutionGrant,
        permit: MutationPermit<'a>,
        requested_params: serde_json::Value,
    ) -> Self {
        Self {
            requested_action: grant.action().to_string(),
            requested_params,
            observation,
            grant,
            permit,
        }
    }
}

// Compile-time proof that the observation context and grant are thread-safe.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<HostExecutionContext>();
    assert_send_sync::<ExecutionGrant>();
    assert_send_sync::<SessionContext>();
    assert_send_sync::<AuditAdmissionToken>();
    assert_send_sync::<AcquiredResourceLeaseSet>();
};

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    // A compile-time enumeration of the mutation-context surface. This test's
    // real assertion is structural: `AdmittedMutationContext` / `MutationPermit`
    // have no public constructor, so no code here (or in any provider/adapter
    // module) can build one until Task 1.7's runtime sealing lands. If a public
    // constructor were ever added, the deferral invariant would be broken.
    #[test]
    fn mutation_context_has_no_public_constructor() {
        // We can name the types and their read-only accessors, but there is no
        // callable path to construct one. Observation-only context, by contrast,
        // is freely inspectable.
        fn _accepts_admitted<'a>(ctx: &'a AdmittedMutationContext<'a>) -> &'a ExecutionGrant {
            ctx.grant()
        }
        // AcquiredResourceLeaseSet / AuditAdmissionToken also expose only
        // read-only digest evidence and no public constructor in Task 1.1.
        fn _reads_lease(set: &AcquiredResourceLeaseSet) -> &Digest {
            set.resource_set_digest()
        }
        fn _reads_admission(tok: &AuditAdmissionToken) -> &AuditAdmissionId {
            tok.admission_id()
        }
    }

    #[test]
    fn session_context_carries_bound_identity() {
        let ctx = SessionContext::new(SessionId::new("session-1"));
        assert_eq!(ctx.session_id.as_str(), "session-1");
    }
}
