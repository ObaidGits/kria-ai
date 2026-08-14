//! Governed relationship command validation gate (task **F2.2.3**, design
//! §4.2/§5, §19.12; MGR-005 AC2, MGR-002, MGR-018; MGD-008–MGD-009).
//!
//! This is the **pre-transaction validation stage** a governed relationship
//! command (create/edit/confirm/expire/delete/restore/undo — task 2.2.5) must
//! pass *before* the semantic write, mirroring the F1.3.2
//! [`super::validation::CommandValidator`] pattern: it consumes a typed
//! request, runs a fixed deterministic sequence of checks, and produces a
//! typed [`RelationshipValidationOutcome`] — [`Proceed`](RelationshipValidationOutcome::Proceed)
//! (carrying the resolved [`RelationDefinition`], the governing
//! [`PolicyPartition`], and the computed [`RelationshipIdentity`]) or
//! [`Rejected`](RelationshipValidationOutcome::Rejected) (carrying every
//! applicable [`RelationshipRejectionReason`]).
//!
//! Design §19.12: "Mixed-kind endpoint existence, policy meet, relation
//! endpoint kinds, non-reflexivity, evidence minimums, revision continuity,
//! and graph-change completeness are checked by `AuthorityTx` immediately
//! before commit because polymorphic foreign keys cannot express them
//! safely." This module implements that check set (endpoint existence/kinds,
//! policy meet, reflexivity, evidence minimums) as a **read-only** stage —
//! consistent with "validate before BEGIN" — so a later `AuthorityTx` command
//! stage (task 2.2.5) can call it and then open the transaction only on
//! [`RelationshipValidationOutcome::Proceed`].
//!
//! ## The nine checks (task 2.2.3 scope)
//!
//! 1. **Endpoint existence/kinds** — [`RelationshipValidator::check_endpoint_kinds`]
//!    (pure, direction-aware kind legality) +
//!    [`RelationshipValidator::check_existence`] (I/O: the referenced row
//!    actually exists in its owning v2 table for the declared kind).
//! 2. **Canonical entity IDs** — [`RelationshipValidator::check_canonical_ids`].
//!    The schema mandates canonical lower-case UUID text as the primary key of
//!    *every* endpoint-owning table (`entities_v2`, `records`, `events_v2`,
//!    `episodes_v2`, `goals_v2`, `evidence_v2`, `relationships_v2`) — not only
//!    `entities_v2` — so this check validates the UUID shape uniformly across
//!    every [`EndpointKind`], reusing [`canonical_uuid`] (the same validator
//!    [`RecordId`]/[`EntityId`]/… construct through), rather than special-casing
//!    `EndpointKind::Entity`.
//! 3. **Relation alias resolution** — [`RelationshipValidator::validate`]'s
//!    first step, via [`EndpointReads::resolve_relation`] (backed by
//!    [`RelationRegistry::resolve_definition`], which already accepts a
//!    canonical name *or* a free-text alias — the seeded canonical names are
//!    also materialized into `relation_aliases` as aliases of themselves).
//! 4. **Direction** — folded into [`RelationshipValidator::check_endpoint_kinds`]:
//!    a [`DirectionClass::Symmetric`] relation accepts either endpoint
//!    ordering (source/target roles are canonicalized later, task 2.2.2); a
//!    [`DirectionClass::Directed`] relation requires the *declared* order to
//!    satisfy `source_kinds`/`target_kinds` — swapping is never silently
//!    tolerated for a directed relation.
//! 5. **Reflexivity** — [`RelationshipValidator::check_reflexivity`]: same
//!    kind + same canonical id with `relation.reflexive == false` rejects.
//! 6. **Valid Time** — [`RelationshipValidator::check_valid_time`]: enforces
//!    [`ValidityPolicy`] against the supplied [`ValidInterval`].
//! 7. **Evidence** — [`RelationshipValidator::check_evidence`]: enforces
//!    [`EvidencePolicy`] (`min_evidence`, `required_polarity`,
//!    `required_attributes`) against the supplied evidence descriptors. This
//!    is **policy compliance only** — appending the actual `evidence_v2` row is
//!    task 2.2.4.
//! 8. **Capability** — [`RelationshipValidator::check_capability`]: the caller
//!    must carry the [`Capability`] the invoking governed command requires.
//!    No new capability constant is introduced: relationship edges are memory
//!    graph rows, so the existing generic write capabilities already
//!    generalize (`ObserveMemory` for create, `CorrectMemory` for
//!    edit/confirm, `ForgetMemory` for expire/delete/restore) — the *specific*
//!    capability is supplied by the invoking command builder (task 2.2.5) via
//!    [`RelationshipWriteRequest::required_capability`], this gate only checks
//!    membership.
//! 9. **Effective Policy** — [`RelationshipValidator::validate`]'s final step:
//!    runs the F1.4 [`EffectivePolicy`] meet over the caller's own
//!    [`ContributingPolicy`] plus one policy-only contributor per existing
//!    endpoint (its stored [`PolicyPartition`]), denying on empty intersection
//!    (design: "deny on empty intersection per F1.4.2"). An endpoint is not a
//!    *source* in the trust/capability sense, so its synthetic contributor
//!    carries the full [`Capability::ALL`] set, [`SourceTrust::System`], and
//!    [`ConsentRequirement::NotRequired`] — the three dimensions a stored graph
//!    row cannot meaningfully restrict — while its namespace/scope/sensitivity/
//!    owner still participate in the meet exactly as any other contributor
//!    (A5 isolation: linking across an incompatible namespace/scope denies).
//!
//! ## Deterministic order
//!
//! Mirroring [`super::validation::CommandValidator::validate`]: relation
//! resolution (#3) runs first because every other check needs the resolved
//! [`RelationDefinition`] — an unresolvable relation short-circuits with a
//! single [`RelationshipRejectionCode::UnknownRelation`] reason. The remaining
//! pure checks (#2, #1-kind/#4, #5, #6, #7, #8) and the I/O existence check
//! (#1-existence) then all run and collect *every* applicable reason, mirroring
//! "checks evaluated so every applicable reason is reported together". The
//! Effective-Policy meet (#9) is the most expensive step (it depends on real
//! endpoint policies) and only runs once every earlier reason list is empty.
//!
//! ## Explicitly out of scope (later F2.2 subtasks)
//!
//! Evidence row append (2.2.4), the governed create/edit/confirm/expire/
//! delete/restore/undo commands themselves (2.2.5), legacy free-text
//! relationship migration/reconciliation (2.2.6), and legacy relationship
//! table deletion (2.2.7). This module is the **validation gate only**,
//! callable by those later commands; it performs no writes.

use std::collections::BTreeSet;
use std::sync::Arc;

use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use crate::authority::SourceTrust;
use crate::db::encoding::canonical_uuid;
use crate::db::Database;
use crate::error::MemoryResult;
use crate::model::relation_registry::{
    EvidencePolicy, RelationDefinition, RelationRegistry, ValidityPolicy,
};
use crate::model::relationship_identity::{RelationEndpoint, RelationshipIdentity};
use crate::model::{
    EndpointKind, EvidencePolarity, PolicyPartition, ValidInterval, Version,
};
use crate::policy::effective_policy::{ContributingPolicy, EffectivePolicy, PolicyOutcome};
use crate::policy::source_trust::{Capability, CapabilitySet, ConsentRequirement};

// ─────────────────────────────────────────────────────────────────────────
// Rejection reason codes
// ─────────────────────────────────────────────────────────────────────────

/// A stable rejection reason code for the relationship validation gate.
/// Mirrors [`super::validation::RejectionCode`]'s role: one stable vocabulary
/// shared by this gate, the audit trail the later governed command writes, and
/// the adapter error surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipRejectionCode {
    /// The supplied relation name/alias + version does not resolve to a
    /// registry row.
    UnknownRelation,
    /// A source/target endpoint id is not a canonical UUID.
    InvalidEndpointId,
    /// The declared source/target kind pairing is not legal for the resolved
    /// relation's direction class.
    UnsupportedEndpointKind,
    /// A source/target endpoint does not exist in its owning table.
    MissingEndpoint,
    /// Source and target are the same kind+id but the relation forbids
    /// reflexivity.
    ReflexiveNotAllowed,
    /// The relation requires a Valid Time interval but none was supplied.
    ValidTimeRequired,
    /// The relation forbids a Valid Time interval but one was supplied.
    ValidTimeForbidden,
    /// The supplied evidence does not meet the relation's evidence policy.
    EvidencePolicyUnmet,
    /// The caller lacks the capability the command requires.
    MissingCapability,
    /// The Effective-Policy meet over caller + endpoint policies denied.
    EffectivePolicyDenied,
}

impl RelationshipRejectionCode {
    /// The canonical snake_case text (stable for audit/logging).
    pub fn as_str(self) -> &'static str {
        match self {
            RelationshipRejectionCode::UnknownRelation => "unknown_relation",
            RelationshipRejectionCode::InvalidEndpointId => "invalid_endpoint_id",
            RelationshipRejectionCode::UnsupportedEndpointKind => "unsupported_endpoint_kind",
            RelationshipRejectionCode::MissingEndpoint => "missing_endpoint",
            RelationshipRejectionCode::ReflexiveNotAllowed => "reflexive_not_allowed",
            RelationshipRejectionCode::ValidTimeRequired => "valid_time_required",
            RelationshipRejectionCode::ValidTimeForbidden => "valid_time_forbidden",
            RelationshipRejectionCode::EvidencePolicyUnmet => "evidence_policy_unmet",
            RelationshipRejectionCode::MissingCapability => "missing_capability",
            RelationshipRejectionCode::EffectivePolicyDenied => "effective_policy_denied",
        }
    }
}

impl std::fmt::Display for RelationshipRejectionCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One rejection reason: a stable [`RelationshipRejectionCode`] plus a
/// bounded, human-readable, log-safe detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelationshipRejectionReason {
    /// The stable reason code.
    pub code: RelationshipRejectionCode,
    /// A short explanation (safe to log; never carries secret content).
    pub detail: String,
}

impl RelationshipRejectionReason {
    fn new(code: RelationshipRejectionCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Request types
// ─────────────────────────────────────────────────────────────────────────

/// One declared relationship endpoint: an [`EndpointKind`] plus the caller's
/// (not-yet-canonicalized) id string. Distinct from [`RelationEndpoint`]
/// (task 2.2.2's validated value object) because this gate must accept and
/// diagnose an id that turns out not to be canonical (check #2) — the
/// validated type is only constructed once every check passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointRef {
    /// The declared endpoint kind.
    pub kind: EndpointKind,
    /// The declared endpoint id (validated for canonical UUID shape by this
    /// gate; not yet a [`RelationEndpoint`]).
    pub id: String,
}

impl EndpointRef {
    /// Construct an endpoint reference.
    pub fn new(kind: EndpointKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }
}

/// One evidence descriptor supplied with a relationship command, for evidence
/// **policy compliance checking only** (task 2.2.3 scope) — not the persisted
/// `evidence_v2` row itself (that append is task 2.2.4). `attributes` names
/// the provenance attributes this evidence descriptor carries (e.g.
/// `"locator"`, `"method"`, `"method_version"`, `"rationale"`), matching
/// [`EvidencePolicy::required_attributes`]'s vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceInput {
    /// Whether this evidence supports or contradicts the relationship.
    pub polarity: EvidencePolarity,
    /// The provenance attributes this evidence descriptor carries.
    pub attributes: BTreeSet<String>,
}

impl EvidenceInput {
    /// Construct an evidence descriptor.
    pub fn new(polarity: EvidencePolarity, attributes: impl IntoIterator<Item = String>) -> Self {
        Self {
            polarity,
            attributes: attributes.into_iter().collect(),
        }
    }
}

/// A governed relationship command's validation-relevant content: everything
/// [`RelationshipValidator::validate`] needs to run the nine checks. The
/// invoking governed command builder (task 2.2.5) constructs one of these per
/// create/edit/confirm/expire/delete/restore/undo command.
#[derive(Debug, Clone)]
pub struct RelationshipWriteRequest {
    /// The relation as declared by the caller: a canonical name or a
    /// free-text alias (both resolve through the same materialized lookup).
    pub relation_surface_form: String,
    /// The registry version the caller believes governs this relation.
    pub relation_version: Version,
    /// The declared source endpoint.
    pub source: EndpointRef,
    /// The declared target endpoint.
    pub target: EndpointRef,
    /// The declared Valid Time interval (open on both ends when the caller
    /// supplies none).
    pub validity: ValidInterval,
    /// The evidence descriptors supplied with the command, for evidence
    /// policy compliance checking (task 2.2.3 scope; see [`EvidenceInput`]).
    pub evidence: Vec<EvidenceInput>,
    /// The capability [`RelationshipRejectionCode::MissingCapability`] checks
    /// for — supplied by the invoking command builder based on its own
    /// `CommandKind` (e.g. `ObserveMemory` for create, `CorrectMemory` for
    /// edit/confirm, `ForgetMemory` for expire/delete/restore).
    pub required_capability: Capability,
    /// The caller's own contributing policy (identity, partition,
    /// capabilities, trust, consent) — the first Effective-Policy (#9)
    /// contributor.
    pub caller_policy: ContributingPolicy,
}

// ─────────────────────────────────────────────────────────────────────────
// Read-only endpoint/relation lookups
// ─────────────────────────────────────────────────────────────────────────

/// The read-only lookups the relationship validation gate needs.
///
/// Implementations MUST be side-effect free (WAL-snapshot reads only), mirroring
/// [`super::validation::ValidationReads`]'s "reads never synchronously write"
/// invariant.
pub trait EndpointReads {
    /// Resolve a relation surface form (canonical name or alias) + registry
    /// version to its [`RelationDefinition`], or `None` if unresolvable.
    fn resolve_relation(
        &self,
        surface_form: &str,
        version: Version,
    ) -> MemoryResult<Option<RelationDefinition>>;

    /// Look up an endpoint's stored [`PolicyPartition`] by kind + canonical id,
    /// or `None` if no such row exists in the owning table.
    fn lookup_endpoint(
        &self,
        kind: EndpointKind,
        canonical_id: &str,
    ) -> MemoryResult<Option<PolicyPartition>>;
}

/// Concrete [`EndpointReads`] over the single authority [`Database`].
pub struct SqliteEndpointReads {
    db: Arc<Database>,
}

impl SqliteEndpointReads {
    /// Build the endpoint read surface over the injected authority handle.
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

/// Build a [`PolicyPartition`] from the four policy columns every
/// endpoint-owning v2 table carries, using the same lossless
/// `owner_id` `""` ↔ `None` bijection as [`super::super::model::row_mapping`].
fn partition_from_columns(
    namespace: String,
    owner_id: String,
    scope: String,
    sensitivity: i64,
) -> MemoryResult<PolicyPartition> {
    let owner = if owner_id.is_empty() {
        None
    } else {
        Some(owner_id)
    };
    PolicyPartition::with_owner(namespace, scope, sensitivity.max(0) as u8, owner)
}

/// The four policy columns, read generically for any endpoint-owning table.
type PolicyRow = (String, String, String, i64);

fn policy_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PolicyRow> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

impl EndpointReads for SqliteEndpointReads {
    fn resolve_relation(
        &self,
        surface_form: &str,
        version: Version,
    ) -> MemoryResult<Option<RelationDefinition>> {
        self.db
            .with_read(|conn| RelationRegistry::resolve_definition(conn, surface_form, version))
    }

    fn lookup_endpoint(
        &self,
        kind: EndpointKind,
        canonical_id: &str,
    ) -> MemoryResult<Option<PolicyPartition>> {
        self.db.with_read(|conn| {
            // Every endpoint-owning v2 table carries the same four policy
            // columns (design §4.1); `records` additionally discriminates by
            // `record_kind` since it is the shared table for the four
            // record-kind endpoints. Polymorphic across mixed kinds, so no
            // single hard FK/query serves every kind (design §4.2/§19.12).
            let row: Option<PolicyRow> = match kind {
                EndpointKind::Entity => conn
                    .query_row(
                        "SELECT namespace, owner_id, scope, sensitivity \
                         FROM entities_v2 WHERE id = ?1",
                        params![canonical_id],
                        policy_row,
                    )
                    .optional(),
                EndpointKind::Memory
                | EndpointKind::Summary
                | EndpointKind::Skill
                | EndpointKind::Rule => conn
                    .query_row(
                        "SELECT namespace, owner_id, scope, sensitivity \
                         FROM records WHERE id = ?1 AND record_kind = ?2",
                        params![canonical_id, record_kind_str(kind)],
                        policy_row,
                    )
                    .optional(),
                EndpointKind::Event => conn
                    .query_row(
                        "SELECT namespace, owner_id, scope, sensitivity \
                         FROM events_v2 WHERE id = ?1",
                        params![canonical_id],
                        policy_row,
                    )
                    .optional(),
                EndpointKind::Episode => conn
                    .query_row(
                        "SELECT namespace, owner_id, scope, sensitivity \
                         FROM episodes_v2 WHERE id = ?1",
                        params![canonical_id],
                        policy_row,
                    )
                    .optional(),
                EndpointKind::Goal => conn
                    .query_row(
                        "SELECT namespace, owner_id, scope, sensitivity \
                         FROM goals_v2 WHERE id = ?1",
                        params![canonical_id],
                        policy_row,
                    )
                    .optional(),
                EndpointKind::Evidence => conn
                    .query_row(
                        "SELECT namespace, owner_id, scope, sensitivity \
                         FROM evidence_v2 WHERE id = ?1",
                        params![canonical_id],
                        policy_row,
                    )
                    .optional(),
                EndpointKind::Relationship => conn
                    .query_row(
                        "SELECT namespace, owner_id, scope, sensitivity \
                         FROM relationships_v2 WHERE id = ?1",
                        params![canonical_id],
                        policy_row,
                    )
                    .optional(),
            }
            .map_err(crate::error::StorageError::Sqlite)?;

            row.map(|(ns, owner, scope, sens)| partition_from_columns(ns, owner, scope, sens))
                .transpose()
        })
    }
}

/// The `records.record_kind` value for the four record-kind endpoint kinds.
fn record_kind_str(kind: EndpointKind) -> &'static str {
    match kind {
        EndpointKind::Memory => "memory",
        EndpointKind::Summary => "summary",
        EndpointKind::Skill => "skill",
        EndpointKind::Rule => "rule",
        other => unreachable!("record_kind_str called for non-record endpoint kind {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Validation outcome
// ─────────────────────────────────────────────────────────────────────────

/// The resolved state a [`RelationshipValidationOutcome::Proceed`] carries so
/// the governed command stage (task 2.2.5) can open its `AuthorityTx` and
/// write the semantic row without re-deriving anything this gate already
/// computed.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRelationship {
    /// The resolved registry definition.
    pub relation: RelationDefinition,
    /// The governing policy partition (the Effective-Policy grant's
    /// partition) the relationship row is stored under.
    pub policy_partition: PolicyPartition,
    /// The canonical semantic identity hash (task 2.2.2).
    pub identity: RelationshipIdentity,
}

/// The typed result of the relationship validation gate.
#[derive(Debug, Clone, PartialEq)]
pub enum RelationshipValidationOutcome {
    /// Every check passed; the caller may open the `AuthorityTx`.
    Proceed(Box<ResolvedRelationship>),
    /// The command was rejected; the reasons are recorded verbatim in audit.
    /// Never empty.
    Rejected(Vec<RelationshipRejectionReason>),
}

impl RelationshipValidationOutcome {
    /// Whether the outcome permits opening the transaction.
    pub fn is_proceed(&self) -> bool {
        matches!(self, RelationshipValidationOutcome::Proceed(_))
    }

    /// The rejection reasons, if this outcome is a rejection.
    pub fn rejection_reasons(&self) -> Option<&[RelationshipRejectionReason]> {
        match self {
            RelationshipValidationOutcome::Rejected(reasons) => Some(reasons),
            _ => None,
        }
    }

    /// Whether the rejection set contains a given code.
    pub fn has_code(&self, code: RelationshipRejectionCode) -> bool {
        self.rejection_reasons()
            .is_some_and(|rs| rs.iter().any(|r| r.code == code))
    }
}

// ─────────────────────────────────────────────────────────────────────────
// The validator
// ─────────────────────────────────────────────────────────────────────────

/// The relationship command validation gate. Borrows the read-only
/// [`EndpointReads`] surface; it owns no SQL and performs no writes.
pub struct RelationshipValidator<'r, R: EndpointReads + ?Sized> {
    reads: &'r R,
}

impl<'r, R: EndpointReads + ?Sized> RelationshipValidator<'r, R> {
    /// Build a validator over a read surface.
    pub fn new(reads: &'r R) -> Self {
        Self { reads }
    }

    /// Run the full deterministic validation sequence over `req`.
    pub fn validate(
        &self,
        req: &RelationshipWriteRequest,
    ) -> MemoryResult<RelationshipValidationOutcome> {
        // ── #3 relation alias resolution (I/O, must run first). ────────────
        let relation = match self
            .reads
            .resolve_relation(&req.relation_surface_form, req.relation_version)?
        {
            Some(def) => def,
            None => {
                return Ok(RelationshipValidationOutcome::Rejected(vec![
                    RelationshipRejectionReason::new(
                        RelationshipRejectionCode::UnknownRelation,
                        format!(
                            "relation {:?} (version {}) does not resolve to a registry row",
                            req.relation_surface_form,
                            req.relation_version.get()
                        ),
                    ),
                ]));
            }
        };

        let mut reasons = Vec::new();

        // ── #2 canonical endpoint ids (pure). ───────────────────────────────
        let source_canonical = self.check_canonical_id(&req.source, "source", &mut reasons);
        let target_canonical = self.check_canonical_id(&req.target, "target", &mut reasons);

        // ── #1 (kind legality) + #4 (direction) (pure). ────────────────────
        self.check_endpoint_kinds(&relation, req, &mut reasons);

        // ── #5 reflexivity (pure). ──────────────────────────────────────────
        self.check_reflexivity(
            &relation,
            req,
            source_canonical.as_deref(),
            target_canonical.as_deref(),
            &mut reasons,
        );

        // ── #6 Valid Time (pure). ───────────────────────────────────────────
        self.check_valid_time(&relation, req, &mut reasons);

        // ── #7 Evidence (pure). ─────────────────────────────────────────────
        self.check_evidence(&relation, req, &mut reasons);

        // ── #8 capability (pure). ───────────────────────────────────────────
        self.check_capability(req, &mut reasons);

        // ── #1 (existence, I/O). Only meaningful for a canonically-shaped id;
        //    an already-flagged invalid id skips its own doomed lookup. ─────
        let source_policy = self.check_existence(
            req.source.kind,
            source_canonical.as_deref(),
            "source",
            &mut reasons,
        )?;
        let target_policy = self.check_existence(
            req.target.kind,
            target_canonical.as_deref(),
            "target",
            &mut reasons,
        )?;

        if !reasons.is_empty() {
            return Ok(RelationshipValidationOutcome::Rejected(reasons));
        }

        // ── #9 Effective Policy meet (I/O-derived; only once everything else
        //    passed — it needs real endpoint policies). ─────────────────────
        let mut contributors = vec![req.caller_policy.clone()];
        if let Some(p) = source_policy {
            contributors.push(endpoint_contributor("endpoint:source", p)?);
        }
        if let Some(p) = target_policy {
            contributors.push(endpoint_contributor("endpoint:target", p)?);
        }
        let effective = EffectivePolicy::meet_all(contributors);
        let grant = match effective.outcome() {
            PolicyOutcome::Allow(grant) => grant.clone(),
            PolicyOutcome::Deny(deny_reasons) => {
                let detail = deny_reasons
                    .iter()
                    .map(|r| r.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Ok(RelationshipValidationOutcome::Rejected(vec![
                    RelationshipRejectionReason::new(
                        RelationshipRejectionCode::EffectivePolicyDenied,
                        format!("effective policy meet denied: {detail}"),
                    ),
                ]));
            }
        };
        let policy_partition = grant.partition()?;

        // ── Compute the canonical semantic identity (task 2.2.2). ──────────
        // Canonical ids are guaranteed `Some` here: both existence checks
        // above returned `Some(policy)` (else `reasons` would be non-empty and
        // we would have already returned), and existence lookup only runs for
        // a canonically-shaped id.
        let source_endpoint = RelationEndpoint::new(
            req.source.kind,
            source_canonical.expect("existence check succeeded ⇒ canonical id present"),
        )?;
        let target_endpoint = RelationEndpoint::new(
            req.target.kind,
            target_canonical.expect("existence check succeeded ⇒ canonical id present"),
        )?;
        let identity = RelationshipIdentity::compute(
            &relation,
            &source_endpoint,
            &target_endpoint,
            &req.validity,
            &policy_partition,
        );

        Ok(RelationshipValidationOutcome::Proceed(Box::new(
            ResolvedRelationship {
                relation,
                policy_partition,
                identity,
            },
        )))
    }

    // ── #2 canonical endpoint ids ───────────────────────────────────────
    /// Validate `endpoint.id` is a canonical UUID, pushing
    /// [`RelationshipRejectionCode::InvalidEndpointId`] on failure. Returns the
    /// canonical (lower-cased) form on success, applied uniformly across every
    /// [`EndpointKind`] (see module docs).
    fn check_canonical_id(
        &self,
        endpoint: &EndpointRef,
        role: &str,
        reasons: &mut Vec<RelationshipRejectionReason>,
    ) -> Option<String> {
        match canonical_uuid(&endpoint.id) {
            Ok(canonical) => Some(canonical),
            Err(_) => {
                reasons.push(RelationshipRejectionReason::new(
                    RelationshipRejectionCode::InvalidEndpointId,
                    format!(
                        "{role} endpoint id {:?} (kind {}) is not a canonical UUID",
                        endpoint.id, endpoint.kind
                    ),
                ));
                None
            }
        }
    }

    // ── #1 (kind) + #4 (direction) ───────────────────────────────────────
    /// A [`DirectionClass::Symmetric`] relation accepts either endpoint
    /// ordering; a [`DirectionClass::Directed`] relation requires the declared
    /// source/target roles to map onto `source_kinds`/`target_kinds` exactly as
    /// given (no swap tolerance).
    fn check_endpoint_kinds(
        &self,
        relation: &RelationDefinition,
        req: &RelationshipWriteRequest,
        reasons: &mut Vec<RelationshipRejectionReason>,
    ) {
        let forward_ok =
            relation.allows_source(req.source.kind) && relation.allows_target(req.target.kind);
        let ok = if relation.direction_class.is_symmetric() {
            let swapped_ok =
                relation.allows_source(req.target.kind) && relation.allows_target(req.source.kind);
            forward_ok || swapped_ok
        } else {
            forward_ok
        };
        if !ok {
            reasons.push(RelationshipRejectionReason::new(
                RelationshipRejectionCode::UnsupportedEndpointKind,
                format!(
                    "relation {:?} ({}) does not permit source kind {} / target kind {} in this order",
                    relation.relation_name.as_str(),
                    relation.direction_class,
                    req.source.kind,
                    req.target.kind
                ),
            ));
        }
    }

    // ── #5 reflexivity ────────────────────────────────────────────────────
    fn check_reflexivity(
        &self,
        relation: &RelationDefinition,
        req: &RelationshipWriteRequest,
        source_canonical: Option<&str>,
        target_canonical: Option<&str>,
        reasons: &mut Vec<RelationshipRejectionReason>,
    ) {
        if relation.reflexive {
            return;
        }
        if req.source.kind != req.target.kind {
            return;
        }
        if let (Some(a), Some(b)) = (source_canonical, target_canonical) {
            if a == b {
                reasons.push(RelationshipRejectionReason::new(
                    RelationshipRejectionCode::ReflexiveNotAllowed,
                    format!(
                        "relation {:?} is non-reflexive but source and target are both {} {:?}",
                        relation.relation_name.as_str(),
                        req.source.kind,
                        a
                    ),
                ));
            }
        }
    }

    // ── #6 Valid Time ─────────────────────────────────────────────────────
    fn check_valid_time(
        &self,
        relation: &RelationDefinition,
        req: &RelationshipWriteRequest,
        reasons: &mut Vec<RelationshipRejectionReason>,
    ) {
        match relation.validity_policy {
            ValidityPolicy::Required => {
                if req.validity.valid_from().is_none() {
                    reasons.push(RelationshipRejectionReason::new(
                        RelationshipRejectionCode::ValidTimeRequired,
                        format!(
                            "relation {:?} requires a Valid Time interval but none was supplied",
                            relation.relation_name.as_str()
                        ),
                    ));
                }
            }
            ValidityPolicy::Forbidden => {
                if !req.validity.is_open() {
                    reasons.push(RelationshipRejectionReason::new(
                        RelationshipRejectionCode::ValidTimeForbidden,
                        format!(
                            "relation {:?} forbids a Valid Time interval but one was supplied",
                            relation.relation_name.as_str()
                        ),
                    ));
                }
            }
            ValidityPolicy::Optional => {}
        }
    }

    // ── #7 Evidence ───────────────────────────────────────────────────────
    fn check_evidence(
        &self,
        relation: &RelationDefinition,
        req: &RelationshipWriteRequest,
        reasons: &mut Vec<RelationshipRejectionReason>,
    ) {
        let policy: &EvidencePolicy = &relation.evidence_policy;

        if req.evidence.len() < policy.min_evidence as usize {
            reasons.push(RelationshipRejectionReason::new(
                RelationshipRejectionCode::EvidencePolicyUnmet,
                format!(
                    "relation {:?} requires at least {} evidence row(s); {} supplied",
                    relation.relation_name.as_str(),
                    policy.min_evidence,
                    req.evidence.len()
                ),
            ));
        }

        if let Some(required_polarity) = policy.required_polarity {
            let has_matching = req.evidence.iter().any(|e| e.polarity == required_polarity);
            if !has_matching {
                reasons.push(RelationshipRejectionReason::new(
                    RelationshipRejectionCode::EvidencePolicyUnmet,
                    format!(
                        "relation {:?} requires evidence with polarity {} but none was supplied",
                        relation.relation_name.as_str(),
                        required_polarity
                    ),
                ));
            }
        }

        for attr in &policy.required_attributes {
            let satisfied = req.evidence.iter().any(|e| e.attributes.contains(attr));
            if !satisfied {
                reasons.push(RelationshipRejectionReason::new(
                    RelationshipRejectionCode::EvidencePolicyUnmet,
                    format!(
                        "relation {:?} requires evidence carrying attribute {attr:?}, none supplied",
                        relation.relation_name.as_str()
                    ),
                ));
            }
        }
    }

    // ── #8 capability ─────────────────────────────────────────────────────
    fn check_capability(
        &self,
        req: &RelationshipWriteRequest,
        reasons: &mut Vec<RelationshipRejectionReason>,
    ) {
        if !req
            .caller_policy
            .capabilities()
            .contains(req.required_capability)
        {
            reasons.push(RelationshipRejectionReason::new(
                RelationshipRejectionCode::MissingCapability,
                format!(
                    "caller lacks capability {} required for this relationship command",
                    req.required_capability
                ),
            ));
        }
    }

    // ── #1 (existence, I/O) ───────────────────────────────────────────────
    /// Look up `kind`/`canonical_id` in its owning table, pushing
    /// [`RelationshipRejectionCode::MissingEndpoint`] when absent. Skips the
    /// lookup (returning `Ok(None)` silently) when `canonical_id` is `None` —
    /// the canonical-id check (#2) already flagged that endpoint.
    fn check_existence(
        &self,
        kind: EndpointKind,
        canonical_id: Option<&str>,
        role: &str,
        reasons: &mut Vec<RelationshipRejectionReason>,
    ) -> MemoryResult<Option<PolicyPartition>> {
        let Some(id) = canonical_id else {
            return Ok(None);
        };
        match self.reads.lookup_endpoint(kind, id)? {
            Some(policy) => Ok(Some(policy)),
            None => {
                reasons.push(RelationshipRejectionReason::new(
                    RelationshipRejectionCode::MissingEndpoint,
                    format!("{role} endpoint {kind} {id:?} does not exist"),
                ));
                Ok(None)
            }
        }
    }
}

/// Build a policy-only Effective-Policy contributor for an existing endpoint's
/// stored [`PolicyPartition`]. An endpoint is not a *source* in the
/// trust/capability sense — it cannot meaningfully restrict what the caller is
/// permitted to do — so it carries the full [`Capability::ALL`] set,
/// [`SourceTrust::System`] (the least-restrictive tier), and
/// [`ConsentRequirement::NotRequired`], letting the meet's namespace/scope/
/// sensitivity/owner combination (A5 isolation) apply without distorting
/// capability/trust/consent (see module docs, check #9).
fn endpoint_contributor(
    label: &str,
    partition: PolicyPartition,
) -> MemoryResult<ContributingPolicy> {
    ContributingPolicy::new(
        label,
        partition,
        CapabilitySet::from_capabilities(Capability::ALL),
        SourceTrust::System,
        ConsentRequirement::NotRequired,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::model::relation_registry::{DirectionClass, RelationName, RelationRegistry};
    use crate::model::UtcTimestamp;
    use std::collections::HashMap;

    const V1: Version = Version::first();

    // ── A controllable in-memory read surface layered over a real relation
    //    registry (so alias/version resolution is exercised for real) plus a
    //    fake endpoint-existence table the tests populate directly. ────────
    struct FakeReads {
        db: Database,
        endpoints: HashMap<(EndpointKind, String), PolicyPartition>,
    }

    impl FakeReads {
        fn new() -> Self {
            Self {
                db: Database::open_in_memory().unwrap(),
                endpoints: HashMap::new(),
            }
        }

        fn with_endpoint(mut self, kind: EndpointKind, id: &str, policy: PolicyPartition) -> Self {
            self.endpoints.insert((kind, id.to_string()), policy);
            self
        }
    }

    impl EndpointReads for FakeReads {
        fn resolve_relation(
            &self,
            surface_form: &str,
            version: Version,
        ) -> MemoryResult<Option<RelationDefinition>> {
            let conn = self.db.write();
            RelationRegistry::resolve_definition(&conn, surface_form, version)
        }

        fn lookup_endpoint(
            &self,
            kind: EndpointKind,
            canonical_id: &str,
        ) -> MemoryResult<Option<PolicyPartition>> {
            Ok(self
                .endpoints
                .get(&(kind, canonical_id.to_string()))
                .cloned())
        }
    }

    fn uuid(byte: u8) -> String {
        format!("018f4e2a-1c3b-7d4e-8f90-abcdef01234{byte}")
    }

    fn partition() -> PolicyPartition {
        PolicyPartition::new("user", "chat", 0).unwrap()
    }

    fn caller_policy(caps: &[Capability]) -> ContributingPolicy {
        ContributingPolicy::new(
            "caller",
            partition(),
            CapabilitySet::from_capabilities(caps.iter().copied()),
            SourceTrust::System,
            ConsentRequirement::NotRequired,
        )
        .unwrap()
    }

    /// A `related_to` (symmetric, entity↔entity, evidence-free) request between
    /// two existing entities, with a capable caller — the happy path.
    fn related_to_request(reads: &FakeReads) -> RelationshipWriteRequest {
        let _ = reads;
        RelationshipWriteRequest {
            relation_surface_form: "related_to".to_string(),
            relation_version: V1,
            source: EndpointRef::new(EndpointKind::Entity, uuid(1)),
            target: EndpointRef::new(EndpointKind::Entity, uuid(2)),
            validity: ValidInterval::open(),
            evidence: Vec::new(),
            required_capability: Capability::ObserveMemory,
            caller_policy: caller_policy(&[Capability::ObserveMemory]),
        }
    }

    fn reads_with_two_entities() -> FakeReads {
        FakeReads::new()
            .with_endpoint(EndpointKind::Entity, &uuid(1), partition())
            .with_endpoint(EndpointKind::Entity, &uuid(2), partition())
    }

    // ── Happy path ───────────────────────────────────────────────────────
    #[test]
    fn happy_path_produces_resolved_relationship_identity() {
        let reads = reads_with_two_entities();
        let req = related_to_request(&reads);
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.is_proceed(), "{outcome:?}");
        if let RelationshipValidationOutcome::Proceed(resolved) = outcome {
            assert_eq!(resolved.relation.relation_name.as_str(), "related_to");
            assert_eq!(resolved.identity.as_str().len(), 64);
        }
    }

    // ── #3 relation alias resolution ─────────────────────────────────────
    #[test]
    fn alias_resolution_success_and_failure() {
        let reads = reads_with_two_entities();
        // Success: a registered alias resolves to its canonical relation.
        let mut req = related_to_request(&reads);
        req.relation_surface_form = "associated_with".to_string();
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.is_proceed());

        // Failure: an unregistered surface form does not resolve.
        req.relation_surface_form = "totally_unknown_relation".to_string();
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.has_code(RelationshipRejectionCode::UnknownRelation));
    }

    // ── #1/#4 endpoint kind + direction ───────────────────────────────────
    #[test]
    fn endpoint_kind_mismatch_rejected() {
        // `supports` requires an Evidence source endpoint; Entity is illegal.
        let reads = reads_with_two_entities();
        let mut req = related_to_request(&reads);
        req.relation_surface_form = "supports".to_string();
        req.required_capability = Capability::ObserveMemory;
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.has_code(RelationshipRejectionCode::UnsupportedEndpointKind));
    }

    #[test]
    fn directed_relation_rejects_swapped_roles() {
        // `part_of` is directed entity->entity but both kinds are Entity in
        // both slots, so swap alone cannot be kind-detected here; instead
        // exercise a relation whose source/target kind sets differ: `supports`
        // (Evidence -> {memory,...,relationship,goal}). Swapping is rejected
        // because Entity is used for the (illegal) source role once swapped
        // onto Evidence's target-only kinds.
        let reads = FakeReads::new()
            .with_endpoint(EndpointKind::Evidence, &uuid(3), partition())
            .with_endpoint(EndpointKind::Goal, &uuid(4), partition());
        let mut req = related_to_request(&reads);
        req.relation_surface_form = "supports".to_string();
        // Swapped: source=Goal (illegal as source), target=Evidence (illegal
        // as target).
        req.source = EndpointRef::new(EndpointKind::Goal, uuid(4));
        req.target = EndpointRef::new(EndpointKind::Evidence, uuid(3));
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.has_code(RelationshipRejectionCode::UnsupportedEndpointKind));
    }

    #[test]
    fn symmetric_relation_accepts_either_endpoint_order() {
        let reads = reads_with_two_entities();
        let mut req = related_to_request(&reads);
        // Swap source/target — related_to is symmetric, both are Entity, so
        // this must still proceed.
        std::mem::swap(&mut req.source, &mut req.target);
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.is_proceed());
    }

    // ── #2 canonical endpoint ids ──────────────────────────────────────────
    #[test]
    fn non_canonical_endpoint_id_rejected() {
        let reads = reads_with_two_entities();
        let mut req = related_to_request(&reads);
        req.source.id = "not-a-uuid".to_string();
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.has_code(RelationshipRejectionCode::InvalidEndpointId));
        // A doomed existence lookup for the invalid id is skipped, not
        // reported as MissingEndpoint too.
        assert!(!outcome.has_code(RelationshipRejectionCode::MissingEndpoint));
    }

    // ── #1 existence ──────────────────────────────────────────────────────
    #[test]
    fn missing_endpoint_rejected() {
        // Only entity #1 is registered; #2 does not exist.
        let reads = FakeReads::new().with_endpoint(EndpointKind::Entity, &uuid(1), partition());
        let req = related_to_request(&reads);
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.has_code(RelationshipRejectionCode::MissingEndpoint));
    }

    #[test]
    fn valid_endpoint_of_correct_kind_accepted() {
        let reads = reads_with_two_entities();
        let req = related_to_request(&reads);
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.is_proceed());
    }

    // ── #5 reflexivity ────────────────────────────────────────────────────
    #[test]
    fn reflexivity_violation_rejected_for_nonreflexive_relation() {
        // `derived_from` is non-reflexive; a memory record derived "from
        // itself" must be rejected.
        let rid = uuid(5);
        let reads = FakeReads::new().with_endpoint(EndpointKind::Memory, &rid, partition());
        let mut req = related_to_request(&reads);
        req.relation_surface_form = "derived_from".to_string();
        req.source = EndpointRef::new(EndpointKind::Memory, rid.clone());
        req.target = EndpointRef::new(EndpointKind::Memory, rid);
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.has_code(RelationshipRejectionCode::ReflexiveNotAllowed));
    }

    // ── #6 Valid Time policy ──────────────────────────────────────────────
    #[test]
    fn valid_time_optional_allows_present_or_absent_interval() {
        let reads = reads_with_two_entities();
        // `related_to` is validity_policy = optional: open interval proceeds.
        let req = related_to_request(&reads);
        assert!(RelationshipValidator::new(&reads)
            .validate(&req)
            .unwrap()
            .is_proceed());
    }

    #[test]
    fn valid_time_forbidden_rejects_supplied_interval() {
        // Seed a symmetric domain relation has validity_policy=optional in the
        // fixtures; simulate "forbidden" by asserting the check function logic
        // directly through a relation with a forbidden policy fetched from a
        // registry row we control. Since all seeded rows are `optional`, drive
        // this through the pure check helper via a hand-built definition.
        let reads = reads_with_two_entities();
        let relation = RelationDefinition {
            relation_name: RelationName::new("no_time_allowed").unwrap(),
            version: V1,
            display_forward: "x".into(),
            display_inverse: None,
            aliases: Vec::new(),
            direction_class: DirectionClass::Symmetric,
            inverse_name: None,
            reflexive: true,
            source_kinds: vec![EndpointKind::Entity],
            target_kinds: vec![EndpointKind::Entity],
            validity_policy: ValidityPolicy::Forbidden,
            evidence_policy: EvidencePolicy::none(),
            policy_rule_version: "1".into(),
            writable: true,
        };
        let mut req = related_to_request(&reads);
        req.validity = ValidInterval::new(
            Some(UtcTimestamp::from_rfc3339_utc("2026-01-01T00:00:00Z").unwrap()),
            None,
        )
        .unwrap();
        let validator = RelationshipValidator::new(&reads);
        let mut reasons = Vec::new();
        validator.check_valid_time(&relation, &req, &mut reasons);
        assert_eq!(
            reasons.first().map(|r| r.code),
            Some(RelationshipRejectionCode::ValidTimeForbidden)
        );
    }

    #[test]
    fn valid_time_required_rejects_missing_interval() {
        let reads = reads_with_two_entities();
        let relation = RelationDefinition {
            relation_name: RelationName::new("time_required").unwrap(),
            version: V1,
            display_forward: "x".into(),
            display_inverse: None,
            aliases: Vec::new(),
            direction_class: DirectionClass::Symmetric,
            inverse_name: None,
            reflexive: true,
            source_kinds: vec![EndpointKind::Entity],
            target_kinds: vec![EndpointKind::Entity],
            validity_policy: ValidityPolicy::Required,
            evidence_policy: EvidencePolicy::none(),
            policy_rule_version: "1".into(),
            writable: true,
        };
        let req = related_to_request(&reads); // open interval, no valid_from
        let validator = RelationshipValidator::new(&reads);
        let mut reasons = Vec::new();
        validator.check_valid_time(&relation, &req, &mut reasons);
        assert_eq!(
            reasons.first().map(|r| r.code),
            Some(RelationshipRejectionCode::ValidTimeRequired)
        );
    }

    // ── #7 Evidence policy ────────────────────────────────────────────────
    #[test]
    fn evidence_policy_min_count_enforced() {
        // `supports` requires min_evidence=1 with polarity=supports + locator.
        let reads = FakeReads::new()
            .with_endpoint(EndpointKind::Evidence, &uuid(6), partition())
            .with_endpoint(EndpointKind::Goal, &uuid(7), partition());
        let mut req = related_to_request(&reads);
        req.relation_surface_form = "supports".to_string();
        req.source = EndpointRef::new(EndpointKind::Evidence, uuid(6));
        req.target = EndpointRef::new(EndpointKind::Goal, uuid(7));
        req.evidence = Vec::new(); // below min_evidence=1
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.has_code(RelationshipRejectionCode::EvidencePolicyUnmet));
    }

    #[test]
    fn evidence_policy_required_polarity_enforced() {
        let reads = FakeReads::new()
            .with_endpoint(EndpointKind::Evidence, &uuid(6), partition())
            .with_endpoint(EndpointKind::Goal, &uuid(7), partition());
        let mut req = related_to_request(&reads);
        req.relation_surface_form = "supports".to_string();
        req.source = EndpointRef::new(EndpointKind::Evidence, uuid(6));
        req.target = EndpointRef::new(EndpointKind::Goal, uuid(7));
        // Wrong polarity (contradicts) and missing the required "locator"
        // attribute.
        req.evidence = vec![EvidenceInput::new(EvidencePolarity::Contradicts, [])];
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.has_code(RelationshipRejectionCode::EvidencePolicyUnmet));
    }

    #[test]
    fn evidence_policy_required_attributes_enforced() {
        let reads = FakeReads::new()
            .with_endpoint(EndpointKind::Evidence, &uuid(6), partition())
            .with_endpoint(EndpointKind::Goal, &uuid(7), partition());
        let mut req = related_to_request(&reads);
        req.relation_surface_form = "supports".to_string();
        req.source = EndpointRef::new(EndpointKind::Evidence, uuid(6));
        req.target = EndpointRef::new(EndpointKind::Goal, uuid(7));
        // Right polarity and count, but missing the required "locator"
        // attribute.
        req.evidence = vec![EvidenceInput::new(EvidencePolarity::Supports, [])];
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.has_code(RelationshipRejectionCode::EvidencePolicyUnmet));
    }

    #[test]
    fn evidence_policy_fully_satisfied_proceeds() {
        let reads = FakeReads::new()
            .with_endpoint(EndpointKind::Evidence, &uuid(6), partition())
            .with_endpoint(EndpointKind::Goal, &uuid(7), partition());
        let mut req = related_to_request(&reads);
        req.relation_surface_form = "supports".to_string();
        req.source = EndpointRef::new(EndpointKind::Evidence, uuid(6));
        req.target = EndpointRef::new(EndpointKind::Goal, uuid(7));
        req.evidence = vec![EvidenceInput::new(
            EvidencePolarity::Supports,
            ["locator".to_string()],
        )];
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.is_proceed(), "{outcome:?}");
    }

    // ── #8 capability ──────────────────────────────────────────────────────
    #[test]
    fn missing_capability_rejected() {
        let reads = reads_with_two_entities();
        let mut req = related_to_request(&reads);
        req.caller_policy = caller_policy(&[Capability::ReadCore]); // lacks ObserveMemory
        req.required_capability = Capability::ObserveMemory;
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.has_code(RelationshipRejectionCode::MissingCapability));
    }

    #[test]
    fn present_capability_passes() {
        let reads = reads_with_two_entities();
        let mut req = related_to_request(&reads);
        req.caller_policy = caller_policy(&[Capability::ObserveMemory, Capability::ReadCore]);
        req.required_capability = Capability::ObserveMemory;
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.is_proceed());
    }

    // ── #9 Effective Policy meet ───────────────────────────────────────────
    #[test]
    fn effective_policy_meet_denies_on_incompatible_partitions() {
        // Caller writes under namespace "user"; the target entity's stored
        // partition is namespace "system" — the meet must deny (A5 isolation).
        let other_ns = PolicyPartition::new("system", "chat", 0).unwrap();
        let reads = FakeReads::new()
            .with_endpoint(EndpointKind::Entity, &uuid(1), partition())
            .with_endpoint(EndpointKind::Entity, &uuid(2), other_ns);
        let req = related_to_request(&reads);
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.has_code(RelationshipRejectionCode::EffectivePolicyDenied));
    }

    #[test]
    fn effective_policy_meet_allows_compatible_partitions() {
        let reads = reads_with_two_entities();
        let req = related_to_request(&reads);
        let outcome = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert!(outcome.is_proceed());
    }

    // ── Side-effect-free (no writes performed by validate) ────────────────
    #[test]
    fn validate_performs_no_endpoint_table_mutation() {
        // FakeReads has no write surface at all beyond the registry lookup
        // (which itself is read-only); a successful validate() call cannot
        // have mutated `endpoints`, verified structurally by re-running twice
        // with identical results.
        let reads = reads_with_two_entities();
        let req = related_to_request(&reads);
        let outcome1 = RelationshipValidator::new(&reads).validate(&req).unwrap();
        let outcome2 = RelationshipValidator::new(&reads).validate(&req).unwrap();
        assert_eq!(outcome1, outcome2);
    }
}
