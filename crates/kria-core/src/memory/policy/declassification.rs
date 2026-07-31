//! Audited declassification as a **new governed provenance record** (task
//! **F1.4.3**; design §4 MGR-004 AC3, MGR-035, MGR-043; MGD-007).
//!
//! MGR-004 AC3 is categorical: "WHEN an authorized declassification occurs, THE
//! Write_Policy_Engine SHALL **create a new audited provenance record rather
//! than mutate source policy**." A declassification relaxes or reclassifies the
//! [`EffectivePolicy`] that governs a target record or source (e.g. lowering
//! sensitivity, broadening scope, or reclassifying provenance). This module
//! models that operation so it can **only** be expressed as a new, immutable,
//! audited record — never an in-place edit of a contributing `sources` row.
//!
//! ## The non-mutation invariant, by construction
//!
//! A [`Declassification`] captures the **prior** Effective Policy *by value*
//! (its provenance hash + a serialized snapshot) and the **new** declassified
//! policy alongside it. It exposes no API that mutates the prior policy or the
//! contributing source: the only durable effect is an `INSERT` into the
//! append-only `declassifications` table (schema 0016, UPDATE/DELETE forbidden
//! by trigger). The original source policy therefore survives verbatim for
//! audit, and downstream effective-policy computation treats the new record as
//! a superseding derivation rather than an overwrite.
//!
//! ## It is a governed authority write
//!
//! A declassification is itself a governed command
//! ([`CommandKind::Declassify`]), previewed and confirmed like any other
//! corrective operation, and admitted only when the authorizing source carries
//! [`Capability::RequestDeclassification`] (MGR-043 capability context). Routed
//! through the [`AuthorityTransaction`] it commits atomically with an immutable
//! completion [`event`](crate::memory::authority::event_log) and an immutable
//! [`audit_record`](crate::memory::authority::audit), and is **reversible** via
//! a compensating declassification whose audit row links back through
//! `reversal_of` (design §5.1 / MGR-005 AC6). An actor without declassification
//! authority is denied with a typed [`DeclassificationError::Unauthorized`] and
//! no record is created.

use serde_json::{json, Value};

use crate::memory::authority::{
    AuthorityTransaction, CommandEnvelope, CommandKind, CommandRecord, GraphChange,
    GraphChangeKind, SemanticOutcome, TxSemanticStore,
};
use crate::memory::db::{AuthorityTx, Database};
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::ids::blake3_hex;
use crate::memory::model::{AuditId, DeclassificationId, UtcTimestamp};

use super::effective_policy::EffectivePolicy;
use super::source_trust::{Capability, SourceProfile};

/// The stable declassification-record version, mixed into the integrity
/// [`provenance_hash`](Declassification::provenance_hash) for domain
/// separation. Bump only when the record's semantic field set changes.
pub const DECLASSIFICATION_VERSION: &str = "declassification-v1";

/// Maximum length of a declassification justification, in bytes.
pub const REASON_MAX_LEN: usize = 4096;

/// Maximum length of an actor id / target id, in bytes.
pub const ID_MAX_LEN: usize = 512;

// ─────────────────────────────────────────────────────────────────────────
// DeclassificationError — the typed denial / validation outcome
// ─────────────────────────────────────────────────────────────────────────

/// Why a declassification could not be constructed. [`Unauthorized`] is the
/// governance denial MGR-004 AC3 requires — an actor lacking
/// [`Capability::RequestDeclassification`] can never produce a record;
/// [`Invalid`] carries a field-validation message. Neither variant ever writes
/// to the authority (no record is created on error).
///
/// [`Unauthorized`]: DeclassificationError::Unauthorized
/// [`Invalid`]: DeclassificationError::Invalid
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclassificationError {
    /// The authorizing source lacks [`Capability::RequestDeclassification`].
    Unauthorized,
    /// A field failed validation (empty/oversize/control-char actor, target, or
    /// reason).
    Invalid(String),
}

impl std::fmt::Display for DeclassificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeclassificationError::Unauthorized => f.write_str(
                "declassification denied: source lacks request_declassification capability",
            ),
            DeclassificationError::Invalid(msg) => write!(f, "invalid declassification: {msg}"),
        }
    }
}

impl std::error::Error for DeclassificationError {}

/// Validate a bounded, non-empty, control-char-free identifier/justification.
fn validate_text(field: &str, value: &str, max_len: usize) -> Result<(), DeclassificationError> {
    if value.trim().is_empty() {
        return Err(DeclassificationError::Invalid(format!(
            "{field} must not be empty"
        )));
    }
    if value.len() > max_len {
        return Err(DeclassificationError::Invalid(format!(
            "{field} too long: {} bytes (max {max_len})",
            value.len()
        )));
    }
    if let Some(bad) = value.chars().find(|c| c.is_control()) {
        return Err(DeclassificationError::Invalid(format!(
            "{field} contains control character {bad:?}"
        )));
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// DeclassificationTarget — what is being declassified
// ─────────────────────────────────────────────────────────────────────────

/// Whether a declassification targets a cognitive record or a source
/// (`declassifications.target_kind CHECK`). A closed set so the target kind is
/// never a raw unchecked string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeclassificationTargetKind {
    /// A single cognitive record.
    Record,
    /// A contributing source (whose policy is preserved; a new record is added).
    Source,
}

impl DeclassificationTargetKind {
    /// The canonical text stored in `declassifications.target_kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            DeclassificationTargetKind::Record => "record",
            DeclassificationTargetKind::Source => "source",
        }
    }
}

impl std::fmt::Display for DeclassificationTargetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The validated target of a declassification: the kind plus the target's
/// bounded, non-empty identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclassificationTarget {
    kind: DeclassificationTargetKind,
    id: String,
}

impl DeclassificationTarget {
    /// A record target with a validated identifier.
    pub fn record(id: impl Into<String>) -> Result<Self, DeclassificationError> {
        Self::new(DeclassificationTargetKind::Record, id)
    }

    /// A source target with a validated identifier.
    pub fn source(id: impl Into<String>) -> Result<Self, DeclassificationError> {
        Self::new(DeclassificationTargetKind::Source, id)
    }

    fn new(
        kind: DeclassificationTargetKind,
        id: impl Into<String>,
    ) -> Result<Self, DeclassificationError> {
        let id = id.into();
        validate_text("declassification target id", &id, ID_MAX_LEN)?;
        Ok(Self { kind, id })
    }

    /// The target kind.
    pub fn kind(&self) -> DeclassificationTargetKind {
        self.kind
    }

    /// The target identifier.
    pub fn id(&self) -> &str {
        &self.id
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Declassification — the immutable governed provenance record
// ─────────────────────────────────────────────────────────────────────────

/// A new audited declassification provenance record (MGR-004 AC3). Captures the
/// target, the **prior** Effective Policy (by value + hash), the **new**
/// declassified policy (by value + hash), the authorizing actor, the
/// justification, an integrity `provenance_hash` over the semantic content, and
/// an optional `reverses` link to the record a compensating declassification
/// undoes.
///
/// Construct only through [`authorize`](Declassification::authorize) (or
/// [`reverse`](Declassification::reverse)), which enforce the
/// [`Capability::RequestDeclassification`] gate. The type carries no method that
/// mutates the prior policy or a contributing source — its only durable effect
/// is an append to the immutable `declassifications` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declassification {
    id: DeclassificationId,
    target: DeclassificationTarget,
    prior_policy: EffectivePolicy,
    new_policy: EffectivePolicy,
    prior_policy_hash: String,
    new_policy_hash: String,
    actor_id: String,
    reason: String,
    reverses: Option<DeclassificationId>,
    created_at: UtcTimestamp,
    provenance_hash: String,
}

impl Declassification {
    /// Authorize and build a declassification record.
    ///
    /// Denies with [`DeclassificationError::Unauthorized`] — creating **no**
    /// record — unless `authorizing` carries
    /// [`Capability::RequestDeclassification`] (MGR-004 AC3, MGR-043). Validates
    /// the actor id and justification, captures the `prior` and `declassified`
    /// policies by value, and computes the integrity provenance hash.
    pub fn authorize(
        authorizing: &SourceProfile,
        actor_id: impl Into<String>,
        target: DeclassificationTarget,
        prior: EffectivePolicy,
        declassified: EffectivePolicy,
        reason: impl Into<String>,
    ) -> Result<Self, DeclassificationError> {
        Self::build(
            authorizing,
            actor_id,
            target,
            prior,
            declassified,
            reason,
            None,
        )
    }

    /// Build a **compensating** declassification that reverses `self`: it swaps
    /// the prior/new policies (restoring the pre-declassification policy as the
    /// new effective policy) and links `reverses` back to `self`. Like
    /// [`authorize`](Self::authorize) it requires
    /// [`Capability::RequestDeclassification`] and creates a *new* record — the
    /// original is never mutated or deleted (MGR-005 AC6 compensating record).
    pub fn reverse(
        &self,
        authorizing: &SourceProfile,
        actor_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<Self, DeclassificationError> {
        Self::build(
            authorizing,
            actor_id,
            self.target.clone(),
            self.new_policy.clone(),
            self.prior_policy.clone(),
            reason,
            Some(self.id.clone()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        authorizing: &SourceProfile,
        actor_id: impl Into<String>,
        target: DeclassificationTarget,
        prior: EffectivePolicy,
        declassified: EffectivePolicy,
        reason: impl Into<String>,
        reverses: Option<DeclassificationId>,
    ) -> Result<Self, DeclassificationError> {
        // MGR-004 AC3 authorization gate — checked before anything is built.
        if !authorizing.permits(Capability::RequestDeclassification) {
            return Err(DeclassificationError::Unauthorized);
        }
        let actor_id = actor_id.into();
        let reason = reason.into();
        validate_text("declassification actor_id", &actor_id, ID_MAX_LEN)?;
        validate_text("declassification reason", &reason, REASON_MAX_LEN)?;

        let prior_policy_hash = prior.provenance_hash().to_string();
        let new_policy_hash = declassified.provenance_hash().to_string();
        let created_at = UtcTimestamp::now();
        let provenance_hash = compute_provenance_hash(
            &target,
            &prior_policy_hash,
            &new_policy_hash,
            &actor_id,
            &reason,
            reverses.as_ref(),
        );

        Ok(Self {
            id: DeclassificationId::new_v7(),
            target,
            prior_policy: prior,
            new_policy: declassified,
            prior_policy_hash,
            new_policy_hash,
            actor_id,
            reason,
            reverses,
            created_at,
            provenance_hash,
        })
    }

    /// The record identity (`declassifications.id`).
    pub fn id(&self) -> &DeclassificationId {
        &self.id
    }

    /// The declassification target.
    pub fn target(&self) -> &DeclassificationTarget {
        &self.target
    }

    /// The prior Effective Policy, captured by value — the contributing source
    /// policy is preserved here verbatim and never mutated.
    pub fn prior_policy(&self) -> &EffectivePolicy {
        &self.prior_policy
    }

    /// The new declassified policy this record establishes.
    pub fn new_policy(&self) -> &EffectivePolicy {
        &self.new_policy
    }

    /// The provenance hash of the prior policy.
    pub fn prior_policy_hash(&self) -> &str {
        &self.prior_policy_hash
    }

    /// The provenance hash of the new declassified policy.
    pub fn new_policy_hash(&self) -> &str {
        &self.new_policy_hash
    }

    /// The authorizing actor.
    pub fn actor_id(&self) -> &str {
        &self.actor_id
    }

    /// The recorded justification.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The record this declassification reverses, when it is compensating.
    pub fn reverses(&self) -> Option<&DeclassificationId> {
        self.reverses.as_ref()
    }

    /// The integrity digest over the semantic content (version-prefixed BLAKE3).
    pub fn provenance_hash(&self) -> &str {
        &self.provenance_hash
    }

    /// The creation instant.
    pub fn created_at(&self) -> UtcTimestamp {
        self.created_at
    }

    /// The canonical JSON body carried as the governed command payload and
    /// recorded on the immutable completion event.
    pub fn to_json(&self) -> Value {
        json!({
            "version": DECLASSIFICATION_VERSION,
            "id": self.id.as_str(),
            "target": {
                "kind": self.target.kind.as_str(),
                "id": self.target.id,
            },
            "prior_policy": self.prior_policy,
            "prior_policy_hash": self.prior_policy_hash,
            "new_policy": self.new_policy,
            "new_policy_hash": self.new_policy_hash,
            "actor_id": self.actor_id,
            "reason": self.reason,
            "reverses": self.reverses.as_ref().map(DeclassificationId::as_str),
            "provenance_hash": self.provenance_hash,
            "created_at": self.created_at.to_rfc3339(),
        })
    }
}

/// The integrity provenance hash over a declassification's semantic content
/// (version, target, prior/new policy hashes, actor, reason, reversal link).
/// Excludes the random id and creation time so it is a stable content digest.
fn compute_provenance_hash(
    target: &DeclassificationTarget,
    prior_policy_hash: &str,
    new_policy_hash: &str,
    actor_id: &str,
    reason: &str,
    reverses: Option<&DeclassificationId>,
) -> String {
    let mut input = String::new();
    for part in [
        DECLASSIFICATION_VERSION,
        target.kind.as_str(),
        target.id.as_str(),
        prior_policy_hash,
        new_policy_hash,
        actor_id,
        reason,
        reverses.map(DeclassificationId::as_str).unwrap_or(""),
    ] {
        input.push_str(part);
        input.push('\n');
    }
    blake3_hex(input.as_bytes())
}

// ─────────────────────────────────────────────────────────────────────────
// DeclassificationWrite — the transaction-scoped semantic store
// ─────────────────────────────────────────────────────────────────────────

/// The transaction-scoped semantic store that persists a [`Declassification`]
/// as the command's semantic mutation, using **only** the serialized-writer
/// transaction connection (the [`TxSemanticStore`] contract). Its sole effect
/// is an `INSERT` into the append-only `declassifications` table — it never
/// touches the contributing `sources` row (MGR-004 AC3).
pub struct DeclassificationWrite<'a> {
    declassification: &'a Declassification,
}

impl<'a> DeclassificationWrite<'a> {
    /// Wrap the declassification to be written on the transaction.
    pub fn new(declassification: &'a Declassification) -> Self {
        Self { declassification }
    }
}

impl TxSemanticStore for DeclassificationWrite<'_> {
    fn apply(
        &self,
        tx: &mut AuthorityTx<'_>,
        env: &CommandEnvelope,
    ) -> MemoryResult<SemanticOutcome> {
        let d = self.declassification;
        let prior_json = serde_json::to_string(&d.prior_policy)
            .map_err(|e| StorageError::Serde(e.to_string()))?;
        let new_json =
            serde_json::to_string(&d.new_policy).map_err(|e| StorageError::Serde(e.to_string()))?;

        tx.conn()
            .execute(
                "INSERT INTO declassifications(
                     id, target_kind, target_id,
                     prior_policy_hash, prior_policy_json,
                     new_policy_hash, new_policy_json,
                     actor_id, reason, provenance_hash,
                     invocation_id, reverses, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                rusqlite::params![
                    d.id.as_str(),
                    d.target.kind.as_str(),
                    d.target.id,
                    d.prior_policy_hash,
                    prior_json,
                    d.new_policy_hash,
                    new_json,
                    d.actor_id,
                    d.reason,
                    d.provenance_hash,
                    env.source().invocation_id().as_str(),
                    d.reverses.as_ref().map(DeclassificationId::as_str),
                    d.created_at.to_rfc3339(),
                ],
            )
            .map_err(StorageError::Sqlite)?;

        // Graph-visible: a new provenance record was inserted. It is not a
        // projected record (fts/vectors/scene), so the change carries no
        // `record_id` — it reserves a revision but enqueues no projection work.
        let mut change = GraphChange::new(GraphChangeKind::Insert, env.caller().partition_key());
        change.record_kind = Some("declassification".to_string());
        let change = change.with_payload(
            serde_json::to_string(&d.to_json()).map_err(|e| StorageError::Serde(e.to_string()))?,
        );
        Ok(SemanticOutcome::graph_visible(vec![change]))
    }
}

/// Commit a declassification as a governed authority write.
///
/// Runs the full accepted-command flow over one serialized [`AuthorityTransaction`]:
/// start/completion events, the `declassifications` insert (via
/// [`DeclassificationWrite`]), one reserved revision, and the immutable audit
/// row — all atomically. `env` MUST be a [`CommandKind::Declassify`] envelope
/// whose payload is [`Declassification::to_json`] (build it with
/// [`declassification_command`]); `reversal_of` links the audit row to the row a
/// compensating declassification undoes (`None` for an original).
pub fn commit_declassification(
    db: &Database,
    env: &CommandEnvelope,
    declassification: &Declassification,
    reversal_of: Option<&AuditId>,
) -> MemoryResult<CommandRecord> {
    debug_assert_eq!(env.kind(), CommandKind::Declassify);
    let store = DeclassificationWrite::new(declassification);
    let tx = AuthorityTransaction::begin(db)?;
    tx.commit_accepted_command(env, &store, reversal_of)
}

/// Build the governed [`CommandEnvelope`] for a declassification: a
/// [`CommandKind::Declassify`] command carrying [`Declassification::to_json`] as
/// its payload and the confirming [`PreviewToken`]. Keeping the payload derived
/// from the record here guarantees the immutable event log records exactly the
/// declassification that [`commit_declassification`] persists.
#[allow(clippy::too_many_arguments)]
pub fn declassification_command(
    caller: crate::memory::model::CallerContext,
    idempotency_key: crate::memory::model::IdempotencyKey,
    base_revision: crate::memory::model::GraphRevision,
    source: crate::memory::authority::SourceContext,
    mode: crate::memory::types::MemoryMode,
    deadline: crate::memory::authority::Deadline,
    preview_token: crate::memory::authority::PreviewToken,
    declassification: &Declassification,
) -> MemoryResult<CommandEnvelope> {
    CommandEnvelope::new(
        caller,
        CommandKind::Declassify,
        idempotency_key,
        base_revision,
        source,
        mode,
        deadline,
        declassification.to_json(),
        Some(preview_token),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::authority::{SourceContext, SourceKind, SourceTrust};
    use crate::memory::db::Database;
    use crate::memory::model::{
        CallerContext, GraphRevision, IdempotencyKey, InvocationId, PolicyPartition,
    };
    use crate::memory::policy::effective_policy::{ContributingPolicy, EffectivePolicy};
    use crate::memory::policy::source_trust::{
        Capability, CapabilitySet, ConsentRequirement, SourceCategory, SourceProfile,
    };
    use crate::memory::types::MemoryMode;
    use rusqlite::params;
    use std::sync::Arc;

    // ── Fixtures ────────────────────────────────────────────────────────

    /// A source profile that carries `RequestDeclassification` (native, first
    /// party) — the authorized case.
    fn authorized() -> SourceProfile {
        let p = SourceCategory::NativeTool.profile();
        assert!(
            p.permits(Capability::RequestDeclassification),
            "native profile must carry request_declassification"
        );
        p
    }

    /// A source profile that lacks `RequestDeclassification` (cloud is
    /// observe-only) — the denial case.
    fn unauthorized() -> SourceProfile {
        let p = SourceCategory::Cloud.profile();
        assert!(
            !p.permits(Capability::RequestDeclassification),
            "cloud profile must not carry request_declassification"
        );
        p
    }

    /// A single-contributor Effective Policy at the given sensitivity (always an
    /// Allow — it carries observe/read capabilities).
    fn policy_at(sensitivity: u8) -> EffectivePolicy {
        let partition = PolicyPartition::new("user", "chat", sensitivity).unwrap();
        let contributor = ContributingPolicy::new(
            "src-original",
            partition,
            CapabilitySet::from_capabilities([Capability::ObserveMemory, Capability::ReadCore]),
            SourceTrust::System,
            ConsentRequirement::NotRequired,
        )
        .unwrap();
        let ep = EffectivePolicy::of(contributor);
        assert!(ep.is_allowed());
        ep
    }

    fn declassification_for(
        profile: &SourceProfile,
        target: DeclassificationTarget,
    ) -> Result<Declassification, DeclassificationError> {
        // Relax sensitivity 3 → 1.
        Declassification::authorize(
            profile,
            "actor:owner",
            target,
            policy_at(3),
            policy_at(1),
            "declassify quarterly report for sharing",
        )
    }

    // ── Authorization gate (MGR-004 AC3) ────────────────────────────────

    #[test]
    fn unauthorized_source_is_denied_and_creates_no_record() {
        let err = declassification_for(
            &unauthorized(),
            DeclassificationTarget::record("rec-1").unwrap(),
        )
        .expect_err("cloud source must be denied");
        assert_eq!(err, DeclassificationError::Unauthorized);
    }

    #[test]
    fn authorized_source_builds_a_record() {
        let d = declassification_for(
            &authorized(),
            DeclassificationTarget::source("src-original").unwrap(),
        )
        .expect("native source is authorized");
        assert_eq!(d.actor_id(), "actor:owner");
        assert_eq!(d.reason(), "declassify quarterly report for sharing");
        assert_eq!(d.target().kind(), DeclassificationTargetKind::Source);
        assert!(d.reverses().is_none());
    }

    // ── Field validation ────────────────────────────────────────────────

    #[test]
    fn empty_and_oversize_and_control_fields_are_rejected() {
        let target = DeclassificationTarget::record("rec-1").unwrap();
        // Empty actor.
        assert!(matches!(
            Declassification::authorize(
                &authorized(),
                "  ",
                target.clone(),
                policy_at(3),
                policy_at(1),
                "reason"
            ),
            Err(DeclassificationError::Invalid(_))
        ));
        // Empty reason.
        assert!(matches!(
            Declassification::authorize(
                &authorized(),
                "actor",
                target.clone(),
                policy_at(3),
                policy_at(1),
                ""
            ),
            Err(DeclassificationError::Invalid(_))
        ));
        // Control char in reason.
        assert!(matches!(
            Declassification::authorize(
                &authorized(),
                "actor",
                target.clone(),
                policy_at(3),
                policy_at(1),
                "bad\nreason"
            ),
            Err(DeclassificationError::Invalid(_))
        ));
        // Oversize actor.
        assert!(matches!(
            Declassification::authorize(
                &authorized(),
                "a".repeat(ID_MAX_LEN + 1),
                target,
                policy_at(3),
                policy_at(1),
                "reason"
            ),
            Err(DeclassificationError::Invalid(_))
        ));
        // Empty target id.
        assert!(matches!(
            DeclassificationTarget::record("   "),
            Err(DeclassificationError::Invalid(_))
        ));
    }

    // ── Record content: prior/new captured by value; from→to audit trail ──

    #[test]
    fn record_captures_prior_and_new_policy_by_value() {
        let prior = policy_at(3);
        let new = policy_at(1);
        let d = Declassification::authorize(
            &authorized(),
            "actor",
            DeclassificationTarget::source("src-original").unwrap(),
            prior.clone(),
            new.clone(),
            "reason",
        )
        .unwrap();

        // Prior policy is preserved verbatim (the contributing source policy is
        // captured, never referenced/mutated).
        assert_eq!(d.prior_policy(), &prior);
        assert_eq!(d.new_policy(), &new);
        assert_eq!(d.prior_policy_hash(), prior.provenance_hash());
        assert_eq!(d.new_policy_hash(), new.provenance_hash());
        // A relaxation actually changed the policy hash (from → to).
        assert_ne!(d.prior_policy_hash(), d.new_policy_hash());
    }

    #[test]
    fn to_json_records_full_audit_trail() {
        let d = declassification_for(
            &authorized(),
            DeclassificationTarget::record("rec-42").unwrap(),
        )
        .unwrap();
        let j = d.to_json();
        assert_eq!(j["version"], DECLASSIFICATION_VERSION);
        assert_eq!(j["actor_id"], "actor:owner"); // who
        assert_eq!(j["reason"], "declassify quarterly report for sharing"); // why
        assert_eq!(j["target"]["kind"], "record");
        assert_eq!(j["target"]["id"], "rec-42");
        assert_eq!(j["prior_policy_hash"], d.prior_policy_hash()); // from
        assert_eq!(j["new_policy_hash"], d.new_policy_hash()); // to
        assert!(j["created_at"].is_string()); // when
        assert!(j["prior_policy"].is_object());
        assert!(j["new_policy"].is_object());
    }

    // ── Provenance hash: content-addressed and deterministic ────────────

    #[test]
    fn provenance_hash_is_content_addressed() {
        let base = || {
            declassification_for(
                &authorized(),
                DeclassificationTarget::record("rec-1").unwrap(),
            )
            .unwrap()
        };
        // Two records with identical semantic content share a provenance hash
        // (random id / timestamp are excluded from the digest).
        assert_eq!(base().provenance_hash(), base().provenance_hash());

        // A different reason changes the hash.
        let other = Declassification::authorize(
            &authorized(),
            "actor:owner",
            DeclassificationTarget::record("rec-1").unwrap(),
            policy_at(3),
            policy_at(1),
            "a different justification entirely",
        )
        .unwrap();
        assert_ne!(base().provenance_hash(), other.provenance_hash());
    }

    // ── Reversal: compensating record, never a mutation/delete ──────────

    #[test]
    fn reverse_swaps_policies_and_links_back() {
        let original = declassification_for(
            &authorized(),
            DeclassificationTarget::source("src-original").unwrap(),
        )
        .unwrap();
        let reversal = original
            .reverse(&authorized(), "actor:owner", "undo the declassification")
            .unwrap();

        // The reversal restores the pre-declassification policy as its new
        // policy (prior/new swapped) and links back to the original.
        assert_eq!(reversal.new_policy(), original.prior_policy());
        assert_eq!(reversal.prior_policy(), original.new_policy());
        assert_eq!(reversal.reverses(), Some(original.id()));
        // A distinct record, not an edit of the original.
        assert_ne!(reversal.id(), original.id());
    }

    #[test]
    fn reverse_also_requires_authorization() {
        let original = declassification_for(
            &authorized(),
            DeclassificationTarget::record("rec-1").unwrap(),
        )
        .unwrap();
        let err = original
            .reverse(&unauthorized(), "actor", "undo")
            .expect_err("reversal must also be authorized");
        assert_eq!(err, DeclassificationError::Unauthorized);
    }

    // ── Governed authority write: end-to-end over the transaction ───────

    fn fresh_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().expect("open in-memory authority"))
    }

    fn caller() -> CallerContext {
        let partition = PolicyPartition::new("user", "chat", 0).unwrap();
        CallerContext::local_desktop("local-desktop", partition).unwrap()
    }

    fn declassify_envelope(d: &Declassification) -> CommandEnvelope {
        declassification_command(
            caller(),
            IdempotencyKey::new("cmd-declassify-1").unwrap(),
            GraphRevision::base(),
            SourceContext::new(
                InvocationId::new_v7(),
                SourceKind::Native,
                "core:cognition",
                SourceTrust::System,
            )
            .unwrap(),
            MemoryMode::Permanent,
            crate::memory::authority::Deadline::default_write(),
            crate::memory::authority::PreviewToken::new("tok-declassify").unwrap(),
            d,
        )
        .unwrap()
    }

    /// Insert a contributing `sources` policy row to prove it is never mutated.
    fn seed_source(db: &Database, id: &str) {
        let tx = db.begin().unwrap();
        tx.conn()
            .execute(
                "INSERT INTO sources(
                     id, source_kind, namespace, owner_id, scope, sensitivity, policy_version)
                 VALUES (?1, 'native', 'user', 'owner-1', 'chat', 3, 'baseline-policy-v1')",
                params![id],
            )
            .unwrap();
        tx.commit().unwrap();
    }

    fn source_policy_row(db: &Database, id: &str) -> (i64, String) {
        db.with_read(|conn| {
            let row = conn
                .query_row(
                    "SELECT sensitivity, policy_version FROM sources WHERE id = ?1",
                    params![id],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                )
                .unwrap();
            Ok(row)
        })
        .unwrap()
    }

    #[test]
    fn commit_creates_new_record_and_never_mutates_source_policy() {
        let db = fresh_db();
        seed_source(&db, "src-original");
        let before = source_policy_row(&db, "src-original");
        assert_eq!(before, (3, "baseline-policy-v1".to_string()));

        let d = declassification_for(
            &authorized(),
            DeclassificationTarget::source("src-original").unwrap(),
        )
        .unwrap();
        let env = declassify_envelope(&d);
        let record = commit_declassification(&db, &env, &d, None).expect("commit declassification");

        // 1) The contributing source policy row is byte-for-byte unchanged.
        let after = source_policy_row(&db, "src-original");
        assert_eq!(after, before, "source policy must never be mutated");

        // 2) A new immutable provenance record captured the relaxation, with the
        //    full who/when/from→to/reason audit trail.
        db.with_read(|conn| {
            let (target_kind, target_id, prior_hash, new_hash, actor, reason, prov, inv): (
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
            ) = conn
                .query_row(
                    "SELECT target_kind, target_id, prior_policy_hash, new_policy_hash,
                            actor_id, reason, provenance_hash, invocation_id
                     FROM declassifications WHERE id = ?1",
                    params![d.id().as_str()],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                            r.get(7)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(target_kind, "source");
            assert_eq!(target_id, "src-original");
            assert_eq!(prior_hash, d.prior_policy_hash());
            assert_eq!(new_hash, d.new_policy_hash());
            assert_ne!(prior_hash, new_hash, "from → to relaxation recorded");
            assert_eq!(actor, "actor:owner");
            assert_eq!(reason, "declassify quarterly report for sharing");
            assert_eq!(prov, d.provenance_hash());
            assert_eq!(inv, env.source().invocation_id().as_str());
            Ok(())
        })
        .unwrap();

        // 3) It committed as an accepted governed command: immutable audit row
        //    with disposition=accepted + command_kind=declassify.
        db.with_read(|conn| {
            let (kind, disposition): (String, String) = conn
                .query_row(
                    "SELECT command_kind, disposition FROM audit_records
                     WHERE event_id = ?1",
                    params![record.event.event_id.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(kind, "declassify");
            assert_eq!(disposition, "accepted");
            Ok(())
        })
        .unwrap();

        // 4) Graph-visible → exactly one revision reserved and one insert change
        //    of record_kind 'declassification' (no projected record_id).
        assert!(
            record.revision.is_some(),
            "declassification reserves a revision"
        );
        db.with_read(|conn| {
            let (change_kind, record_kind, record_id): (String, String, Option<String>) = conn
                .query_row(
                    "SELECT change_kind, record_kind, record_id FROM graph_changes
                     WHERE revision = ?1 AND ordinal = 0",
                    params![record.revision.unwrap().get() as i64],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .unwrap();
            assert_eq!(change_kind, "insert");
            assert_eq!(record_kind, "declassification");
            assert!(
                record_id.is_none(),
                "declassification is not a projected record"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn declassifications_table_is_append_only() {
        let db = fresh_db();
        let d = declassification_for(
            &authorized(),
            DeclassificationTarget::record("rec-1").unwrap(),
        )
        .unwrap();
        let env = declassify_envelope(&d);
        commit_declassification(&db, &env, &d, None).unwrap();

        // UPDATE / DELETE both abort (immutable provenance).
        let tx = db.begin().unwrap();
        let update = tx.conn().execute(
            "UPDATE declassifications SET reason = 'tampered' WHERE id = ?1",
            params![d.id().as_str()],
        );
        assert!(update.is_err(), "declassifications must reject UPDATE");
        drop(tx);

        let tx = db.begin().unwrap();
        let delete = tx.conn().execute(
            "DELETE FROM declassifications WHERE id = ?1",
            params![d.id().as_str()],
        );
        assert!(delete.is_err(), "declassifications must reject DELETE");
    }
}
