//! Merge / split lifecycle operations (memory-upgrade design §21.2, R14/D-17).
//!
//! Both are atomic across the authority in one transaction: originals are
//! **archived, not deleted** (provenance preserved via `memory_derived_from`),
//! and the operation is reversible. A crash after commit is repaired by the
//! reconciliation sweep + idempotent relay.

use std::sync::Arc;

use rusqlite::params;
use uuid::Uuid;

use crate::db::Database;
use crate::error::{MemoryError, MemoryResult, StorageError};
use crate::ids::{new_id, normalized_content_hash};
use crate::stores::ports::RelationalStore;
use crate::types::{Memory, MemoryState, MemoryWorth};

pub struct MergeService {
    db: Arc<Database>,
    relational: Arc<dyn RelationalStore>,
}

impl MergeService {
    pub fn new(db: Arc<Database>, relational: Arc<dyn RelationalStore>) -> Self {
        Self { db, relational }
    }

    fn link_derived(
        &self,
        tx: &mut crate::db::AuthorityTx<'_>,
        parent: Uuid,
        child: Uuid,
    ) -> MemoryResult<()> {
        tx.conn()
            .execute(
                "INSERT OR IGNORE INTO memory_derived_from(parent_id, child_id) VALUES(?1, ?2)",
                params![parent.to_string(), child.to_string()],
            )
            .map_err(StorageError::Sqlite)?;
        Ok(())
    }

    /// Merge two memories into one. The merged memory `derived_from` both
    /// originals; originals are archived (version history). Returns the new id.
    pub fn merge(&self, a: Uuid, b: Uuid) -> MemoryResult<Uuid> {
        let ma = self
            .relational
            .get_memory(a)?
            .ok_or_else(|| MemoryError::Internal(format!("merge: memory {a} not found")))?;
        let mb = self
            .relational
            .get_memory(b)?
            .ok_or_else(|| MemoryError::Internal(format!("merge: memory {b} not found")))?;

        let content = if ma.content == mb.content {
            ma.content.clone()
        } else {
            format!("{}\n{}", ma.content, mb.content)
        };
        let now = chrono::Utc::now();
        let merged = Memory {
            id: new_id(),
            content: content.clone(),
            memory_type: ma.memory_type.clone(),
            compression_level: ma.compression_level.max(mb.compression_level),
            source_event_id: ma.source_event_id,
            namespace: ma.namespace.clone(),
            owner_id: ma.owner_id.clone(),
            device_id: ma.device_id.clone(),
            scope: ma.scope.clone(),
            confidence: ma.confidence.max(mb.confidence),
            importance: ma.importance.max(mb.importance),
            access_count: ma.access_count + mb.access_count,
            decay_score: ma.decay_score.max(mb.decay_score),
            staleness_class: ma.staleness_class.clone(),
            sensitivity: if ma.sensitivity == crate::types::Sensitivity::Secret
                || mb.sensitivity == crate::types::Sensitivity::Secret
            {
                crate::types::Sensitivity::Secret
            } else {
                ma.sensitivity.clone()
            },
            state: MemoryState::Active,
            created_at: now,
            last_accessed: None,
            valid_from: now,
            valid_until: None,
            embedding_id: None,
            embedding_model_version: None,
            estimated_tokens: crate::governance::estimate_tokens(&content),
            content_hash: normalized_content_hash(&content),
            shred_key_id: ma.shred_key_id.clone(),
            verify_against: ma.verify_against.clone(),
            superseded_by: None,
            episode_id: ma.episode_id,
            goal_context_id: ma.goal_context_id,
            worth: MemoryWorth {
                success: ma.worth.success + mb.worth.success,
                failure: ma.worth.failure + mb.worth.failure,
                samples: ma.worth.samples + mb.worth.samples,
            },
            modality: ma.modality.clone(),
            preference_pair_id: None,
            training_eligible: false,
        };

        let mut tx = self.db.begin()?;
        self.relational.upsert_memory(&mut tx, &merged)?;
        self.relational
            .set_memory_state(&mut tx, a, MemoryState::Archived)?;
        self.relational
            .set_memory_state(&mut tx, b, MemoryState::Archived)?;
        self.link_derived(&mut tx, merged.id, a)?;
        self.link_derived(&mut tx, merged.id, b)?;
        tx.commit()?;
        Ok(merged.id)
    }

    /// Split one memory into several (by provided content parts). Each part
    /// `derived_from` the original; the original is archived. Returns new ids.
    pub fn split(&self, id: Uuid, parts: &[String]) -> MemoryResult<Vec<Uuid>> {
        let orig = self
            .relational
            .get_memory(id)?
            .ok_or_else(|| MemoryError::Internal(format!("split: memory {id} not found")))?;
        if parts.is_empty() {
            return Ok(Vec::new());
        }
        let now = chrono::Utc::now();
        let mut new_ids = Vec::with_capacity(parts.len());
        let mut tx = self.db.begin()?;
        for part in parts {
            let child = Memory {
                id: new_id(),
                content: part.clone(),
                content_hash: normalized_content_hash(part),
                estimated_tokens: crate::governance::estimate_tokens(part),
                created_at: now,
                valid_from: now,
                state: MemoryState::Active,
                worth: MemoryWorth::default(),
                access_count: 0,
                embedding_id: None,
                embedding_model_version: None,
                ..orig.clone()
            };
            self.relational.upsert_memory(&mut tx, &child)?;
            self.link_derived(&mut tx, child.id, id)?;
            new_ids.push(child.id);
        }
        self.relational
            .set_memory_state(&mut tx, id, MemoryState::Archived)?;
        tx.commit()?;
        Ok(new_ids)
    }

    /// Reverse a merge: reactivate the originals, delete the merged memory.
    pub fn unmerge(&self, merged_id: Uuid, originals: &[Uuid]) -> MemoryResult<()> {
        let mut tx = self.db.begin()?;
        for o in originals {
            self.relational
                .set_memory_state(&mut tx, *o, MemoryState::Active)?;
        }
        self.relational
            .set_memory_state(&mut tx, merged_id, MemoryState::Deleted)?;
        tx.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stores::ports::EventStore;
    use crate::stores::{SqliteEventStore, SqliteRelationalStore};
    use crate::types::{
        Event, EventType, MemoryType, Modality, Scope, Sensitivity, Source, StalenessClass,
    };

    fn seed(
        db: &Arc<Database>,
        events: &SqliteEventStore,
        rel: &SqliteRelationalStore,
        content: &str,
        hash: &str,
    ) -> Uuid {
        let ev = Event {
            id: new_id(),
            hlc: crate::ids::HlcGenerator::new().now(),
            ts_utc: chrono::Utc::now(),
            tz_offset_min: 0,
            event_type: EventType::UserMessage,
            source: Source::User,
            session_id: None,
            parent_event_id: None,
            shred_key_id: None,
            payload: serde_json::json!({}),
            encrypted: false,
            checksum: "c".into(),
        };
        let now = chrono::Utc::now();
        let m = Memory {
            id: new_id(),
            content: content.into(),
            memory_type: MemoryType::Semantic,
            compression_level: 0,
            source_event_id: ev.id,
            namespace: "core".into(),
            owner_id: "user".into(),
            device_id: "d".into(),
            scope: Scope::Global,
            confidence: 0.7,
            importance: 5.0,
            access_count: 2,
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
            shred_key_id: None,
            verify_against: None,
            superseded_by: None,
            episode_id: None,
            goal_context_id: None,
            worth: MemoryWorth {
                success: 3,
                failure: 1,
                samples: 4,
            },
            modality: Modality::Text,
            preference_pair_id: None,
            training_eligible: false,
        };
        let mut tx = db.begin().unwrap();
        events.append(&mut tx, &ev).unwrap();
        rel.upsert_memory(&mut tx, &m).unwrap();
        tx.commit().unwrap();
        m.id
    }

    #[test]
    fn merge_archives_originals_and_sums_worth() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let rel = Arc::new(SqliteRelationalStore::new(db.clone()));
        let svc = MergeService::new(db.clone(), rel.clone());
        let a = seed(&db, &events, &rel, "fact a", "ha");
        let b = seed(&db, &events, &rel, "fact b", "hb");

        let merged = svc.merge(a, b).unwrap();
        assert_eq!(
            rel.get_memory(a).unwrap().unwrap().state,
            MemoryState::Archived
        );
        assert_eq!(
            rel.get_memory(b).unwrap().unwrap().state,
            MemoryState::Archived
        );
        let m = rel.get_memory(merged).unwrap().unwrap();
        assert_eq!(m.worth.samples, 8); // 4 + 4
        assert_eq!(m.access_count, 4);

        // reversible
        svc.unmerge(merged, &[a, b]).unwrap();
        assert_eq!(
            rel.get_memory(a).unwrap().unwrap().state,
            MemoryState::Active
        );
        assert_eq!(
            rel.get_memory(merged).unwrap().unwrap().state,
            MemoryState::Deleted
        );
    }

    #[test]
    fn split_creates_children_and_archives_original() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let rel = Arc::new(SqliteRelationalStore::new(db.clone()));
        let svc = MergeService::new(db.clone(), rel.clone());
        let id = seed(&db, &events, &rel, "part one. part two.", "hc");
        let kids = svc
            .split(id, &["part one".into(), "part two".into()])
            .unwrap();
        assert_eq!(kids.len(), 2);
        assert_eq!(
            rel.get_memory(id).unwrap().unwrap().state,
            MemoryState::Archived
        );
        for k in kids {
            assert_eq!(
                rel.get_memory(k).unwrap().unwrap().state,
                MemoryState::Active
            );
        }
    }
}
