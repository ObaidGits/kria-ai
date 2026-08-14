//! Narrow authority ports — the governed authority surface adapters depend on.
//!
//! Task **F1.2.3**: define the narrow trait ports that the memory composition
//! root (`MemorySystem`, see [`crate::api`]) exposes for the durable
//! command surface, the read surface, the derived-projection outbox, and the
//! integrity/recovery inspection surface. Per design §19.1 ("adapters depend on
//! ports; only the composition module constructs concrete implementations"),
//! these traits are the seam between the single authority [`Database`] and every
//! caller — so no adapter has to own SQL, policy, or revision semantics.
//!
//! ## What this module is (and is not)
//!
//! * It **defines** the four narrow ports ([`CommandPort`], [`QueryPort`],
//!   [`OutboxPort`], [`IntegrityPort`]) and their DTOs, reusing the validated
//!   value objects from [`crate::model`] so a raw unchecked string can
//!   never cross the boundary.
//! * It provides **concrete, tested implementations** for the two ports that map
//!   directly onto schema/authority surfaces that already exist:
//!   [`SqliteOutbox`] (over the `derived_outbox` table) and
//!   [`AuthorityIntegrity`] (over [`Database::quick_check`],
//!   `authority_meta.graph_revision`, and [`Database::reconciliation_report`]).
//! * [`CommandPort`] and [`QueryPort`] are **trait + DTO definitions only** here:
//!   the real governed write path ([`crate::write_policy::WritePolicy`]
//!   → `AuthorityTx`) and the read pipeline are wired behind these ports in
//!   **F1.3** (authority transaction) / later gates. Defining them now fixes the
//!   dependency direction without rewriting the write path or faking behavior.
//!
//! The single authority handle ([`Arc<Database>`]) is injected once by the
//! composition root and threaded into every concrete port (see
//! [`crate::api::MemorySystem::compose`],
//! [`crate::api::MemorySystem::outbox`], and
//! [`crate::api::MemorySystem::integrity`]).

use std::sync::Arc;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::{Database, ReconciliationReport};
use crate::error::{MemoryResult, StorageError};
use crate::model::{
    EventId, GraphRevision, IdempotencyKey, PolicyPartition, RecordId, UtcTimestamp,
};

pub mod audit;
pub mod bus;
pub mod candidates;
pub mod command;
pub mod event_log;
pub mod idempotency;
pub mod integrity;
pub mod outbox;
pub mod publish;
pub mod relationship_commands;
pub mod relationship_evidence;
pub mod relationship_validation;
pub mod revision;
pub mod transaction;
pub mod validation;

pub use audit::{AppendedAudit, AuditDisposition, AuditDraft, TxAuditLog};
pub use bus::{AuthorityCommandBus, GovernedOutcome};
pub use candidates::{CommandCandidate, WriteContext};
pub use command::{
    CommandEnvelope, CommandHash, CommandKindExt, Deadline, PreviewToken, SourceContext,
    SourceKind, SourceTrust,
};
pub use event_log::{AppendedEvent, EventPhase, TxEventLog};
pub use idempotency::{canonical_result_json, TxIdempotency, COMMITTED_STATUS};
pub use integrity::{
    CapabilityState, IntegrityFaultClass, RecoveryCheckReport, RecoveryFault,
    RecoveryIntegrityChecker, StartupError, StartupIntegrityChecker,
};
pub use outbox::{projection_work_for_changes, TxOutbox, PROJECTION_TARGETS};
pub use publish::{
    revisions_since, CommittedRevision, NoopWakePublisher, RevisionWake, WakePublisher,
};
pub use relationship_commands::{
    RelationshipCommandBus, RelationshipCommandOutcome, RelationshipConfirmInputs,
    RelationshipCreateInputs, RelationshipDeleteInputs, RelationshipEditInputs,
    RelationshipExpireInputs, RelationshipLifecycleError, RelationshipRestoreInputs,
    RelationshipUndoInputs, TxRelationshipConfirm, TxRelationshipCreate, TxRelationshipDelete,
    TxRelationshipEdit, TxRelationshipExpire, TxRelationshipRestore, TxRelationshipUndo,
    UndoTarget,
};
pub use relationship_evidence::{
    AppendedRelationshipEvidence, EvidenceDraft, EvidenceInputs, NewRelationshipInputs,
    TxRelationshipEvidence,
};
pub use relationship_validation::{
    EndpointReads, EndpointRef, EvidenceInput, RelationshipRejectionCode,
    RelationshipRejectionReason, RelationshipValidationOutcome, RelationshipValidator,
    RelationshipWriteRequest, ResolvedRelationship, SqliteEndpointReads,
};
pub use revision::{GraphChange, GraphChangeKind, TxRevisionLog};
pub use transaction::{
    record_rejected_command, AuthorityTransaction, CommandRecord, DeferredSemanticStore,
    SemanticOutcome, TxSemanticStore,
};
pub use validation::{
    is_command_capability_permitted, CommandValidator, RejectionCode, RejectionReason,
    StoredIdempotencyResult, ValidationConfig, ValidationOutcome, ValidationReads,
};

// ─────────────────────────────────────────────────────────────────────────
// CommandPort — the governed durable write boundary (design §5.1)
// ─────────────────────────────────────────────────────────────────────────

/// The kind of governed command submitted to the authority (design §5.1/§19.4).
/// A narrow, closed set: destructive/corrective kinds flow through the same gate
/// as observations so every durable mutation is auditable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    /// Append an observation / new claim.
    Observe,
    /// Correct or supersede an existing claim.
    Correct,
    /// Forget (soft, restorable) a record.
    Forget,
    /// Restore a previously forgotten record.
    Restore,
    /// Hard-delete a record's content (governed, previewed).
    HardDelete,
    /// Authorized declassification: relax/change the Effective Policy for a
    /// target by recording a **new** audited provenance record, never mutating
    /// the contributing source policy (MGR-004 AC3). Governed and previewed.
    Declassify,
}

impl CommandKind {
    /// The canonical snake_case text of this kind (stable for hashing, event
    /// `event_type`, audit `command_kind`, and logs). Mirrors the serde
    /// `rename_all = "snake_case"` on the enum.
    pub fn as_str(self) -> &'static str {
        match self {
            CommandKind::Observe => "observe",
            CommandKind::Correct => "correct",
            CommandKind::Forget => "forget",
            CommandKind::Restore => "restore",
            CommandKind::HardDelete => "hard_delete",
            CommandKind::Declassify => "declassify",
        }
    }
}

/// A governed write command: the authority verifies the caller partition, mode,
/// idempotency, and base revision before committing (design §5.1). The
/// `payload` is the command-kind-specific body; it stays opaque `serde_json`
/// here because the concrete per-kind builders live in `cognition`/`lifecycle`
/// (design §19.1) and are wired behind this port in F1.3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorityCommand {
    /// The caller's policy partition (namespace/scope/sensitivity + owner).
    pub caller: PolicyPartition,
    /// Caller-chosen idempotency token, paired with `caller` for replay
    /// detection (`idempotency_results (caller_partition, idempotency_key)`).
    pub idempotency_key: IdempotencyKey,
    /// The revision the caller believes is current; the command commits only if
    /// it still matches (optimistic concurrency, design §5.2).
    pub base_revision: GraphRevision,
    /// The governed operation.
    pub kind: CommandKind,
    /// Command-kind-specific body (validated by the per-kind builder in F1.3).
    pub payload: serde_json::Value,
}

/// Terminal status of a submitted [`AuthorityCommand`] (design §5.1 state
/// machine: Committed / Replay / Rejected / Previewed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    /// Semantic + event + audit + outbox + revision committed atomically.
    Committed,
    /// A matching idempotency result already existed; the prior outcome replays.
    Replayed,
    /// Rejected by schema/authz/mode/policy/limit checks (a normal outcome).
    Rejected,
    /// A destructive/corrective command returned a preview awaiting confirm.
    Previewed,
}

/// The outcome of a governed command. On commit it carries the completion
/// [`EventId`] and the reserved [`GraphRevision`] (only reserved when the change
/// is graph-visible, design §5.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutcome {
    /// Terminal status of the command.
    pub status: CommandStatus,
    /// The completion event id, when an event was appended.
    pub event_id: Option<EventId>,
    /// The revision the authority now reflects (reserved on commit; the base
    /// revision echoed back for replay/rejected/previewed).
    pub revision: GraphRevision,
}

/// The governed durable write surface (design §5.1, §19.1 `authority`).
///
/// This is the *only* sanctioned path to durable state: a thin trait the current
/// [`WritePolicy`](crate::write_policy::WritePolicy) is adapted to in
/// **F1.3**, where [`submit`](CommandPort::submit) runs
/// validate → (preview|replay) → open `AuthorityTx` → append events → write
/// changes/audit/idempotency/outbox → reserve revision → commit. Defining the
/// port now lets adapters and orchestration depend on it before that wiring
/// lands.
pub trait CommandPort: Send + Sync {
    /// Submit a governed command. Returns the terminal [`CommandOutcome`]; a
    /// policy rejection is an `Ok(Rejected)` outcome, not an `Err` (errors are
    /// reserved for storage/consistency failures — see [`crate::error`]).
    fn submit(&self, command: AuthorityCommand) -> MemoryResult<CommandOutcome>;
}

// ─────────────────────────────────────────────────────────────────────────
// QueryPort — the narrow read surface (design §5.2)
// ─────────────────────────────────────────────────────────────────────────

/// A minimal read projection of a cognitive record returned by [`QueryPort`].
/// Narrow by design: it exposes identity, the revision the row was last written
/// at, and content — not the full internal record shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordSnapshot {
    /// The record identity.
    pub id: RecordId,
    /// The authority revision the record content reflects.
    pub revision: GraphRevision,
    /// The record's stored content.
    pub content: String,
}

/// The narrow authority read surface (design §5.2 snapshot reads).
///
/// Reads capture `R = authority_meta.graph_revision` and execute against that
/// WAL snapshot. This port intentionally exposes only the *shape* of the read
/// boundary plus one or two representative methods — the full five-strategy
/// retrieval pipeline stays in `retrieval` (design §19.1) and is composed behind
/// this port in a later gate.
pub trait QueryPort: Send + Sync {
    /// Fetch a single record by id, or `None` if it does not exist / is not
    /// visible under the current default policy.
    fn get_record(&self, id: &RecordId) -> MemoryResult<Option<RecordSnapshot>>;

    /// The revision a fresh read snapshot would observe (design §5.2 `R`).
    fn snapshot_revision(&self) -> MemoryResult<GraphRevision>;
}

// ─────────────────────────────────────────────────────────────────────────
// OutboxPort — the derived-projection delivery surface (design §4.4/§19.5)
// ─────────────────────────────────────────────────────────────────────────

/// The operation an outbox work item represents (design §19.5: upsert vs
/// delete/purge, where delete/purge has priority over upsert for the same
/// target/record).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxOp {
    /// Create/update a derived projection row.
    Upsert,
    /// Remove a derived projection row.
    Delete,
    /// Purge (hard-delete reconciliation) a derived projection row.
    Purge,
}

impl OutboxOp {
    /// The canonical text stored in `derived_outbox.op`.
    pub fn as_str(self) -> &'static str {
        match self {
            OutboxOp::Upsert => "upsert",
            OutboxOp::Delete => "delete",
            OutboxOp::Purge => "purge",
        }
    }

    /// Parse the canonical `derived_outbox.op` text.
    fn from_db(s: &str) -> MemoryResult<Self> {
        match s {
            "upsert" => Ok(OutboxOp::Upsert),
            "delete" => Ok(OutboxOp::Delete),
            "purge" => Ok(OutboxOp::Purge),
            other => Err(StorageError::Encoding(format!("unknown outbox op {other:?}")).into()),
        }
    }
}

/// Retry/dead-letter state of an outbox work item (design §4.4 outbox state,
/// §19.5 relay algorithm).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxStatus {
    /// Awaiting (or eligible for) delivery.
    Pending,
    /// Successfully applied to the target projection.
    Applied,
    /// Superseded by a newer content hash for the same target/record.
    Superseded,
    /// Exhausted its retry budget without losing reconciliation eligibility.
    DeadLetter,
}

impl OutboxStatus {
    /// The canonical text stored in `derived_outbox.status`.
    pub fn as_str(self) -> &'static str {
        match self {
            OutboxStatus::Pending => "pending",
            OutboxStatus::Applied => "applied",
            OutboxStatus::Superseded => "superseded",
            OutboxStatus::DeadLetter => "dead_letter",
        }
    }

    /// Parse the canonical `derived_outbox.status` text.
    fn from_db(s: &str) -> MemoryResult<Self> {
        match s {
            "pending" => Ok(OutboxStatus::Pending),
            "applied" => Ok(OutboxStatus::Applied),
            "superseded" => Ok(OutboxStatus::Superseded),
            "dead_letter" => Ok(OutboxStatus::DeadLetter),
            other => Err(StorageError::Encoding(format!("unknown outbox status {other:?}")).into()),
        }
    }
}

/// A single derived-projection work item mapped 1:1 onto a `derived_outbox` row
/// (design §4.4). Uses the validated value objects so ids/times/revisions cannot
/// be raw unchecked strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxWork {
    /// The `derived_outbox.id` (AUTOINCREMENT). `None` before insertion; `Some`
    /// once read back via [`OutboxPort::pending`].
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub id: Option<i64>,
    /// The delivery target (e.g. `"fts"`, `"vectors"`, `"scene"`).
    pub target: String,
    /// The operation to apply.
    pub op: OutboxOp,
    /// The kind of record the work concerns (e.g. `"memory"`), if applicable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub record_kind: Option<String>,
    /// The record identity the work concerns, if applicable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub record_id: Option<RecordId>,
    /// The content hash the work is keyed to (drives supersede-by-newer, §19.5).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content_hash: Option<String>,
    /// The embedding/model partition, when the target is model-versioned.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model_partition: Option<String>,
    /// The authority revision at which the work was enqueued (lease order key).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub authority_revision: Option<GraphRevision>,
    /// Delivery attempts so far.
    pub attempts: u32,
    /// Retry/dead-letter status.
    pub status: OutboxStatus,
    /// Earliest time the item is eligible for the next delivery attempt.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub next_attempt_at: Option<UtcTimestamp>,
    /// Last error code recorded on a failed attempt.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_code: Option<String>,
    /// When the item was created.
    pub created_at: UtcTimestamp,
}

impl OutboxWork {
    /// Construct a fresh pending outbox item (`attempts = 0`, `status =
    /// Pending`, `created_at = now`, `id = None`). The optional record/content
    /// fields default to `None` and can be set on the returned value.
    pub fn new(target: impl Into<String>, op: OutboxOp) -> Self {
        Self {
            id: None,
            target: target.into(),
            op,
            record_kind: None,
            record_id: None,
            content_hash: None,
            model_partition: None,
            authority_revision: None,
            attempts: 0,
            status: OutboxStatus::Pending,
            next_attempt_at: None,
            error_code: None,
            created_at: UtcTimestamp::now(),
        }
    }

    /// Builder: attach the record kind/id this work concerns.
    pub fn with_record(mut self, kind: impl Into<String>, id: RecordId) -> Self {
        self.record_kind = Some(kind.into());
        self.record_id = Some(id);
        self
    }

    /// Builder: attach the content hash this work is keyed to.
    pub fn with_content_hash(mut self, hash: impl Into<String>) -> Self {
        self.content_hash = Some(hash.into());
        self
    }

    /// Builder: attach the authority revision the work was enqueued at.
    pub fn with_revision(mut self, revision: GraphRevision) -> Self {
        self.authority_revision = Some(revision);
        self
    }
}

/// The derived-projection delivery surface (design §4.4/§19.5).
///
/// The outbox is the authoritative queue of work needed to converge rebuildable
/// projections (FTS/vectors/scene) toward authority truth. This port is the
/// narrow enqueue/lease/complete/fail contract over the `derived_outbox` table;
/// the relay loop that drains it lives in the convergence worker (later gate).
pub trait OutboxPort: Send + Sync {
    /// Enqueue a work item (one `derived_outbox` row).
    fn enqueue(&self, work: OutboxWork) -> MemoryResult<()>;

    /// The pending, currently-eligible work for `target`, oldest first, capped
    /// at `limit`. "Eligible" means `status = pending` and `next_attempt_at` is
    /// unset or in the past.
    fn pending(&self, target: &str, limit: usize) -> MemoryResult<Vec<OutboxWork>>;

    /// Mark an item successfully applied (`status = applied`, clears error).
    fn mark_done(&self, id: i64) -> MemoryResult<()>;

    /// Mark a delivery attempt failed: increments `attempts`, records
    /// `error_code`, and either re-schedules (`retry_at` set → `pending`) or
    /// dead-letters it (`retry_at` unset → `dead_letter`).
    fn mark_failed(
        &self,
        id: i64,
        error_code: &str,
        retry_at: Option<UtcTimestamp>,
    ) -> MemoryResult<()>;
}

/// Concrete [`OutboxPort`] over the single authority [`Database`] and its
/// `derived_outbox` table. Writes go through a serialized [`AuthorityTx`]; the
/// pending lease read uses the WAL read pool.
///
/// [`AuthorityTx`]: crate::db::AuthorityTx
pub struct SqliteOutbox {
    db: Arc<Database>,
}

impl SqliteOutbox {
    /// Build the outbox port over the injected authority handle.
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

/// Raw `derived_outbox` row read from SQLite before value-object validation.
struct OutboxRow {
    id: i64,
    target: String,
    op: String,
    record_kind: Option<String>,
    record_id: Option<String>,
    content_hash: Option<String>,
    model_partition: Option<String>,
    authority_revision: Option<i64>,
    attempts: i64,
    status: String,
    next_attempt_at: Option<String>,
    error_code: Option<String>,
    created_at: String,
}

impl OutboxRow {
    /// Validate the raw row into a typed [`OutboxWork`].
    fn into_work(self) -> MemoryResult<OutboxWork> {
        Ok(OutboxWork {
            id: Some(self.id),
            target: self.target,
            op: OutboxOp::from_db(&self.op)?,
            record_kind: self.record_kind,
            record_id: self.record_id.map(RecordId::new).transpose()?,
            content_hash: self.content_hash,
            model_partition: self.model_partition,
            authority_revision: self
                .authority_revision
                .map(|n| GraphRevision::new(n.max(0) as u64)),
            attempts: self.attempts.max(0) as u32,
            status: OutboxStatus::from_db(&self.status)?,
            next_attempt_at: self
                .next_attempt_at
                .map(|s| UtcTimestamp::from_rfc3339_utc(&s))
                .transpose()?,
            error_code: self.error_code,
            created_at: UtcTimestamp::from_rfc3339_utc(&self.created_at)?,
        })
    }
}

impl OutboxPort for SqliteOutbox {
    fn enqueue(&self, work: OutboxWork) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO derived_outbox(
                     target, op, record_kind, record_id, content_hash, model_partition,
                     authority_revision, attempts, status, next_attempt_at, error_code, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    work.target,
                    work.op.as_str(),
                    work.record_kind,
                    work.record_id.as_ref().map(RecordId::as_str),
                    work.content_hash,
                    work.model_partition,
                    work.authority_revision.map(|r| r.get() as i64),
                    work.attempts as i64,
                    work.status.as_str(),
                    work.next_attempt_at.map(|t| t.to_rfc3339()),
                    work.error_code,
                    work.created_at.to_rfc3339(),
                ],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    fn pending(&self, target: &str, limit: usize) -> MemoryResult<Vec<OutboxWork>> {
        let now = UtcTimestamp::now().to_rfc3339();
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, target, op, record_kind, record_id, content_hash,
                            model_partition, authority_revision, attempts, status,
                            next_attempt_at, error_code, created_at
                     FROM derived_outbox
                     WHERE target = ?1
                       AND status = 'pending'
                       AND (next_attempt_at IS NULL OR next_attempt_at <= ?2)
                     ORDER BY authority_revision, id
                     LIMIT ?3",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![target, now, limit as i64], |row| {
                    Ok(OutboxRow {
                        id: row.get(0)?,
                        target: row.get(1)?,
                        op: row.get(2)?,
                        record_kind: row.get(3)?,
                        record_id: row.get(4)?,
                        content_hash: row.get(5)?,
                        model_partition: row.get(6)?,
                        authority_revision: row.get(7)?,
                        attempts: row.get(8)?,
                        status: row.get(9)?,
                        next_attempt_at: row.get(10)?,
                        error_code: row.get(11)?,
                        created_at: row.get(12)?,
                    })
                })
                .map_err(StorageError::Sqlite)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(StorageError::Sqlite)?.into_work()?);
            }
            Ok(out)
        })
    }

    fn mark_done(&self, id: i64) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "UPDATE derived_outbox
                 SET status = 'applied', error_code = NULL, next_attempt_at = NULL
                 WHERE id = ?1",
                params![id],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    fn mark_failed(
        &self,
        id: i64,
        error_code: &str,
        retry_at: Option<UtcTimestamp>,
    ) -> MemoryResult<()> {
        let (status, next_attempt_at) = match &retry_at {
            Some(ts) => (OutboxStatus::Pending, Some(ts.to_rfc3339())),
            None => (OutboxStatus::DeadLetter, None),
        };
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "UPDATE derived_outbox
                 SET attempts = attempts + 1,
                     status = ?2,
                     error_code = ?3,
                     next_attempt_at = ?4
                 WHERE id = ?1",
                params![id, status.as_str(), error_code, next_attempt_at],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// IntegrityPort — the integrity/recovery inspection surface (design §5.3)
// ─────────────────────────────────────────────────────────────────────────

/// The integrity/recovery inspection surface (design §5.3).
///
/// Inspection-only: it reports whether the authority is structurally sound, what
/// revision it currently reflects, and an honest account of legacy-vs-v2 rows.
/// It never mutates the authority and never fabricates rows — the recovery
/// actions themselves (`verify_candidate`, `activate_verified_candidate`,
/// `reset_empty`) belong to the lifecycle/recovery flow, not this port.
pub trait IntegrityPort: Send + Sync {
    /// Run SQLite's fast structural check (design §30 startup integrity). `true`
    /// means the authority reported `ok`.
    fn quick_check(&self) -> MemoryResult<bool>;

    /// The current authority revision (`authority_meta.graph_revision`); a fresh
    /// authority reports [`GraphRevision::base`] (0).
    fn authority_revision(&self) -> MemoryResult<GraphRevision>;

    /// A deterministic account of legacy authority-competing rows a hard reset
    /// would discard, alongside the current v2 authority counts (F1.1).
    fn reconciliation_report(&self) -> MemoryResult<ReconciliationReport>;

    /// Run the deep recovery/release checker (design §5.3, task 1.8.2).
    ///
    /// This is intentionally slow and must NOT be called on startup.
    /// Use for explicit recovery triage or the release gate.
    /// Returns a [`RecoveryCheckReport`] aggregating all five deep checks.
    fn deep_check(&self) -> RecoveryCheckReport;
}

/// Concrete [`IntegrityPort`] over the single authority [`Database`].
pub struct AuthorityIntegrity {
    db: Arc<Database>,
}

impl AuthorityIntegrity {
    /// Build the integrity port over the injected authority handle.
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

impl IntegrityPort for AuthorityIntegrity {
    fn quick_check(&self) -> MemoryResult<bool> {
        self.db.quick_check()
    }

    fn authority_revision(&self) -> MemoryResult<GraphRevision> {
        self.db.with_read(|conn| {
            let rev: i64 = conn
                .query_row(
                    "SELECT graph_revision FROM authority_meta WHERE id = 1",
                    [],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            Ok(GraphRevision::new(rev.max(0) as u64))
        })
    }

    fn reconciliation_report(&self) -> MemoryResult<ReconciliationReport> {
        self.db.reconciliation_report()
    }

    fn deep_check(&self) -> RecoveryCheckReport {
        RecoveryIntegrityChecker::new(Arc::clone(&self.db)).run_all()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// SqliteValidationReads — read-only lookups for the pre-transaction validator
// ─────────────────────────────────────────────────────────────────────────

/// Concrete [`ValidationReads`] over the single authority [`Database`]. Every
/// method reads the WAL snapshot via the read pool and performs **no** writes —
/// no access counters, no touch timestamps — so the pre-transaction validation
/// stage (task F1.3.2) never mutates authority state (parent-task invariant).
pub struct SqliteValidationReads {
    db: Arc<Database>,
}

impl SqliteValidationReads {
    /// Build the validation read surface over the injected authority handle.
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

impl ValidationReads for SqliteValidationReads {
    fn lookup_idempotency(
        &self,
        caller_partition: &str,
        idempotency_key: &IdempotencyKey,
    ) -> MemoryResult<Option<StoredIdempotencyResult>> {
        self.db.with_read(|conn| {
            let row = conn
                .query_row(
                    "SELECT command_hash, result_json, committed_revision, event_id
                     FROM idempotency_results
                     WHERE caller_partition = ?1 AND idempotency_key = ?2",
                    params![caller_partition, idempotency_key.as_str()],
                    |row| {
                        let command_hash: String = row.get(0)?;
                        let result_json: String = row.get(1)?;
                        let committed_revision: Option<i64> = row.get(2)?;
                        let event_id: Option<String> = row.get(3)?;
                        Ok((command_hash, result_json, committed_revision, event_id))
                    },
                )
                .optional()
                .map_err(StorageError::Sqlite)?;

            row.map(
                |(command_hash, result_json, committed_revision, event_id)| {
                    Ok(StoredIdempotencyResult {
                        command_hash,
                        result_json,
                        committed_revision: committed_revision
                            .map(|n| GraphRevision::new(n.max(0) as u64)),
                        event_id: event_id.map(EventId::new).transpose()?,
                    })
                },
            )
            .transpose()
        })
    }

    fn current_revision(&self) -> MemoryResult<GraphRevision> {
        self.db.with_read(|conn| {
            let rev: i64 = conn
                .query_row(
                    "SELECT graph_revision FROM authority_meta WHERE id = 1",
                    [],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            Ok(GraphRevision::new(rev.max(0) as u64))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GraphRevision;

    /// A fresh in-memory authority handle for port tests.
    fn fresh_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().expect("open in-memory authority"))
    }

    // ── OutboxPort concrete round-trip (task item (b)) ──────────────────
    #[test]
    fn outbox_round_trip_enqueue_pending_done() {
        let db = fresh_db();
        let outbox = SqliteOutbox::new(db.clone());

        // Nothing pending initially.
        assert!(outbox.pending("fts", 10).unwrap().is_empty());

        // Enqueue one upsert work item.
        let rid = RecordId::new_v7();
        let work = OutboxWork::new("fts", OutboxOp::Upsert)
            .with_record("memory", rid.clone())
            .with_content_hash("deadbeef")
            .with_revision(GraphRevision::new(1));
        outbox.enqueue(work).unwrap();

        // pending() returns it, with a populated row id and the typed fields.
        let pending = outbox.pending("fts", 10).unwrap();
        assert_eq!(pending.len(), 1, "one pending item after enqueue");
        let item = &pending[0];
        assert!(item.id.is_some(), "read-back item carries its row id");
        assert_eq!(item.target, "fts");
        assert_eq!(item.op, OutboxOp::Upsert);
        assert_eq!(item.record_id.as_ref(), Some(&rid));
        assert_eq!(item.content_hash.as_deref(), Some("deadbeef"));
        assert_eq!(item.authority_revision, Some(GraphRevision::new(1)));
        assert_eq!(item.status, OutboxStatus::Pending);

        // A different target sees nothing.
        assert!(outbox.pending("vectors", 10).unwrap().is_empty());

        // mark_done removes it from the pending set.
        outbox.mark_done(item.id.unwrap()).unwrap();
        assert!(
            outbox.pending("fts", 10).unwrap().is_empty(),
            "applied item is no longer pending"
        );
    }

    #[test]
    fn outbox_mark_failed_retry_then_dead_letter() {
        let db = fresh_db();
        let outbox = SqliteOutbox::new(db.clone());
        outbox
            .enqueue(OutboxWork::new("vectors", OutboxOp::Upsert))
            .unwrap();
        let id = outbox.pending("vectors", 10).unwrap()[0].id.unwrap();

        // Fail with a future retry time → still pending (eligible later), attempts++.
        let future = UtcTimestamp::from_rfc3339_utc("2999-01-01T00:00:00Z").unwrap();
        outbox.mark_failed(id, "E_TRANSIENT", Some(future)).unwrap();
        // Not eligible now (next_attempt_at in the far future).
        assert!(
            outbox.pending("vectors", 10).unwrap().is_empty(),
            "item scheduled in the future is not currently eligible"
        );

        // Fail with no retry → dead-lettered, no longer pending.
        outbox.mark_failed(id, "E_FATAL", None).unwrap();
        assert!(outbox.pending("vectors", 10).unwrap().is_empty());
    }

    // ── IntegrityPort on a fresh authority (task item (c)) ──────────────
    #[test]
    fn integrity_quick_check_and_revision_on_fresh_authority() {
        let db = fresh_db();
        let integrity = AuthorityIntegrity::new(db.clone());

        assert!(
            integrity.quick_check().unwrap(),
            "fresh authority passes quick_check"
        );
        assert_eq!(
            integrity.authority_revision().unwrap(),
            GraphRevision::base(),
            "fresh authority is at base revision 0"
        );
        // Reconciliation on a v2-only in-memory DB reports no legacy rows.
        let report = integrity.reconciliation_report().unwrap();
        assert_eq!(report.legacy_total(), 0);
    }

    // ── SqliteValidationReads concrete round-trip (task F1.3.2) ─────────
    #[test]
    fn validation_reads_current_revision_and_idempotency_lookup() {
        let db = fresh_db();
        let reads = SqliteValidationReads::new(db.clone());

        // Fresh authority is at base revision 0, and no idempotency rows exist.
        assert_eq!(reads.current_revision().unwrap(), GraphRevision::base());
        let key = IdempotencyKey::new("cmd-xyz").unwrap();
        assert!(reads
            .lookup_idempotency("user/chat/0", &key)
            .unwrap()
            .is_none());

        // Insert an idempotency result directly through a serialized tx.
        let tx = db.begin().unwrap();
        tx.conn()
            .execute(
                "INSERT INTO idempotency_results(
                     caller_partition, idempotency_key, command_hash, result_json,
                     committed_revision, event_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)",
                params![
                    "user/chat/0",
                    "cmd-xyz",
                    "deadbeefhash",
                    r#"{"status":"committed"}"#,
                    3_i64,
                    UtcTimestamp::now().to_rfc3339(),
                ],
            )
            .unwrap();
        tx.commit().unwrap();

        // The lookup now returns the typed stored result.
        let found = reads
            .lookup_idempotency("user/chat/0", &key)
            .unwrap()
            .expect("row present after insert");
        assert_eq!(found.command_hash, "deadbeefhash");
        assert_eq!(found.result_json, r#"{"status":"committed"}"#);
        assert_eq!(found.committed_revision, Some(GraphRevision::new(3)));
        assert_eq!(found.event_id, None);

        // A different partition/key still misses.
        assert!(reads
            .lookup_idempotency("user/other/0", &key)
            .unwrap()
            .is_none());
    }

    // ── Trait-object compile checks for the F1.3-wired ports ────────────
    // CommandPort/QueryPort are defined here but implemented in F1.3; prove a
    // stub satisfies each trait and is object-safe behind the port type.
    struct StubCommand;
    impl CommandPort for StubCommand {
        fn submit(&self, command: AuthorityCommand) -> MemoryResult<CommandOutcome> {
            Ok(CommandOutcome {
                status: CommandStatus::Committed,
                event_id: Some(EventId::new_v7()),
                revision: command.base_revision.next(),
            })
        }
    }

    struct StubQuery;
    impl QueryPort for StubQuery {
        fn get_record(&self, id: &RecordId) -> MemoryResult<Option<RecordSnapshot>> {
            Ok(Some(RecordSnapshot {
                id: id.clone(),
                revision: GraphRevision::base(),
                content: String::new(),
            }))
        }
        fn snapshot_revision(&self) -> MemoryResult<GraphRevision> {
            Ok(GraphRevision::base())
        }
    }

    #[test]
    fn command_and_query_ports_are_object_safe() {
        let cmd: Arc<dyn CommandPort> = Arc::new(StubCommand);
        let query: Arc<dyn QueryPort> = Arc::new(StubQuery);

        let outcome = cmd
            .submit(AuthorityCommand {
                caller: PolicyPartition::new("user", "chat", 0).unwrap(),
                idempotency_key: IdempotencyKey::new("cmd-1").unwrap(),
                base_revision: GraphRevision::base(),
                kind: CommandKind::Observe,
                payload: serde_json::json!({"content": "hi"}),
            })
            .unwrap();
        assert_eq!(outcome.status, CommandStatus::Committed);
        assert_eq!(outcome.revision, GraphRevision::new(1));

        let rid = RecordId::new_v7();
        assert!(query.get_record(&rid).unwrap().is_some());
        assert_eq!(query.snapshot_revision().unwrap(), GraphRevision::base());
    }
}
