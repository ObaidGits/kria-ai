//! The **Effective-Policy meet** (task **F1.4.2**; design §4.1 policy columns,
//! §2 A5 isolation; MGR-004 AC2, MGR-035, MGR-043; property suite V-POLICY-01).
//!
//! A durable write is frequently attributed to *several* contributing sources
//! (a native tool acting on an imported document, a conversation turn quoting a
//! cloud result, …). Design §4.1 requires the governed row to record the
//! **most restrictive** combination of every contributing source's policy —
//! "`effective = max(contributors)`" for sensitivity, with
//! "namespace/scope/capability intersections … computed by policy code and
//! materialized with a provenance hash", and MGR-004 AC2 names the result the
//! *Effective Policy as the most restrictive contributing policy*.
//!
//! This module implements that combination as a **meet** over a
//! [meet-semilattice](https://en.wikipedia.org/wiki/Semilattice): a binary
//! operation that is associative, commutative, and idempotent, and that is
//! never *more permissive* than either operand (monotonic restriction). The
//! meet is a closed operation on [`EffectivePolicy`] so it folds over any number
//! of contributors:
//!
//! ```text
//! meet(a, meet(b, c)) == meet(meet(a, b), c)   // associative
//! meet(a, b)          == meet(b, a)            // commutative
//! meet(a, a)          == a                     // idempotent
//! ```
//!
//! ## How each policy dimension combines
//!
//! | Dimension     | Combination rule                                             | Rationale |
//! |---------------|--------------------------------------------------------------|-----------|
//! | sensitivity   | numeric **max** of contributors                             | higher = more sensitive/restrictive (§4.1) |
//! | capabilities  | set **intersection** ([`CapabilitySet::intersection`])      | a lacked capability can never be regained; empty ⇒ **deny** |
//! | trust         | most restrictive ([`more_restrictive_trust`])               | `System < Trusted < Limited < Untrusted` (§7.3) |
//! | consent       | `Required` dominates `NotRequired`                          | if any source needs consent, the combination does (§14) |
//! | namespace     | must be **identical** across contributors, else **deny**    | A5 isolation: no silent cross-namespace combination |
//! | scope         | must be **identical** across contributors, else **deny**    | A5 isolation |
//! | owner         | `None` is unconstrained; a single owner narrows it; two distinct owners ⇒ **deny** | isolation: disjoint owners never merge |
//!
//! ## Deny is a typed outcome, never a permissive fallback
//!
//! When the meet produces an empty capability intersection or an incompatible
//! namespace / scope / owner, the result is a typed [`EffectivePolicy`] whose
//! outcome is [`PolicyOutcome::Deny`] carrying the exact [`DenyReason`]s. The
//! Write-Policy engine (F1.4.5) treats a denied Effective Policy as a hard
//! reject — there is **no** permissive default and no all-permitting identity
//! element (an empty contributor set denies with [`DenyReason::NoContributors`]).
//!
//! ## Determinism and the provenance hash
//!
//! Each [`EffectivePolicy`] carries the canonical set of
//! [`ContributingPolicy`] inputs it was built from (deduplicated, ordered) and
//! a `provenance_hash` computed with [`blake3_hex`] over that sorted canonical
//! set. Because the meet combines contributors by set-union and derives its
//! outcome as a *pure function of the resulting set*, the same contributor set
//! always yields the same outcome **and** the same provenance hash regardless
//! of the order (or nesting) in which the meet was applied.

use std::collections::{BTreeMap, BTreeSet};

use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

use crate::memory::authority::SourceTrust;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::ids::blake3_hex;
use crate::memory::model::PolicyPartition;

use super::source_trust::{
    more_restrictive_trust, CapabilitySet, ConsentRequirement, SourceProfile,
};

/// The stable policy-algebra version recorded as `policy_version` (design §4.1)
/// and mixed into the provenance hash for domain separation. Bump only when the
/// meet semantics themselves change.
pub const POLICY_VERSION: &str = "effective-policy-v2";

/// Build a canonical-encoding validation error (`StorageError::Encoding`).
fn encoding_err(msg: impl Into<String>) -> crate::memory::error::MemoryError {
    StorageError::Encoding(msg.into()).into()
}

// ─────────────────────────────────────────────────────────────────────────
// ContributingPolicy — one source's contribution to the meet
// ─────────────────────────────────────────────────────────────────────────

/// A single contributing source's policy: the [`PolicyPartition`] it writes
/// under (namespace / scope / sensitivity / owner), the [`CapabilitySet`] it may
/// contribute, its [`SourceTrust`] tier, and its [`ConsentRequirement`], tagged
/// with the `source_id` for provenance. Every field is a validated value object
/// — never a raw unchecked string.
///
/// The Effective-Policy meet intersects/maximizes these across all contributing
/// sources. This is the *input* the meet consumes; combine inputs with
/// [`EffectivePolicy::of`] / [`EffectivePolicy::meet_all`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContributingPolicy {
    source_id: String,
    partition: PolicyPartition,
    capabilities: CapabilitySet,
    trust: SourceTrust,
    consent: ConsentRequirement,
}

impl ContributingPolicy {
    /// Construct a contributing policy. Validates `source_id` is non-empty.
    pub fn new(
        source_id: impl Into<String>,
        partition: PolicyPartition,
        capabilities: CapabilitySet,
        trust: SourceTrust,
        consent: ConsentRequirement,
    ) -> MemoryResult<Self> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() {
            return Err(encoding_err(
                "contributing policy source_id must not be empty",
            ));
        }
        Ok(Self {
            source_id,
            partition,
            capabilities,
            trust,
            consent,
        })
    }

    /// Construct a contributing policy from a resolved [`SourceProfile`] and the
    /// partition the source writes under. The profile supplies the capability
    /// set, trust tier, and consent requirement; the caller supplies identity
    /// and partition.
    pub fn from_profile(
        source_id: impl Into<String>,
        partition: PolicyPartition,
        profile: &SourceProfile,
    ) -> MemoryResult<Self> {
        Self::new(
            source_id,
            partition,
            *profile.capabilities(),
            profile.trust(),
            profile.consent(),
        )
    }

    /// The contributing source's identifier.
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// The partition this source writes under.
    pub fn partition(&self) -> &PolicyPartition {
        &self.partition
    }

    /// The capabilities this source contributes.
    pub fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// This source's trust tier.
    pub fn trust(&self) -> SourceTrust {
        self.trust
    }

    /// This source's consent requirement.
    pub fn consent(&self) -> ConsentRequirement {
        self.consent
    }

    /// The canonical, deterministic key identifying this contributor. Two
    /// semantically identical contributors share a key (so they deduplicate in
    /// the meet), and the key ordering is stable (so the provenance hash is
    /// order-independent).
    fn canonical_key(&self) -> String {
        serde_json::to_string(self).expect("ContributingPolicy always serializes")
    }
}

// ─────────────────────────────────────────────────────────────────────────
// DenyReason — why an Effective Policy denies
// ─────────────────────────────────────────────────────────────────────────

/// The reason(s) a meet produced a denying [`EffectivePolicy`]. A denied policy
/// is a hard reject at the Write-Policy boundary (F1.4.5), never a permissive
/// fallback. Ordered/​hashable so reasons collect into a stable set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    /// The meet had no contributing sources (empty fold denies — there is no
    /// all-permitting identity element).
    NoContributors,
    /// Contributors disagreed on `namespace` (A5 isolation: no cross-namespace
    /// combination).
    NamespaceConflict,
    /// Contributors disagreed on `scope` (A5 isolation).
    ScopeConflict,
    /// Contributors named two distinct `owner_id`s (disjoint owners never merge).
    OwnerConflict,
    /// The capability intersection is empty — no operation is jointly permitted.
    EmptyCapabilityIntersection,
}

impl DenyReason {
    /// The canonical snake_case text (stable for audit/logging).
    pub fn as_str(self) -> &'static str {
        match self {
            DenyReason::NoContributors => "no_contributors",
            DenyReason::NamespaceConflict => "namespace_conflict",
            DenyReason::ScopeConflict => "scope_conflict",
            DenyReason::OwnerConflict => "owner_conflict",
            DenyReason::EmptyCapabilityIntersection => "empty_capability_intersection",
        }
    }
}

impl std::fmt::Display for DenyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─────────────────────────────────────────────────────────────────────────
// EffectiveGrant — the combined allow
// ─────────────────────────────────────────────────────────────────────────

/// The materialized, most-restrictive policy when the meet permits admission:
/// the agreed namespace / scope / owner, the numeric-max sensitivity, the
/// intersected capability set, the most-restrictive trust tier, and the
/// combined consent requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectiveGrant {
    namespace: String,
    scope: String,
    sensitivity: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner_id: Option<String>,
    capabilities: CapabilitySet,
    trust: SourceTrust,
    consent: ConsentRequirement,
}

impl EffectiveGrant {
    /// The agreed namespace.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The agreed scope.
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// The effective (max) sensitivity level (`0..=3`).
    pub fn sensitivity(&self) -> u8 {
        self.sensitivity
    }

    /// The effective owner, if any contributor constrained it.
    pub fn owner_id(&self) -> Option<&str> {
        self.owner_id.as_deref()
    }

    /// The intersected capability set (never empty in a grant).
    pub fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// The most-restrictive trust tier of the contributors.
    pub fn trust(&self) -> SourceTrust {
        self.trust
    }

    /// The combined consent requirement.
    pub fn consent(&self) -> ConsentRequirement {
        self.consent
    }

    /// The effective grant re-expressed as a validated [`PolicyPartition`] for
    /// materialization on the governed row.
    pub fn partition(&self) -> MemoryResult<PolicyPartition> {
        PolicyPartition::with_owner(
            self.namespace.clone(),
            self.scope.clone(),
            self.sensitivity,
            self.owner_id.clone(),
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────
// PolicyOutcome — allow or deny
// ─────────────────────────────────────────────────────────────────────────

/// The outcome of a meet: either an [`EffectiveGrant`] or a set of
/// [`DenyReason`]s. Deny is a typed, terminal outcome — not a permissive
/// default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyOutcome {
    /// Admission permitted under the combined [`EffectiveGrant`].
    Allow(EffectiveGrant),
    /// Admission denied for the given reasons (at least one).
    Deny(BTreeSet<DenyReason>),
}

// ─────────────────────────────────────────────────────────────────────────
// EffectivePolicy — the meet result
// ─────────────────────────────────────────────────────────────────────────

/// The Effective Policy: the most-restrictive combination of every contributing
/// source policy (MGR-004 AC2), materialized with a `policy_version` and a
/// deterministic `provenance_hash`.
///
/// Combine contributors with [`EffectivePolicy::of`] (one source),
/// [`EffectivePolicy::meet_all`] (N sources), or the binary
/// [`EffectivePolicy::meet`] (which is associative / commutative / idempotent
/// and never more permissive than either operand).
///
/// The outcome and provenance hash are pure functions of the deduplicated,
/// canonically ordered contributor set, so equal contributor sets always
/// compare equal regardless of construction order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectivePolicy {
    /// Canonical key → contributor. A `BTreeMap` keeps contributors
    /// deduplicated and in stable order, making the meet's set-union
    /// commutative / associative / idempotent and the provenance hash
    /// order-independent.
    contributors: BTreeMap<String, ContributingPolicy>,
    outcome: PolicyOutcome,
    provenance_hash: String,
}

impl EffectivePolicy {
    /// Lift a single contributing source into an Effective Policy. A lone
    /// contributor still denies if it carries no capabilities.
    pub fn of(contributor: ContributingPolicy) -> Self {
        Self::from_contributor_map({
            let mut map = BTreeMap::new();
            map.insert(contributor.canonical_key(), contributor);
            map
        })
    }

    /// Fold the meet over N contributing sources. An empty iterator denies with
    /// [`DenyReason::NoContributors`] (no permissive identity element).
    pub fn meet_all<I>(contributors: I) -> Self
    where
        I: IntoIterator<Item = ContributingPolicy>,
    {
        let mut map = BTreeMap::new();
        for c in contributors {
            map.insert(c.canonical_key(), c);
        }
        Self::from_contributor_map(map)
    }

    /// The Effective-Policy **meet**: the most-restrictive combination of two
    /// Effective Policies. Combines their contributor sets by union and
    /// re-derives the outcome and provenance hash from the result, so the
    /// operation is associative, commutative, idempotent, and never more
    /// permissive than either operand.
    pub fn meet(&self, other: &Self) -> Self {
        let mut map = self.contributors.clone();
        for (k, v) in &other.contributors {
            map.insert(k.clone(), v.clone());
        }
        Self::from_contributor_map(map)
    }

    /// Evaluate the outcome and provenance hash for a canonical contributor set.
    fn from_contributor_map(contributors: BTreeMap<String, ContributingPolicy>) -> Self {
        let outcome = evaluate(&contributors);
        let provenance_hash = provenance_hash(&contributors);
        Self {
            contributors,
            outcome,
            provenance_hash,
        }
    }

    /// The combined outcome (allow or deny).
    pub fn outcome(&self) -> &PolicyOutcome {
        &self.outcome
    }

    /// Whether admission is permitted.
    pub fn is_allowed(&self) -> bool {
        matches!(self.outcome, PolicyOutcome::Allow(_))
    }

    /// Whether admission is denied.
    pub fn is_denied(&self) -> bool {
        matches!(self.outcome, PolicyOutcome::Deny(_))
    }

    /// The effective grant when permitted, else `None`.
    pub fn grant(&self) -> Option<&EffectiveGrant> {
        match &self.outcome {
            PolicyOutcome::Allow(g) => Some(g),
            PolicyOutcome::Deny(_) => None,
        }
    }

    /// The deny reasons when denied, else `None`.
    pub fn deny_reasons(&self) -> Option<&BTreeSet<DenyReason>> {
        match &self.outcome {
            PolicyOutcome::Deny(r) => Some(r),
            PolicyOutcome::Allow(_) => None,
        }
    }

    /// The policy-algebra version recorded on the governed row.
    pub fn policy_version(&self) -> &'static str {
        POLICY_VERSION
    }

    /// The deterministic provenance hash over the canonical contributor set.
    pub fn provenance_hash(&self) -> &str {
        &self.provenance_hash
    }

    /// The number of distinct contributing sources.
    pub fn contributor_count(&self) -> usize {
        self.contributors.len()
    }

    /// The distinct contributing policies in canonical order.
    pub fn contributors(&self) -> impl Iterator<Item = &ContributingPolicy> {
        self.contributors.values()
    }
}

impl Serialize for EffectivePolicy {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let mut map = ser.serialize_map(None)?;
        map.serialize_entry("policy_version", POLICY_VERSION)?;
        map.serialize_entry("provenance_hash", &self.provenance_hash)?;
        match &self.outcome {
            PolicyOutcome::Allow(grant) => {
                map.serialize_entry("decision", "allow")?;
                map.serialize_entry("namespace", grant.namespace())?;
                map.serialize_entry("scope", grant.scope())?;
                map.serialize_entry("sensitivity", &grant.sensitivity())?;
                if let Some(owner) = grant.owner_id() {
                    map.serialize_entry("owner_id", owner)?;
                }
                map.serialize_entry("capabilities", &grant.capabilities)?;
                map.serialize_entry("trust", &grant.trust)?;
                map.serialize_entry("consent", &grant.consent)?;
            }
            PolicyOutcome::Deny(reasons) => {
                map.serialize_entry("decision", "deny")?;
                map.serialize_entry("deny_reasons", reasons)?;
            }
        }
        map.end()
    }
}

/// Combine two consent requirements: `Required` dominates.
fn most_restrictive_consent(a: ConsentRequirement, b: ConsentRequirement) -> ConsentRequirement {
    if a.is_required() || b.is_required() {
        ConsentRequirement::Required
    } else {
        ConsentRequirement::NotRequired
    }
}

/// Derive the meet outcome as a **pure function** of the contributor set. This
/// purity is what makes the binary meet associative / commutative / idempotent:
/// the meet unions contributor sets and calls this, so the outcome depends only
/// on the resulting set, never on the order or nesting of combination.
fn evaluate(contributors: &BTreeMap<String, ContributingPolicy>) -> PolicyOutcome {
    if contributors.is_empty() {
        let mut reasons = BTreeSet::new();
        reasons.insert(DenyReason::NoContributors);
        return PolicyOutcome::Deny(reasons);
    }

    let mut namespaces: BTreeSet<&str> = BTreeSet::new();
    let mut scopes: BTreeSet<&str> = BTreeSet::new();
    let mut owners: BTreeSet<&str> = BTreeSet::new();
    let mut sensitivity: u8 = 0;
    let mut capabilities: Option<CapabilitySet> = None;
    // Trust folds toward the most restrictive; start at the most trusted tier so
    // the max-fold can only descend.
    let mut trust = SourceTrust::System;
    let mut consent = ConsentRequirement::NotRequired;

    for c in contributors.values() {
        namespaces.insert(c.partition.namespace());
        scopes.insert(c.partition.scope());
        if let Some(owner) = c.partition.owner_id() {
            owners.insert(owner);
        }
        sensitivity = sensitivity.max(c.partition.sensitivity());
        capabilities = Some(match capabilities {
            None => c.capabilities,
            Some(acc) => acc.intersection(&c.capabilities),
        });
        trust = more_restrictive_trust(trust, c.trust);
        consent = most_restrictive_consent(consent, c.consent);
    }

    let capabilities = capabilities.unwrap_or_else(CapabilitySet::empty);

    let mut reasons = BTreeSet::new();
    if namespaces.len() > 1 {
        reasons.insert(DenyReason::NamespaceConflict);
    }
    if scopes.len() > 1 {
        reasons.insert(DenyReason::ScopeConflict);
    }
    if owners.len() > 1 {
        reasons.insert(DenyReason::OwnerConflict);
    }
    if capabilities.is_empty() {
        reasons.insert(DenyReason::EmptyCapabilityIntersection);
    }

    if !reasons.is_empty() {
        return PolicyOutcome::Deny(reasons);
    }

    // Exactly one distinct namespace / scope here; at most one owner.
    let namespace = (*namespaces.iter().next().expect("non-empty")).to_string();
    let scope = (*scopes.iter().next().expect("non-empty")).to_string();
    let owner_id = owners.iter().next().map(|o| (*o).to_string());

    PolicyOutcome::Allow(EffectiveGrant {
        namespace,
        scope,
        sensitivity,
        owner_id,
        capabilities,
        trust,
        consent,
    })
}

/// The deterministic provenance hash over the canonical, sorted contributor
/// set. `BTreeMap` iteration is sorted by canonical key, so the hash is
/// order-independent; the [`POLICY_VERSION`] prefix provides domain separation.
fn provenance_hash(contributors: &BTreeMap<String, ContributingPolicy>) -> String {
    let mut input = String::new();
    input.push_str(POLICY_VERSION);
    input.push('\n');
    for key in contributors.keys() {
        input.push_str(key);
        input.push('\n');
    }
    blake3_hex(input.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::policy::source_trust::Capability;
    use proptest::prelude::*;

    // ── Helpers ─────────────────────────────────────────────────────────

    fn partition(ns: &str, scope: &str, sensitivity: u8, owner: Option<&str>) -> PolicyPartition {
        PolicyPartition::with_owner(ns, scope, sensitivity, owner.map(|o| o.to_string())).unwrap()
    }

    fn contributor(
        id: &str,
        p: PolicyPartition,
        caps: &[Capability],
        trust: SourceTrust,
        consent: ConsentRequirement,
    ) -> ContributingPolicy {
        ContributingPolicy::new(
            id,
            p,
            CapabilitySet::from_capabilities(caps.iter().copied()),
            trust,
            consent,
        )
        .unwrap()
    }

    // ── Unit tests: meet semantics per dimension ────────────────────────

    #[test]
    fn single_contributor_grants_its_own_policy() {
        let c = contributor(
            "s1",
            partition("user", "chat", 2, None),
            &[Capability::ObserveMemory, Capability::ReadCore],
            SourceTrust::Trusted,
            ConsentRequirement::NotRequired,
        );
        let ep = EffectivePolicy::of(c);
        let grant = ep.grant().expect("single capable contributor allows");
        assert_eq!(grant.namespace(), "user");
        assert_eq!(grant.scope(), "chat");
        assert_eq!(grant.sensitivity(), 2);
        assert_eq!(grant.trust(), SourceTrust::Trusted);
        assert!(grant.capabilities().contains(Capability::ObserveMemory));
        assert_eq!(ep.contributor_count(), 1);
    }

    #[test]
    fn meet_takes_max_sensitivity_and_intersects_capabilities() {
        let a = EffectivePolicy::of(contributor(
            "a",
            partition("user", "chat", 1, None),
            &[
                Capability::ObserveMemory,
                Capability::CorrectMemory,
                Capability::ReadCore,
            ],
            SourceTrust::System,
            ConsentRequirement::NotRequired,
        ));
        let b = EffectivePolicy::of(contributor(
            "b",
            partition("user", "chat", 3, None),
            &[Capability::ObserveMemory, Capability::ReadCore],
            SourceTrust::Untrusted,
            ConsentRequirement::Required,
        ));
        let m = a.meet(&b);
        let grant = m.grant().expect("compatible partitions allow");
        // sensitivity = max(1, 3)
        assert_eq!(grant.sensitivity(), 3);
        // capabilities = intersection ⇒ {Observe, ReadCore}
        assert_eq!(
            grant.capabilities().to_vec(),
            vec![Capability::ObserveMemory, Capability::ReadCore]
        );
        // trust = most restrictive
        assert_eq!(grant.trust(), SourceTrust::Untrusted);
        // consent = Required dominates
        assert_eq!(grant.consent(), ConsentRequirement::Required);
    }

    #[test]
    fn meet_denies_on_empty_capability_intersection() {
        let a = EffectivePolicy::of(contributor(
            "a",
            partition("user", "chat", 0, None),
            &[Capability::CorrectMemory],
            SourceTrust::System,
            ConsentRequirement::NotRequired,
        ));
        let b = EffectivePolicy::of(contributor(
            "b",
            partition("user", "chat", 0, None),
            &[Capability::ObserveMemory],
            SourceTrust::System,
            ConsentRequirement::NotRequired,
        ));
        let m = a.meet(&b);
        assert!(m.is_denied());
        assert!(m
            .deny_reasons()
            .unwrap()
            .contains(&DenyReason::EmptyCapabilityIntersection));
    }

    #[test]
    fn meet_denies_on_namespace_scope_and_owner_conflict() {
        let base = |ns: &str, sc: &str, owner: Option<&str>| {
            EffectivePolicy::of(contributor(
                "s",
                partition(ns, sc, 0, owner),
                &[Capability::ObserveMemory],
                SourceTrust::System,
                ConsentRequirement::NotRequired,
            ))
        };
        // Different namespaces
        let ns = base("user", "chat", None).meet(&base("system", "chat", None));
        assert!(ns
            .deny_reasons()
            .unwrap()
            .contains(&DenyReason::NamespaceConflict));
        // Different scopes
        let sc = base("user", "chat", None).meet(&base("user", "notes", None));
        assert!(sc
            .deny_reasons()
            .unwrap()
            .contains(&DenyReason::ScopeConflict));
        // Different owners
        let ow = base("user", "chat", Some("o1")).meet(&base("user", "chat", Some("o2")));
        assert!(ow
            .deny_reasons()
            .unwrap()
            .contains(&DenyReason::OwnerConflict));
    }

    #[test]
    fn owner_none_is_narrowed_by_a_present_owner() {
        let unowned = EffectivePolicy::of(contributor(
            "a",
            partition("user", "chat", 0, None),
            &[Capability::ObserveMemory],
            SourceTrust::System,
            ConsentRequirement::NotRequired,
        ));
        let owned = EffectivePolicy::of(contributor(
            "b",
            partition("user", "chat", 0, Some("owner-1")),
            &[Capability::ObserveMemory],
            SourceTrust::System,
            ConsentRequirement::NotRequired,
        ));
        let merged = unowned.meet(&owned);
        let grant = merged.grant().expect("allow");
        assert_eq!(grant.owner_id(), Some("owner-1"));
    }

    #[test]
    fn empty_meet_denies_with_no_contributors() {
        let ep = EffectivePolicy::meet_all(std::iter::empty());
        assert!(ep.is_denied());
        assert_eq!(
            ep.deny_reasons()
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![DenyReason::NoContributors]
        );
    }

    #[test]
    fn provenance_hash_is_order_independent() {
        let a = contributor(
            "a",
            partition("user", "chat", 1, None),
            &[Capability::ObserveMemory],
            SourceTrust::System,
            ConsentRequirement::NotRequired,
        );
        let b = contributor(
            "b",
            partition("user", "chat", 2, None),
            &[Capability::ObserveMemory, Capability::ReadCore],
            SourceTrust::Trusted,
            ConsentRequirement::NotRequired,
        );
        let ab = EffectivePolicy::meet_all([a.clone(), b.clone()]);
        let ba = EffectivePolicy::meet_all([b, a]);
        assert_eq!(ab.provenance_hash(), ba.provenance_hash());
        assert_eq!(ab, ba);
    }

    // ── Property strategies ─────────────────────────────────────────────

    fn cap_strategy() -> impl Strategy<Value = CapabilitySet> {
        proptest::sample::subsequence(Capability::ALL.to_vec(), 0..=Capability::ALL.len())
            .prop_map(CapabilitySet::from_capabilities)
    }

    fn trust_strategy() -> impl Strategy<Value = SourceTrust> {
        prop_oneof![
            Just(SourceTrust::System),
            Just(SourceTrust::Trusted),
            Just(SourceTrust::Limited),
            Just(SourceTrust::Untrusted),
        ]
    }

    fn consent_strategy() -> impl Strategy<Value = ConsentRequirement> {
        prop_oneof![
            Just(ConsentRequirement::NotRequired),
            Just(ConsentRequirement::Required),
        ]
    }

    // Small partition domains so meets frequently agree (and frequently
    // conflict), exercising both allow and deny paths.
    fn contributor_strategy() -> impl Strategy<Value = ContributingPolicy> {
        (
            "[a-z]{1,6}",
            prop_oneof![Just("user"), Just("system"), Just("work")],
            prop_oneof![Just("chat"), Just("notes"), Just("code")],
            0u8..=3,
            prop_oneof![
                Just(None),
                Just(Some("o1".to_string())),
                Just(Some("o2".to_string()))
            ],
            cap_strategy(),
            trust_strategy(),
            consent_strategy(),
        )
            .prop_map(|(id, ns, scope, sens, owner, caps, trust, consent)| {
                ContributingPolicy::new(
                    format!("src-{id}"),
                    PolicyPartition::with_owner(ns, scope, sens, owner).unwrap(),
                    caps,
                    trust,
                    consent,
                )
                .unwrap()
            })
    }

    fn policy_strategy() -> impl Strategy<Value = EffectivePolicy> {
        contributor_strategy().prop_map(EffectivePolicy::of)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// V-POLICY-01: the meet is ASSOCIATIVE.
        /// **Validates: Requirements MGR-004**
        #[test]
        fn prop_meet_is_associative(
            a in policy_strategy(),
            b in policy_strategy(),
            c in policy_strategy(),
        ) {
            let left = a.meet(&b.meet(&c));
            let right = a.meet(&b).meet(&c);
            prop_assert_eq!(&left, &right);
            // provenance hash also agrees under re-association.
            prop_assert_eq!(left.provenance_hash(), right.provenance_hash());
        }

        /// V-POLICY-01: the meet is COMMUTATIVE.
        /// **Validates: Requirements MGR-004**
        #[test]
        fn prop_meet_is_commutative(
            a in policy_strategy(),
            b in policy_strategy(),
        ) {
            let ab = a.meet(&b);
            let ba = b.meet(&a);
            prop_assert_eq!(&ab, &ba);
            prop_assert_eq!(ab.provenance_hash(), ba.provenance_hash());
        }

        /// V-POLICY-01: the meet is IDEMPOTENT.
        /// **Validates: Requirements MGR-004**
        #[test]
        fn prop_meet_is_idempotent(a in policy_strategy()) {
            prop_assert_eq!(a.meet(&a), a.clone());
        }

        /// V-POLICY-01: MONOTONIC RESTRICTION — the meet is never more
        /// permissive than either operand. When the result allows, both
        /// operands allowed, the result sensitivity is >= each, its capabilities
        /// are a subset of each, and its trust is no more trusted than either.
        /// **Validates: Requirements MGR-004**
        #[test]
        fn prop_meet_is_monotonic_restriction(
            a in policy_strategy(),
            b in policy_strategy(),
        ) {
            let m = a.meet(&b);
            if let Some(mg) = m.grant() {
                let ag = a.grant().expect("allow result implies operand a allowed");
                let bg = b.grant().expect("allow result implies operand b allowed");
                // sensitivity never decreases (higher = more restrictive).
                prop_assert!(mg.sensitivity() >= ag.sensitivity());
                prop_assert!(mg.sensitivity() >= bg.sensitivity());
                // capabilities never widen.
                prop_assert!(mg.capabilities().is_subset_of(ag.capabilities()));
                prop_assert!(mg.capabilities().is_subset_of(bg.capabilities()));
                // trust never becomes more trusted (System < .. < Untrusted).
                prop_assert!(mg.trust() >= ag.trust());
                prop_assert!(mg.trust() >= bg.trust());
                // consent never becomes weaker.
                if ag.consent().is_required() || bg.consent().is_required() {
                    prop_assert!(mg.consent().is_required());
                }
            }
        }

        /// V-POLICY-01: DENY ON EMPTY INTERSECTION — contributors with disjoint
        /// capabilities, or incompatible namespace/scope, always deny.
        /// **Validates: Requirements MGR-004**
        #[test]
        fn prop_disjoint_contributors_deny(
            ns_a in prop_oneof![Just("user"), Just("system")],
            ns_b in prop_oneof![Just("user"), Just("system")],
        ) {
            // Disjoint capabilities: {Correct} ∩ {Observe} = ∅.
            let a = EffectivePolicy::of(contributor(
                "a",
                partition("user", "chat", 0, None),
                &[Capability::CorrectMemory],
                SourceTrust::System,
                ConsentRequirement::NotRequired,
            ));
            let b = EffectivePolicy::of(contributor(
                "b",
                partition("user", "chat", 0, None),
                &[Capability::ObserveMemory],
                SourceTrust::System,
                ConsentRequirement::NotRequired,
            ));
            prop_assert!(a.meet(&b).is_denied());

            // Namespace disagreement denies; agreement with a shared capability allows.
            let x = EffectivePolicy::of(contributor(
                "x",
                partition(ns_a, "chat", 0, None),
                &[Capability::ObserveMemory],
                SourceTrust::System,
                ConsentRequirement::NotRequired,
            ));
            let y = EffectivePolicy::of(contributor(
                "y",
                partition(ns_b, "chat", 0, None),
                &[Capability::ObserveMemory],
                SourceTrust::System,
                ConsentRequirement::NotRequired,
            ));
            let m = x.meet(&y);
            if ns_a == ns_b {
                prop_assert!(m.is_allowed());
            } else {
                prop_assert!(m.is_denied());
                prop_assert!(m.deny_reasons().unwrap().contains(&DenyReason::NamespaceConflict));
            }
        }

        /// V-POLICY-01: the provenance hash is DETERMINISTIC and
        /// ORDER-INDEPENDENT for a given contributor set.
        /// **Validates: Requirements MGR-004**
        #[test]
        fn prop_provenance_hash_is_order_independent(
            contributors in proptest::collection::vec(contributor_strategy(), 1..6),
        ) {
            let forward = EffectivePolicy::meet_all(contributors.clone());
            let mut reversed = contributors.clone();
            reversed.reverse();
            let backward = EffectivePolicy::meet_all(reversed);
            prop_assert_eq!(forward.provenance_hash(), backward.provenance_hash());
            prop_assert_eq!(&forward, &backward);

            // Re-computing over the identical set is stable.
            let again = EffectivePolicy::meet_all(contributors);
            prop_assert_eq!(forward.provenance_hash(), again.provenance_hash());
        }
    }
}
