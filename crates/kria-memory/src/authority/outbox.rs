//! The transaction-scoped, idempotent derived-projection outbox enqueue (task
//! **F1.3.6**, design §4.4 `derived_outbox`, §5.1 "AuthorityTx … writes …
//! outbox … then commits", §19.5 relay).
//!
//! [`TxOutbox`] is the **transaction-scoped repository** that enqueues
//! `derived_outbox` rows *using only the serialized-writer transaction
//! connection* handed to it — exactly like [`TxEventLog`](super::event_log::TxEventLog)
//! (F1.3.3), [`TxAuditLog`](super::audit::TxAuditLog) (F1.3.4), and
//! [`TxRevisionLog`](super::revision::TxRevisionLog) (F1.3.5). It carries no
//! [`Database`] handle, so mis-wiring an outbox write onto a second connection
//! is structurally impossible (F1.3 non-negotiable: "all writes must occur on
//! the transaction connection").
//!
//! This is the transaction-scoped counterpart to
//! [`SqliteOutbox`](super::SqliteOutbox), whose `enqueue` opens its **own**
//! `db.begin()` transaction (the drain/relay path) and therefore cannot be used
//! inside an in-flight [`AuthorityTx`]. The two write the same table with the
//! same column layout; only the connection ownership differs.
//!
//! ## Idempotent enqueue (design §4.4 semantic-uniqueness key)
//!
//! `derived_outbox` has a UNIQUE index over the **semantic key**
//! `(target, op, record_kind, record_id, content_hash, COALESCE(model_partition,''))`
//! (`idx_derived_outbox_semantic`, migration 0014). [`TxOutbox::enqueue`] inserts
//! with `ON CONFLICT DO NOTHING`, so re-enqueuing the *same* semantic work
//! inside the same command (or across replayed commands) never creates a
//! duplicate row — it is a no-op that reports `false`. A `NULL` `model_partition`
//! collapses to `''` in the key, so two null-model rows that otherwise match are
//! duplicates while a null-model and a `p`-model row are distinct (the model
//! dimension).
//!
//! ## Change → projection-work mapping ([`projection_work_for_changes`])
//!
//! A graph-visible command's [`SemanticOutcome`](super::transaction::SemanticOutcome)
//! carries the ordered [`GraphChange`] set the mutation touched. Each
//! *record-scoped* change (one that names a concrete [`RecordId`]) implies
//! convergence work for the rebuildable projections; a change that names no
//! record (e.g. the deferred placeholder change the [`DeferredSemanticStore`]
//! emits until F2) implies **no** projection work — there is no concrete record
//! to project. See [`projection_work_for_changes`] for the exact mapping and
//! what is deferred to the F2 per-kind builders.
//!
//! [`Database`]: crate::db::Database
//! [`DeferredSemanticStore`]: super::transaction::DeferredSemanticStore

use rusqlite::params;

use crate::db::AuthorityTx;
use crate::error::{MemoryResult, StorageError};
use crate::model::{GraphRevision, RecordId};

use super::revision::{GraphChange, GraphChangeKind};
use super::{OutboxOp, OutboxStatus, OutboxWork};

/// The representative derived-projection targets a record-scoped graph change
/// fans out to (design §4.4/§19.5 rebuildable projections):
///
/// * `"fts"`    — the full-text search projection (content-addressed).
/// * `"vectors"`— the embedding/vector projection. The *model dimension*
///   (`model_partition`) is resolved downstream by the embedding/convergence
///   worker (which fans one logical vector work item out per active embedding
///   model), so the work enqueued here carries `model_partition = None`; the
///   idempotency mechanics across the model dimension are exercised directly in
///   [`TxOutbox`] tests.
/// * `"scene"`  — the scene/graph projection (content-addressed).
///
/// The set is intentionally representative: the enqueue mechanism itself accepts
/// any target string, and the precise per-kind fan-out (including which targets
/// a given cognitive-record mutation actually touches) is produced by the F2
/// semantic builders when they return their concrete change sets.
pub const PROJECTION_TARGETS: &[&str] = &["fts", "vectors", "scene"];

// ─────────────────────────────────────────────────────────────────────────
// Change → projection-work mapping
// ─────────────────────────────────────────────────────────────────────────

/// Map an ordered graph-change set to the idempotent derived-projection work it
/// implies, at the given committed `authority_revision` (the outbox lease order
/// key).
///
/// Mapping rules (representative; the F2 builders refine per-kind fan-out):
/// * A change with **no** `record_id` implies **no** work (projection rows are
///   keyed to a concrete record; the deferred placeholder change is such a
///   change and is skipped — nothing is enqueued for it).
/// * A record-scoped change fans out one work item per [`PROJECTION_TARGETS`]
///   entry, with:
///   - `op` = [`OutboxOp::Upsert`] for `Insert`/`Update`/`State` (the record's
///     current content/visibility is (re)projected), or [`OutboxOp::Delete`]
///     for `Delete`/`Invalidate` (the projection row is removed);
///   - `content_hash` = the change's `after_hash` for an upsert (the content
///     now to project) or `before_hash` for a delete (the content being
///     removed) — either may be `None`;
///   - `record_kind` / `record_id` copied from the change;
///   - `authority_revision` = `authority_revision`.
///
/// The returned items are ready to hand to [`TxOutbox::enqueue`]; enqueuing the
/// same list twice is a no-op the second time (idempotent semantic key).
pub fn projection_work_for_changes(
    changes: &[GraphChange],
    authority_revision: Option<GraphRevision>,
) -> Vec<OutboxWork> {
    let mut work = Vec::new();
    for change in changes {
        // Projection work is keyed to a concrete record. A change that touches
        // no record (e.g. the deferred placeholder) implies no projection work.
        let Some(record_id) = change.record_id.clone() else {
            continue;
        };
        // A record-scoped change always names its kind; default defensively.
        let record_kind = change
            .record_kind
            .clone()
            .unwrap_or_else(|| "memory".to_string());

        let (op, content_hash) = match change.change_kind {
            GraphChangeKind::Insert | GraphChangeKind::Update | GraphChangeKind::State => {
                (OutboxOp::Upsert, change.after_hash.clone())
            }
            GraphChangeKind::Delete | GraphChangeKind::Invalidate => {
                (OutboxOp::Delete, change.before_hash.clone())
            }
        };

        for target in PROJECTION_TARGETS {
            work.push(build_work(
                target,
                op,
                &record_kind,
                &record_id,
                content_hash.as_deref(),
                authority_revision,
            ));
        }
    }
    work
}

/// Assemble one [`OutboxWork`] item for a single (target, record, op) tuple.
fn build_work(
    target: &str,
    op: OutboxOp,
    record_kind: &str,
    record_id: &RecordId,
    content_hash: Option<&str>,
    authority_revision: Option<GraphRevision>,
) -> OutboxWork {
    let mut item = OutboxWork::new(target, op).with_record(record_kind, record_id.clone());
    if let Some(hash) = content_hash {
        item = item.with_content_hash(hash);
    }
    if let Some(rev) = authority_revision {
        item = item.with_revision(rev);
    }
    item
}

// ─────────────────────────────────────────────────────────────────────────
// TxOutbox — the transaction-scoped, idempotent outbox enqueue repository
// ─────────────────────────────────────────────────────────────────────────

/// The transaction-scoped enqueue surface over `derived_outbox`.
///
/// A zero-sized handle: [`enqueue`](Self::enqueue) takes the `&mut AuthorityTx`
/// it must write through, so — exactly like the other F1.3 transaction-scoped
/// repositories — it is structurally impossible for it to write anywhere other
/// than the serialized-writer transaction (F1.3 invariant). It owns no
/// [`Database`](crate::db::Database) / connection / pool.
#[derive(Debug, Default, Clone, Copy)]
pub struct TxOutbox;

impl TxOutbox {
    /// Construct the (stateless) outbox enqueue repository.
    pub fn new() -> Self {
        TxOutbox
    }

    /// Enqueue one derived-projection work item on `tx`, **idempotently** with
    /// respect to the semantic-uniqueness key.
    ///
    /// Uses `INSERT … ON CONFLICT DO NOTHING`: the first enqueue of a given
    /// semantic key `(target, op, record_kind, record_id, content_hash,
    /// COALESCE(model_partition,''))` inserts a fresh `pending` row and returns
    /// `Ok(true)`; a subsequent enqueue of the *same* semantic key is a no-op
    /// that returns `Ok(false)` (no duplicate row). The `id` (AUTOINCREMENT),
    /// `attempts`, and `status` supplied on `work` are the fresh-row defaults;
    /// they are ignored on a conflict.
    ///
    /// Runs on `tx`'s connection, so the enqueued row commits (or rolls back)
    /// atomically with the rest of the command.
    pub fn enqueue(&self, tx: &mut AuthorityTx<'_>, work: &OutboxWork) -> MemoryResult<bool> {
        let inserted = tx
            .conn()
            .execute(
                "INSERT INTO derived_outbox(
                     target, op, record_kind, record_id, content_hash, model_partition,
                     authority_revision, attempts, status, next_attempt_at, error_code, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT DO NOTHING",
                params![
                    work.target,
                    work.op.as_str(),
                    work.record_kind,
                    work.record_id.as_ref().map(RecordId::as_str),
                    work.content_hash,
                    work.model_partition,
                    work.authority_revision.map(|r| r.get() as i64),
                    work.attempts as i64,
                    // A fresh in-transaction enqueue is always pending; a caller
                    // that hands a non-pending status is normalized here so the
                    // enqueued row is always eligible for the relay.
                    OutboxStatus::Pending.as_str(),
                    work.next_attempt_at.map(|t| t.to_rfc3339()),
                    work.error_code,
                    work.created_at.to_rfc3339(),
                ],
            )
            .map_err(StorageError::Sqlite)?;
        Ok(inserted > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::model::RecordId;
    use rusqlite::params;
    use std::sync::Arc;

    fn fresh_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().expect("open in-memory authority"))
    }

    fn outbox_count(db: &Database) -> i64 {
        db.with_read(|conn| {
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM derived_outbox", [], |r| r.get(0))
                .unwrap();
            Ok(n)
        })
        .unwrap()
    }

    fn semantic_key_count(db: &Database, target: &str, record_id: &str) -> i64 {
        db.with_read(|conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM derived_outbox WHERE target = ?1 AND record_id = ?2",
                    params![target, record_id],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(n)
        })
        .unwrap()
    }

    // ── Idempotent enqueue: identical semantic key enqueued twice → one row ─
    #[test]
    fn enqueue_is_idempotent_for_identical_semantic_key() {
        let db = fresh_db();
        let rid = RecordId::new_v7();
        let work = OutboxWork::new("fts", OutboxOp::Upsert)
            .with_record("memory", rid.clone())
            .with_content_hash("deadbeef")
            .with_revision(GraphRevision::new(1));

        let tx_outbox = TxOutbox::new();
        let mut tx = db.begin().unwrap();
        // First enqueue inserts a fresh row.
        assert!(
            tx_outbox.enqueue(&mut tx, &work).unwrap(),
            "first enqueue inserts"
        );
        // Second enqueue of the SAME semantic key is a no-op.
        assert!(
            !tx_outbox.enqueue(&mut tx, &work).unwrap(),
            "re-enqueue of identical semantic key is a no-op"
        );
        tx.commit().unwrap();

        assert_eq!(outbox_count(&db), 1, "exactly one row for the semantic key");
        assert_eq!(semantic_key_count(&db, "fts", rid.as_str()), 1);
    }

    // ── The model dimension distinguishes otherwise-identical work ────────
    #[test]
    fn model_partition_dimension_distinguishes_work() {
        let db = fresh_db();
        let rid = RecordId::new_v7();
        let base = OutboxWork::new("vectors", OutboxOp::Upsert)
            .with_record("memory", rid.clone())
            .with_content_hash("cafe");

        let mut model_a = base.clone();
        model_a.model_partition = Some("model-a".to_string());
        let mut model_b = base.clone();
        model_b.model_partition = Some("model-b".to_string());

        let tx_outbox = TxOutbox::new();
        let mut tx = db.begin().unwrap();
        // A NULL-model row and two distinct model rows are three distinct keys.
        assert!(tx_outbox.enqueue(&mut tx, &base).unwrap());
        assert!(tx_outbox.enqueue(&mut tx, &model_a).unwrap());
        assert!(tx_outbox.enqueue(&mut tx, &model_b).unwrap());
        // Re-enqueuing model-a is a no-op (same model dimension).
        assert!(!tx_outbox.enqueue(&mut tx, &model_a).unwrap());
        tx.commit().unwrap();

        assert_eq!(
            outbox_count(&db),
            3,
            "null-model + model-a + model-b are three distinct semantic keys"
        );
    }

    // ── Change → work mapping: record-scoped fans out; unscoped maps to none
    #[test]
    fn mapping_skips_changes_without_a_record() {
        // The deferred placeholder change carries no record_id → no work.
        let placeholder = GraphChange::new(GraphChangeKind::Insert, "user/chat/0");
        let work = projection_work_for_changes(&[placeholder], Some(GraphRevision::new(1)));
        assert!(work.is_empty(), "a change with no record implies no work");

        // A record-scoped insert fans out one upsert per projection target.
        let rid = RecordId::new_v7();
        let insert = GraphChange::new(GraphChangeKind::Insert, "user/chat/0")
            .with_record("memory", rid.clone())
            .with_hashes(None, Some("after-hash".to_string()));
        let work = projection_work_for_changes(&[insert], Some(GraphRevision::new(2)));
        assert_eq!(work.len(), PROJECTION_TARGETS.len(), "one item per target");
        for item in &work {
            assert_eq!(item.op, OutboxOp::Upsert);
            assert_eq!(item.record_id.as_ref(), Some(&rid));
            assert_eq!(item.content_hash.as_deref(), Some("after-hash"));
            assert_eq!(item.authority_revision, Some(GraphRevision::new(2)));
        }
    }

    // ── Delete/Invalidate map to a Delete op keyed by the before-hash ─────
    #[test]
    fn mapping_delete_uses_before_hash_and_delete_op() {
        let rid = RecordId::new_v7();
        let del = GraphChange::new(GraphChangeKind::Delete, "user/chat/0")
            .with_record("memory", rid)
            .with_hashes(Some("before-hash".to_string()), None);
        let work = projection_work_for_changes(&[del], None);
        assert_eq!(work.len(), PROJECTION_TARGETS.len());
        for item in &work {
            assert_eq!(item.op, OutboxOp::Delete);
            assert_eq!(item.content_hash.as_deref(), Some("before-hash"));
            assert_eq!(item.authority_revision, None);
        }
    }
}
