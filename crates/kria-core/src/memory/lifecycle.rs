//! Memory lifecycle: forget / delete cascade + crypto-shred (design §21, L9).
//!
//! `forget` tombstones (reversible 30 days); `hard_delete` cascades across all
//! stores in one authority transaction and crypto-shreds the subject key so the
//! content becomes cryptographically unreadable (design §21.1 / ADR-006). MVP key
//! storage is a single local keyfile row (`shred_keys`); KEK/DEK rotation +
//! recovery are deferred (§47.1).

use std::sync::Arc;

use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::stores::ports::{RelationalStore, SearchStore, VectorStore};
use crate::memory::stores::sqlite_search::delete_fts_in_tx;
use crate::memory::types::{MemoryState, ModelVersion};

/// What to forget/delete.
#[derive(Clone, Debug)]
pub enum ForgetScope {
    /// A single memory by id.
    Memory(Uuid),
    /// Every memory whose `source` provenance tag has this prefix
    /// (e.g. `tool:file_ops`, `mcp:github`, `library:{item}`) — per-source cascade.
    SourcePrefix(String),
    /// A session's memories (Temporary purge / session delete).
    Session(Uuid),
    /// An erasure subject key (person/employer/project) — crypto-shred target.
    Subject(String),
}

/// Lifecycle service.
pub struct Lifecycle {
    db: Arc<Database>,
    relational: Arc<dyn RelationalStore>,
    vectors: Arc<dyn VectorStore>,
    search: Arc<dyn SearchStore>,
    embedding_model: ModelVersion,
}

impl Lifecycle {
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

    /// Resolve a scope to the concrete memory ids it targets.
    pub fn resolve(&self, scope: &ForgetScope) -> MemoryResult<Vec<Uuid>> {
        self.db.with_read(|conn| {
            let mut ids = Vec::new();
            match scope {
                ForgetScope::Memory(id) => ids.push(*id),
                ForgetScope::SourcePrefix(prefix) => {
                    // Memories whose source event tag starts with `prefix`.
                    let like = format!("{prefix}%");
                    let mut stmt = conn
                        .prepare(
                            "SELECT m.id FROM memories m JOIN events e ON m.source_event_id = e.id \
                             WHERE e.source LIKE ?1",
                        )
                        .map_err(StorageError::Sqlite)?;
                    let rows = stmt
                        .query_map(params![like], |r| r.get::<_, String>(0))
                        .map_err(StorageError::Sqlite)?;
                    for r in rows {
                        let s = r.map_err(StorageError::Sqlite)?;
                        if let Ok(u) = Uuid::parse_str(&s) {
                            ids.push(u);
                        }
                    }
                }
                ForgetScope::Session(sid) => {
                    let mut stmt = conn
                        .prepare(
                            "SELECT m.id FROM memories m JOIN events e ON m.source_event_id = e.id \
                             WHERE e.session_id = ?1",
                        )
                        .map_err(StorageError::Sqlite)?;
                    let rows = stmt
                        .query_map(params![sid.to_string()], |r| r.get::<_, String>(0))
                        .map_err(StorageError::Sqlite)?;
                    for r in rows {
                        let s = r.map_err(StorageError::Sqlite)?;
                        if let Ok(u) = Uuid::parse_str(&s) {
                            ids.push(u);
                        }
                    }
                }
                ForgetScope::Subject(subject) => {
                    let mut stmt = conn
                        .prepare("SELECT id FROM memories WHERE shred_key_id = ?1")
                        .map_err(StorageError::Sqlite)?;
                    let rows = stmt
                        .query_map(params![subject], |r| r.get::<_, String>(0))
                        .map_err(StorageError::Sqlite)?;
                    for r in rows {
                        let s = r.map_err(StorageError::Sqlite)?;
                        if let Ok(u) = Uuid::parse_str(&s) {
                            ids.push(u);
                        }
                    }
                }
            }
            Ok(ids)
        })
    }

    /// Forget: tombstone the targeted memories (state = Forgotten), reversible for
    /// 30 days. Does not destroy anything yet (design §21.1).
    pub fn forget(&self, scope: &ForgetScope) -> MemoryResult<usize> {
        let ids = self.resolve(scope)?;
        let mut tx = self.db.begin()?;
        for id in &ids {
            self.relational
                .set_memory_state(&mut tx, *id, MemoryState::Forgotten)?;
        }
        tx.commit()?;
        Ok(ids.len())
    }

    /// Restore one tombstoned memory. Only `Forgotten` can transition back to
    /// `Active`; deleted, superseded, archived, and active rows are rejected.
    /// Forget keeps derived indexes in place, so restoring the authority state
    /// makes the same memory id queryable again without re-admission.
    pub fn restore(&self, id: Uuid) -> MemoryResult<()> {
        let memory = self.relational.get_memory(id)?.ok_or_else(|| {
            crate::memory::error::MemoryError::Internal(format!("restore: memory {id} not found"))
        })?;
        if memory.state != MemoryState::Forgotten {
            return Err(crate::memory::error::MemoryError::Internal(format!(
                "restore: memory {id} is {}, expected forgotten",
                memory.state
            )));
        }
        let mut tx = self.db.begin()?;
        self.relational
            .set_memory_state(&mut tx, id, MemoryState::Active)?;
        tx.commit()
    }

    /// Hard-delete: cascade across all stores in one authority transaction, then
    /// purge derived indexes and crypto-shred fully-deleted subjects (design §21.1).
    /// After this, no surface returns the content (L9 / CP-10).
    pub async fn hard_delete(&self, scope: &ForgetScope) -> MemoryResult<usize> {
        let ids = self.resolve(scope)?;
        if ids.is_empty() {
            return Ok(0);
        }

        // 1) Authority txn: mark deleted, prune graph edges, delete FTS in-txn.
        {
            let mut tx = self.db.begin()?;
            for id in &ids {
                self.relational
                    .set_memory_state(&mut tx, *id, MemoryState::Deleted)?;
                // Prune dangling graph edges referencing this memory's entities.
                tx.conn()
                    .execute(
                        "DELETE FROM memory_mentions_entity WHERE memory_id = ?1",
                        params![id.to_string()],
                    )
                    .map_err(StorageError::Sqlite)?;
            }
            delete_fts_in_tx(&mut tx, &ids)?;
            tx.commit()?;
        }

        // 2) Purge vectors (derived index).
        self.vectors.delete(&self.embedding_model, &ids).await?;
        // Belt-and-suspenders: also delete via the search store id path.
        self.search.delete(&ids).await?;

        // 3) Crypto-shred any subject whose memories are now all deleted.
        if let ForgetScope::Subject(subject) = scope {
            self.shred_subject(subject)?;
        }

        Ok(ids.len())
    }

    /// Destroy a subject's shred key → its encrypted payloads become unreadable
    /// (L9). Idempotent.
    pub fn shred_subject(&self, subject: &str) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "UPDATE shred_keys SET status = 'destroyed', destroyed_at = ?2 WHERE subject_id = ?1",
                params![subject, chrono::Utc::now().to_rfc3339()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// Whether a subject's key has been shredded.
    pub fn is_shredded(&self, subject: &str) -> MemoryResult<bool> {
        self.db.with_read(|conn| {
            let status: Option<String> = conn
                .query_row(
                    "SELECT status FROM shred_keys WHERE subject_id = ?1",
                    params![subject],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StorageError::Sqlite)?;
            Ok(status.as_deref() == Some("destroyed"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::stores::ports::EventStore;
    use crate::memory::stores::{
        SqliteEventStore, SqliteRelationalStore, SqliteSearchStore, SqliteVectorStore,
    };
    use crate::memory::types::{
        Event, EventType, Memory, MemoryType, MemoryWorth, Modality, Scope, Sensitivity, Source,
        StalenessClass, VectorPayload,
    };

    async fn setup() -> (
        Arc<Database>,
        Lifecycle,
        Arc<SqliteEventStore>,
        Arc<SqliteRelationalStore>,
        Arc<SqliteVectorStore>,
    ) {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let rel = Arc::new(SqliteRelationalStore::new(db.clone()));
        let vectors = Arc::new(SqliteVectorStore::new(db.clone()));
        let search = Arc::new(SqliteSearchStore::new(db.clone()));
        let lc = Lifecycle::new(
            db.clone(),
            rel.clone(),
            vectors.clone(),
            search.clone(),
            ModelVersion("fake_v1".into()),
        );
        (db, lc, events, rel, vectors)
    }

    fn make_memory(source_event: Uuid, hash: &str) -> Memory {
        let now = chrono::Utc::now();
        Memory {
            id: crate::memory::ids::new_id(),
            content: "sensitive fact".into(),
            memory_type: MemoryType::Semantic,
            compression_level: 0,
            source_event_id: source_event,
            namespace: "core".into(),
            owner_id: "user".into(),
            device_id: "d".into(),
            scope: Scope::Global,
            confidence: 0.8,
            importance: 5.0,
            access_count: 0,
            decay_score: 1.0,
            staleness_class: StalenessClass::Slow,
            sensitivity: Sensitivity::Private,
            state: MemoryState::Active,
            created_at: now,
            last_accessed: None,
            valid_from: now,
            valid_until: None,
            embedding_id: None,
            embedding_model_version: None,
            estimated_tokens: 3,
            content_hash: hash.into(),
            shred_key_id: Some("person:alice".into()),
            verify_against: None,
            superseded_by: None,
            episode_id: None,
            goal_context_id: None,
            worth: MemoryWorth::default(),
            modality: Modality::Text,
            preference_pair_id: None,
            training_eligible: false,
        }
    }

    async fn seed_memory(
        db: &Arc<Database>,
        events: &SqliteEventStore,
        rel: &SqliteRelationalStore,
        vectors: &SqliteVectorStore,
        source: Source,
        hash: &str,
    ) -> Uuid {
        let ev = Event {
            id: crate::memory::ids::new_id(),
            hlc: crate::memory::ids::HlcGenerator::new().now(),
            ts_utc: chrono::Utc::now(),
            tz_offset_min: 0,
            event_type: EventType::UserMessage,
            source,
            session_id: Some(crate::memory::ids::new_id()),
            parent_event_id: None,
            shred_key_id: Some("person:alice".into()),
            payload: serde_json::json!({}),
            encrypted: false,
            checksum: "c".into(),
        };
        let m = make_memory(ev.id, hash);
        {
            let mut tx = db.begin().unwrap();
            // seed the shred key first (events/memories FK-reference it)
            tx.conn()
                .execute(
                    "INSERT OR IGNORE INTO shred_keys(subject_id, subject_type, key_ref, status, created_at) \
                     VALUES('person:alice','person','keyfile:local','active',?1)",
                    params![chrono::Utc::now().to_rfc3339()],
                )
                .unwrap();
            events.append(&mut tx, &ev).unwrap();
            rel.upsert_memory(&mut tx, &m).unwrap();
            tx.commit().unwrap();
        }
        vectors
            .upsert(
                &ModelVersion("fake_v1".into()),
                m.id,
                &[0.1, 0.2, 0.3],
                &VectorPayload {
                    namespace: "core".into(),
                    scope: Scope::Global,
                    sensitivity: Sensitivity::Private,
                    memory_type: MemoryType::Semantic,
                    content_hash: hash.into(),
                    created_at: chrono::Utc::now(),
                },
            )
            .await
            .unwrap();
        m.id
    }

    #[tokio::test]
    async fn forget_is_reversible_then_hard_delete_shreds() {
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "h1").await;

        // forget → tombstone, reversible.
        assert_eq!(lc.forget(&ForgetScope::Memory(id)).unwrap(), 1);
        assert_eq!(
            rel.get_memory(id).unwrap().unwrap().state,
            MemoryState::Forgotten
        );
        lc.restore(id).unwrap();
        assert_eq!(
            rel.get_memory(id).unwrap().unwrap().state,
            MemoryState::Active
        );

        // hard delete a subject → cascade + crypto-shred.
        let n = lc
            .hard_delete(&ForgetScope::Subject("person:alice".into()))
            .await
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            rel.get_memory(id).unwrap().unwrap().state,
            MemoryState::Deleted
        );
        assert!(lc.is_shredded("person:alice").unwrap());
        // Vector purged.
        assert!(vectors
            .all_ids(&ModelVersion("fake_v1".into()))
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn per_source_cascade() {
        let (db, lc, events, rel, vectors) = setup().await;
        seed_memory(
            &db,
            &events,
            &rel,
            &vectors,
            Source::Mcp {
                server: "github".into(),
                tool: "search".into(),
            },
            "h2",
        )
        .await;
        // forget by mcp:github source prefix.
        let n = lc
            .hard_delete(&ForgetScope::SourcePrefix("mcp:github".into()))
            .await
            .unwrap();
        assert_eq!(n, 1);
    }
}
