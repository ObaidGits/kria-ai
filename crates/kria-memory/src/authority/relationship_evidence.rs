//! Transaction-scoped Evidence append + "append vs. new edge" branching (task
//! **F2.2.4**, design §4.2, MGR-005 AC4, MGR-018; MGD-008–MGD-009).
//!
//! Design §4.2 / MGR-005 AC4: "WHEN additional observations support an
//! existing relationship identity, THE Cognitive_Memory_System SHALL append
//! Evidence without duplicating the active semantic edge." Migration 0019's
//! unique **active** `identity_hash` index on `relationships_v2` is exactly the
//! "same edge" key this module consumes: when a governed relationship
//! command's [`RelationshipIdentity`] (computed by task 2.2.2, carried on the
//! [`ResolvedRelationship`] a successful 2.2.3 validation produces) matches an
//! **existing active** row, this module appends an `evidence_v2` row against
//! that row's id instead of inserting a second `relationships_v2` row for the
//! same identity — the semantic edge is never duplicated, only its evidence
//! grows.
//!
//! ## Transaction-scoped repository (mirrors F1.3)
//!
//! [`TxRelationshipEvidence`] is a zero-sized handle exactly like
//! [`TxEventLog`](super::event_log::TxEventLog),
//! [`TxAuditLog`](super::audit::TxAuditLog),
//! [`TxRevisionLog`](super::revision::TxRevisionLog), and
//! [`TxOutbox`](super::outbox::TxOutbox): every method takes the `&mut
//! AuthorityTx` it must write through, so it is structurally impossible for
//! this repository to write anywhere other than the serialized-writer
//! transaction (F1.3 invariant). It owns no [`Database`](crate::db::Database)
//! / connection / pool.
//!
//! ## The append-vs-new-edge branch ([`TxRelationshipEvidence::append_or_create`])
//!
//! Given a [`ResolvedRelationship`] (the 2.2.3 validation gate's `Proceed`
//! output) this module's top-level entry point:
//!
//! 1. **Looks up** `relationships_v2` for an **ACTIVE** row (`truth_state IS
//!    NULL OR truth_state NOT IN ('superseded','forgotten','deleted')` —
//!    exactly the predicate the migration-0019 unique index already encodes)
//!    whose `identity_hash` equals `resolved.identity.as_str()`
//!    ([`TxRelationshipEvidence::find_active_relationship`]).
//! 2. **If found** → the command is an *additional observation* about an
//!    existing edge: no new `relationships_v2` row is inserted; the evidence
//!    is appended against the existing row's id.
//! 3. **If not found** → the command is a *genuinely new* semantic edge: a
//!    fresh `relationships_v2` row is inserted (carrying `identity_hash`) and
//!    the evidence is appended against the new row's id.
//!
//! Because a **superseded/forgotten/deleted** row is excluded from both the
//! lookup predicate here *and* the migration-0019 unique index's `WHERE`
//! clause, a lingering non-active row sharing the same `identity_hash` never
//! blocks step 3 from inserting a fresh active row — the two predicates are
//! kept identical by construction (both read from
//! [`ACTIVE_TRUTH_STATE_PREDICATE`]) so they can never drift apart.
//!
//! Task 2.2.4 scope note: the fresh-row insert in step 3 is the **minimal
//! insert-if-absent** the task instructions call for to demonstrate/test this
//! branch — it is not the full governed create-command lifecycle (preview/
//! confirm, edit, expire, delete, restore, undo), which is task **2.2.5**.
//!
//! ## Idempotency / no-duplicate-observation decision (task 2.2.4 item 3)
//!
//! Two independent layers cooperate, deliberately at different granularity:
//!
//! * **Outer layer (F1.3, already built):** `idempotency_results
//!   (caller_partition, idempotency_key)` deduplicates a **replayed identical
//!   command** — the same caller resubmitting the same idempotency key never
//!   re-executes the semantic mutation at all (MGR-005 AC3). This is the
//!   correct layer for "the caller retried the whole append-evidence command."
//! * **Inner layer (this module, migration 0020):** a structural,
//!   defense-in-depth partial UNIQUE index
//!   `uq_evidence_v2_subject_event (subject_kind, subject_id, source_event_id)
//!   WHERE source_event_id IS NOT NULL` on `evidence_v2` — independent of
//!   caller idempotency-key discipline, the **same authority event** can never
//!   be appended as evidence for the **same subject** twice. This is the
//!   correct layer for "a different idempotency key names the same underlying
//!   observation" (a narrower, content-addressed guarantee the outer layer
//!   cannot express, since it only sees the caller's chosen key, not the
//!   observation's own identity).
//!
//! [`TxRelationshipEvidence::append_evidence`] enforces the inner layer by
//! checking for an existing `(subject_kind, subject_id, source_event_id)` row
//! **before** inserting (`SELECT` then conditional `INSERT`, run inside the
//! same serialized-writer transaction so the check-then-act is race-free — no
//! second writer can interleave). A dedup hit returns the **existing**
//! [`EvidenceId`] with `appended = false`; a fresh row returns the **new**
//! [`EvidenceId`] with `appended = true`. When `source_event_id` is absent
//! (evidence with no linked authority event — e.g. a manually authored
//! rationale), there is no event-level identity to deduplicate on, and this
//! layer intentionally defers to the outer idempotency-key layer instead — a
//! documented decision, not an oversight.
//!
//! ## Explicitly out of scope (later F2.2 subtasks)
//!
//! The full governed create/edit/confirm/expire/delete/restore/undo command
//! lifecycle with revision-bound compensating history (2.2.5), legacy
//! free-text relationship migration/reconciliation (2.2.6), and legacy
//! relationship table deletion (2.2.7). This module writes `relationships_v2`
//! rows only via the minimal insert-if-absent described above — it does not
//! implement edit/expire/delete/restore/undo.

use rusqlite::{params, OptionalExtension};

use crate::db::AuthorityTx;
use crate::error::{MemoryResult, StorageError};
use crate::model::entity::EvidencePolarity;
use crate::model::provenance::{Actor, Locator, Method};
use crate::model::relationship_identity::{RelationEndpoint, RelationshipIdentity};
use crate::model::truth::TruthState;
use crate::model::{
    EndpointKind, EventId, EvidenceId, PolicyPartition, SourceId, UtcTimestamp, ValidInterval,
};

use super::relationship_validation::ResolvedRelationship;
use crate::model::RelationshipId;
// Only referenced from doc comments (intra-doc link) and by `#[cfg(test)]`
// code below; the import itself is otherwise unused in a non-test build.
#[cfg_attr(not(test), allow(unused_imports))]
use super::event_log::PENDING_POLICY_VERSION;

/// The **single** ACTIVE-row SQL predicate fragment shared by
/// [`TxRelationshipEvidence::find_active_relationship`] and mirrored by the
/// migration-0019 unique index's `WHERE` clause (`db/schema/0019_relationships_v2.sql`).
/// Defined once so the two can never silently drift apart — a non-active
/// (superseded/forgotten/deleted) row sharing an `identity_hash` must be
/// invisible to *both* "is there an active edge to append to" (this module)
/// and "may a fresh active row be inserted" (the DB constraint) identically.
const ACTIVE_TRUTH_STATE_PREDICATE: &str =
    "(truth_state IS NULL OR truth_state NOT IN ('superseded','forgotten','deleted'))";

// ─────────────────────────────────────────────────────────────────────────
// EvidenceDraft — the caller-supplied content of one evidence observation
// ─────────────────────────────────────────────────────────────────────────

/// The validated content of one supporting/contradicting observation, before
/// it is tied to a subject and appended (task 2.2.4 item 1: "locator/actor/
/// method/version/polarity/score semantics/policy").
///
/// Every field is a validated value object — [`Locator`] (policy-safe
/// structured provenance), [`Actor`]/[`Method`] (bounded structural
/// references) — so a raw unchecked string can never reach the `evidence_v2`
/// row. `score_semantics` is required whenever `score` is present (mirrors
/// the design's "score semantics" naming discipline also enforced on
/// [`crate::model::entity::Mention`]).
#[derive(Debug, Clone, PartialEq)]
pub struct EvidenceDraft {
    locator: Locator,
    actor: Actor,
    method: Method,
    polarity: EvidencePolarity,
    score: Option<f64>,
    score_semantics: Option<String>,
    source_record: Option<RelationEndpoint>,
    source_event_id: Option<EventId>,
    observed_at: Option<UtcTimestamp>,
}

impl EvidenceDraft {
    /// Construct the required core of an evidence observation: where it comes
    /// from ([`Locator`]), who/what recorded it ([`Actor`]), the assessment
    /// method ([`Method`]), and whether it supports or contradicts its
    /// subject.
    pub fn new(locator: Locator, actor: Actor, method: Method, polarity: EvidencePolarity) -> Self {
        Self {
            locator,
            actor,
            method,
            polarity,
            score: None,
            score_semantics: None,
            source_record: None,
            source_event_id: None,
            observed_at: None,
        }
    }

    /// Builder: attach a score and its **required** semantics (what the score
    /// means — never a bare unexplained "confidence").
    pub fn with_score(mut self, score: f64, semantics: impl Into<String>) -> MemoryResult<Self> {
        let semantics = semantics.into();
        if semantics.trim().is_empty() {
            return Err(StorageError::Encoding(
                "evidence score_semantics must not be empty when score is present".into(),
            )
            .into());
        }
        self.score = Some(score);
        self.score_semantics = Some(semantics);
        Ok(self)
    }

    /// Builder: attach the polymorphic source-record endpoint the evidence was
    /// extracted from (`evidence_v2.source_record_kind`/`source_record_id`).
    pub fn with_source_record(mut self, endpoint: RelationEndpoint) -> Self {
        self.source_record = Some(endpoint);
        self
    }

    /// Builder: attach the originating authority event
    /// (`evidence_v2.source_event_id`) — also the dedup key this module's
    /// inner idempotency layer keys on (see module docs).
    pub fn with_source_event(mut self, event_id: EventId) -> Self {
        self.source_event_id = Some(event_id);
        self
    }

    /// Builder: attach the observation instant (`evidence_v2.observed_at`).
    pub fn with_observed_at(mut self, observed_at: UtcTimestamp) -> Self {
        self.observed_at = Some(observed_at);
        self
    }

    /// The evidence polarity (supports/contradicts).
    pub fn polarity(&self) -> EvidencePolarity {
        self.polarity
    }

    /// The originating authority event, if any (the inner dedup key).
    pub fn source_event_id(&self) -> Option<&EventId> {
        self.source_event_id.as_ref()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// NewRelationshipInputs — what a genuinely-new edge needs to be inserted
// ─────────────────────────────────────────────────────────────────────────

/// The inputs needed to insert a fresh `relationships_v2` row when no active
/// row shares the command's [`RelationshipIdentity`] (task 2.2.4 item 2,
/// branch 3). This is the **minimal insert-if-absent** the task calls for —
/// not the full governed create-command lifecycle (task 2.2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct NewRelationshipInputs {
    /// The declared source endpoint (already validated, task 2.2.2/2.2.3).
    pub source: RelationEndpoint,
    /// The declared target endpoint.
    pub target: RelationEndpoint,
    /// The Valid Time interval the relationship holds under.
    pub validity: ValidInterval,
    /// The contributing policy source for the new row's `policy_source_id`.
    pub policy_source_id: SourceId,
    /// The policy version tag for the new row's `policy_version` column.
    /// Pass [`PENDING_POLICY_VERSION`] until the F1.4 Effective-Policy layer
    /// stamps the resolved version (mirrors the same placeholder-sentinel
    /// convention every other F1.3 transaction-scoped repository uses).
    pub policy_version: String,
    /// The creating authority event, if one was appended for this command.
    pub created_event_id: Option<EventId>,
}

// ─────────────────────────────────────────────────────────────────────────
// EvidenceInputs — what one evidence append needs, beyond its subject
// ─────────────────────────────────────────────────────────────────────────

/// The inputs needed to append one `evidence_v2` row, beyond the subject it is
/// attributed to (task 2.2.4 item 1).
#[derive(Debug, Clone, PartialEq)]
pub struct EvidenceInputs {
    /// The observation content.
    pub draft: EvidenceDraft,
    /// The contributing source for the evidence row's `source_id` column.
    pub source_id: SourceId,
    /// The policy partition the evidence row is stored under.
    pub policy: PolicyPartition,
    /// The policy version tag for the evidence row's `policy_version` column.
    pub policy_version: String,
    /// The creating authority event, if one was appended for this command.
    pub created_event_id: Option<EventId>,
}

// ─────────────────────────────────────────────────────────────────────────
// AppendedRelationshipEvidence — the outcome of append_or_create
// ─────────────────────────────────────────────────────────────────────────

/// The outcome of [`TxRelationshipEvidence::append_or_create`]: which
/// relationship the evidence now bears on, whether that row was freshly
/// inserted or an existing active edge, and which evidence row resulted
/// (freshly appended or a deduplicated existing observation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendedRelationshipEvidence {
    /// The relationship (existing or newly inserted) the evidence bears on.
    pub relationship_id: RelationshipId,
    /// `true` iff a **new** `relationships_v2` row was inserted (no active row
    /// shared the identity); `false` iff an existing active edge was found and
    /// reused — the "never duplicate an active semantic edge" branch.
    pub relationship_created: bool,
    /// The evidence row's identity (existing or newly inserted).
    pub evidence_id: EvidenceId,
    /// `true` iff a **new** `evidence_v2` row was appended; `false` iff an
    /// identical `(subject, source_event_id)` observation already existed and
    /// was deduplicated (see module docs, inner idempotency layer).
    pub evidence_appended: bool,
}

// ─────────────────────────────────────────────────────────────────────────
// TxRelationshipEvidence — the transaction-scoped repository
// ─────────────────────────────────────────────────────────────────────────

/// The transaction-scoped append surface over `relationships_v2` (minimal
/// insert-if-absent only) and `evidence_v2`.
///
/// A zero-sized handle: every method takes the `&mut AuthorityTx` it must
/// write through, so — exactly like [`TxEventLog`](super::event_log::TxEventLog),
/// [`TxAuditLog`](super::audit::TxAuditLog),
/// [`TxRevisionLog`](super::revision::TxRevisionLog), and
/// [`TxOutbox`](super::outbox::TxOutbox) — it is structurally impossible for
/// this repository to write anywhere other than the serialized-writer
/// transaction (F1.3 invariant). It owns no
/// [`Database`](crate::db::Database) / connection / pool.
#[derive(Debug, Default, Clone, Copy)]
pub struct TxRelationshipEvidence;

impl TxRelationshipEvidence {
    /// Construct the (stateless) relationship-evidence repository.
    pub fn new() -> Self {
        TxRelationshipEvidence
    }

    /// **The append-vs-new-edge branch** (task 2.2.4 item 2). Given a
    /// successful 2.2.3 validation outcome (`resolved`), decides whether the
    /// command is an additional observation about an existing active edge or a
    /// genuinely new one, and appends the evidence accordingly:
    ///
    /// * An **active** row already carries `resolved.identity` → the evidence
    ///   is appended against that row's id; `new_relationship` is **not**
    ///   consulted at all (`relationship_created = false`).
    /// * **No active** row carries `resolved.identity` → a fresh
    ///   `relationships_v2` row is inserted from `new_relationship` first
    ///   (`relationship_created = true`), then the evidence is appended
    ///   against the new row's id.
    ///
    /// Either way the evidence append itself is idempotent per the inner
    /// dedup layer (module docs): resubmitting the same `(subject,
    /// source_event_id)` observation never creates a second `evidence_v2` row.
    pub fn append_or_create(
        &self,
        tx: &mut AuthorityTx<'_>,
        resolved: &ResolvedRelationship,
        new_relationship: &NewRelationshipInputs,
        evidence: &EvidenceInputs,
    ) -> MemoryResult<AppendedRelationshipEvidence> {
        let (relationship_id, relationship_created) =
            match self.find_active_relationship(tx, &resolved.identity)? {
                Some(existing) => (existing, false),
                None => {
                    let inserted = self.insert_relationship(tx, resolved, new_relationship)?;
                    (inserted, true)
                }
            };

        let subject = RelationEndpoint::new(EndpointKind::Relationship, relationship_id.as_str())?;
        let (evidence_id, evidence_appended) = self.append_evidence(tx, &subject, evidence)?;

        Ok(AppendedRelationshipEvidence {
            relationship_id,
            relationship_created,
            evidence_id,
            evidence_appended,
        })
    }

    /// Look up an **ACTIVE** `relationships_v2` row whose `identity_hash`
    /// equals `identity`, or `None` if no such row exists. "Active" uses
    /// [`ACTIVE_TRUTH_STATE_PREDICATE`] — the same predicate the migration-0019
    /// unique index encodes — so a superseded/forgotten/deleted row sharing the
    /// identity is correctly invisible here (task 2.2.4 verification: a
    /// non-active row must not block inserting a fresh active one).
    pub fn find_active_relationship(
        &self,
        tx: &AuthorityTx<'_>,
        identity: &RelationshipIdentity,
    ) -> MemoryResult<Option<RelationshipId>> {
        let sql = format!(
            "SELECT id FROM relationships_v2 \
             WHERE identity_hash = ?1 AND {ACTIVE_TRUTH_STATE_PREDICATE} \
             LIMIT 1"
        );
        let found: Option<String> = tx
            .conn()
            .query_row(&sql, params![identity.as_str()], |r| r.get(0))
            .optional()
            .map_err(StorageError::Sqlite)?;
        found.map(RelationshipId::new).transpose()
    }

    /// Insert the **minimal** fresh `relationships_v2` row for a genuinely new
    /// semantic edge (task 2.2.4 item 2, branch 3 — not the full governed
    /// create-command lifecycle, which is task 2.2.5). The new row's initial
    /// truth disposition is [`TruthState::initial`] (`unverified`, mirroring
    /// every other cognitive-record table's fresh-observation default); its
    /// `authority_class` is `"stored"` (directly authored, neither derived nor
    /// inferred); `revision`/`superseded_by`/`algorithm`/`algorithm_version`
    /// are left unset — governed revision-bound lifecycle wiring is task 2.2.5.
    pub fn insert_relationship(
        &self,
        tx: &mut AuthorityTx<'_>,
        resolved: &ResolvedRelationship,
        new_relationship: &NewRelationshipInputs,
    ) -> MemoryResult<RelationshipId> {
        let id = RelationshipId::new_v7();
        let relation = &resolved.relation;
        let policy = &resolved.policy_partition;

        tx.conn()
            .execute(
                "INSERT INTO relationships_v2(
                     id, source_kind, source_id, target_kind, target_id,
                     relation_name, relation_version, direction_class,
                     valid_from, valid_until, truth_state, authority_class,
                     namespace, owner_id, scope, sensitivity,
                     policy_source_id, policy_version,
                     identity_hash, algorithm, algorithm_version,
                     created_event_id, revision, superseded_by)
                 VALUES (
                     ?1, ?2, ?3, ?4, ?5,
                     ?6, ?7, ?8,
                     ?9, ?10, ?11, ?12,
                     ?13, ?14, ?15, ?16,
                     ?17, ?18,
                     ?19, NULL, NULL,
                     ?20, NULL, NULL)",
                params![
                    id.as_str(),
                    new_relationship.source.kind().as_str(),
                    new_relationship.source.id(),
                    new_relationship.target.kind().as_str(),
                    new_relationship.target.id(),
                    relation.relation_name.as_str(),
                    relation.version.get(),
                    relation.direction_class.as_str(),
                    new_relationship
                        .validity
                        .valid_from()
                        .map(|ts| ts.to_rfc3339()),
                    new_relationship
                        .validity
                        .valid_until()
                        .map(|ts| ts.to_rfc3339()),
                    TruthState::initial().as_str(),
                    "stored",
                    policy.namespace(),
                    policy.owner_id().unwrap_or(""),
                    policy.scope(),
                    policy.sensitivity() as i64,
                    new_relationship.policy_source_id.as_str(),
                    new_relationship.policy_version,
                    resolved.identity.as_str(),
                    new_relationship
                        .created_event_id
                        .as_ref()
                        .map(EventId::as_str),
                ],
            )
            .map_err(StorageError::Sqlite)?;

        Ok(id)
    }

    /// Append one `evidence_v2` row against `subject`, **idempotently** with
    /// respect to the inner dedup key (module docs): when `evidence.draft`
    /// carries a `source_event_id`, an existing row for the same
    /// `(subject_kind, subject_id, source_event_id)` is reused
    /// (`appended = false`); otherwise (or when no such row exists) a fresh
    /// row is inserted (`appended = true`).
    ///
    /// The check-then-act runs inside the caller's serialized-writer
    /// transaction, so no concurrent writer can interleave between the lookup
    /// and the insert (the authority has exactly one writer at a time, F1.3).
    pub fn append_evidence(
        &self,
        tx: &mut AuthorityTx<'_>,
        subject: &RelationEndpoint,
        evidence: &EvidenceInputs,
    ) -> MemoryResult<(EvidenceId, bool)> {
        if let Some(source_event_id) = evidence.draft.source_event_id() {
            if let Some(existing) = self.find_existing_evidence(tx, subject, source_event_id)? {
                return Ok((existing, false));
            }
        }

        let id = EvidenceId::new_v7();
        let draft = &evidence.draft;
        let (source_record_kind, source_record_id) = draft
            .source_record
            .as_ref()
            .map(|e| (Some(e.kind().as_str()), Some(e.id())))
            .unwrap_or((None, None));
        let locator_json = draft.locator.to_json()?;

        tx.conn()
            .execute(
                "INSERT INTO evidence_v2(
                     id, subject_kind, subject_id,
                     source_record_kind, source_record_id, source_event_id,
                     locator_json, actor_id, method, method_version,
                     polarity, score, score_semantics,
                     namespace, owner_id, scope, sensitivity, source_id, policy_version,
                     observed_at, removed_at, created_event_id)
                 VALUES (
                     ?1, ?2, ?3,
                     ?4, ?5, ?6,
                     ?7, ?8, ?9, ?10,
                     ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, ?19,
                     ?20, NULL, ?21)",
                params![
                    id.as_str(),
                    subject.kind().as_str(),
                    subject.id(),
                    source_record_kind,
                    source_record_id,
                    draft.source_event_id.as_ref().map(EventId::as_str),
                    locator_json,
                    draft.actor.as_str(),
                    draft.method.name(),
                    draft.method.version(),
                    draft.polarity.as_str(),
                    draft.score,
                    draft.score_semantics,
                    evidence.policy.namespace(),
                    evidence.policy.owner_id().unwrap_or(""),
                    evidence.policy.scope(),
                    evidence.policy.sensitivity() as i64,
                    evidence.source_id.as_str(),
                    evidence.policy_version,
                    draft.observed_at.map(|ts| ts.to_rfc3339()),
                    evidence.created_event_id.as_ref().map(EventId::as_str),
                ],
            )
            .map_err(StorageError::Sqlite)?;

        Ok((id, true))
    }

    /// Look up an existing `evidence_v2` row sharing `(subject_kind,
    /// subject_id, source_event_id)` — the inner dedup key (module docs) — or
    /// `None` if none exists.
    fn find_existing_evidence(
        &self,
        tx: &AuthorityTx<'_>,
        subject: &RelationEndpoint,
        source_event_id: &EventId,
    ) -> MemoryResult<Option<EvidenceId>> {
        let found: Option<String> = tx
            .conn()
            .query_row(
                "SELECT id FROM evidence_v2 \
                 WHERE subject_kind = ?1 AND subject_id = ?2 AND source_event_id = ?3 \
                 LIMIT 1",
                params![
                    subject.kind().as_str(),
                    subject.id(),
                    source_event_id.as_str()
                ],
                |r| r.get(0),
            )
            .optional()
            .map_err(StorageError::Sqlite)?;
        found.map(EvidenceId::new).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::model::relation_registry::RelationRegistry;
    use crate::model::{PolicyPartition, Version};
    use std::sync::Arc;

    const V1: Version = Version::first();

    fn fresh_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().expect("open in-memory authority"))
    }

    fn partition() -> PolicyPartition {
        PolicyPartition::new("user", "chat", 0).unwrap()
    }

    fn uuid(byte: u8) -> String {
        format!("018f4e2a-1c3b-7d4e-8f90-abcdef01234{byte}")
    }

    /// Resolve `related_to` (symmetric, entity↔entity) between two fixed
    /// entities, mirroring the 2.2.3 validator's happy-path fixture, and return
    /// the [`ResolvedRelationship`] a real 2.2.3 `Proceed` outcome would carry.
    fn resolved_related_to(db: &Database) -> ResolvedRelationship {
        let relation = db
            .with_read(|conn| RelationRegistry::resolve_definition(conn, "related_to", V1))
            .unwrap()
            .expect("related_to seeded by migration 0018");
        let source = RelationEndpoint::new(EndpointKind::Entity, uuid(1)).unwrap();
        let target = RelationEndpoint::new(EndpointKind::Entity, uuid(2)).unwrap();
        let validity = ValidInterval::open();
        let policy = partition();
        let identity =
            RelationshipIdentity::compute(&relation, &source, &target, &validity, &policy);
        ResolvedRelationship {
            relation,
            policy_partition: policy,
            identity,
        }
    }

    /// Fresh-insert inputs matching `resolved_related_to`'s endpoints/validity
    /// (the identity these inputs would insert with must equal
    /// `resolved.identity` for the tests below to be meaningful).
    fn new_relationship_inputs(_resolved: &ResolvedRelationship) -> NewRelationshipInputs {
        NewRelationshipInputs {
            source: RelationEndpoint::new(EndpointKind::Entity, uuid(1)).unwrap(),
            target: RelationEndpoint::new(EndpointKind::Entity, uuid(2)).unwrap(),
            validity: ValidInterval::open(),
            policy_source_id: SourceId::new_v7(),
            policy_version: PENDING_POLICY_VERSION.to_string(),
            created_event_id: None,
        }
    }

    fn evidence_inputs(polarity: EvidencePolarity) -> EvidenceInputs {
        EvidenceInputs {
            draft: EvidenceDraft::new(
                Locator::url("https://example.com/doc", None).unwrap(),
                Actor::new("tester").unwrap(),
                Method::new("manual_review", Some("1".to_string())).unwrap(),
                polarity,
            ),
            source_id: SourceId::new_v7(),
            policy: partition(),
            policy_version: PENDING_POLICY_VERSION.to_string(),
            created_event_id: None,
        }
    }

    fn relationship_row_count(db: &Database, identity_hash: &str) -> i64 {
        db.with_read(|conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM relationships_v2 WHERE identity_hash = ?1",
                    params![identity_hash],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(n)
        })
        .unwrap()
    }

    fn evidence_row_count(db: &Database, subject_id: &str) -> i64 {
        db.with_read(|conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM evidence_v2 WHERE subject_id = ?1",
                    params![subject_id],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(n)
        })
        .unwrap()
    }

    // ── No active row exists → a new relationship + evidence are created ──
    #[test]
    fn no_active_row_creates_new_relationship_and_evidence() {
        let db = fresh_db();
        let resolved = resolved_related_to(&db);
        let repo = TxRelationshipEvidence::new();

        let mut tx = db.begin().unwrap();
        let outcome = repo
            .append_or_create(
                &mut tx,
                &resolved,
                &new_relationship_inputs(&resolved),
                &evidence_inputs(EvidencePolarity::Supports),
            )
            .unwrap();
        tx.commit().unwrap();

        assert!(outcome.relationship_created);
        assert!(outcome.evidence_appended);
        assert_eq!(
            relationship_row_count(&db, resolved.identity.as_str()),
            1,
            "exactly one relationships_v2 row for this identity"
        );
        assert_eq!(evidence_row_count(&db, outcome.relationship_id.as_str()), 1);
    }

    // ── Existing active row with the same identity → evidence appends,
    //    NO second relationships_v2 row is created ────────────────────────
    #[test]
    fn existing_active_row_appends_evidence_without_duplicating_edge() {
        let db = fresh_db();
        let resolved = resolved_related_to(&db);
        let repo = TxRelationshipEvidence::new();

        // First observation creates the edge.
        let mut tx = db.begin().unwrap();
        let first = repo
            .append_or_create(
                &mut tx,
                &resolved,
                &new_relationship_inputs(&resolved),
                &evidence_inputs(EvidencePolarity::Supports),
            )
            .unwrap();
        tx.commit().unwrap();
        assert!(first.relationship_created);

        // Second, independent observation about the SAME identity.
        let mut tx = db.begin().unwrap();
        let second = repo
            .append_or_create(
                &mut tx,
                &resolved,
                &new_relationship_inputs(&resolved),
                &evidence_inputs(EvidencePolarity::Contradicts),
            )
            .unwrap();
        tx.commit().unwrap();

        assert!(
            !second.relationship_created,
            "an active edge already exists for this identity"
        );
        assert_eq!(second.relationship_id, first.relationship_id);
        assert_eq!(
            relationship_row_count(&db, resolved.identity.as_str()),
            1,
            "still exactly one relationships_v2 row for this identity"
        );
        assert_eq!(
            evidence_row_count(&db, first.relationship_id.as_str()),
            2,
            "evidence count increased"
        );
    }

    /// Insert a minimal `events_v2` row so a foreign-key-checked
    /// `source_event_id` reference is satisfiable in tests.
    fn insert_event(db: &Database, id: &EventId) {
        let tx = db.begin().unwrap();
        tx.conn()
            .execute(
                "INSERT INTO events_v2(
                     id, phase, hlc, ts_utc, tz_offset_min, event_type,
                     source_kind, source_id, actor_id,
                     namespace, owner_id, scope, sensitivity, policy_version,
                     payload_plain, payload_encoding, payload_checksum, schema_version)
                 VALUES (?1, 'observation', ?1, '2026-01-01T00:00:00+00:00', 0, 'test',
                         'native', 'src', 'tester',
                         'user', '', 'chat', 0, 'p1',
                         '{}', 'json/utf8', 'deadbeef', 1)",
                params![id.as_str()],
            )
            .unwrap();
        tx.commit().unwrap();
    }

    // ── Repeated identical evidence submission does not duplicate ─────────
    #[test]
    fn identical_evidence_resubmission_is_deduplicated() {
        let db = fresh_db();
        let resolved = resolved_related_to(&db);
        let repo = TxRelationshipEvidence::new();
        let source_event = EventId::new_v7();
        insert_event(&db, &source_event);

        let mut evidence = evidence_inputs(EvidencePolarity::Supports);
        evidence.draft = evidence.draft.with_source_event(source_event.clone());

        let mut tx = db.begin().unwrap();
        let first = repo
            .append_or_create(
                &mut tx,
                &resolved,
                &new_relationship_inputs(&resolved),
                &evidence,
            )
            .unwrap();
        tx.commit().unwrap();
        assert!(first.evidence_appended);

        // Resubmit the SAME observation (same subject + same source_event_id).
        let mut tx = db.begin().unwrap();
        let second = repo
            .append_or_create(
                &mut tx,
                &resolved,
                &new_relationship_inputs(&resolved),
                &evidence,
            )
            .unwrap();
        tx.commit().unwrap();

        assert!(
            !second.evidence_appended,
            "identical observation deduplicated"
        );
        assert_eq!(second.evidence_id, first.evidence_id);
        assert_eq!(
            evidence_row_count(&db, first.relationship_id.as_str()),
            1,
            "no duplicate evidence row"
        );
    }

    // ── Evidence row carries the correct locator/actor/method/version/
    //    polarity/score/policy ─────────────────────────────────────────────
    #[test]
    fn evidence_row_carries_expected_fields() {
        let db = fresh_db();
        let resolved = resolved_related_to(&db);
        let repo = TxRelationshipEvidence::new();

        let mut evidence = evidence_inputs(EvidencePolarity::Contradicts);
        evidence.draft = evidence
            .draft
            .with_score(0.75, "manual_confidence_estimate")
            .unwrap();

        let mut tx = db.begin().unwrap();
        let outcome = repo
            .append_or_create(
                &mut tx,
                &resolved,
                &new_relationship_inputs(&resolved),
                &evidence,
            )
            .unwrap();
        tx.commit().unwrap();

        db.with_read(|conn| {
            let (locator_json, actor_id, method, method_version, polarity, score, score_semantics): (
                String,
                String,
                String,
                Option<String>,
                String,
                Option<f64>,
                Option<String>,
            ) = conn
                .query_row(
                    "SELECT locator_json, actor_id, method, method_version, polarity, score, score_semantics \
                     FROM evidence_v2 WHERE id = ?1",
                    params![outcome.evidence_id.as_str()],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                        ))
                    },
                )
                .unwrap();
            assert!(locator_json.contains("example.com"));
            assert_eq!(actor_id, "tester");
            assert_eq!(method, "manual_review");
            assert_eq!(method_version, Some("1".to_string()));
            assert_eq!(polarity, "contradicts");
            assert_eq!(score, Some(0.75));
            assert_eq!(score_semantics, Some("manual_confidence_estimate".to_string()));
            Ok(())
        })
        .unwrap();
    }

    // ── A superseded row sharing the identity does NOT block a fresh active
    //    row, and lookup correctly treats it as inactive ────────────────────
    #[test]
    fn superseded_row_does_not_block_new_active_row() {
        let db = fresh_db();
        let resolved = resolved_related_to(&db);
        let repo = TxRelationshipEvidence::new();

        // Insert a row for this identity directly and mark it superseded —
        // simulating a prior edge that has since been superseded (task 2.2.5
        // territory; here we only need the row to exist with that disposition).
        {
            let tx = db.begin().unwrap();
            tx.conn()
                .execute(
                    "INSERT INTO relationships_v2(
                         id, source_kind, source_id, target_kind, target_id,
                         relation_name, relation_version, direction_class,
                         truth_state, authority_class,
                         namespace, owner_id, scope, sensitivity,
                         policy_source_id, policy_version, identity_hash)
                     VALUES ('018f4e2a-1c3b-7d4e-8f90-abcdef012349',
                              'entity', ?1, 'entity', ?2,
                              'related_to', 1, 'symmetric',
                              'superseded', 'stored',
                              'user', '', 'chat', 0,
                              ?3, 'p1', ?4)",
                    params![
                        uuid(1),
                        uuid(2),
                        SourceId::new_v7().as_str(),
                        resolved.identity.as_str(),
                    ],
                )
                .unwrap();
            tx.commit().unwrap();
        }

        // The lookup must NOT treat the superseded row as an active edge.
        {
            let tx = db.begin().unwrap();
            let found = repo
                .find_active_relationship(&tx, &resolved.identity)
                .unwrap();
            assert!(
                found.is_none(),
                "superseded row must be invisible to the active lookup"
            );
        }

        // append_or_create therefore inserts a NEW active row (does not fail
        // on the superseded row, and the unique index permits it since it only
        // constrains active rows).
        let mut tx = db.begin().unwrap();
        let outcome = repo
            .append_or_create(
                &mut tx,
                &resolved,
                &new_relationship_inputs(&resolved),
                &evidence_inputs(EvidencePolarity::Supports),
            )
            .unwrap();
        tx.commit().unwrap();

        assert!(outcome.relationship_created);
        assert_eq!(
            relationship_row_count(&db, resolved.identity.as_str()),
            2,
            "one superseded + one new active row for this identity"
        );
    }
}
