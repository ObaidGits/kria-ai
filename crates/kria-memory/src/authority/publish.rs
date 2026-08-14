//! Post-commit publication — the advisory wake/cursor and the durable reconnect
//! read path (task **F1.3.7**, design §5.1 `Committed --> Published: post-commit
//! patch wake`, §5.2 patch `{baseRevision,targetRevision,…,recoveryCursor}`).
//!
//! This is the **last** stage of the AuthorityTx flow, and the only one that
//! runs *outside* the serialized-writer transaction — strictly **after**
//! [`AuthorityTransaction::commit`](super::transaction::AuthorityTransaction::commit)
//! returns. Two things follow from that placement, and they are the
//! non-negotiables this module exists to guarantee:
//!
//! 1. **Publication cannot alter committed truth.** By the time a [`RevisionWake`]
//!    is built and handed to a [`WakePublisher`], the transaction is already
//!    committed and *consumed* — there is no open `AuthorityTx`, no connection,
//!    nothing writable in scope. A publish that fails, finds no subscriber, or
//!    never runs (crash between commit and publish) leaves every committed row
//!    exactly as it was. The wake is advisory; the durable rows are the truth.
//! 2. **A lost wake is fully recoverable from durable state.** The wake carries
//!    *no* authority data — only a `{base_revision → target_revision}` cursor and
//!    a "there is pending derived work" flag. A consumer that missed the wake
//!    reconstructs everything it needs from the committed `graph_revisions`
//!    ledger ([`revisions_since`], the recovery cursor) and the committed
//!    `derived_outbox` pending rows
//!    ([`OutboxPort::pending`](super::OutboxPort::pending)). Correctness comes
//!    from the durable cursor, never from wake delivery.
//!
//! This mirrors the existing enrichment slow-path contract
//! ([`crate::write_policy`]): "a full channel drops the *wake*, never
//! the data — the event is already durable and the catch-up sweep recovers it".
//! The F1.3.7 wake is the same shape of promise at the authority-revision level.
//!
//! ## Reuse of the existing notification mechanism
//!
//! [`WakePublisher`] is a narrow seam, not a parallel channel: the live memory
//! system already owns a `broadcast::Sender<MemoryChange>`
//! ([`crate::api::MemorySystem::subscribe_changes`]). The adapter that
//! maps a [`RevisionWake`] onto that existing channel lives with the composition
//! root (which already depends on this module), so the payload published on the
//! wire is a cursor wake — never the committed data — without inventing a second
//! notification path or changing any Tauri/adapter event name.

use rusqlite::params;

use crate::db::Database;
use crate::error::{MemoryResult, StorageError};
use crate::model::{GraphRevision, UtcTimestamp};

use super::transaction::CommandRecord;

// ─────────────────────────────────────────────────────────────────────────
// RevisionWake — the advisory post-commit wake/cursor payload
// ─────────────────────────────────────────────────────────────────────────

/// The advisory post-commit wake: a *cursor*, never the committed data.
///
/// Emitted once, after commit, for a graph-visible command that advanced the
/// authority revision. It carries exactly three facts — the revision the
/// consumer's snapshot should move from (`base_revision`), the revision the
/// authority now reflects (`target_revision`), and whether the commit enqueued
/// derived-projection work worth waking a converger for (`has_pending_work`).
///
/// It deliberately holds **no** record ids, content, hashes, or change rows: a
/// subscriber uses the wake only to decide *that* it should read, then reads the
/// authoritative durable state ([`revisions_since`] + the outbox `pending`
/// rows). Because the payload is a pure cursor, a wake can never diverge from —
/// or be mistaken for — committed truth, and re-delivering the same wake is
/// harmless (see [`WakePublisher`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevisionWake {
    /// The revision the change was applied on top of (the client/consumer
    /// snapshot this wake advances *from*). Equals `target_revision - 1` for a
    /// single committed revision.
    base_revision: GraphRevision,
    /// The revision the authority now reflects (the snapshot this wake advances
    /// *to*). Always strictly greater than `base_revision`.
    target_revision: GraphRevision,
    /// Whether the committed command enqueued derived-projection outbox work
    /// (FTS/vectors/scene). A converger uses this as a hint to drain; a `false`
    /// wake still advances the read cursor but implies no projection work.
    has_pending_work: bool,
}

impl RevisionWake {
    /// Build the wake for a committed [`CommandRecord`], or `None` when there is
    /// nothing to wake about.
    ///
    /// A wake is produced **only** for a graph-visible accepted command — one
    /// whose [`CommandRecord::revision`] is `Some` (the revision counter
    /// advanced). A revision-neutral / deferred command returns `None`: nothing
    /// advanced, so there is no cursor to publish. `base` is the command's base
    /// revision (`CommandEnvelope::base_revision`); `has_pending_work` is whether
    /// the F1.3.6 projection-work enqueue produced any outbox items.
    ///
    /// The returned wake is *derived from already-committed facts* — it can only
    /// describe what the transaction durably wrote, never override it.
    pub fn for_committed(
        base: GraphRevision,
        record: &CommandRecord,
        has_pending_work: bool,
    ) -> Option<Self> {
        Self::advancing(base, record.revision?, has_pending_work)
    }

    /// Build a wake advancing `base → target`, or `None` when `target` does not
    /// strictly advance past `base` (a wake always describes a revision advance;
    /// a non-advancing pair is not a wake). Used by [`for_committed`](Self::for_committed)
    /// and by adapters/consumers that reconstruct a wake from an explicit cursor.
    pub fn advancing(
        base: GraphRevision,
        target: GraphRevision,
        has_pending_work: bool,
    ) -> Option<Self> {
        if target <= base {
            return None;
        }
        Some(Self {
            base_revision: base,
            target_revision: target,
            has_pending_work,
        })
    }

    /// The revision the consumer's snapshot advances *from* (patch `baseRevision`).
    pub fn base_revision(self) -> GraphRevision {
        self.base_revision
    }

    /// The revision the authority now reflects (patch `targetRevision`).
    pub fn target_revision(self) -> GraphRevision {
        self.target_revision
    }

    /// Whether the commit enqueued derived-projection outbox work.
    pub fn has_pending_work(self) -> bool {
        self.has_pending_work
    }

    /// The durable **recovery cursor** a consumer that missed this wake reads
    /// from (patch `recoveryCursor`): the base revision. Reading
    /// [`revisions_since`] with this cursor returns every revision from the one
    /// after `base_revision` up to (and past) `target_revision`, so a dropped
    /// wake loses no committed work.
    pub fn recovery_cursor(self) -> GraphRevision {
        self.base_revision
    }
}

// ─────────────────────────────────────────────────────────────────────────
// WakePublisher — the post-commit publication seam
// ─────────────────────────────────────────────────────────────────────────

/// The post-commit publication seam: emit an advisory [`RevisionWake`].
///
/// Implementations are **best-effort and infallible from the caller's view** —
/// [`publish`](Self::publish) returns nothing, because a publication failure
/// (no subscribers, a dropped channel, backpressure) must never surface as a
/// command error or roll anything back. The committed transaction has already
/// returned by the time this is called; the wake is a courtesy that lets live
/// subscribers react without polling, and every consumer can reconstruct the
/// same information from durable state if the wake never arrives.
///
/// Re-publishing the same wake is safe: subscribers treat a wake as advisory and
/// converge from the durable cursor, so a duplicate wake triggers at most a
/// redundant (idempotent) read — it can neither duplicate committed truth nor
/// re-run a semantic effect.
pub trait WakePublisher: Send + Sync {
    /// Publish one advisory revision wake. Never fails the command; a dropped
    /// wake is recovered from [`revisions_since`] + the outbox.
    fn publish(&self, wake: &RevisionWake);
}

/// A publisher that drops every wake — the honest model of "no subscribers /
/// publication dropped". Committed truth still stands and stays recoverable from
/// the durable revision/outbox cursor; this type exists to make that path
/// explicit (and to wire the authority flow when no live channel is attached).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopWakePublisher;

impl WakePublisher for NoopWakePublisher {
    fn publish(&self, _wake: &RevisionWake) {
        // Intentionally nothing: the wake is advisory and the durable
        // revision/outbox cursor is the recovery path.
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Reconnect / recovery read path — "revisions since cursor"
// ─────────────────────────────────────────────────────────────────────────

/// One committed revision summary read back from the durable `graph_revisions`
/// ledger for reconnect/recovery. Carries the ledger metadata a consumer needs
/// to catch up — **not** record content (that is read from the projections /
/// query surface once the consumer knows *which* revisions it missed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedRevision {
    /// The revision number.
    pub revision: GraphRevision,
    /// The base revision it was applied on (`revision - 1`, contiguous).
    pub base_revision: GraphRevision,
    /// The unique transaction id (the command's completion event id).
    pub tx_id: String,
    /// Number of `graph_changes` rows this revision recorded.
    pub change_count: u64,
    /// When the revision committed (canonical UTC).
    pub committed_at: UtcTimestamp,
}

/// Read the durable committed revisions *after* `after`, oldest first — the
/// authoritative reconnect/recovery path (design §5.1 "reconnect reads
/// revisions", §5.2 `recoveryCursor`).
///
/// This is the query a consumer runs when it (re)connects with a last-known
/// revision cursor — whether it never subscribed, missed a wake, or crashed:
/// it returns exactly the revisions committed since `after`, contiguously and
/// deterministically, from the append-only `graph_revisions` ledger. Combined
/// with [`OutboxPort::pending`](super::OutboxPort::pending) for the outstanding
/// derived work, it reconstructs *all* outstanding work from durable state with
/// no dependence on wake delivery.
///
/// Determinism / idempotency: the read is ordered by `revision` and reads only
/// committed rows, so calling it twice with the same cursor returns byte-equal
/// results, and processing the same revision twice converges (the ledger is
/// append-only and never rewritten). `limit` bounds a single page; pass a large
/// bound (or loop advancing the cursor) to drain everything.
pub fn revisions_since(
    db: &Database,
    after: GraphRevision,
    limit: usize,
) -> MemoryResult<Vec<CommittedRevision>> {
    db.with_read(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT revision, base_revision, tx_id, change_count, committed_at
                 FROM graph_revisions
                 WHERE revision > ?1
                 ORDER BY revision ASC
                 LIMIT ?2",
            )
            .map_err(StorageError::Sqlite)?;
        let rows = stmt
            .query_map(params![after.get() as i64, limit as i64], |row| {
                let revision: i64 = row.get(0)?;
                let base_revision: i64 = row.get(1)?;
                let tx_id: String = row.get(2)?;
                let change_count: i64 = row.get(3)?;
                let committed_at: String = row.get(4)?;
                Ok((revision, base_revision, tx_id, change_count, committed_at))
            })
            .map_err(StorageError::Sqlite)?;

        let mut out = Vec::new();
        for row in rows {
            let (revision, base_revision, tx_id, change_count, committed_at) =
                row.map_err(StorageError::Sqlite)?;
            out.push(CommittedRevision {
                revision: GraphRevision::new(revision.max(0) as u64),
                base_revision: GraphRevision::new(base_revision.max(0) as u64),
                tx_id,
                change_count: change_count.max(0) as u64,
                committed_at: UtcTimestamp::from_rfc3339_utc(&committed_at)?,
            });
        }
        Ok(out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::command::CommandEnvelope;
    use crate::authority::command::{Deadline, SourceContext, SourceKind, SourceTrust};
    use crate::authority::revision::{GraphChange, GraphChangeKind};
    use crate::authority::transaction::{
        AuthorityTransaction, DeferredSemanticStore, SemanticOutcome, TxSemanticStore,
    };
    use crate::authority::{CommandKind, OutboxPort, SqliteOutbox};
    use crate::db::{AuthorityTx, Database};
    use crate::model::{
        CallerContext, GraphRevision, IdempotencyKey, InvocationId, PolicyPartition, RecordId,
    };
    use crate::types::MemoryMode;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn fresh_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().expect("open in-memory authority"))
    }

    fn caller() -> CallerContext {
        let partition = PolicyPartition::new("user", "chat", 0).unwrap();
        CallerContext::local_desktop("local-desktop", partition).unwrap()
    }

    fn source() -> SourceContext {
        SourceContext::new(
            InvocationId::new_v7(),
            SourceKind::Native,
            "core:cognition",
            SourceTrust::System,
        )
        .unwrap()
    }

    /// A command envelope with a fresh idempotency key / invocation, issued
    /// against `base`.
    fn observe_env(key: &str, base: GraphRevision) -> CommandEnvelope {
        CommandEnvelope::new(
            caller(),
            CommandKind::Observe,
            IdempotencyKey::new(key).unwrap(),
            base,
            source(),
            MemoryMode::Permanent,
            Deadline::default_write(),
            serde_json::json!({"content": "hello"}),
            None,
        )
        .unwrap()
    }

    /// A test semantic store that returns a **record-scoped** graph-visible
    /// change, so the F1.3.6 projection-work enqueue produces real
    /// `derived_outbox` rows (the deferred placeholder store emits none). Writes
    /// no semantic rows (there is no cognitive-record table yet) — exactly like
    /// [`DeferredSemanticStore`] — it only shapes the outcome.
    struct RecordSemanticStore {
        record_id: RecordId,
    }

    impl TxSemanticStore for RecordSemanticStore {
        fn apply(
            &self,
            _tx: &mut AuthorityTx<'_>,
            env: &CommandEnvelope,
        ) -> MemoryResult<SemanticOutcome> {
            let change = GraphChange::new(GraphChangeKind::Insert, env.caller().partition_key())
                .with_record("memory", self.record_id.clone())
                .with_hashes(None, Some("after-hash".to_string()));
            Ok(SemanticOutcome::graph_visible(vec![change]))
        }
    }

    /// A publisher that records every wake it received (order preserved), so a
    /// test can assert publication happened and with what cursor.
    #[derive(Default)]
    struct CollectingWakePublisher {
        wakes: Mutex<Vec<RevisionWake>>,
        count: AtomicUsize,
    }

    impl CollectingWakePublisher {
        fn wakes(&self) -> Vec<RevisionWake> {
            self.wakes.lock().unwrap().clone()
        }
        fn count(&self) -> usize {
            self.count.load(Ordering::SeqCst)
        }
    }

    impl WakePublisher for CollectingWakePublisher {
        fn publish(&self, wake: &RevisionWake) {
            self.count.fetch_add(1, Ordering::SeqCst);
            self.wakes.lock().unwrap().push(*wake);
        }
    }

    fn revision_count(db: &Database) -> i64 {
        db.with_read(|conn| {
            Ok(conn
                .query_row("SELECT COUNT(*) FROM graph_revisions", [], |r| r.get(0))
                .unwrap())
        })
        .unwrap()
    }

    fn current_revision(db: &Database) -> i64 {
        db.with_read(|conn| {
            Ok(conn
                .query_row(
                    "SELECT graph_revision FROM authority_meta WHERE id = 1",
                    [],
                    |r| r.get(0),
                )
                .unwrap())
        })
        .unwrap()
    }

    // ── A wake is built only for a graph-visible revision advance ─────────
    #[test]
    fn for_committed_only_wakes_on_graph_visible_advance() {
        let db = fresh_db();
        let publisher = CollectingWakePublisher::default();

        // Graph-visible command advances the revision → exactly one wake.
        let env = observe_env("cmd-1", GraphRevision::base());
        let record = AuthorityTransaction::begin(&db)
            .unwrap()
            .commit_and_publish(&env, &DeferredSemanticStore, None, &publisher)
            .unwrap();

        assert_eq!(record.revision, Some(GraphRevision::new(1)));
        assert_eq!(publisher.count(), 1, "one wake for a graph-visible commit");
        let wake = publisher.wakes()[0];
        assert_eq!(wake.base_revision(), GraphRevision::base());
        assert_eq!(wake.target_revision(), GraphRevision::new(1));
        assert_eq!(wake.recovery_cursor(), GraphRevision::base());
    }

    // ── for_committed maps a revision-neutral record to no wake ───────────
    #[test]
    fn for_committed_is_none_for_revision_neutral_command() {
        // A record with no reserved revision (revision-neutral / deferred).
        let db = fresh_db();
        let env = observe_env("cmd-neutral", GraphRevision::base());
        // Build a CommandRecord with revision = None by recording a deferred
        // disposition (no revision reserved).
        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        let neutral = SemanticOutcome::revision_neutral();
        let record = txn
            .record_disposition_with_revision(
                &env,
                crate::authority::AuditDisposition::Deferred,
                &neutral,
                &[],
                None,
            )
            .unwrap();
        txn.commit().unwrap();

        assert_eq!(record.revision, None);
        assert!(
            RevisionWake::for_committed(GraphRevision::base(), &record, false).is_none(),
            "no revision advance → no wake"
        );
    }

    // ── Publication is post-commit: committed rows stand regardless ───────
    #[test]
    fn post_commit_publish_happens_and_committed_rows_intact() {
        let db = fresh_db();
        let publisher = CollectingWakePublisher::default();
        let rid = RecordId::new_v7();
        let store = RecordSemanticStore {
            record_id: rid.clone(),
        };

        let env = observe_env("cmd-1", GraphRevision::base());
        let record = AuthorityTransaction::begin(&db)
            .unwrap()
            .commit_and_publish(&env, &store, None, &publisher)
            .unwrap();

        // The commit is the durable truth boundary: rows exist after it returns.
        assert_eq!(record.revision, Some(GraphRevision::new(1)));
        assert_eq!(current_revision(&db), 1, "authority_meta advanced once");
        assert_eq!(revision_count(&db), 1, "one graph_revisions row committed");

        // The post-commit wake fired once and carries the committed cursor and a
        // "has pending work" flag (the record-scoped change enqueued outbox work).
        assert_eq!(publisher.count(), 1);
        let wake = publisher.wakes()[0];
        assert_eq!(wake.target_revision(), GraphRevision::new(1));
        assert!(
            wake.has_pending_work(),
            "record-scoped change enqueued work"
        );
    }

    // ── Lost publication is fully recoverable from revisions + outbox ─────
    #[test]
    fn lost_publication_is_recoverable_from_revisions_and_outbox() {
        let db = fresh_db();
        // "Publication dropped": no subscriber, the wake is thrown away.
        let dropped = NoopWakePublisher;
        let rid = RecordId::new_v7();
        let store = RecordSemanticStore {
            record_id: rid.clone(),
        };

        let env = observe_env("cmd-1", GraphRevision::base());
        let record = AuthorityTransaction::begin(&db)
            .unwrap()
            .commit_and_publish(&env, &store, None, &dropped)
            .unwrap();
        assert_eq!(record.revision, Some(GraphRevision::new(1)));

        // A consumer that never saw a wake reconnects from its last-known cursor
        // (the base revision 0) and reconstructs the missed revision purely from
        // the durable ledger.
        let missed = revisions_since(&db, GraphRevision::base(), 100).unwrap();
        assert_eq!(missed.len(), 1, "the committed revision is recoverable");
        assert_eq!(missed[0].revision, GraphRevision::new(1));
        assert_eq!(missed[0].base_revision, GraphRevision::base());
        assert_eq!(missed[0].change_count, 1);

        // …and the outstanding derived work is recoverable from the outbox with
        // no dependence on the wake.
        let outbox = SqliteOutbox::new(db.clone());
        for target in crate::authority::PROJECTION_TARGETS {
            let pending = outbox.pending(target, 100).unwrap();
            assert_eq!(pending.len(), 1, "pending work recoverable for {target}");
            assert_eq!(pending[0].record_id.as_ref(), Some(&rid));
        }
    }

    // ── Reconnect reconstructs ALL missed revisions, contiguously ─────────
    #[test]
    fn reconnect_reconstructs_all_missed_revisions() {
        let db = fresh_db();
        let dropped = NoopWakePublisher;

        // Commit three graph-visible commands; every wake is dropped.
        for i in 0..3 {
            let base = GraphRevision::new(i);
            let env = observe_env(&format!("cmd-{i}"), base);
            AuthorityTransaction::begin(&db)
                .unwrap()
                .commit_and_publish(&env, &DeferredSemanticStore, None, &dropped)
                .unwrap();
        }
        assert_eq!(current_revision(&db), 3);

        // A consumer at cursor 0 recovers all three, in contiguous order.
        let missed = revisions_since(&db, GraphRevision::base(), 100).unwrap();
        let revs: Vec<u64> = missed.iter().map(|r| r.revision.get()).collect();
        assert_eq!(revs, vec![1, 2, 3], "all missed revisions, contiguous");

        // A consumer partway through (cursor 2) recovers only what it missed.
        let tail = revisions_since(&db, GraphRevision::new(2), 100).unwrap();
        let tail_revs: Vec<u64> = tail.iter().map(|r| r.revision.get()).collect();
        assert_eq!(tail_revs, vec![3], "only revisions after the cursor");
    }

    // ── Recovery reads are deterministic and never re-emit committed truth ─
    #[test]
    fn revisions_since_is_deterministic_and_no_duplication() {
        let db = fresh_db();
        let dropped = NoopWakePublisher;
        let env = observe_env("cmd-1", GraphRevision::base());
        AuthorityTransaction::begin(&db)
            .unwrap()
            .commit_and_publish(&env, &DeferredSemanticStore, None, &dropped)
            .unwrap();

        // Reading the same cursor twice returns byte-equal results (no dup, no
        // loss): re-processing a revision converges.
        let a = revisions_since(&db, GraphRevision::base(), 100).unwrap();
        let b = revisions_since(&db, GraphRevision::base(), 100).unwrap();
        assert_eq!(a, b, "the ledger read is deterministic");

        // A consumer already at the tip (cursor = target) sees nothing new — the
        // committed revision is never re-emitted.
        let caught_up = revisions_since(&db, GraphRevision::new(1), 100).unwrap();
        assert!(caught_up.is_empty(), "no re-emission past the cursor");
    }

    // ── Re-publishing the same wake is advisory: no duplicated truth ──────
    #[test]
    fn republishing_same_wake_does_not_duplicate_committed_truth() {
        let db = fresh_db();
        let publisher = CollectingWakePublisher::default();
        let rid = RecordId::new_v7();
        let store = RecordSemanticStore {
            record_id: rid.clone(),
        };
        let env = observe_env("cmd-1", GraphRevision::base());
        let record = AuthorityTransaction::begin(&db)
            .unwrap()
            .commit_and_publish(&env, &store, None, &publisher)
            .unwrap();

        // Snapshot durable truth right after the (single) post-commit publish.
        let revs_before = revision_count(&db);
        let outbox = SqliteOutbox::new(db.clone());
        let pending_before: usize = crate::authority::PROJECTION_TARGETS
            .iter()
            .map(|t| outbox.pending(t, 100).unwrap().len())
            .sum();

        // Re-publish the SAME wake several more times (advisory redelivery).
        let wake = RevisionWake::for_committed(env.base_revision(), &record, true).unwrap();
        for _ in 0..5 {
            publisher.publish(&wake);
        }
        assert_eq!(
            publisher.count(),
            6,
            "1 post-commit + 5 manual redeliveries"
        );

        // The durable truth is unchanged by re-publication — the wake never
        // touches committed rows.
        assert_eq!(revision_count(&db), revs_before, "revisions unchanged");
        let pending_after: usize = crate::authority::PROJECTION_TARGETS
            .iter()
            .map(|t| outbox.pending(t, 100).unwrap().len())
            .sum();
        assert_eq!(
            pending_after, pending_before,
            "outbox unchanged by re-publish"
        );
    }
}
