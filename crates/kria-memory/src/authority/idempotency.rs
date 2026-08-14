//! The transaction-scoped idempotency-result ledger (task **F1.3.6**, design
//! §4.1 `idempotency_results`, §5.1 "AuthorityTx … writes … idempotency result
//! … then commits", MGR-005/MGR-033 replay).
//!
//! [`TxIdempotency`] is the **transaction-scoped repository** that stores the
//! canonical committed result of a governed command in `idempotency_results`
//! *using only the serialized-writer transaction connection* handed to it —
//! exactly like the other F1.3 transaction-scoped repositories
//! ([`TxEventLog`](super::event_log::TxEventLog),
//! [`TxAuditLog`](super::audit::TxAuditLog),
//! [`TxRevisionLog`](super::revision::TxRevisionLog),
//! [`TxOutbox`](super::outbox::TxOutbox)). It carries no [`Database`] handle, so
//! mis-wiring the write onto a second connection is structurally impossible
//! (F1.3 non-negotiable: "all writes must occur on the transaction connection").
//!
//! ## Why the write is inside the command transaction (all-or-none)
//!
//! The idempotency row must commit **atomically** with the semantic / event /
//! audit / revision / outbox rows of the same command. If it committed
//! separately a crash between the two could either (a) leave a stored result
//! for a command whose effects rolled back, or (b) re-execute a command whose
//! effects committed — both violate replay correctness (MGR-005 AC3). Writing
//! it on the same [`AuthorityTx`] makes the record and its result one unit.
//!
//! ## What a later replay reads
//!
//! The pre-transaction validator (F1.3.2,
//! [`ValidationReads::lookup_idempotency`](super::validation::ValidationReads::lookup_idempotency))
//! reads this row back by `(caller_partition, idempotency_key)`; a matching
//! `command_hash` replays the stored [`result_json`](StoredResult), a different
//! hash is an idempotency conflict. The composite primary key
//! `(caller_partition, idempotency_key)` (migration 0013) enforces one result
//! per caller-partition key.
//!
//! [`Database`]: crate::db::Database
//! [`AuthorityTx`]: crate::db::AuthorityTx
//! [`StoredResult`]: super::validation::StoredIdempotencyResult

use rusqlite::params;

use crate::db::AuthorityTx;
use crate::error::{MemoryResult, StorageError};
use crate::model::{EventId, GraphRevision, UtcTimestamp};

use super::command::CommandEnvelope;

/// The `status` stored in a committed command's canonical result JSON. Mirrors
/// the `snake_case` serialization of
/// [`CommandStatus::Committed`](super::CommandStatus) so the stored result and
/// the live command outcome speak one vocabulary.
pub const COMMITTED_STATUS: &str = "committed";

/// Build the **canonical result JSON** persisted in
/// `idempotency_results.result_json` and returned verbatim on a later replay.
///
/// The object carries the command outcome's `status`, completion `event_id`,
/// and committed `revision` (design §5.1 command outcome). It is serialized
/// deterministically: `serde_json`'s map iterates its keys in sorted order (this
/// build does not enable `preserve_order`), so the same inputs always produce
/// byte-identical JSON — a stored result never drifts across serializations.
///
/// * `event_id` is `null` when no completion event was appended.
/// * `revision` is `null` for a revision-neutral committed command.
pub fn canonical_result_json(
    status: &str,
    event_id: Option<&EventId>,
    committed_revision: Option<GraphRevision>,
) -> MemoryResult<String> {
    let value = serde_json::json!({
        "status": status,
        "event_id": event_id.map(EventId::as_str),
        "revision": committed_revision.map(|r| r.get()),
    });
    serde_json::to_string(&value).map_err(|e| StorageError::Serde(e.to_string()).into())
}

// ─────────────────────────────────────────────────────────────────────────
// TxIdempotency — the transaction-scoped idempotency-result repository
// ─────────────────────────────────────────────────────────────────────────

/// The transaction-scoped write surface over `idempotency_results`.
///
/// A zero-sized handle: [`persist`](Self::persist) takes the `&mut AuthorityTx`
/// it must write through, so — exactly like the other F1.3 transaction-scoped
/// repositories — it is structurally impossible for it to write anywhere other
/// than the serialized-writer transaction (F1.3 invariant). It owns no
/// [`Database`](crate::db::Database) / connection / pool.
#[derive(Debug, Default, Clone, Copy)]
pub struct TxIdempotency;

impl TxIdempotency {
    /// Construct the (stateless) idempotency-result repository.
    pub fn new() -> Self {
        TxIdempotency
    }

    /// Persist the canonical result of an accepted command on `tx`, keyed by the
    /// envelope's `(caller_partition, idempotency_key)` and stamped with its
    /// canonical `command_hash`.
    ///
    /// The `command_hash`, `caller_partition`, and `idempotency_key` are taken
    /// from `env` so they can never disagree with the command being recorded;
    /// the caller supplies the already-serialized canonical `result_json` (see
    /// [`canonical_result_json`]), the committed `revision` (the
    /// `committed_revision` column, `None` for a revision-neutral command), and
    /// the completion `event_id` (the nullable `event_id` FK into `events_v2`,
    /// which is present in the same transaction).
    ///
    /// This is a plain `INSERT`: the pre-transaction validator (F1.3.2) already
    /// rejected a replay/conflict before the transaction opened, and the
    /// serialized writer guarantees no concurrent insert under the same key — so
    /// a primary-key conflict here would be a genuine invariant violation
    /// surfaced as a storage error, never silently ignored.
    pub fn persist(
        &self,
        tx: &mut AuthorityTx<'_>,
        env: &CommandEnvelope,
        result_json: &str,
        committed_revision: Option<GraphRevision>,
        event_id: Option<&EventId>,
    ) -> MemoryResult<()> {
        tx.conn()
            .execute(
                "INSERT INTO idempotency_results(
                     caller_partition, idempotency_key, command_hash, result_json,
                     committed_revision, event_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    env.caller().partition_key(),
                    env.idempotency_key().as_str(),
                    env.command_hash().as_str(),
                    result_json,
                    committed_revision.map(|r| r.get() as i64),
                    event_id.map(EventId::as_str),
                    UtcTimestamp::now().to_rfc3339(),
                ],
            )
            .map_err(StorageError::Sqlite)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EventId, GraphRevision};

    // ── Canonical result JSON is deterministic and carries the outcome ────
    #[test]
    fn canonical_result_json_is_deterministic_and_complete() {
        let event = EventId::new_v7();
        let a = canonical_result_json(COMMITTED_STATUS, Some(&event), Some(GraphRevision::new(7)))
            .unwrap();
        let b = canonical_result_json(COMMITTED_STATUS, Some(&event), Some(GraphRevision::new(7)))
            .unwrap();
        assert_eq!(a, b, "same inputs → byte-identical JSON");

        // Keys are present and sorted (serde_json map iterates sorted here).
        let parsed: serde_json::Value = serde_json::from_str(&a).unwrap();
        assert_eq!(parsed["status"], "committed");
        assert_eq!(parsed["event_id"], event.as_str());
        assert_eq!(parsed["revision"], 7);
    }

    // ── A revision-neutral committed result carries null revision ─────────
    #[test]
    fn canonical_result_json_nulls_absent_fields() {
        let json = canonical_result_json(COMMITTED_STATUS, None, None).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["status"], "committed");
        assert!(parsed["event_id"].is_null(), "absent event → null");
        assert!(parsed["revision"].is_null(), "revision-neutral → null");
    }
}
