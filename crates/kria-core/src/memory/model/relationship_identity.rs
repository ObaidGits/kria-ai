//! Canonical semantic identity hash for a relationship (design §4.2 `identity_hash`,
//! §19.3 endpoint canonicalization, task F2.2.2).
//!
//! The `relationships` table (design §4.2) carries `identity_hash TEXT NOT NULL`
//! with a **unique active `identity_hash` where not superseded/deleted**
//! constraint: it is the "same edge" key that lets the authority (task 2.2.3+)
//! tell an idempotent replay / additional supporting observation (append
//! Evidence, MGR-005 AC4) apart from a genuinely new semantic edge.
//!
//! ## What is hashed (identity-relevant fields only)
//!
//! [`RelationshipIdentity::compute`] is a deterministic, order-independent
//! digest over exactly the fields design §4.2/§19.3 name as identity-bearing:
//!
//! 1. **Relation identity** — `relation_name` + registry `version`
//!    ([`RelationDefinition::relation_name`] / [`RelationDefinition::version`]).
//!    A registry version bump is a new relation identity (design §19.3: "Relation
//!    version changes create a new relationship version"), so it changes the hash.
//! 2. **Endpoints** — `(source_kind, source_id)` / `(target_kind, target_id)`.
//!    * **Directed** ([`DirectionClass::Directed`]) relations retain source→target
//!      order: swapping source and target changes the hash.
//!    * **Symmetric** ([`DirectionClass::Symmetric`]) relations canonicalize the
//!      two endpoints by stable id *before* hashing (design §19.3: "canonicalizes
//!      endpoints by stable ID before the identity hash so an endpoint swap
//!      preserves identity"), so swapping source and target does **not** change
//!      the hash.
//! 3. **Validity identity** — `valid_from` only (see "Validity identity" below).
//! 4. **Policy partition** — [`PolicyPartition::partition_key`]
//!    (`namespace/scope/sensitivity`; owner is deliberately excluded — see
//!    below), per A5 policy isolation: the same claim stored under a different
//!    partition is a different semantic edge.
//!
//! `direction_class` itself is not a separate hashed field: it is fully
//! determined by `(relation_name, version)` (the registry row is the single
//! source of relation identity, design §4.2), so it is already implied by the
//! relation-identity component. It *does* control which of the two endpoint
//! orderings gets hashed (the canonicalization step above).
//!
//! ## Validity identity — precise definition
//!
//! Design §4.2 models Valid Time as a half-open `[valid_from, valid_until)`
//! interval, and §19.3 records that closing/superseding a relationship sets
//! `valid_until` on the *existing* row rather than creating a new claim
//! ("closes predecessor at successor valid start when known"). That means
//! `valid_until` is naturally mutable over the lifetime of one semantic claim —
//! it is a closure fact about the same edge, not a distinguishing fact between
//! two different edges. `valid_from`, by contrast, anchors *when the claim
//! became true* and is what distinguishes two otherwise-identical claims that
//! hold at different times (e.g. two distinct employment stints for the same
//! person at the same company are different semantic edges precisely because
//! they start at different times).
//!
//! Therefore **validity identity = `valid_from` only** (`valid_until` is
//! intentionally excluded from the hash). A `None` `valid_from` (unknown/open
//! past) is its own stable identity bucket for that relation/endpoint/policy
//! combination.
//!
//! ## Policy partition — owner excluded
//!
//! [`PolicyPartition::partition_key`] is `namespace/scope/sensitivity`; the
//! optional `owner_id` is a separate column and is **not** part of the identity
//! hash, mirroring how [`PolicyPartition`] itself defines its own partition key
//! and how [`super::super::authority::command::CommandHash`] uses
//! `caller.partition_key()` (not a raw owner-qualified string) for its
//! partition component.
//!
//! ## Canonicalization pattern
//!
//! This mirrors [`EffectivePolicy::provenance_hash`](crate::memory::policy::effective_policy::EffectivePolicy::provenance_hash)
//! and [`CommandHash::compute`](crate::memory::authority::command::CommandHash):
//! a version-tagged preimage built as a `serde_json::Map` (whose default
//! `BTreeMap` backing sorts keys, so field-insertion order never affects the
//! hash) is serialized and digested with [`blake3_hex`].
//!
//! ## Scope of this task (F2.2.2 only)
//!
//! This module defines the identity-hash **algorithm and types** only. It does
//! **not** create the `relationships` / `memory_links` tables (design §4.2) —
//! those, plus AuthorityTx endpoint/direction/reflexivity/Valid-Time/Evidence/
//! policy validation, governed writes, and the "append evidence vs. new edge"
//! decision that consumes this hash, are task 2.2.3+ (deferred here exactly as
//! migration `0018_relation_registry_v2.sql` already documents).

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::provenance::validated_field;
use super::relation_registry::{DirectionClass, EndpointKind, RelationDefinition};
use super::{PolicyPartition, ValidInterval};
use crate::memory::error::MemoryResult;
use crate::memory::ids::blake3_hex;

/// Maximum length (bytes) of a [`RelationEndpoint`] id. Bounded so an endpoint
/// reference can never become a content dump (mirrors
/// [`super::provenance::PROV_FIELD_MAX_LEN`]).
pub const RELATION_ENDPOINT_ID_MAX_LEN: usize = 1024;

/// Version tag mixed into the identity-hash preimage. Bump only if the hashed
/// field set changes, so an old stored `identity_hash` never silently collides
/// with a new hashing scheme (mirrors
/// [`crate::memory::authority::command::CommandHash`]'s schema tag).
const IDENTITY_HASH_SCHEMA: u32 = 1;

// ── RelationEndpoint ────────────────────────────────────────────────────────

/// One polymorphic relationship endpoint: an [`EndpointKind`] plus a bounded,
/// control-character-free stable id. This is the typed counterpart of the
/// `relationships.source_kind`/`source_id` (and `target_*`) columns (design
/// §4.2) — never a raw unchecked `(String, String)` pair at the identity
/// boundary.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RelationEndpoint {
    kind: EndpointKind,
    id: String,
}

impl RelationEndpoint {
    /// Validate and construct an endpoint. `id` must be non-empty, bounded, and
    /// free of control characters (the same structural-reference rule as
    /// [`super::provenance::Actor`] / [`super::provenance::ParentRef::endpoint`]).
    pub fn new(kind: EndpointKind, id: impl Into<String>) -> MemoryResult<Self> {
        let id = validated_field("relation endpoint id", id, RELATION_ENDPOINT_ID_MAX_LEN)?;
        Ok(Self { kind, id })
    }

    /// The endpoint's node kind.
    pub fn kind(&self) -> EndpointKind {
        self.kind
    }

    /// The endpoint's stable id.
    pub fn id(&self) -> &str {
        &self.id
    }
}

// ── RelationshipIdentity ─────────────────────────────────────────────────────

/// The deterministic canonical semantic identity hash for a relationship
/// (`relationships.identity_hash`, design §4.2). Hex-encoded BLAKE3 (design §14
/// hashing), computed by [`RelationshipIdentity::compute`].
///
/// Two commands whose [`RelationshipIdentity`] agree refer to the *same*
/// semantic edge (append Evidence, MGR-005 AC4); a different hash is a
/// different edge. Comparison is byte-for-byte string equality — this type is
/// used both to compute a hash when creating a relationship and to check a new
/// command's identity against an existing relationship's stored
/// `identity_hash` column (`self.as_str() == stored_identity_hash`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct RelationshipIdentity(String);

impl RelationshipIdentity {
    /// Compute the canonical identity hash for a relationship instance.
    ///
    /// * `relation` — the resolved [`RelationDefinition`] (its
    ///   `relation_name` + `version` are the relation-identity component; its
    ///   `direction_class` selects endpoint canonicalization vs. order
    ///   preservation).
    /// * `source` / `target` — the two endpoints as authored by the caller
    ///   (pre-canonicalization order; this function performs the symmetric
    ///   normalization).
    /// * `validity` — the relationship's Valid Time interval; only
    ///   `valid_from` is identity-relevant (see module docs).
    /// * `policy` — the policy partition the relationship is stored under;
    ///   only `partition_key()` (`namespace/scope/sensitivity`) is
    ///   identity-relevant (owner is excluded).
    pub fn compute(
        relation: &RelationDefinition,
        source: &RelationEndpoint,
        target: &RelationEndpoint,
        validity: &ValidInterval,
        policy: &PolicyPartition,
    ) -> Self {
        let (a, b) = canonicalize_endpoints(relation.direction_class, source, target);

        let mut preimage = Map::new();
        preimage.insert("v".into(), Value::from(IDENTITY_HASH_SCHEMA));
        preimage.insert(
            "relation_name".into(),
            Value::from(relation.relation_name.as_str()),
        );
        preimage.insert(
            "relation_version".into(),
            Value::from(relation.version.get()),
        );
        preimage.insert("source_kind".into(), Value::from(a.kind.as_str()));
        preimage.insert("source_id".into(), Value::from(a.id.as_str()));
        preimage.insert("target_kind".into(), Value::from(b.kind.as_str()));
        preimage.insert("target_id".into(), Value::from(b.id.as_str()));
        preimage.insert(
            "valid_from".into(),
            match validity.valid_from() {
                Some(ts) => Value::from(ts.to_rfc3339()),
                None => Value::Null,
            },
        );
        preimage.insert(
            "policy_partition".into(),
            Value::from(policy.partition_key()),
        );

        // `preimage` is a `serde_json::Map`, `BTreeMap`-backed by default (this
        // workspace does not enable serde_json's `preserve_order` feature), so
        // it already serializes with sorted keys regardless of insertion
        // order — mirroring `CommandHash::compute`'s canonicalization.
        let bytes = serde_json::to_vec(&Value::Object(preimage))
            .expect("canonical identity preimage always serializes");
        RelationshipIdentity(blake3_hex(&bytes))
    }

    /// The hex digest string (as stored in `relationships.identity_hash`).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the newtype, returning the owned hex digest string.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for RelationshipIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Canonicalize the endpoint order for hashing: a symmetric relation sorts the
/// two endpoints by stable id (kind as a tie-break, for the degenerate case of
/// two different-kind endpoints sharing an id) so an endpoint swap produces the
/// identical pair; a directed relation always preserves source→target order
/// (design §19.3).
fn canonicalize_endpoints<'a>(
    direction_class: DirectionClass,
    source: &'a RelationEndpoint,
    target: &'a RelationEndpoint,
) -> (&'a RelationEndpoint, &'a RelationEndpoint) {
    if !direction_class.is_symmetric() {
        return (source, target);
    }
    let source_key = (source.id.as_str(), source.kind.as_str());
    let target_key = (target.id.as_str(), target.kind.as_str());
    if source_key <= target_key {
        (source, target)
    } else {
        (target, source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::model::relation_registry::{EvidencePolicy, RelationName, ValidityPolicy};
    use crate::memory::model::{UtcTimestamp, Version};
    use proptest::prelude::*;

    /// Build a minimal [`RelationDefinition`] for identity-hash tests. Only
    /// `relation_name`, `version`, and `direction_class` are identity-relevant;
    /// the rest are filled with permissive defaults (mirrors the
    /// `relation_registry` test helpers).
    fn definition(name: &str, version: u32, direction: DirectionClass) -> RelationDefinition {
        RelationDefinition {
            relation_name: RelationName::new(name).unwrap(),
            version: Version::new(version),
            display_forward: name.into(),
            display_inverse: None,
            aliases: Vec::new(),
            direction_class: direction,
            inverse_name: None,
            reflexive: true,
            source_kinds: vec![EndpointKind::Entity],
            target_kinds: vec![EndpointKind::Entity],
            validity_policy: ValidityPolicy::Optional,
            evidence_policy: EvidencePolicy::none(),
            policy_rule_version: "1".into(),
            writable: true,
        }
    }

    fn endpoint(id: &str) -> RelationEndpoint {
        RelationEndpoint::new(EndpointKind::Entity, id).unwrap()
    }

    fn partition() -> PolicyPartition {
        PolicyPartition::new("user", "chat", 0).unwrap()
    }

    fn open_validity() -> ValidInterval {
        ValidInterval::open()
    }

    #[test]
    fn endpoint_rejects_empty_and_control_and_oversized() {
        assert!(RelationEndpoint::new(EndpointKind::Entity, "").is_err());
        assert!(RelationEndpoint::new(EndpointKind::Entity, "bad\nid").is_err());
        let oversized = "x".repeat(RELATION_ENDPOINT_ID_MAX_LEN + 1);
        assert!(RelationEndpoint::new(EndpointKind::Entity, oversized).is_err());
        assert!(RelationEndpoint::new(EndpointKind::Entity, "ok-id").is_ok());
    }

    #[test]
    fn symmetric_relation_swap_preserves_identity() {
        let def = definition("related_to", 1, DirectionClass::Symmetric);
        let a = endpoint("aaa");
        let b = endpoint("bbb");
        let h1 = RelationshipIdentity::compute(&def, &a, &b, &open_validity(), &partition());
        let h2 = RelationshipIdentity::compute(&def, &b, &a, &open_validity(), &partition());
        assert_eq!(h1, h2, "symmetric relation must ignore endpoint order");
    }

    #[test]
    fn directed_relation_swap_changes_identity() {
        let def = definition("part_of", 1, DirectionClass::Directed);
        let a = endpoint("aaa");
        let b = endpoint("bbb");
        let h1 = RelationshipIdentity::compute(&def, &a, &b, &open_validity(), &partition());
        let h2 = RelationshipIdentity::compute(&def, &b, &a, &open_validity(), &partition());
        assert_ne!(h1, h2, "directed relation must preserve orientation");
    }

    #[test]
    fn determinism_same_inputs_same_hash() {
        let def = definition("supports", 1, DirectionClass::Directed);
        let a = endpoint("aaa");
        let b = endpoint("bbb");
        let h1 = RelationshipIdentity::compute(&def, &a, &b, &open_validity(), &partition());
        let h2 = RelationshipIdentity::compute(&def, &a, &b, &open_validity(), &partition());
        assert_eq!(h1, h2);
        assert_eq!(h1.as_str().len(), 64, "blake3 hex digest is 64 chars");
    }

    #[test]
    fn sensitivity_to_relation_name_version_validity_and_policy() {
        let base_def = definition("supports", 1, DirectionClass::Directed);
        let a = endpoint("aaa");
        let b = endpoint("bbb");
        let base = RelationshipIdentity::compute(&base_def, &a, &b, &open_validity(), &partition());

        // Different relation_name.
        let other_name = definition("contradicts", 1, DirectionClass::Directed);
        assert_ne!(
            base,
            RelationshipIdentity::compute(&other_name, &a, &b, &open_validity(), &partition())
        );

        // Different registry version.
        let other_version = definition("supports", 2, DirectionClass::Directed);
        assert_ne!(
            base,
            RelationshipIdentity::compute(&other_version, &a, &b, &open_validity(), &partition())
        );

        // Different valid_from.
        let from1 = UtcTimestamp::from_rfc3339_utc("2026-01-01T00:00:00Z").unwrap();
        let from2 = UtcTimestamp::from_rfc3339_utc("2027-01-01T00:00:00Z").unwrap();
        let v1 = ValidInterval::new(Some(from1), None).unwrap();
        let v2 = ValidInterval::new(Some(from2), None).unwrap();
        assert_ne!(
            RelationshipIdentity::compute(&base_def, &a, &b, &v1, &partition()),
            RelationshipIdentity::compute(&base_def, &a, &b, &v2, &partition())
        );

        // Different policy partition.
        let other_partition = PolicyPartition::new("system", "chat", 0).unwrap();
        assert_ne!(
            base,
            RelationshipIdentity::compute(&base_def, &a, &b, &open_validity(), &other_partition)
        );
    }

    #[test]
    fn valid_until_alone_does_not_change_identity() {
        // Only `valid_from` is identity-relevant (module docs "Validity identity").
        let def = definition("supports", 1, DirectionClass::Directed);
        let a = endpoint("aaa");
        let b = endpoint("bbb");
        let from = UtcTimestamp::from_rfc3339_utc("2026-01-01T00:00:00Z").unwrap();
        let until1 = UtcTimestamp::from_rfc3339_utc("2026-06-01T00:00:00Z").unwrap();
        let until2 = UtcTimestamp::from_rfc3339_utc("2026-12-01T00:00:00Z").unwrap();
        let v1 = ValidInterval::new(Some(from), Some(until1)).unwrap();
        let v2 = ValidInterval::new(Some(from), Some(until2)).unwrap();
        assert_eq!(
            RelationshipIdentity::compute(&def, &a, &b, &v1, &partition()),
            RelationshipIdentity::compute(&def, &a, &b, &v2, &partition()),
            "closing/extending valid_until must not mint a new identity for the same claim"
        );
    }

    // ── Property tests ──────────────────────────────────────────────────

    fn direction_strategy() -> impl Strategy<Value = DirectionClass> {
        prop_oneof![
            Just(DirectionClass::Directed),
            Just(DirectionClass::Symmetric),
        ]
    }

    fn endpoint_strategy() -> impl Strategy<Value = RelationEndpoint> {
        ("[a-z0-9]{1,12}").prop_map(|id| endpoint(&id))
    }

    fn definition_strategy() -> impl Strategy<Value = RelationDefinition> {
        ("[a-z][a-z0-9_]{0,10}", 1u32..5, direction_strategy())
            .prop_map(|(name, version, direction)| definition(&name, version, direction))
    }

    fn partition_strategy() -> impl Strategy<Value = PolicyPartition> {
        (
            prop_oneof![Just("user"), Just("system"), Just("work")],
            prop_oneof![Just("chat"), Just("notes"), Just("code")],
            0u8..=3,
        )
            .prop_map(|(ns, scope, sens)| PolicyPartition::new(ns, scope, sens).unwrap())
    }

    fn valid_from_strategy() -> impl Strategy<Value = Option<UtcTimestamp>> {
        prop_oneof![
            Just(None),
            (2020i32..2030, 1u32..13, 1u32..28).prop_map(|(y, m, d)| {
                Some(
                    UtcTimestamp::from_rfc3339_utc(&format!("{y:04}-{m:02}-{d:02}T00:00:00Z"))
                        .unwrap(),
                )
            }),
        ]
    }

    proptest! {
        /// SYMMETRIC identity is order-independent; DIRECTED identity preserves
        /// orientation (differs on swap whenever the two endpoints differ).
        /// **Validates: Requirements MGR-005, MGR-018**
        #[test]
        fn prop_symmetric_swap_invariant_directed_swap_sensitive(
            def in definition_strategy(),
            a in endpoint_strategy(),
            b in endpoint_strategy(),
            valid_from in valid_from_strategy(),
            policy in partition_strategy(),
        ) {
            let validity = ValidInterval::new(valid_from, None).unwrap();
            let forward = RelationshipIdentity::compute(&def, &a, &b, &validity, &policy);
            let backward = RelationshipIdentity::compute(&def, &b, &a, &validity, &policy);

            if def.direction_class.is_symmetric() {
                prop_assert_eq!(forward, backward);
            } else if a != b {
                prop_assert_ne!(forward, backward);
            }
        }

        /// DETERMINISM: identical inputs always compute the identical hash.
        /// **Validates: Requirements MGR-005**
        #[test]
        fn prop_determinism(
            def in definition_strategy(),
            a in endpoint_strategy(),
            b in endpoint_strategy(),
            valid_from in valid_from_strategy(),
            policy in partition_strategy(),
        ) {
            let validity = ValidInterval::new(valid_from, None).unwrap();
            let h1 = RelationshipIdentity::compute(&def, &a, &b, &validity, &policy);
            let h2 = RelationshipIdentity::compute(&def, &a, &b, &validity, &policy);
            prop_assert_eq!(h1, h2);
        }

        /// SENSITIVITY: changing relation_name, version, valid_from, or policy
        /// partition (holding the rest fixed) changes the hash.
        /// **Validates: Requirements MGR-005, MGR-018**
        #[test]
        fn prop_sensitive_to_each_identity_field(
            name in "[a-z][a-z0-9_]{0,10}",
            other_name in "[a-z][a-z0-9_]{0,10}",
            version in 1u32..5,
            other_version in 1u32..5,
            direction in direction_strategy(),
            a in endpoint_strategy(),
            b in endpoint_strategy(),
            policy in partition_strategy(),
            other_policy in partition_strategy(),
        ) {
            let validity = ValidInterval::open();
            let def = definition(&name, version, direction);
            let base = RelationshipIdentity::compute(&def, &a, &b, &validity, &policy);

            if other_name != name {
                let d2 = definition(&other_name, version, direction);
                prop_assert_ne!(
                    &base,
                    &RelationshipIdentity::compute(&d2, &a, &b, &validity, &policy)
                );
            }
            if other_version != version {
                let d2 = definition(&name, other_version, direction);
                prop_assert_ne!(
                    &base,
                    &RelationshipIdentity::compute(&d2, &a, &b, &validity, &policy)
                );
            }
            if other_policy != policy {
                prop_assert_ne!(
                    &base,
                    &RelationshipIdentity::compute(&def, &a, &b, &validity, &other_policy)
                );
            }

            // valid_from sensitivity, checked independently with a fixed pair
            // of distinct timestamps.
            let from1 = UtcTimestamp::from_rfc3339_utc("2026-01-01T00:00:00Z").unwrap();
            let from2 = UtcTimestamp::from_rfc3339_utc("2027-01-01T00:00:00Z").unwrap();
            let v1 = ValidInterval::new(Some(from1), None).unwrap();
            let v2 = ValidInterval::new(Some(from2), None).unwrap();
            prop_assert_ne!(
                RelationshipIdentity::compute(&def, &a, &b, &v1, &policy),
                RelationshipIdentity::compute(&def, &a, &b, &v2, &policy)
            );
        }
    }
}
