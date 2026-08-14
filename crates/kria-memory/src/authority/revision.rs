//! The transaction-scoped revision + change-set ledger (task **F1.3.5**, design
//! §4.1 `authority_meta`/`graph_revisions`/`graph_changes`, §5.1 "AuthorityTx …
//! reserves exactly one revision … writes ordered changes …").
//!
//! [`TxRevisionLog`] is the **transaction-scoped repository** that, for a
//! graph-visible accepted command, advances the authority revision counter
//! exactly once and appends the contiguous `graph_revisions` row plus the
//! stable-ordinal `graph_changes` rows — *using only the serialized-writer
//! transaction connection* handed to it, exactly like [`TxEventLog`]
//! (F1.3.3) and [`TxAuditLog`] (F1.3.4). It carries no [`Database`] handle, so
//! mis-wiring a revision write onto a second connection is structurally
//! impossible (F1.3 non-negotiable: "all writes must occur on the transaction
//! connection").
//!
//! ## Scope of this module (F1.3.5)
//!
//! The revision stage runs after the completion/observation event + audit
//! (F1.3.4) and enforces the F1.3 non-negotiables for the revision ledger:
//!
//! * **Reserve a revision only for a graph-visible accepted change.** A
//!   revision-neutral or rejected/deferred command never reaches
//!   [`TxRevisionLog::reserve`]; the orchestrator passes `None` for its audit
//!   `authority_revision` (design §5.1).
//! * **Increment `authority_meta.graph_revision` exactly once**: the new
//!   revision is `old + 1`, written back inside the same transaction so a
//!   rollback restores the prior counter.
//! * **Contiguous `graph_revisions`**: the appended row has `revision = new`
//!   and `base_revision = new - 1`, matching the schema
//!   `CHECK (base_revision = revision - 1)`.
//! * **Stable, contiguous `graph_changes` ordinals**: the caller-supplied
//!   ordered [`GraphChange`] list is written with ordinals `0, 1, 2, …` in the
//!   order given, and `graph_revisions.change_count` equals that count.
//!
//! ## Where the change descriptors come from
//!
//! The ordered [`GraphChange`] list is produced by the semantic seam
//! ([`TxSemanticStore`](super::transaction::TxSemanticStore)) and carried on
//! [`SemanticOutcome`](super::transaction::SemanticOutcome). This module does
//! **not** invent them: the per-kind semantic builders (F2) return exactly the
//! rows their mutation touched, and this stage appends them verbatim with
//! stable ordinals. Establishing the contract now — an ordered, caller-supplied
//! descriptor list — fixes the dependency direction before those builders exist.
//!
//! [`Database`]: crate::db::Database
//! [`TxEventLog`]: super::event_log::TxEventLog
//! [`TxAuditLog`]: super::audit::TxAuditLog

use rusqlite::params;

use crate::db::AuthorityTx;
use crate::error::{MemoryResult, StorageError};
use crate::model::{GraphRevision, RecordId, UtcTimestamp};

use super::command::CommandEnvelope;
use super::event_log::PENDING_POLICY_VERSION;

/// Placeholder `graph_revisions.policy_hash` until the Effective-Policy layer
/// (F1.4) computes and stamps the real policy hash. The column is NOT NULL, so
/// a stable, honest sentinel is written meanwhile — the **same** sentinel the
/// event log / audit ledger use for `policy_version` (never faked as a real
/// resolved policy hash). Replaced when F1.4 wires the policy computation.
pub const PENDING_POLICY_HASH: &str = PENDING_POLICY_VERSION;

// ─────────────────────────────────────────────────────────────────────────
// GraphChangeKind — the change_kind column (graph_changes.change_kind CHECK)
// ─────────────────────────────────────────────────────────────────────────

/// The kind of graph-visible mutation a [`GraphChange`] records, mirroring the
/// schema `graph_changes.change_kind CHECK (change_kind IN
/// ('insert','update','state','delete','invalidate'))` (design §4.1). A closed
/// set so `change_kind` can never be a raw unchecked string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphChangeKind {
    /// A new record/link row was inserted.
    Insert,
    /// An existing record/link row's content was updated (corrected/superseded).
    Update,
    /// A record's lifecycle state changed (e.g. forgotten/restored).
    State,
    /// A record/link row was deleted (hard delete).
    Delete,
    /// A claim/link was invalidated (truth maintenance).
    Invalidate,
}

impl GraphChangeKind {
    /// The canonical text stored in `graph_changes.change_kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            GraphChangeKind::Insert => "insert",
            GraphChangeKind::Update => "update",
            GraphChangeKind::State => "state",
            GraphChangeKind::Delete => "delete",
            GraphChangeKind::Invalidate => "invalidate",
        }
    }
}

impl std::fmt::Display for GraphChangeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─────────────────────────────────────────────────────────────────────────
// GraphChange — one ordered change descriptor (one graph_changes row)
// ─────────────────────────────────────────────────────────────────────────

/// One ordered graph-visible change, mapped 1:1 onto a `graph_changes` row
/// (design §4.1). Produced by the semantic seam and carried on
/// [`SemanticOutcome`](super::transaction::SemanticOutcome); the revision stage
/// assigns the `(revision, ordinal)` primary key when it appends the row, so
/// the descriptor itself carries only the *content* of the change, never its
/// position. Uses the validated [`RecordId`] value object so a record id can
/// never be a raw unchecked string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphChange {
    /// The kind of record the change concerns (e.g. `"memory"`, `"link"`), or
    /// `None` for a change that is not record-scoped.
    pub record_kind: Option<String>,
    /// The record identity the change concerns, or `None` when not applicable.
    pub record_id: Option<RecordId>,
    /// The kind of mutation.
    pub change_kind: GraphChangeKind,
    /// Content hash before the change (`None` for an insert).
    pub before_hash: Option<String>,
    /// Content hash after the change (`None` for a delete).
    pub after_hash: Option<String>,
    /// The policy partition the change is attributed to (`graph_changes`
    /// `policy_partition` is NOT NULL).
    pub policy_partition: String,
    /// Optional canonical-JSON detail payload for the change.
    pub payload_json: Option<String>,
}

impl GraphChange {
    /// Construct a change descriptor for `change_kind` attributed to
    /// `policy_partition`, with the optional record / hash / payload fields left
    /// unset (settable via the builders below).
    pub fn new(change_kind: GraphChangeKind, policy_partition: impl Into<String>) -> Self {
        Self {
            record_kind: None,
            record_id: None,
            change_kind,
            before_hash: None,
            after_hash: None,
            policy_partition: policy_partition.into(),
            payload_json: None,
        }
    }

    /// Builder: attach the record kind/id this change concerns.
    pub fn with_record(mut self, kind: impl Into<String>, id: RecordId) -> Self {
        self.record_kind = Some(kind.into());
        self.record_id = Some(id);
        self
    }

    /// Builder: attach the before/after content hashes.
    pub fn with_hashes(mut self, before_hash: Option<String>, after_hash: Option<String>) -> Self {
        self.before_hash = before_hash;
        self.after_hash = after_hash;
        self
    }

    /// Builder: attach a canonical-JSON detail payload.
    pub fn with_payload(mut self, payload_json: impl Into<String>) -> Self {
        self.payload_json = Some(payload_json.into());
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────
// TxRevisionLog — the transaction-scoped revision + change-set repository
// ─────────────────────────────────────────────────────────────────────────

/// The transaction-scoped append surface over `authority_meta` (the singleton
/// revision counter), `graph_revisions` (the append-only revision ledger), and
/// `graph_changes` (the append-only per-revision change set).
///
/// A zero-sized handle: [`reserve`](Self::reserve) takes the `&mut AuthorityTx`
/// it must write through, so — exactly like [`TxEventLog`](super::event_log::TxEventLog)
/// and [`TxAuditLog`](super::audit::TxAuditLog) — it is structurally impossible
/// for this repository to write anywhere other than the serialized-writer
/// transaction (F1.3 invariant). It owns no [`Database`](crate::db::Database)
/// / connection / pool.
#[derive(Debug, Default, Clone, Copy)]
pub struct TxRevisionLog;

impl TxRevisionLog {
    /// Construct the (stateless) revision-log repository.
    pub fn new() -> Self {
        TxRevisionLog
    }

    /// Reserve **exactly one** new revision for a graph-visible accepted command
    /// and append its contiguous ledger row plus the ordered change set — all on
    /// `tx`, so it commits (or rolls back) atomically with the rest of the
    /// command.
    ///
    /// Steps (design §5.1 revision reservation), in order:
    /// 1. Read the current `authority_meta.graph_revision` (`old`).
    /// 2. Compute `new = old + 1` and write it back (**the single increment**).
    /// 3. Append the `graph_revisions` row: `revision = new`,
    ///    `base_revision = new - 1` (contiguous), the unique `tx_id`, the
    ///    committed-at timestamp, the caller `actor_id`, the pending policy hash
    ///    sentinel, and `change_count = changes.len()`.
    /// 4. Append the `graph_changes` rows with stable, contiguous ordinals
    ///    `0..changes.len()` in the given order.
    ///
    /// `tx_id` must be unique across all revisions (`graph_revisions.tx_id
    /// UNIQUE`); the orchestrator passes the command's completion event id,
    /// which is freshly minted per command. Returns the reserved [`GraphRevision`].
    ///
    /// This method is called **only** for a graph-visible accepted change; a
    /// revision-neutral / rejected / deferred command never invokes it (that is
    /// how the "reserve a revision only for a graph-visible accepted change"
    /// invariant is kept — the neutrality is enforced by the *caller* not
    /// reaching this method at all).
    pub fn reserve(
        &self,
        tx: &mut AuthorityTx<'_>,
        env: &CommandEnvelope,
        tx_id: &str,
        changes: &[GraphChange],
    ) -> MemoryResult<GraphRevision> {
        // 1. Current revision counter.
        let old: i64 = tx
            .conn()
            .query_row(
                "SELECT graph_revision FROM authority_meta WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .map_err(StorageError::Sqlite)?;
        let base = GraphRevision::new(old.max(0) as u64);
        let new = base.next();

        // 2. The single increment, written back on the transaction connection.
        tx.conn()
            .execute(
                "UPDATE authority_meta SET graph_revision = ?1 WHERE id = 1",
                params![new.get() as i64],
            )
            .map_err(StorageError::Sqlite)?;

        // 3. The contiguous revision-ledger row. `change_count` is the number of
        //    ordered changes appended below; the schema `CHECK (base_revision =
        //    revision - 1)` structurally enforces contiguity.
        let committed_at = UtcTimestamp::now();
        tx.conn()
            .execute(
                "INSERT INTO graph_revisions(
                     revision, base_revision, tx_id, committed_at,
                     actor_id, policy_hash, change_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    new.get() as i64,
                    base.get() as i64,
                    tx_id,
                    committed_at.to_rfc3339(),
                    env.caller().actor_id(),
                    PENDING_POLICY_HASH,
                    changes.len() as i64,
                ],
            )
            .map_err(StorageError::Sqlite)?;

        // 4. The ordered change set with stable, contiguous ordinals.
        for (ordinal, change) in changes.iter().enumerate() {
            tx.conn()
                .execute(
                    "INSERT INTO graph_changes(
                         revision, ordinal, record_kind, record_id, change_kind,
                         before_hash, after_hash, policy_partition, payload_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        new.get() as i64,
                        ordinal as i64,
                        change.record_kind,
                        change.record_id.as_ref().map(RecordId::as_str),
                        change.change_kind.as_str(),
                        change.before_hash,
                        change.after_hash,
                        change.policy_partition,
                        change.payload_json,
                    ],
                )
                .map_err(StorageError::Sqlite)?;
        }

        Ok(new)
    }
}
