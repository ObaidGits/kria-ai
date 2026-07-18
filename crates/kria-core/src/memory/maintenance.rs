//! Reconciliation sweep + outbox relay (memory-upgrade design §25, D-5/D-16, N12).
//!
//! The reconciliation sweep repairs referential integrity of the derived indexes
//! against the SQLite authority (the sole source of truth, Issue 29): orphan
//! vectors/FTS rows and dangling graph edges are purged. The outbox relay applies
//! queued index operations idempotently (the delete path for the MVP; the upsert
//! path activates with the LanceDB backend).

use std::collections::HashSet;
use std::sync::Arc;

use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::stores::ports::{RelationalStore, SearchStore, VectorStore};
use crate::memory::types::{IndexTarget, ModelVersion, OutboxOp, OutboxStatus};

/// What a reconciliation sweep repaired.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepairReport {
    pub orphan_vectors_removed: usize,
    pub orphan_fts_removed: usize,
    pub dangling_edges_removed: usize,
}

/// Maintenance service (reconciliation + relay).
pub struct Maintenance {
    db: Arc<Database>,
    #[allow(dead_code)]
    relational: Arc<dyn RelationalStore>,
    vectors: Arc<dyn VectorStore>,
    search: Arc<dyn SearchStore>,
    embedding_model: ModelVersion,
}

impl Maintenance {
    pub fn new(
        db: Arc<Database>,
        relational: Arc<dyn RelationalStore>,
        vectors: Arc<dyn VectorStore>,
        search: Arc<dyn SearchStore>,
        embedding_model: ModelVersion,
    ) -> Self {
        Self {
            db,
            relational,
            vectors,
            search,
            embedding_model,
        }
    }

    /// The set of memory ids that *should* have a live derived-index presence
    /// (active or promoted). Everything else in an index is an orphan.
    fn live_ids(&self) -> MemoryResult<HashSet<Uuid>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare("SELECT id FROM memories WHERE state IN ('active','promoted')")
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(StorageError::Sqlite)?;
            let mut set = HashSet::new();
            for r in rows {
                if let Ok(u) = Uuid::parse_str(&r.map_err(StorageError::Sqlite)?) {
                    set.insert(u);
                }
            }
            Ok(set)
        })
    }

    /// Run the reconciliation sweep (design §25, N12). Idempotent.
    pub async fn reconcile(&self) -> MemoryResult<RepairReport> {
        let live = self.live_ids()?;
        let mut report = RepairReport::default();

        // 1) Orphan vectors: present in the index but not live in the authority.
        let vec_ids = self.vectors.all_ids(&self.embedding_model).await?;
        let orphan_vecs: Vec<Uuid> = vec_ids
            .into_iter()
            .filter(|id| !live.contains(id))
            .collect();
        if !orphan_vecs.is_empty() {
            self.vectors
                .delete(&self.embedding_model, &orphan_vecs)
                .await?;
            report.orphan_vectors_removed = orphan_vecs.len();
        }

        // 2) Orphan FTS rows.
        let fts_ids = self.search.all_ids().await?;
        let orphan_fts: Vec<Uuid> = fts_ids
            .into_iter()
            .filter(|id| !live.contains(id))
            .collect();
        if !orphan_fts.is_empty() {
            self.search.delete(&orphan_fts).await?;
            report.orphan_fts_removed = orphan_fts.len();
        }

        // 3) Dangling graph edges (source/target entity no longer present).
        let dangling = {
            let db = self.db.clone();
            db.with_read(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM relationships r WHERE \
                     NOT EXISTS (SELECT 1 FROM entities e WHERE e.id = r.source_id) \
                     OR NOT EXISTS (SELECT 1 FROM entities e WHERE e.id = r.target_id)",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(StorageError::Sqlite)
                .map_err(Into::into)
            })?
        };
        if dangling > 0 {
            let tx = self.db.begin()?;
            tx.conn()
                .execute(
                    "DELETE FROM relationships WHERE \
                     NOT EXISTS (SELECT 1 FROM entities e WHERE e.id = relationships.source_id) \
                     OR NOT EXISTS (SELECT 1 FROM entities e WHERE e.id = relationships.target_id)",
                    [],
                )
                .map_err(StorageError::Sqlite)?;
            tx.commit()?;
            report.dangling_edges_removed = dangling as usize;
        }

        Ok(report)
    }

    /// Relay pending outbox entries for a target, idempotently (design §25/D-5).
    /// The MVP producers enqueue **delete** ops; upsert ops are applied by the
    /// LanceDB backend path (no-op here, left pending-safe).
    pub async fn relay(&self, target: IndexTarget, batch: usize) -> MemoryResult<usize> {
        let pending = self.relational_pending(target, batch)?;
        let mut done = 0usize;
        for entry in pending {
            let applied = match entry.op {
                OutboxOp::Delete => {
                    match target {
                        IndexTarget::LanceDb => {
                            self.vectors
                                .delete(&self.embedding_model, &[entry.memory_id])
                                .await?;
                        }
                        IndexTarget::Fts | IndexTarget::Tantivy => {
                            self.search.delete(&[entry.memory_id]).await?;
                        }
                    }
                    true
                }
                // Upsert requires the vector payload, carried by the LanceDB
                // backend path; skip here without marking done so it is retried
                // once that path is active.
                OutboxOp::Upsert => false,
            };
            if applied {
                let mut tx = self.db.begin()?;
                self.relational.mark_outbox(
                    &mut tx,
                    entry.id,
                    OutboxStatus::Done,
                    entry.attempts + 1,
                )?;
                tx.commit()?;
                done += 1;
            }
        }
        Ok(done)
    }

    fn relational_pending(
        &self,
        target: IndexTarget,
        batch: usize,
    ) -> MemoryResult<Vec<crate::memory::types::OutboxEntry>> {
        self.relational.pending_outbox(target, batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::stores::{SqliteRelationalStore, SqliteSearchStore, SqliteVectorStore};
    use crate::memory::types::{MemoryType, OutboxEntry, Scope, Sensitivity, VectorPayload};

    fn build() -> (
        Arc<Database>,
        Maintenance,
        Arc<SqliteVectorStore>,
        Arc<SqliteSearchStore>,
        Arc<SqliteRelationalStore>,
    ) {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let rel = Arc::new(SqliteRelationalStore::new(db.clone()));
        let vectors = Arc::new(SqliteVectorStore::new(db.clone()));
        let search = Arc::new(SqliteSearchStore::new(db.clone()));
        let m = Maintenance::new(
            db.clone(),
            rel.clone(),
            vectors.clone(),
            search.clone(),
            ModelVersion("fake_v1".into()),
        );
        (db, m, vectors, search, rel)
    }

    #[tokio::test]
    async fn reconcile_purges_orphan_vector() {
        let (_db, maint, vectors, _search, _rel) = build();
        // A vector with no corresponding live memory → orphan.
        let orphan = Uuid::now_v7();
        vectors
            .upsert(
                &ModelVersion("fake_v1".into()),
                orphan,
                &[0.1, 0.2],
                &VectorPayload {
                    namespace: "core".into(),
                    scope: Scope::Global,
                    sensitivity: Sensitivity::Private,
                    memory_type: MemoryType::Semantic,
                    content_hash: "h".into(),
                    created_at: chrono::Utc::now(),
                },
            )
            .await
            .unwrap();
        let report = maint.reconcile().await.unwrap();
        assert_eq!(report.orphan_vectors_removed, 1);
        assert!(vectors
            .all_ids(&ModelVersion("fake_v1".into()))
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn relay_applies_delete_ops() {
        let (db, maint, _vectors, search, rel) = build();
        let mem_id = Uuid::now_v7();
        // Index an FTS row then enqueue its delete via the outbox.
        search.index(mem_id, "to be deleted", "core").await.unwrap();
        {
            let mut tx = db.begin().unwrap();
            rel.enqueue_outbox(&mut tx, &OutboxEntry::delete(mem_id, IndexTarget::Fts))
                .unwrap();
            tx.commit().unwrap();
        }
        let done = maint.relay(IndexTarget::Fts, 10).await.unwrap();
        assert_eq!(done, 1);
        assert!(search.all_ids().await.unwrap().is_empty());
        // Outbox row marked done → not pending.
        assert!(rel.pending_outbox(IndexTarget::Fts, 10).unwrap().is_empty());
    }
}
