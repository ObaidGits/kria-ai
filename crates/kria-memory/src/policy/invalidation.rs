//! In-flight policy-change invalidation (task **F1.4.6**; design §5.2 snapshot
//! /patch rules, §9.2 UI session state machine; MGR-004 AC6, MGR-007 AC4,
//! MGR-008 AC7, MGR-035, MGR-043; MGD-007).
//!
//! MGR-004 AC6 is categorical: "WHEN identity or scope changes during an
//! in-flight request, THE Cognitive_Memory_System SHALL discard the response and
//! invalidate incompatible cache entries." Design §9.2 restates it for the read
//! path — "every request binds instance, generation, query hash, policy hash,
//! and base revision … mismatches are discarded" — and §5.2 restates it for the
//! write/patch path — a policy change "performs active-query bounded refetch" and
//! "a pending write is visually confirmed only after matching revision."
//!
//! ## Why this builds on [`AuthorizedScope::policy_hash`]
//!
//! F1.4.5 already proved that the [`AuthorizedScope::policy_hash`] is a pure
//! function of the authorized partition (namespace / scope / sensitivity ceiling
//! / owner — i.e. caller **identity + scope**), the read **capabilities**, and
//! the contributing Effective **Policy**'s provenance hash. Therefore **any**
//! change to identity, scope, capability, or effective policy changes the policy
//! hash, and equal authorizations hash equally. This module reuses that single
//! contract — it does **not** introduce a parallel keying scheme. Detecting "the
//! caller's authorization changed while a request was in flight" reduces to
//! comparing the policy hash a request was **bound** to against the policy hash
//! the caller **now** holds.
//!
//! ## What this module delivers (and what it defers)
//!
//! The concrete request pipeline, the durable cache/cursor stores, and the
//! multi-window UI session (design §9.2 `MemoryWindowSessionV2`) do not exist
//! yet — they are F3 (retrieval/cursors), F3/F4 (caches), and F4/§5.2.4 (UI
//! session invalidation across windows). This module delivers the **core
//! invalidation primitive and enforcement discipline** those later stages MUST
//! route through, so that "discard on identity/scope change" is *structurally*
//! true rather than retrofitted:
//!
//! * [`PolicyEpoch`] — the identity an in-flight request/response/pending-write
//!   /trace/cursor/cache entry is **bound** to: the [`AuthorizedScope::policy_hash`]
//!   it was authorized under plus a monotonic [`RequestGeneration`] (focus/refocus
//!   increments the generation so a late response from a superseded request is
//!   discarded even when the policy is unchanged — design §9.2).
//! * [`PolicyEpoch::relation`] classifies the bound epoch against the caller's
//!   **current** epoch into [`EpochRelation`] (`Current` / `Superseded` /
//!   `PolicyChanged`) — a single deterministic decision every enforcement helper
//!   routes through.
//! * [`PolicyGuard`] wraps a bound epoch and enforces discard on each surface:
//!   [`admit_response`](PolicyGuard::admit_response) discards an **in-flight
//!   response** (consumes it so it can never be returned),
//!   [`admit_trace`](PolicyGuard::admit_trace) discards a **trace**, and
//!   [`resolve_pending`](PolicyGuard::resolve_pending) refuses to confirm a
//!   **pending write** (rolls it back / withholds it, MGR-008 AC7).
//! * [`CursorGuard`] rejects a **cursor** carrying a superseded policy hash *or*
//!   an incompatible revision with a typed [`BoundedRefetch`] instruction
//!   (MGR-007 AC4).
//! * [`cache_entry_servable`] / [`invalidate_superseded`] give **cache** stores
//!   an explicit invalidation predicate/eviction so entries keyed under a
//!   superseded policy hash are discarded and never served (MGR-004 AC6).
//!
//! ## Downstream stages that consume this primitive (deferred gates)
//!
//! | Surface        | Gate  | How it consumes the primitive |
//! |----------------|-------|-------------------------------|
//! | In-flight read | F3    | bind a [`PolicyGuard`]; deliver only on [`EpochRelation::Current`] |
//! | Cursor pages   | F3    | validate each page with [`CursorGuard::admit`]; emit [`BoundedRefetch`] |
//! | Result caches  | F3/F4 | gate serving with [`cache_entry_servable`]; evict via [`invalidate_superseded`] |
//! | Pending writes | F3/F5 | confirm only when [`resolve_pending`](PolicyGuard::resolve_pending) returns [`PendingResolution::Confirm`] |
//! | Traces         | F1–F5 | retain only via [`admit_trace`](PolicyGuard::admit_trace) |
//! | UI session     | F4    | `MemoryWindowSessionV2` binds `policyHash`+`requestGeneration`; discards mismatches |

use crate::model::GraphRevision;

use super::read_authorization::AuthorizedScope;

// ─────────────────────────────────────────────────────────────────────────
// RequestGeneration — monotonic per-caller request generation
// ─────────────────────────────────────────────────────────────────────────

/// A monotonic per-caller request generation. Focus/refocus increments it so a
/// late response from a *superseded* request is discarded even when the policy
/// hash is unchanged (design §9.2: "focus increments generation and cancels
/// prior work; mismatches are discarded"). It is orthogonal to the policy hash:
/// the policy hash detects identity/scope/capability/policy change; the
/// generation detects same-policy supersession.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestGeneration(u64);

impl RequestGeneration {
    /// The first generation for a fresh caller session.
    pub const FIRST: Self = RequestGeneration(0);

    /// Construct a generation from a raw counter value.
    pub fn new(value: u64) -> Self {
        RequestGeneration(value)
    }

    /// The raw counter value.
    pub fn get(self) -> u64 {
        self.0
    }

    /// The next generation (issued on focus/refocus). Saturates at the maximum
    /// rather than wrapping, so a later request can never be mistaken for an
    /// earlier one.
    pub fn next(self) -> Self {
        RequestGeneration(self.0.saturating_add(1))
    }
}

impl Default for RequestGeneration {
    fn default() -> Self {
        Self::FIRST
    }
}

// ─────────────────────────────────────────────────────────────────────────
// PolicyEpoch — the identity an in-flight request is bound to
// ─────────────────────────────────────────────────────────────────────────

/// The identity an in-flight request/response/pending-write/trace/cursor/cache
/// entry is **bound** to: the [`AuthorizedScope::policy_hash`] it was authorized
/// under plus the [`RequestGeneration`] it was issued in.
///
/// Because the policy hash already folds in caller identity, scope, capability,
/// and effective policy (F1.4.5), two epochs share a policy hash **iff** they
/// run under the same authorization. Comparing a bound epoch against the
/// caller's current epoch is therefore the whole of "did identity/scope change
/// during this in-flight request?" — see [`relation`](Self::relation).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PolicyEpoch {
    policy_hash: String,
    generation: RequestGeneration,
}

impl PolicyEpoch {
    /// Capture the epoch a request is authorized under, from the caller's
    /// [`AuthorizedScope`] and the current [`RequestGeneration`]. This is the
    /// single sanctioned constructor bound to the F1.4.5 policy-hash contract.
    pub fn capture(scope: &AuthorizedScope, generation: RequestGeneration) -> Self {
        Self {
            policy_hash: scope.policy_hash().to_string(),
            generation,
        }
    }

    /// The policy hash this epoch is bound to.
    pub fn policy_hash(&self) -> &str {
        &self.policy_hash
    }

    /// The generation this epoch was issued in.
    pub fn generation(&self) -> RequestGeneration {
        self.generation
    }

    /// Classify this **bound** epoch against the caller's **current** epoch. This
    /// is the single deterministic decision every enforcement helper routes
    /// through:
    ///
    /// * [`EpochRelation::PolicyChanged`] — the policy hash differs, so caller
    ///   identity, scope, capability, or effective policy changed while the
    ///   request was in flight. Takes precedence over generation: a policy change
    ///   invalidates regardless of generation.
    /// * [`EpochRelation::Superseded`] — same policy hash but a different
    ///   generation (a newer focus/refocus superseded this request).
    /// * [`EpochRelation::Current`] — same policy hash and same generation; the
    ///   request is still current and its result may be delivered.
    pub fn relation(&self, current: &PolicyEpoch) -> EpochRelation {
        if self.policy_hash != current.policy_hash {
            EpochRelation::PolicyChanged
        } else if self.generation != current.generation {
            EpochRelation::Superseded
        } else {
            EpochRelation::Current
        }
    }

    /// Whether this bound epoch is still current under `current` — a convenience
    /// over [`relation`](Self::relation).
    pub fn is_current(&self, current: &PolicyEpoch) -> bool {
        matches!(self.relation(current), EpochRelation::Current)
    }
}

/// The relation between a **bound** [`PolicyEpoch`] and the caller's **current**
/// one — the deterministic classification that decides whether in-flight state
/// survives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochRelation {
    /// Same policy hash and generation — the request is still current.
    Current,
    /// Same policy hash but a newer generation superseded the request.
    Superseded,
    /// The policy hash changed (identity / scope / capability / effective
    /// policy) — the request must be discarded and incompatible state invalidated.
    PolicyChanged,
}

impl EpochRelation {
    /// Whether state bound to the epoch must be discarded/invalidated.
    pub fn is_invalidated(self) -> bool {
        !matches!(self, EpochRelation::Current)
    }

    /// The [`InvalidationReason`] for a non-current relation, or `None` when the
    /// epoch is still current.
    pub fn invalidation_reason(self) -> Option<InvalidationReason> {
        match self {
            EpochRelation::Current => None,
            EpochRelation::Superseded => Some(InvalidationReason::Superseded),
            EpochRelation::PolicyChanged => Some(InvalidationReason::PolicyChanged),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// InvalidationReason — why in-flight state was discarded
// ─────────────────────────────────────────────────────────────────────────

/// Why in-flight state (a response, trace, pending write, cursor, or cache
/// entry) was discarded. A stable, content-free reason suitable for policy-safe
/// logging (MGR-028 AC2): it names *why* without revealing any record identity,
/// label, or count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidationReason {
    /// Caller identity, scope, capability, or effective policy changed while the
    /// request was in flight (the policy hash differs). MGR-004 AC6.
    PolicyChanged,
    /// A newer request generation (focus/refocus) superseded this request while
    /// it was in flight (design §9.2).
    Superseded,
}

impl InvalidationReason {
    /// A stable, content-free reason code for policy-safe logging.
    pub fn reason_code(self) -> &'static str {
        match self {
            InvalidationReason::PolicyChanged => "policy_changed",
            InvalidationReason::Superseded => "request_superseded",
        }
    }
}

impl std::fmt::Display for InvalidationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            InvalidationReason::PolicyChanged => {
                "discarded: caller identity/scope/capability/policy changed in flight"
            }
            InvalidationReason::Superseded => "discarded: request superseded by a newer generation",
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────
// ResponseDisposition — deliver-or-discard for an in-flight value
// ─────────────────────────────────────────────────────────────────────────

/// The outcome of guarding an in-flight value (a response or a trace) against a
/// policy change. `Discard` **consumes** the value, so a response computed under
/// a superseded policy can never be returned to the caller — the discard is
/// structural, not a convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseDisposition<T> {
    /// The bound epoch is still current; deliver the value.
    Deliver(T),
    /// The bound epoch changed; the value was discarded for this reason.
    Discard(InvalidationReason),
}

impl<T> ResponseDisposition<T> {
    /// Whether the value may be delivered.
    pub fn is_delivered(&self) -> bool {
        matches!(self, ResponseDisposition::Deliver(_))
    }

    /// Whether the value was discarded.
    pub fn is_discarded(&self) -> bool {
        matches!(self, ResponseDisposition::Discard(_))
    }

    /// The discard reason, if the value was discarded.
    pub fn discard_reason(&self) -> Option<InvalidationReason> {
        match self {
            ResponseDisposition::Deliver(_) => None,
            ResponseDisposition::Discard(reason) => Some(*reason),
        }
    }

    /// Consume into the deliverable value, or `None` if it was discarded.
    pub fn into_delivered(self) -> Option<T> {
        match self {
            ResponseDisposition::Deliver(value) => Some(value),
            ResponseDisposition::Discard(_) => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// PendingResolution — confirm-or-rollback for a pending write
// ─────────────────────────────────────────────────────────────────────────

/// The outcome of resolving a **pending write** against the caller's current
/// policy (MGR-008 AC6/AC7). A pending write awaits matching revision
/// confirmation; if the caller's policy changed before confirmation, the
/// optimistic/pending write must **not** be confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingResolution {
    /// The bound epoch is still current; the pending write may be confirmed.
    Confirm,
    /// The bound epoch changed; the optimistic/pending write must be rolled back
    /// (withheld from confirmed styling) for this reason.
    Rollback(InvalidationReason),
}

impl PendingResolution {
    /// Whether the pending write may be confirmed.
    pub fn is_confirmed(self) -> bool {
        matches!(self, PendingResolution::Confirm)
    }

    /// The rollback reason, if the write must be rolled back.
    pub fn rollback_reason(self) -> Option<InvalidationReason> {
        match self {
            PendingResolution::Confirm => None,
            PendingResolution::Rollback(reason) => Some(reason),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// PolicyGuard — enforce discard on response / trace / pending write
// ─────────────────────────────────────────────────────────────────────────

/// Binds an in-flight response / trace / pending write to the [`PolicyEpoch`] it
/// was authorized under, and enforces discard when the caller's current epoch no
/// longer matches. A stage constructs one guard when it begins work
/// ([`PolicyGuard::capture`]) and consults it before delivering / confirming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyGuard {
    bound: PolicyEpoch,
}

impl PolicyGuard {
    /// Bind a guard to an already-captured [`PolicyEpoch`].
    pub fn new(bound: PolicyEpoch) -> Self {
        Self { bound }
    }

    /// Capture a guard directly from the authorized scope and request generation
    /// a stage is running under.
    pub fn capture(scope: &AuthorizedScope, generation: RequestGeneration) -> Self {
        Self::new(PolicyEpoch::capture(scope, generation))
    }

    /// The epoch this guard is bound to.
    pub fn bound(&self) -> &PolicyEpoch {
        &self.bound
    }

    /// Guard an in-flight **response**: deliver it only if the bound epoch is
    /// still current under `current`, otherwise discard it (consuming the value
    /// so it can never be returned). MGR-004 AC6.
    pub fn admit_response<T>(&self, current: &PolicyEpoch, response: T) -> ResponseDisposition<T> {
        match self.bound.relation(current).invalidation_reason() {
            None => ResponseDisposition::Deliver(response),
            Some(reason) => ResponseDisposition::Discard(reason),
        }
    }

    /// Guard an in-flight **trace**: retain it only if the bound epoch is still
    /// current, otherwise discard it. Traces share the response discard rule so a
    /// trace produced under a superseded policy is never emitted.
    pub fn admit_trace<T>(&self, current: &PolicyEpoch, trace: T) -> ResponseDisposition<T> {
        self.admit_response(current, trace)
    }

    /// Resolve a **pending write**: confirm it only if the bound epoch is still
    /// current, otherwise instruct rollback / withholding (MGR-008 AC6/AC7). Note
    /// that a newer *generation* under the same policy does **not** roll back a
    /// pending write on its own — supersession discards reads, but a durable
    /// write in flight is only invalidated by a genuine policy change; callers
    /// that also want generation supersession can inspect
    /// [`PolicyEpoch::relation`] directly.
    pub fn resolve_pending(&self, current: &PolicyEpoch) -> PendingResolution {
        match self.bound.relation(current) {
            EpochRelation::PolicyChanged => {
                PendingResolution::Rollback(InvalidationReason::PolicyChanged)
            }
            EpochRelation::Current | EpochRelation::Superseded => PendingResolution::Confirm,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// CursorGuard — reject a stale cursor with a bounded-refetch instruction
// ─────────────────────────────────────────────────────────────────────────

/// The reason a cursor was rejected and a bounded refetch is required
/// (MGR-007 AC4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefetchReason {
    /// The caller's policy changed since the cursor was issued (the cursor's
    /// bound policy hash no longer matches). MGR-004 AC6 + MGR-007 AC4.
    PolicyChanged,
    /// The cursor's snapshot revision is incompatible with the current authority
    /// revision. MGR-007 AC4.
    RevisionIncompatible,
}

impl RefetchReason {
    /// A stable, content-free reason code for policy-safe logging.
    pub fn reason_code(self) -> &'static str {
        match self {
            RefetchReason::PolicyChanged => "cursor_policy_changed",
            RefetchReason::RevisionIncompatible => "cursor_revision_incompatible",
        }
    }
}

/// A typed bounded-refetch instruction returned when a cursor is rejected
/// (MGR-007 AC4). It carries only the [`RefetchReason`] — no record identity,
/// count, or topology — so the client performs a fresh bounded active-query
/// refetch under its *current* policy rather than resuming a stale page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundedRefetch {
    reason: RefetchReason,
}

impl BoundedRefetch {
    /// The reason the refetch is required.
    pub fn reason(&self) -> RefetchReason {
        self.reason
    }
}

impl std::fmt::Display for BoundedRefetch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bounded_refetch:{}", self.reason.reason_code())
    }
}

/// The outcome of validating a cursor against the caller's current policy and
/// authority revision (MGR-007 AC3/AC4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorAdmission {
    /// The cursor is compatible; the next page may be served from the same
    /// snapshot.
    Resume,
    /// The cursor is stale; the client must perform a bounded refetch.
    Refetch(BoundedRefetch),
}

impl CursorAdmission {
    /// Whether the cursor may resume.
    pub fn is_resumable(self) -> bool {
        matches!(self, CursorAdmission::Resume)
    }

    /// The bounded-refetch instruction, if the cursor was rejected.
    pub fn refetch(self) -> Option<BoundedRefetch> {
        match self {
            CursorAdmission::Resume => None,
            CursorAdmission::Refetch(instruction) => Some(instruction),
        }
    }
}

/// Binds a paginated **cursor** to the policy hash and snapshot revision it was
/// issued under (design §5.2: cursor pages preserve one snapshot revision; a
/// policy or revision change forces a bounded active-query refetch). It mirrors
/// the [`AuthorizedScope::cursor_key`] contract from F1.4.5 (which already folds
/// the policy hash into the key) with an explicit *rejection* decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorGuard {
    policy_hash: String,
    revision: GraphRevision,
}

impl CursorGuard {
    /// Capture a cursor's binding from the authorized scope it was issued under
    /// and the snapshot revision it paginates.
    pub fn capture(scope: &AuthorizedScope, revision: GraphRevision) -> Self {
        Self {
            policy_hash: scope.policy_hash().to_string(),
            revision,
        }
    }

    /// The policy hash the cursor is bound to.
    pub fn policy_hash(&self) -> &str {
        &self.policy_hash
    }

    /// The snapshot revision the cursor paginates.
    pub fn revision(&self) -> GraphRevision {
        self.revision
    }

    /// Validate the cursor against the caller's current scope and authority
    /// revision. A superseded policy hash or an incompatible revision yields a
    /// typed [`BoundedRefetch`]; otherwise the cursor may resume. Policy change
    /// takes precedence over revision drift (a caller who lost authorization must
    /// refetch under the new policy, not merely re-snapshot).
    pub fn admit(
        &self,
        current_scope: &AuthorizedScope,
        current_revision: GraphRevision,
    ) -> CursorAdmission {
        if self.policy_hash != current_scope.policy_hash() {
            CursorAdmission::Refetch(BoundedRefetch {
                reason: RefetchReason::PolicyChanged,
            })
        } else if self.revision != current_revision {
            CursorAdmission::Refetch(BoundedRefetch {
                reason: RefetchReason::RevisionIncompatible,
            })
        } else {
            CursorAdmission::Resume
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Cache invalidation — evict / refuse entries under a superseded policy hash
// ─────────────────────────────────────────────────────────────────────────

/// A cache entry that carries the policy hash it was computed under. Result
/// caches implement this so [`invalidate_superseded`] can evict entries keyed
/// under a superseded policy and [`cache_entry_servable`] can refuse to serve
/// them. The F1.4.5 [`AuthorizedScope::cache_key`] already binds the policy hash
/// *into* the key so cross-policy reuse is impossible; this trait adds the
/// explicit *invalidation* MGR-004 AC6 demands.
pub trait PolicyKeyed {
    /// The policy hash this entry was computed/keyed under.
    fn policy_hash(&self) -> &str;
}

/// Whether a cache entry keyed under `entry_policy_hash` is still servable under
/// the caller's current scope. An entry computed under a superseded policy is
/// **not** servable — it is invalidated, never returned (MGR-004 AC6). Caches
/// call this before serving a hit.
pub fn cache_entry_servable(entry_policy_hash: &str, current: &AuthorizedScope) -> bool {
    entry_policy_hash == current.policy_hash()
}

/// Evict every cache entry whose policy hash is not the caller's current one,
/// retaining only entries computed under the current policy. Returns the number
/// of entries invalidated. This is the explicit eviction pass a cache runs when
/// the caller's policy changes (MGR-004 AC6: "invalidate incompatible cache
/// entries").
pub fn invalidate_superseded<T: PolicyKeyed>(
    entries: &mut Vec<T>,
    current: &AuthorizedScope,
) -> usize {
    let before = entries.len();
    entries.retain(|entry| cache_entry_servable(entry.policy_hash(), current));
    before - entries.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::SourceTrust;
    use crate::model::{CallerContext, PolicyPartition};
    use crate::policy::effective_policy::{ContributingPolicy, EffectivePolicy};
    use crate::policy::read_authorization::authorize_read;
    use crate::policy::source_trust::{Capability, CapabilitySet, ConsentRequirement};
    use proptest::prelude::*;

    // ── Fixtures (mirror the F1.4.5 read_authorization test fixtures) ────

    fn partition(ns: &str, scope: &str, sensitivity: u8, owner: Option<&str>) -> PolicyPartition {
        PolicyPartition::with_owner(ns, scope, sensitivity, owner.map(str::to_string)).unwrap()
    }

    fn caller(partition: PolicyPartition) -> CallerContext {
        CallerContext::local_desktop("device-1", partition).unwrap()
    }

    fn read_policy(partition: PolicyPartition, caps: &[Capability]) -> EffectivePolicy {
        let contributor = ContributingPolicy::new(
            "src-read",
            partition,
            CapabilitySet::from_capabilities(caps.iter().copied()),
            SourceTrust::System,
            ConsentRequirement::NotRequired,
        )
        .unwrap();
        EffectivePolicy::of(contributor)
    }

    /// An authorized read scope over the given clearance and capability set.
    fn scope_with(
        ns: &str,
        scope: &str,
        sensitivity: u8,
        owner: Option<&str>,
        caps: &[Capability],
    ) -> AuthorizedScope {
        let part = partition(ns, scope, sensitivity, owner);
        let policy = read_policy(part.clone(), caps);
        authorize_read(&caller(part), &policy).expect("read is authorized")
    }

    /// The canonical read scope for these tests (read + observe).
    fn read_scope(ns: &str, scope: &str, sensitivity: u8, owner: Option<&str>) -> AuthorizedScope {
        scope_with(
            ns,
            scope,
            sensitivity,
            owner,
            &[Capability::ReadCore, Capability::ObserveMemory],
        )
    }

    // ── (a) response captured under policy A discarded when current is B ─

    #[test]
    fn response_delivered_when_policy_unchanged() {
        let scope = read_scope("user", "chat", 2, None);
        let bound = PolicyEpoch::capture(&scope, RequestGeneration::FIRST);
        let current = PolicyEpoch::capture(&scope, RequestGeneration::FIRST);
        let guard = PolicyGuard::new(bound);

        let disposition = guard.admit_response(&current, "result-body");
        assert!(disposition.is_delivered());
        assert_eq!(disposition.into_delivered(), Some("result-body"));
    }

    #[test]
    fn response_discarded_when_policy_hash_changes() {
        // Request authorized under policy A (scope "chat")…
        let scope_a = read_scope("user", "chat", 2, None);
        let guard = PolicyGuard::capture(&scope_a, RequestGeneration::FIRST);

        // …but the caller's current scope changed to B (scope "notes") — a
        // different partition ⇒ different policy hash.
        let scope_b = read_scope("user", "notes", 2, None);
        let current = PolicyEpoch::capture(&scope_b, RequestGeneration::FIRST);

        let disposition = guard.admit_response(&current, "result-body");
        assert!(disposition.is_discarded());
        assert_eq!(
            disposition.discard_reason(),
            Some(InvalidationReason::PolicyChanged)
        );
        // The value is consumed on discard: it can never be returned.
        assert_eq!(disposition.into_delivered(), None);
    }

    #[test]
    fn response_discarded_when_capability_changes() {
        // Same caller partition, capability set narrowed ⇒ different policy hash.
        let scope_a = scope_with(
            "user",
            "chat",
            2,
            None,
            &[Capability::ReadCore, Capability::ObserveMemory],
        );
        let scope_b = scope_with("user", "chat", 2, None, &[Capability::ReadCore]);
        let guard = PolicyGuard::capture(&scope_a, RequestGeneration::FIRST);
        let current = PolicyEpoch::capture(&scope_b, RequestGeneration::FIRST);

        assert_eq!(
            guard.admit_response(&current, ()).discard_reason(),
            Some(InvalidationReason::PolicyChanged)
        );
    }

    #[test]
    fn response_discarded_when_superseded_by_newer_generation() {
        let scope = read_scope("user", "chat", 2, None);
        let guard = PolicyGuard::capture(&scope, RequestGeneration::FIRST);
        // Focus/refocus under the same policy increments the generation.
        let current = PolicyEpoch::capture(&scope, RequestGeneration::FIRST.next());

        let disposition = guard.admit_response(&current, "stale-body");
        assert!(disposition.is_discarded());
        assert_eq!(
            disposition.discard_reason(),
            Some(InvalidationReason::Superseded)
        );
    }

    #[test]
    fn trace_follows_response_discard_rule() {
        let scope_a = read_scope("user", "chat", 2, None);
        let scope_b = read_scope("work", "chat", 2, None);
        let guard = PolicyGuard::capture(&scope_a, RequestGeneration::FIRST);
        let current = PolicyEpoch::capture(&scope_b, RequestGeneration::FIRST);
        assert!(guard.admit_trace(&current, "trace").is_discarded());
    }

    // ── (b) cache entries under old policy_hash invalidated / not served ─

    struct CacheEntry {
        label: &'static str,
        policy_hash: String,
    }
    impl PolicyKeyed for CacheEntry {
        fn policy_hash(&self) -> &str {
            &self.policy_hash
        }
    }

    #[test]
    fn cache_entry_under_old_policy_is_not_servable() {
        let scope_a = read_scope("user", "chat", 2, None);
        let scope_b = read_scope("user", "notes", 2, None);

        // An entry computed under A is servable under A but not under B.
        assert!(cache_entry_servable(scope_a.policy_hash(), &scope_a));
        assert!(!cache_entry_servable(scope_a.policy_hash(), &scope_b));
    }

    #[test]
    fn invalidate_superseded_evicts_entries_under_old_policy() {
        let scope_a = read_scope("user", "chat", 2, None);
        let scope_b = read_scope("user", "notes", 2, None);

        let mut cache = vec![
            CacheEntry {
                label: "under-a-1",
                policy_hash: scope_a.policy_hash().to_string(),
            },
            CacheEntry {
                label: "under-b",
                policy_hash: scope_b.policy_hash().to_string(),
            },
            CacheEntry {
                label: "under-a-2",
                policy_hash: scope_a.policy_hash().to_string(),
            },
        ];

        // Caller now holds policy B: entries under A are invalidated.
        let evicted = invalidate_superseded(&mut cache, &scope_b);
        assert_eq!(evicted, 2);
        let remaining: Vec<&str> = cache.iter().map(|e| e.label).collect();
        assert_eq!(remaining, vec!["under-b"]);
    }

    // ── (c) stale cursor rejected with a bounded-refetch instruction ─────

    #[test]
    fn cursor_with_stale_policy_hash_is_rejected_with_bounded_refetch() {
        let scope_a = read_scope("user", "chat", 2, None);
        let scope_b = read_scope("user", "notes", 2, None);
        let rev = GraphRevision::new(7);

        let cursor = CursorGuard::capture(&scope_a, rev);
        let admission = cursor.admit(&scope_b, rev);

        assert!(!admission.is_resumable());
        let refetch = admission.refetch().expect("stale cursor yields refetch");
        assert_eq!(refetch.reason(), RefetchReason::PolicyChanged);
        assert_eq!(refetch.reason().reason_code(), "cursor_policy_changed");
    }

    #[test]
    fn cursor_with_incompatible_revision_is_rejected_with_bounded_refetch() {
        let scope = read_scope("user", "chat", 2, None);
        let cursor = CursorGuard::capture(&scope, GraphRevision::new(7));

        // Same policy, advanced revision ⇒ revision-incompatible refetch.
        let admission = cursor.admit(&scope, GraphRevision::new(8));
        assert_eq!(
            admission.refetch().map(|r| r.reason()),
            Some(RefetchReason::RevisionIncompatible)
        );
    }

    #[test]
    fn cursor_resumes_when_policy_and_revision_match() {
        let scope = read_scope("user", "chat", 2, None);
        let rev = GraphRevision::new(7);
        let cursor = CursorGuard::capture(&scope, rev);
        assert!(cursor.admit(&scope, rev).is_resumable());
    }

    // ── (d) pending write not confirmed after policy change ──────────────

    #[test]
    fn pending_write_confirmed_when_policy_unchanged() {
        let scope = read_scope("user", "chat", 2, None);
        let guard = PolicyGuard::capture(&scope, RequestGeneration::FIRST);
        let current = PolicyEpoch::capture(&scope, RequestGeneration::FIRST);
        assert_eq!(guard.resolve_pending(&current), PendingResolution::Confirm);
    }

    #[test]
    fn pending_write_rolled_back_after_policy_change() {
        let scope_a = read_scope("user", "chat", 2, Some("owner-1"));
        let scope_b = read_scope("user", "chat", 2, Some("owner-2"));
        let guard = PolicyGuard::capture(&scope_a, RequestGeneration::FIRST);
        let current = PolicyEpoch::capture(&scope_b, RequestGeneration::FIRST);

        let resolution = guard.resolve_pending(&current);
        assert!(!resolution.is_confirmed());
        assert_eq!(
            resolution.rollback_reason(),
            Some(InvalidationReason::PolicyChanged)
        );
    }

    #[test]
    fn pending_write_not_rolled_back_by_generation_alone() {
        // A newer generation supersedes reads, but does not by itself roll back a
        // durable pending write (only a genuine policy change does).
        let scope = read_scope("user", "chat", 2, None);
        let guard = PolicyGuard::capture(&scope, RequestGeneration::FIRST);
        let current = PolicyEpoch::capture(&scope, RequestGeneration::FIRST.next());
        assert_eq!(guard.resolve_pending(&current), PendingResolution::Confirm);
    }

    // ── Property: ANY identity/scope/capability/policy change ⇒ invalidate ─

    /// A distinct partition dimension to mutate, so each generated pair differs
    /// in exactly one authorization input.
    #[derive(Debug, Clone)]
    enum Mutation {
        Namespace,
        Scope,
        Sensitivity,
        Owner,
        Capability,
    }

    fn mutation_strategy() -> impl Strategy<Value = Mutation> {
        prop_oneof![
            Just(Mutation::Namespace),
            Just(Mutation::Scope),
            Just(Mutation::Sensitivity),
            Just(Mutation::Owner),
            Just(Mutation::Capability),
        ]
    }

    proptest! {
        /// **Validates: Requirements 4.6**
        ///
        /// Any change to caller identity, scope, capability, or effective policy
        /// yields a different `policy_hash` (proven in F1.4.5) and therefore an
        /// invalidated epoch: the in-flight response is discarded, the pending
        /// write is rolled back, and a stale cursor is rejected with a bounded
        /// refetch. This exercises the F1.4.6 guarantee across every
        /// authorization input.
        #[test]
        fn prop_any_authorization_change_invalidates_in_flight_state(
            mutation in mutation_strategy(),
        ) {
            // A fixed baseline authorization.
            let base = scope_with(
                "user",
                "chat",
                2,
                Some("owner-1"),
                &[Capability::ReadCore, Capability::ObserveMemory],
            );

            // The same authorization with exactly one input changed.
            let changed = match mutation {
                Mutation::Namespace => scope_with(
                    "work", "chat", 2, Some("owner-1"),
                    &[Capability::ReadCore, Capability::ObserveMemory],
                ),
                Mutation::Scope => scope_with(
                    "user", "notes", 2, Some("owner-1"),
                    &[Capability::ReadCore, Capability::ObserveMemory],
                ),
                Mutation::Sensitivity => scope_with(
                    "user", "chat", 1, Some("owner-1"),
                    &[Capability::ReadCore, Capability::ObserveMemory],
                ),
                Mutation::Owner => scope_with(
                    "user", "chat", 2, Some("owner-2"),
                    &[Capability::ReadCore, Capability::ObserveMemory],
                ),
                Mutation::Capability => scope_with(
                    "user", "chat", 2, Some("owner-1"),
                    &[Capability::ReadCore],
                ),
            };

            // The change is observable as a different policy hash (F1.4.5).
            prop_assert_ne!(base.policy_hash(), changed.policy_hash());

            let bound = PolicyEpoch::capture(&base, RequestGeneration::FIRST);
            let current = PolicyEpoch::capture(&changed, RequestGeneration::FIRST);

            // Epoch relation classifies it as a policy change.
            prop_assert_eq!(bound.relation(&current), EpochRelation::PolicyChanged);

            // Response is discarded…
            let guard = PolicyGuard::new(bound);
            prop_assert!(guard.admit_response(&current, "body").is_discarded());
            // …pending write is rolled back…
            prop_assert!(!guard.resolve_pending(&current).is_confirmed());
            // …cursor is rejected with a bounded refetch…
            let cursor = CursorGuard::capture(&base, GraphRevision::new(3));
            let admission = cursor.admit(&changed, GraphRevision::new(3));
            prop_assert_eq!(admission.refetch().map(|r| r.reason()), Some(RefetchReason::PolicyChanged));
            // …and a cache entry under the old policy is no longer servable.
            prop_assert!(!cache_entry_servable(base.policy_hash(), &changed));
        }

        /// Equal authorizations (identical identity/scope/capability/policy) hash
        /// equally and therefore remain current — no spurious invalidation.
        #[test]
        fn prop_equal_authorization_stays_current(sensitivity in 0u8..=3) {
            let a = read_scope("user", "chat", sensitivity, None);
            let b = read_scope("user", "chat", sensitivity, None);
            let bound = PolicyEpoch::capture(&a, RequestGeneration::FIRST);
            let current = PolicyEpoch::capture(&b, RequestGeneration::FIRST);
            prop_assert!(bound.is_current(&current));
            prop_assert!(guard_delivers(&bound, &current));
        }
    }

    /// Helper: whether a guard bound to `bound` delivers under `current`.
    fn guard_delivers(bound: &PolicyEpoch, current: &PolicyEpoch) -> bool {
        PolicyGuard::new(bound.clone())
            .admit_response(current, ())
            .is_delivered()
    }
}
