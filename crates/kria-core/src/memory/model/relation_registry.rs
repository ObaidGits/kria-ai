//! Relation registry value objects and loader (design §4.2/§19.3, task F2.2.1).
//!
//! The `relation_registry` table is the **single source of relation identity**
//! for the semantic graph: every `relationships` / `memory_links` row references
//! a `(relation_name, version)` here for its direction class, inverse, endpoint
//! kinds, reflexivity, validity/evidence rules, and writable disposition. No
//! parallel untyped link table is permitted (design §4.2).
//!
//! This module models one registry row as a validated [`RelationDefinition`]
//! value object (no raw unchecked strings — every closed enum is validated on
//! construction) and provides a read-only [`RelationRegistry`] loader that
//! resolves a definition by `(relation_name, version)` and resolves a free-text
//! surface form through the materialized `relation_aliases` lookup.
//!
//! Row → value-object projection lives in [`super::row_mapping::relation_definition`],
//! mirroring the 2.1.5 `row_mapping` pattern; this module owns the value objects
//! and the SQL queries that use that projector.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::{encoding_err, row_mapping, EvidencePolarity, Version};
use crate::memory::error::{MemoryResult, StorageError};
use rusqlite::Connection;

// ── RelationName ─────────────────────────────────────────────────────────

/// A validated canonical relation name (`relation_registry.relation_name`).
///
/// The canonical form is a non-empty lower-case snake identifier
/// (`[a-z][a-z0-9_]*`): this is a closed shape so a relation name can never be a
/// raw arbitrary string at the domain boundary, and it matches the seeded
/// canonical names (`derived_from`, `mentions_entity`, …).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RelationName(String);

impl RelationName {
    /// Validate and wrap a relation name. Rejects empty, leading-digit, and any
    /// character outside `[a-z0-9_]`.
    pub fn new(s: impl Into<String>) -> MemoryResult<Self> {
        let s = s.into();
        let mut chars = s.chars();
        match chars.next() {
            None => return Err(encoding_err("relation name must not be empty")),
            Some(c) if !c.is_ascii_lowercase() => {
                return Err(encoding_err(format!(
                    "relation name {s:?} must start with a lower-case ascii letter"
                )))
            }
            Some(_) => {}
        }
        if let Some(bad) = s
            .chars()
            .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_'))
        {
            return Err(encoding_err(format!(
                "relation name {s:?} contains illegal character {bad:?} (expected [a-z0-9_])"
            )));
        }
        Ok(Self(s))
    }

    /// The canonical relation-name string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the newtype, returning the owned canonical string.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for RelationName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for RelationName {
    type Err = crate::memory::error::MemoryError;
    fn from_str(s: &str) -> MemoryResult<Self> {
        Self::new(s)
    }
}

impl Serialize for RelationName {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RelationName {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(de)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

// ── DirectionClass ─────────────────────────────────────────────────────────

/// Whether a relation's endpoints are ordered (`directed`) or unordered
/// (`symmetric`) — `relation_registry.direction_class`, a **closed** set
/// matching the schema `CHECK(directed/symmetric)` (design §4.2).
///
/// A symmetric relation canonicalizes its endpoints by stable ID before the
/// identity hash so an endpoint swap preserves identity; a directed relation
/// retains order (design §19.3, MGR-018). That hashing is task 2.2.2; this type
/// only carries the classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectionClass {
    /// Endpoints are ordered: `(a → b)` differs from `(b → a)`.
    Directed,
    /// Endpoints are unordered: `(a — b)` equals `(b — a)`.
    Symmetric,
}

impl DirectionClass {
    /// The canonical text form stored in `direction_class`.
    pub fn as_str(self) -> &'static str {
        match self {
            DirectionClass::Directed => "directed",
            DirectionClass::Symmetric => "symmetric",
        }
    }

    /// All known variants.
    pub fn all() -> &'static [DirectionClass] {
        &[DirectionClass::Directed, DirectionClass::Symmetric]
    }

    /// Whether this is the symmetric (unordered-endpoint) class.
    pub fn is_symmetric(self) -> bool {
        matches!(self, DirectionClass::Symmetric)
    }
}

impl FromStr for DirectionClass {
    type Err = crate::memory::error::MemoryError;
    fn from_str(s: &str) -> MemoryResult<Self> {
        Ok(match s {
            "directed" => DirectionClass::Directed,
            "symmetric" => DirectionClass::Symmetric,
            other => return Err(encoding_err(format!("unknown direction class {other:?}"))),
        })
    }
}

impl std::fmt::Display for DirectionClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DirectionClass {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// ── ValidityPolicy ─────────────────────────────────────────────────────────

/// The Valid Time disposition a relation imposes on its relationships
/// (`relation_registry.validity_policy`). A **closed** set: whether a valid
/// interval is `optional`, `required`, or `forbidden` for the relation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidityPolicy {
    /// A valid interval may be present or absent.
    Optional,
    /// A valid interval must be present.
    Required,
    /// A valid interval must not be present.
    Forbidden,
}

impl ValidityPolicy {
    /// The canonical text form stored in `validity_policy`.
    pub fn as_str(self) -> &'static str {
        match self {
            ValidityPolicy::Optional => "optional",
            ValidityPolicy::Required => "required",
            ValidityPolicy::Forbidden => "forbidden",
        }
    }

    /// All known variants.
    pub fn all() -> &'static [ValidityPolicy] {
        &[
            ValidityPolicy::Optional,
            ValidityPolicy::Required,
            ValidityPolicy::Forbidden,
        ]
    }
}

impl FromStr for ValidityPolicy {
    type Err = crate::memory::error::MemoryError;
    fn from_str(s: &str) -> MemoryResult<Self> {
        Ok(match s {
            "optional" => ValidityPolicy::Optional,
            "required" => ValidityPolicy::Required,
            "forbidden" => ValidityPolicy::Forbidden,
            other => return Err(encoding_err(format!("unknown validity policy {other:?}"))),
        })
    }
}

impl std::fmt::Display for ValidityPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ValidityPolicy {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// ── EndpointKind ───────────────────────────────────────────────────────────

/// A legal endpoint (node) kind for a relation
/// (`relation_registry.source_kinds_json` / `target_kinds_json`). A **closed**
/// set of the graph's node kinds: the four record kinds plus the distinct
/// semantic node kinds that appear as relation endpoints (design §4.2/§19.3).
///
/// The registry is authority-controlled seed data, so an unrecognized endpoint
/// kind read back is a corruption, not a forward-compat value: [`FromStr`]
/// rejects it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointKind {
    /// A graph entity (`entities_v2`).
    Entity,
    /// A `memory` record.
    Memory,
    /// A `summary` record.
    Summary,
    /// A `skill` record.
    Skill,
    /// A `rule` record.
    Rule,
    /// An authority event (`events_v2`).
    Event,
    /// An episode (`episodes_v2`).
    Episode,
    /// A goal (`goals_v2`).
    Goal,
    /// An evidence artifact (`evidence_v2`).
    Evidence,
    /// A registry-governed relationship (a claim endpoint).
    Relationship,
}

impl EndpointKind {
    /// The canonical text form stored inside the endpoint-kind JSON arrays.
    pub fn as_str(self) -> &'static str {
        match self {
            EndpointKind::Entity => "entity",
            EndpointKind::Memory => "memory",
            EndpointKind::Summary => "summary",
            EndpointKind::Skill => "skill",
            EndpointKind::Rule => "rule",
            EndpointKind::Event => "event",
            EndpointKind::Episode => "episode",
            EndpointKind::Goal => "goal",
            EndpointKind::Evidence => "evidence",
            EndpointKind::Relationship => "relationship",
        }
    }

    /// All known variants.
    pub fn all() -> &'static [EndpointKind] {
        &[
            EndpointKind::Entity,
            EndpointKind::Memory,
            EndpointKind::Summary,
            EndpointKind::Skill,
            EndpointKind::Rule,
            EndpointKind::Event,
            EndpointKind::Episode,
            EndpointKind::Goal,
            EndpointKind::Evidence,
            EndpointKind::Relationship,
        ]
    }
}

impl FromStr for EndpointKind {
    type Err = crate::memory::error::MemoryError;
    fn from_str(s: &str) -> MemoryResult<Self> {
        Ok(match s {
            "entity" => EndpointKind::Entity,
            "memory" => EndpointKind::Memory,
            "summary" => EndpointKind::Summary,
            "skill" => EndpointKind::Skill,
            "rule" => EndpointKind::Rule,
            "event" => EndpointKind::Event,
            "episode" => EndpointKind::Episode,
            "goal" => EndpointKind::Goal,
            "evidence" => EndpointKind::Evidence,
            "relationship" => EndpointKind::Relationship,
            other => return Err(encoding_err(format!("unknown endpoint kind {other:?}"))),
        })
    }
}

impl std::fmt::Display for EndpointKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EndpointKind {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// ── EvidencePolicy ─────────────────────────────────────────────────────────

/// The evidence rule a relation imposes (`relation_registry.evidence_policy_json`,
/// encoding the design §19.3 "policy and provenance" column).
///
/// `min_evidence` is the minimum number of evidence rows required for the
/// relationship to be active; `required_polarity`, when present, constrains that
/// evidence's polarity (`supports` for `supports`, `contradicts` for
/// `contradicts`); `required_attributes` names the provenance attributes the
/// evidence must carry (e.g. `locator`, `method_version`, `rationale`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidencePolicy {
    /// Minimum evidence rows required for an active relationship.
    #[serde(default)]
    pub min_evidence: u32,
    /// Required evidence polarity, if the relation constrains it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_polarity: Option<EvidencePolarity>,
    /// Provenance attributes the evidence must carry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_attributes: Vec<String>,
}

impl EvidencePolicy {
    /// The permissive default: no minimum, no polarity constraint, no required
    /// attributes.
    pub fn none() -> Self {
        Self {
            min_evidence: 0,
            required_polarity: None,
            required_attributes: Vec::new(),
        }
    }
}

// ── RelationDefinition ─────────────────────────────────────────────────────

/// One validated `relation_registry` row (design §4.2). Every closed enum is
/// validated, so an invalid direction class / endpoint kind / validity policy is
/// unrepresentable once inside the domain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelationDefinition {
    /// Canonical relation name (part of the identity).
    pub relation_name: RelationName,
    /// Registry version (part of the identity).
    pub version: Version,
    /// Human label for forward traversal.
    pub display_forward: String,
    /// Human label for reverse traversal, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_inverse: Option<String>,
    /// Alternate surface forms materialized into `relation_aliases`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// Directed vs symmetric endpoint semantics (closed set).
    pub direction_class: DirectionClass,
    /// The paired inverse relation name (directed only; `None` for symmetric).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverse_name: Option<RelationName>,
    /// Whether an endpoint may relate to itself.
    pub reflexive: bool,
    /// Legal source endpoint kinds (closed set).
    pub source_kinds: Vec<EndpointKind>,
    /// Legal target endpoint kinds (closed set).
    pub target_kinds: Vec<EndpointKind>,
    /// Valid Time disposition (closed set).
    pub validity_policy: ValidityPolicy,
    /// Evidence rule for active relationships.
    pub evidence_policy: EvidencePolicy,
    /// Version tag of the governing policy rule set.
    pub policy_rule_version: String,
    /// Whether raw clients may author this relation directly (design §19.3: the
    /// five canonical links are `false` — only governed domain commands create
    /// them).
    pub writable: bool,
}

impl RelationDefinition {
    /// Validate the cross-field invariants the schema also enforces: a symmetric
    /// relation must not name a distinct inverse relation (it is its own
    /// inverse — design §19.3). Returns `self` for chaining.
    pub fn validate(self) -> MemoryResult<Self> {
        if self.direction_class.is_symmetric() {
            if let Some(inv) = &self.inverse_name {
                if inv != &self.relation_name {
                    return Err(encoding_err(format!(
                        "symmetric relation {:?} must not name a distinct inverse ({:?})",
                        self.relation_name.as_str(),
                        inv.as_str()
                    )));
                }
            }
        }
        Ok(self)
    }

    /// Whether `kind` is a legal source endpoint for this relation.
    pub fn allows_source(&self, kind: EndpointKind) -> bool {
        self.source_kinds.contains(&kind)
    }

    /// Whether `kind` is a legal target endpoint for this relation.
    pub fn allows_target(&self, kind: EndpointKind) -> bool {
        self.target_kinds.contains(&kind)
    }
}

// ── RelationRegistry loader ────────────────────────────────────────────────

/// A read-only view over the `relation_registry` + `relation_aliases` authority
/// tables. Every query projects rows through
/// [`row_mapping::relation_definition`], so a malformed row surfaces as a typed
/// [`StorageError::Encoding`] rather than a corrupt value object.
///
/// This is the single lookup other F2.2 subtasks (identity hashing, AuthorityTx
/// endpoint/direction/evidence validation, governed writes) consult for relation
/// identity — they never re-derive it.
#[derive(Debug, Clone, Copy)]
pub struct RelationRegistry;

impl RelationRegistry {
    /// Load a relation definition by its `(relation_name, version)` identity, or
    /// `None` if no such registry row exists.
    pub fn load(
        conn: &Connection,
        relation_name: &RelationName,
        version: Version,
    ) -> MemoryResult<Option<RelationDefinition>> {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM relation_registry \
                 WHERE relation_name = ?1 AND version = ?2",
            )
            .map_err(StorageError::Sqlite)?;
        let mut rows = stmt
            .query(rusqlite::params![
                relation_name.as_str(),
                i64::from(version.get())
            ])
            .map_err(StorageError::Sqlite)?;
        match rows.next().map_err(StorageError::Sqlite)? {
            Some(row) => Ok(Some(row_mapping::relation_definition(row)?)),
            None => Ok(None),
        }
    }

    /// Resolve a free-text surface form to its `(relation_name, version)` through
    /// the materialized `relation_aliases` lookup. The alias is normalized the
    /// same way the seeds are (lower-case, trimmed, whitespace/hyphen → `_`), so
    /// `"Related To"`, `"related-to"`, and `"related_to"` all resolve.
    pub fn resolve_alias(
        conn: &Connection,
        alias: &str,
        version: Version,
    ) -> MemoryResult<Option<RelationName>> {
        let normalized = normalize_alias(alias);
        let resolved: Option<String> = conn
            .query_row(
                "SELECT relation_name FROM relation_aliases \
                 WHERE alias = ?1 AND version = ?2",
                rusqlite::params![normalized, i64::from(version.get())],
                |r| r.get(0),
            )
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(StorageError::Sqlite)?;
        match resolved {
            Some(name) => Ok(Some(RelationName::new(name)?)),
            None => Ok(None),
        }
    }

    /// Resolve a surface form all the way to its [`RelationDefinition`], or
    /// `None` if the alias is unknown for `version`.
    pub fn resolve_definition(
        conn: &Connection,
        alias: &str,
        version: Version,
    ) -> MemoryResult<Option<RelationDefinition>> {
        match Self::resolve_alias(conn, alias, version)? {
            Some(name) => Self::load(conn, &name, version),
            None => Ok(None),
        }
    }

    /// Load every relation definition, ordered by `(relation_name, version)`.
    /// Malformed rows surface as an `Err` for that row only (MGR-034 isolation).
    pub fn all(conn: &Connection) -> MemoryResult<Vec<MemoryResult<RelationDefinition>>> {
        row_mapping::read_isolated(
            conn,
            "SELECT * FROM relation_registry ORDER BY relation_name, version",
            row_mapping::relation_definition,
        )
    }
}

/// Normalize a free-text relation surface form to its lookup key: lower-case,
/// trimmed, with runs of whitespace or hyphens collapsed to a single `_`.
pub(crate) fn normalize_alias(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_sep = false;
    for c in s.trim().chars() {
        if c.is_whitespace() || c == '-' {
            if !prev_sep && !out.is_empty() {
                out.push('_');
            }
            prev_sep = true;
        } else {
            out.extend(c.to_lowercase());
            prev_sep = false;
        }
    }
    // Trim a trailing separator produced by trailing whitespace/hyphens.
    while out.ends_with('_') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_name_accepts_canonical_and_rejects_bad() {
        assert_eq!(
            RelationName::new("derived_from").unwrap().as_str(),
            "derived_from"
        );
        assert!(RelationName::new("").is_err());
        assert!(RelationName::new("1abc").is_err());
        assert!(RelationName::new("Derived").is_err());
        assert!(RelationName::new("has-part").is_err());
        assert!(RelationName::new("has part").is_err());
        // serde round-trip with validation.
        let n = RelationName::new("part_of").unwrap();
        let back: RelationName = serde_json::from_str(&serde_json::to_string(&n).unwrap()).unwrap();
        assert_eq!(back, n);
        assert!(serde_json::from_str::<RelationName>("\"Bad Name\"").is_err());
    }

    #[test]
    fn closed_enums_roundtrip_and_reject_unknown() {
        for d in DirectionClass::all() {
            assert_eq!(DirectionClass::from_str(d.as_str()).unwrap(), *d);
        }
        assert!(DirectionClass::from_str("bidirectional").is_err());

        for v in ValidityPolicy::all() {
            assert_eq!(ValidityPolicy::from_str(v.as_str()).unwrap(), *v);
        }
        assert!(ValidityPolicy::from_str("maybe").is_err());

        for k in EndpointKind::all() {
            assert_eq!(EndpointKind::from_str(k.as_str()).unwrap(), *k);
        }
        assert!(EndpointKind::from_str("planet").is_err());
        assert!(serde_json::from_str::<EndpointKind>("\"planet\"").is_err());
    }

    #[test]
    fn symmetric_validate_rejects_distinct_inverse() {
        let base = RelationDefinition {
            relation_name: RelationName::new("related_to").unwrap(),
            version: Version::first(),
            display_forward: "related to".into(),
            display_inverse: Some("related to".into()),
            aliases: vec!["related_to".into()],
            direction_class: DirectionClass::Symmetric,
            inverse_name: Some(RelationName::new("part_of").unwrap()),
            reflexive: false,
            source_kinds: vec![EndpointKind::Entity],
            target_kinds: vec![EndpointKind::Entity],
            validity_policy: ValidityPolicy::Optional,
            evidence_policy: EvidencePolicy::none(),
            policy_rule_version: "1".into(),
            writable: true,
        };
        assert!(base.clone().validate().is_err());

        // Symmetric with no inverse (its own inverse) validates.
        let ok = RelationDefinition {
            inverse_name: None,
            ..base
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn relation_definition_serde_roundtrips() {
        let d = RelationDefinition {
            relation_name: RelationName::new("supports").unwrap(),
            version: Version::first(),
            display_forward: "supports".into(),
            display_inverse: Some("supported by".into()),
            aliases: vec!["supports".into(), "supported_by".into()],
            direction_class: DirectionClass::Directed,
            inverse_name: None,
            reflexive: false,
            source_kinds: vec![EndpointKind::Evidence],
            target_kinds: vec![EndpointKind::Memory, EndpointKind::Relationship],
            validity_policy: ValidityPolicy::Optional,
            evidence_policy: EvidencePolicy {
                min_evidence: 1,
                required_polarity: Some(EvidencePolarity::Supports),
                required_attributes: vec!["locator".into()],
            },
            policy_rule_version: "1".into(),
            writable: false,
        };
        let back: RelationDefinition =
            serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn normalize_alias_handles_case_space_hyphen() {
        assert_eq!(normalize_alias("related_to"), "related_to");
        assert_eq!(normalize_alias("Related To"), "related_to");
        assert_eq!(normalize_alias("  related-to  "), "related_to");
        assert_eq!(normalize_alias("IS  PART  OF"), "is_part_of");
    }
}

// ── DB-backed tests: fresh-create, seeds, alias resolution, CHECKs (2.2.1) ──
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::memory::db::Database;

    const V1: Version = Version::first();

    fn name(s: &str) -> RelationName {
        RelationName::new(s).unwrap()
    }

    /// The five REQUIRED canonical Memory Link registry rows (design §4.2/§19.3).
    const CANONICAL: &[&str] = &[
        "derived_from",
        "supports",
        "contradicts",
        "mentions_entity",
        "superseded_by",
    ];

    #[test]
    fn fresh_create_seeds_registry_and_aliases_tables() {
        // A fresh in-memory authority runs migrations 0018 + 0030, creating both
        // tables and seeding eight relation rows + their alias lookup:
        // 5 canonical links + 2 domain-ontology rows + 1 extraction signal row
        // (co_mentioned_with, added in migration 0030).
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        let table_exists = |t: &str| -> bool {
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [t],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
                == 1
        };
        assert!(table_exists("relation_registry"));
        assert!(table_exists("relation_aliases"));

        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM relation_registry", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            rows, 8,
            "5 canonical links + 2 domain-ontology rows + 1 extraction signal (co_mentioned_with)"
        );
    }

    #[test]
    fn all_five_canonical_links_are_seeded_directed_nonreflexive_nonwritable() {
        // The five required links MUST exist and are directed, non-reflexive,
        // and not writable by raw clients (design §19.3).
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        for n in CANONICAL {
            let def = RelationRegistry::load(&conn, &name(n), V1)
                .unwrap()
                .unwrap_or_else(|| panic!("canonical link {n} must be seeded"));
            assert_eq!(def.relation_name.as_str(), *n);
            assert_eq!(def.version, V1);
            assert_eq!(def.direction_class, DirectionClass::Directed, "{n}");
            assert!(!def.reflexive, "{n} must be non-reflexive");
            assert!(!def.writable, "{n} must not be raw-client writable");
            assert!(def.inverse_name.is_none(), "{n} registers no inverse row");
            assert!(def.display_inverse.is_some(), "{n} has a reverse label");
        }
    }

    #[test]
    fn canonical_row_attributes_match_design() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        // supports: Evidence → claim/relationship/goal; min 1 supporting evidence
        // with a locator.
        let supports = RelationRegistry::load(&conn, &name("supports"), V1)
            .unwrap()
            .unwrap();
        assert_eq!(supports.source_kinds, vec![EndpointKind::Evidence]);
        assert!(supports.allows_target(EndpointKind::Relationship));
        assert!(supports.allows_target(EndpointKind::Goal));
        assert!(!supports.allows_target(EndpointKind::Entity));
        assert_eq!(supports.evidence_policy.min_evidence, 1);
        assert_eq!(
            supports.evidence_policy.required_polarity,
            Some(EvidencePolarity::Supports)
        );
        assert_eq!(
            supports.evidence_policy.required_attributes,
            vec!["locator".to_string()]
        );
        assert_eq!(supports.validity_policy, ValidityPolicy::Optional);

        // mentions_entity: Event/Memory/… → Entity only.
        let mentions = RelationRegistry::load(&conn, &name("mentions_entity"), V1)
            .unwrap()
            .unwrap();
        assert_eq!(mentions.target_kinds, vec![EndpointKind::Entity]);
        assert!(mentions.allows_source(EndpointKind::Event));
        assert!(mentions.allows_source(EndpointKind::Memory));

        // contradicts requires contradicts-polarity evidence.
        let contra = RelationRegistry::load(&conn, &name("contradicts"), V1)
            .unwrap()
            .unwrap();
        assert_eq!(
            contra.evidence_policy.required_polarity,
            Some(EvidencePolarity::Contradicts)
        );
    }

    #[test]
    fn domain_rows_cover_symmetric_and_directed_with_inverse() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        // related_to: symmetric, writable, its own inverse (no inverse_name).
        let related = RelationRegistry::load(&conn, &name("related_to"), V1)
            .unwrap()
            .unwrap();
        assert_eq!(related.direction_class, DirectionClass::Symmetric);
        assert!(related.direction_class.is_symmetric());
        assert!(related.writable);
        assert!(related.inverse_name.is_none());

        // part_of: directed, writable, with a distinct registered inverse label.
        let part_of = RelationRegistry::load(&conn, &name("part_of"), V1)
            .unwrap()
            .unwrap();
        assert_eq!(part_of.direction_class, DirectionClass::Directed);
        assert!(part_of.writable);
        assert_eq!(part_of.inverse_name.as_ref().unwrap().as_str(), "has_part");
    }

    #[test]
    fn alias_resolution_via_materialized_lookup() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        // Canonical name resolves to itself.
        assert_eq!(
            RelationRegistry::resolve_alias(&conn, "derived_from", V1)
                .unwrap()
                .unwrap()
                .as_str(),
            "derived_from"
        );
        // A registered alias resolves to its canonical relation.
        assert_eq!(
            RelationRegistry::resolve_alias(&conn, "derives_from", V1)
                .unwrap()
                .unwrap()
                .as_str(),
            "derived_from"
        );
        // Free-text normalization: case + spaces/hyphens → the seeded key.
        assert_eq!(
            RelationRegistry::resolve_alias(&conn, "Associated With", V1)
                .unwrap()
                .unwrap()
                .as_str(),
            "related_to"
        );
        // Unknown alias resolves to None (not an error).
        assert!(
            RelationRegistry::resolve_alias(&conn, "no_such_relation", V1)
                .unwrap()
                .is_none()
        );

        // End-to-end alias → definition.
        let def = RelationRegistry::resolve_definition(&conn, "supersedes", V1)
            .unwrap()
            .unwrap();
        assert_eq!(def.relation_name.as_str(), "superseded_by");
    }

    #[test]
    fn load_unknown_name_or_version_returns_none() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        assert!(RelationRegistry::load(&conn, &name("does_not_exist"), V1)
            .unwrap()
            .is_none());
        // A seeded name at an unseeded version is absent.
        assert!(
            RelationRegistry::load(&conn, &name("supports"), Version::new(2))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn all_returns_every_seeded_row_projected() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let defs = RelationRegistry::all(&conn).unwrap();
        // 5 canonical + 2 domain-ontology + 1 extraction signal (co_mentioned_with, 0030)
        assert_eq!(defs.len(), 8);
        // Every seeded row projects to a valid definition (no isolation errors).
        for d in &defs {
            assert!(d.is_ok(), "seeded row failed projection: {d:?}");
        }
    }

    // ── schema CHECK enforcement ────────────────────────────────────────
    // Insert a full row, overriding direction_class and writable so the
    // closed-set / boolean CHECKs can be exercised directly.
    fn insert_raw(
        conn: &rusqlite::Connection,
        direction: &str,
        writable: i64,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO relation_registry
                (relation_name, version, display_forward, display_inverse, aliases_json,
                 direction_class, inverse_name, reflexive, source_kinds_json,
                 target_kinds_json, validity_policy, evidence_policy_json,
                 policy_rule_version, writable)
             VALUES ('probe', 1, 'probe', NULL, json('[]'),
                     ?1, NULL, 0, json('[\"entity\"]'),
                     json('[\"entity\"]'), 'optional', json('{}'),
                     '1', ?2)",
            rusqlite::params![direction, writable],
        )
    }

    #[test]
    fn direction_class_check_rejects_out_of_set() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        assert!(
            insert_raw(&conn, "bidirectional", 0).is_err(),
            "direction_class outside {{directed,symmetric}} must be rejected"
        );
        // A valid value is accepted.
        assert!(insert_raw(&conn, "symmetric", 0).is_ok());
    }

    #[test]
    fn writable_check_rejects_non_boolean() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        assert!(
            insert_raw(&conn, "directed", 2).is_err(),
            "writable outside (0,1) must be rejected"
        );
        assert!(insert_raw(&conn, "directed", 1).is_ok());
    }

    #[test]
    fn primary_key_prevents_duplicate_name_version() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        // derived_from@1 is already seeded; re-inserting the same identity fails.
        let dup = conn.execute(
            "INSERT INTO relation_registry
                (relation_name, version, display_forward, display_inverse, aliases_json,
                 direction_class, inverse_name, reflexive, source_kinds_json,
                 target_kinds_json, validity_policy, evidence_policy_json,
                 policy_rule_version, writable)
             VALUES ('derived_from', 1, 'x', NULL, json('[]'),
                     'directed', NULL, 0, json('[]'), json('[]'),
                     'optional', json('{}'), '1', 0)",
            [],
        );
        assert!(
            dup.is_err(),
            "duplicate (relation_name, version) must be rejected"
        );
    }
}
