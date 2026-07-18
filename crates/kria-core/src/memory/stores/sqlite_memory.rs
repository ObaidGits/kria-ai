//! SQLite-backed [`RelationalStore`] (memory-upgrade design §14/§16).
//!
//! Owns derived memories + the transactional outbox. Writes join the authority
//! transaction; reads use the WAL read pool.

use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::memory::db::{AuthorityTx, Database};
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::types::{
    AuditRecord, IndexTarget, Memory, MemoryState, MemoryType, MemoryWorth, Modality, ModelVersion,
    OutboxEntry, OutboxOp, OutboxStatus, Scope, Sensitivity, StalenessClass, VerifyPredicate,
};

use super::ports::RelationalStore;

const MEM_COLS: &str = "id, content, memory_type, compression_level, source_event_id, namespace, \
     owner_id, device_id, scope, confidence, importance, access_count, decay_score, \
     staleness_class, sensitivity, state, created_at, last_accessed, valid_from, valid_until, \
     embedding_id, embedding_model_version, estimated_tokens, content_hash, shred_key_id, \
     verify_against, superseded_by, episode_id, goal_context_id, memory_worth_success, \
     memory_worth_failure, memory_worth_samples, modality, preference_pair_id, training_eligible";

fn parse_uuid(s: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(s).map_err(|e| StorageError::Serde(format!("bad uuid {s:?}: {e}")))
}
fn parse_uuid_opt(s: Option<String>) -> Result<Option<Uuid>, StorageError> {
    s.map(|s| parse_uuid(&s)).transpose()
}
fn parse_ts(s: &str) -> Result<chrono::DateTime<chrono::Utc>, StorageError> {
    Ok(chrono::DateTime::parse_from_rfc3339(s)
        .map_err(|e| StorageError::Serde(format!("bad timestamp {s:?}: {e}")))?
        .with_timezone(&chrono::Utc))
}
fn parse_ts_opt(s: Option<String>) -> Result<Option<chrono::DateTime<chrono::Utc>>, StorageError> {
    s.map(|s| parse_ts(&s)).transpose()
}

/// Build a [`Memory`] from a full `memories` row (column order = `MEM_COLS`).
fn row_to_memory(row: &Row<'_>) -> MemoryResult<Memory> {
    // Extract raw values first (rusqlite errors), then convert (richer errors).
    let id: String = row.get(0).map_err(StorageError::Sqlite)?;
    let content: String = row.get(1).map_err(StorageError::Sqlite)?;
    let memory_type: String = row.get(2).map_err(StorageError::Sqlite)?;
    let compression_level: i64 = row.get(3).map_err(StorageError::Sqlite)?;
    let source_event_id: String = row.get(4).map_err(StorageError::Sqlite)?;
    let namespace: String = row.get(5).map_err(StorageError::Sqlite)?;
    let owner_id: String = row.get(6).map_err(StorageError::Sqlite)?;
    let device_id: String = row.get(7).map_err(StorageError::Sqlite)?;
    let scope: String = row.get(8).map_err(StorageError::Sqlite)?;
    let confidence: f64 = row.get(9).map_err(StorageError::Sqlite)?;
    let importance: f64 = row.get(10).map_err(StorageError::Sqlite)?;
    let access_count: i64 = row.get(11).map_err(StorageError::Sqlite)?;
    let decay_score: f64 = row.get(12).map_err(StorageError::Sqlite)?;
    let staleness_class: String = row.get(13).map_err(StorageError::Sqlite)?;
    let sensitivity: String = row.get(14).map_err(StorageError::Sqlite)?;
    let state: String = row.get(15).map_err(StorageError::Sqlite)?;
    let created_at: String = row.get(16).map_err(StorageError::Sqlite)?;
    let last_accessed: Option<String> = row.get(17).map_err(StorageError::Sqlite)?;
    let valid_from: String = row.get(18).map_err(StorageError::Sqlite)?;
    let valid_until: Option<String> = row.get(19).map_err(StorageError::Sqlite)?;
    let embedding_id: Option<String> = row.get(20).map_err(StorageError::Sqlite)?;
    let embedding_model_version: Option<String> = row.get(21).map_err(StorageError::Sqlite)?;
    let estimated_tokens: i64 = row.get(22).map_err(StorageError::Sqlite)?;
    let content_hash: String = row.get(23).map_err(StorageError::Sqlite)?;
    let shred_key_id: Option<String> = row.get(24).map_err(StorageError::Sqlite)?;
    let verify_against: Option<String> = row.get(25).map_err(StorageError::Sqlite)?;
    let superseded_by: Option<String> = row.get(26).map_err(StorageError::Sqlite)?;
    let episode_id: Option<String> = row.get(27).map_err(StorageError::Sqlite)?;
    let goal_context_id: Option<String> = row.get(28).map_err(StorageError::Sqlite)?;
    let mw_success: i64 = row.get(29).map_err(StorageError::Sqlite)?;
    let mw_failure: i64 = row.get(30).map_err(StorageError::Sqlite)?;
    let mw_samples: i64 = row.get(31).map_err(StorageError::Sqlite)?;
    let modality: String = row.get(32).map_err(StorageError::Sqlite)?;
    let preference_pair_id: Option<String> = row.get(33).map_err(StorageError::Sqlite)?;
    let training_eligible: i64 = row.get(34).map_err(StorageError::Sqlite)?;

    let verify = match verify_against {
        Some(s) => Some(
            serde_json::from_str::<VerifyPredicate>(&s)
                .map_err(|e| StorageError::Serde(format!("bad verify_against: {e}")))?,
        ),
        None => None,
    };

    Ok(Memory {
        id: parse_uuid(&id)?,
        content,
        memory_type: memory_type.parse::<MemoryType>().unwrap(),
        compression_level: compression_level.clamp(0, 3) as u8,
        source_event_id: parse_uuid(&source_event_id)?,
        namespace,
        owner_id,
        device_id,
        scope: scope.parse::<Scope>().unwrap(),
        confidence: confidence as f32,
        importance: importance as f32,
        access_count: access_count.max(0) as u64,
        decay_score: decay_score as f32,
        staleness_class: staleness_class.parse::<StalenessClass>().unwrap(),
        sensitivity: sensitivity.parse::<Sensitivity>().unwrap(),
        state: state.parse::<MemoryState>().unwrap(),
        created_at: parse_ts(&created_at)?,
        last_accessed: parse_ts_opt(last_accessed)?,
        valid_from: parse_ts(&valid_from)?,
        valid_until: parse_ts_opt(valid_until)?,
        embedding_id: parse_uuid_opt(embedding_id)?,
        embedding_model_version: embedding_model_version.map(ModelVersion),
        estimated_tokens: estimated_tokens.max(0) as u32,
        content_hash,
        shred_key_id,
        verify_against: verify,
        superseded_by: parse_uuid_opt(superseded_by)?,
        episode_id: parse_uuid_opt(episode_id)?,
        goal_context_id: parse_uuid_opt(goal_context_id)?,
        worth: MemoryWorth {
            success: mw_success.max(0) as u32,
            failure: mw_failure.max(0) as u32,
            samples: mw_samples.max(0) as u32,
        },
        modality: modality.parse::<Modality>().unwrap(),
        preference_pair_id,
        training_eligible: training_eligible != 0,
    })
}

pub struct SqliteRelationalStore {
    db: Arc<Database>,
}

impl SqliteRelationalStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

impl RelationalStore for SqliteRelationalStore {
    fn upsert_memory(&self, tx: &mut AuthorityTx<'_>, m: &Memory) -> MemoryResult<()> {
        let verify = match &m.verify_against {
            Some(v) => {
                Some(serde_json::to_string(v).map_err(|e| StorageError::Serde(e.to_string()))?)
            }
            None => None,
        };
        tx.conn()
            .execute(
                "INSERT INTO memories (\
                 id, content, memory_type, compression_level, source_event_id, namespace, \
                 owner_id, device_id, scope, confidence, importance, access_count, decay_score, \
                 staleness_class, sensitivity, state, created_at, last_accessed, valid_from, \
                 valid_until, embedding_id, embedding_model_version, estimated_tokens, \
                 content_hash, shred_key_id, verify_against, superseded_by, episode_id, \
                 goal_context_id, memory_worth_success, memory_worth_failure, \
                 memory_worth_samples, modality, preference_pair_id, training_eligible) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,\
                 ?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35) \
                 ON CONFLICT(id) DO UPDATE SET \
                 content=excluded.content, memory_type=excluded.memory_type, \
                 compression_level=excluded.compression_level, namespace=excluded.namespace, \
                 owner_id=excluded.owner_id, device_id=excluded.device_id, scope=excluded.scope, \
                 confidence=excluded.confidence, importance=excluded.importance, \
                 access_count=excluded.access_count, decay_score=excluded.decay_score, \
                 staleness_class=excluded.staleness_class, sensitivity=excluded.sensitivity, \
                 state=excluded.state, last_accessed=excluded.last_accessed, \
                 valid_from=excluded.valid_from, valid_until=excluded.valid_until, \
                 embedding_id=excluded.embedding_id, \
                 embedding_model_version=excluded.embedding_model_version, \
                 estimated_tokens=excluded.estimated_tokens, content_hash=excluded.content_hash, \
                 shred_key_id=excluded.shred_key_id, verify_against=excluded.verify_against, \
                 superseded_by=excluded.superseded_by, episode_id=excluded.episode_id, \
                 goal_context_id=excluded.goal_context_id, \
                 memory_worth_success=excluded.memory_worth_success, \
                 memory_worth_failure=excluded.memory_worth_failure, \
                 memory_worth_samples=excluded.memory_worth_samples, modality=excluded.modality, \
                 preference_pair_id=excluded.preference_pair_id, \
                 training_eligible=excluded.training_eligible",
                params![
                    m.id.to_string(),
                    m.content,
                    m.memory_type.as_str(),
                    m.compression_level as i64,
                    m.source_event_id.to_string(),
                    m.namespace,
                    m.owner_id,
                    m.device_id,
                    m.scope.as_str(),
                    m.confidence as f64,
                    m.importance as f64,
                    m.access_count as i64,
                    m.decay_score as f64,
                    m.staleness_class.as_str(),
                    m.sensitivity.as_str(),
                    m.state.as_str(),
                    m.created_at.to_rfc3339(),
                    m.last_accessed.map(|t| t.to_rfc3339()),
                    m.valid_from.to_rfc3339(),
                    m.valid_until.map(|t| t.to_rfc3339()),
                    m.embedding_id.map(|u| u.to_string()),
                    m.embedding_model_version.as_ref().map(|v| v.0.clone()),
                    m.estimated_tokens as i64,
                    m.content_hash,
                    m.shred_key_id,
                    verify,
                    m.superseded_by.map(|u| u.to_string()),
                    m.episode_id.map(|u| u.to_string()),
                    m.goal_context_id.map(|u| u.to_string()),
                    m.worth.success as i64,
                    m.worth.failure as i64,
                    m.worth.samples as i64,
                    m.modality.as_str(),
                    m.preference_pair_id,
                    m.training_eligible as i64,
                ],
            )
            .map_err(StorageError::Sqlite)?;
        Ok(())
    }

    fn get_memory(&self, id: Uuid) -> MemoryResult<Option<Memory>> {
        self.db.with_read(|conn: &Connection| {
            conn.query_row(
                &format!("SELECT {MEM_COLS} FROM memories WHERE id = ?1"),
                params![id.to_string()],
                |r| Ok(row_to_memory(r)),
            )
            .optional()
            .map_err(StorageError::Sqlite)?
            .transpose()
        })
    }

    fn set_memory_state(
        &self,
        tx: &mut AuthorityTx<'_>,
        id: Uuid,
        state: MemoryState,
    ) -> MemoryResult<()> {
        tx.conn()
            .execute(
                "UPDATE memories SET state = ?2 WHERE id = ?1",
                params![id.to_string(), state.as_str()],
            )
            .map_err(StorageError::Sqlite)?;
        Ok(())
    }

    fn find_by_content_hash(
        &self,
        namespace: &str,
        memory_type: &str,
        content_hash: &str,
    ) -> MemoryResult<Option<Memory>> {
        self.db.with_read(|conn: &Connection| {
            conn.query_row(
                &format!(
                    "SELECT {MEM_COLS} FROM memories WHERE namespace=?1 AND memory_type=?2 \
                     AND content_hash=?3 AND state='active'"
                ),
                params![namespace, memory_type, content_hash],
                |r| Ok(row_to_memory(r)),
            )
            .optional()
            .map_err(StorageError::Sqlite)?
            .transpose()
        })
    }

    fn enqueue_outbox(&self, tx: &mut AuthorityTx<'_>, e: &OutboxEntry) -> MemoryResult<()> {
        tx.conn()
            .execute(
                "INSERT INTO embedding_outbox(memory_id, index_target, op, content_hash, \
                 attempts, status, created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    e.memory_id.to_string(),
                    e.index_target.as_str(),
                    e.op.as_str(),
                    e.content_hash,
                    e.attempts as i64,
                    e.status.as_str(),
                    e.created_at.to_rfc3339(),
                ],
            )
            .map_err(StorageError::Sqlite)?;
        Ok(())
    }

    fn pending_outbox(&self, target: IndexTarget, limit: usize) -> MemoryResult<Vec<OutboxEntry>> {
        self.db.with_read(|conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, memory_id, index_target, op, content_hash, attempts, status, \
                     created_at FROM embedding_outbox WHERE index_target = ?1 AND status = 'pending' \
                     ORDER BY id ASC LIMIT ?2",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![target.as_str(), limit as i64], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, String>(7)?,
                    ))
                })
                .map_err(StorageError::Sqlite)?;
            let mut out = Vec::new();
            for row in rows {
                let (id, mem, tgt, op, hash, attempts, status, created) =
                    row.map_err(StorageError::Sqlite)?;
                out.push(OutboxEntry {
                    id,
                    memory_id: parse_uuid(&mem)?,
                    index_target: IndexTarget::from_str(&tgt)
                        .ok_or_else(|| StorageError::Serde(format!("bad index_target {tgt}")))?,
                    op: if op == "delete" {
                        OutboxOp::Delete
                    } else {
                        OutboxOp::Upsert
                    },
                    content_hash: hash,
                    attempts: attempts.max(0) as u32,
                    status: OutboxStatus::from_str(&status)
                        .ok_or_else(|| StorageError::Serde(format!("bad status {status}")))?,
                    created_at: parse_ts(&created)?,
                });
            }
            Ok(out)
        })
    }

    fn mark_outbox(
        &self,
        tx: &mut AuthorityTx<'_>,
        id: i64,
        status: OutboxStatus,
        attempts: u32,
    ) -> MemoryResult<()> {
        tx.conn()
            .execute(
                "UPDATE embedding_outbox SET status = ?2, attempts = ?3 WHERE id = ?1",
                params![id, status.as_str(), attempts as i64],
            )
            .map_err(StorageError::Sqlite)?;
        Ok(())
    }

    fn record_audit(&self, tx: &mut AuthorityTx<'_>, r: &AuditRecord) -> MemoryResult<()> {
        tx.conn()
            .execute(
                "INSERT INTO memory_audit(id, ts, decision, reason, candidate_hash, namespace, mode) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    r.id.to_string(),
                    r.ts.to_rfc3339(),
                    r.decision.as_str(),
                    r.reason,
                    r.candidate_hash,
                    r.namespace,
                    r.mode.as_ref().map(|m| m.as_str().to_string()),
                ],
            )
            .map_err(StorageError::Sqlite)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::ids::{new_id, normalized_content_hash, HlcGenerator};
    use crate::memory::stores::ports::EventStore;
    use crate::memory::stores::SqliteEventStore;
    use crate::memory::types::{Event, EventType, Source};

    fn seed_event(db: &Arc<Database>, events: &SqliteEventStore) -> Uuid {
        let gen = HlcGenerator::new();
        let ev = Event {
            id: new_id(),
            hlc: gen.now(),
            ts_utc: chrono::Utc::now(),
            tz_offset_min: 0,
            event_type: EventType::UserMessage,
            source: Source::User,
            session_id: Some(new_id()),
            parent_event_id: None,
            shred_key_id: None,
            payload: serde_json::json!({}),
            encrypted: false,
            checksum: "c".into(),
        };
        let mut tx = db.begin().unwrap();
        events.append(&mut tx, &ev).unwrap();
        tx.commit().unwrap();
        ev.id
    }

    fn sample_memory(source_event_id: Uuid, content: &str) -> Memory {
        let now = chrono::Utc::now();
        Memory {
            id: new_id(),
            content: content.to_string(),
            memory_type: MemoryType::Semantic,
            compression_level: 0,
            source_event_id,
            namespace: "core".into(),
            owner_id: "user".into(),
            device_id: "dev".into(),
            scope: Scope::Global,
            confidence: 0.8,
            importance: 6.0,
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
            estimated_tokens: 5,
            content_hash: normalized_content_hash(content),
            shred_key_id: None,
            verify_against: Some(VerifyPredicate::Path("/tmp/x".into())),
            superseded_by: None,
            episode_id: None,
            goal_context_id: None,
            worth: MemoryWorth::default(),
            modality: Modality::Text,
            preference_pair_id: None,
            training_eligible: false,
        }
    }

    #[test]
    fn memory_roundtrip_and_state_update() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let events = SqliteEventStore::new(db.clone());
        let store = SqliteRelationalStore::new(db.clone());
        let evid = seed_event(&db, &events);
        let m = sample_memory(evid, "the user prefers dark mode");

        let mut tx = db.begin().unwrap();
        store.upsert_memory(&mut tx, &m).unwrap();
        tx.commit().unwrap();

        let got = store.get_memory(m.id).unwrap().expect("present");
        assert_eq!(got.content, m.content);
        assert_eq!(got.confidence, m.confidence);
        assert_eq!(got.verify_against, m.verify_against);
        assert_eq!(got.state, MemoryState::Active);

        let mut tx = db.begin().unwrap();
        store
            .set_memory_state(&mut tx, m.id, MemoryState::Archived)
            .unwrap();
        tx.commit().unwrap();
        assert_eq!(
            store.get_memory(m.id).unwrap().unwrap().state,
            MemoryState::Archived
        );
    }

    #[test]
    fn dedup_lookup_only_matches_active() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let events = SqliteEventStore::new(db.clone());
        let store = SqliteRelationalStore::new(db.clone());
        let evid = seed_event(&db, &events);
        let m = sample_memory(evid, "kria runs locally");

        let mut tx = db.begin().unwrap();
        store.upsert_memory(&mut tx, &m).unwrap();
        tx.commit().unwrap();

        let found = store
            .find_by_content_hash("core", "semantic", &m.content_hash)
            .unwrap();
        assert!(found.is_some());

        // Archive it → no longer a dedup target.
        let mut tx = db.begin().unwrap();
        store
            .set_memory_state(&mut tx, m.id, MemoryState::Archived)
            .unwrap();
        tx.commit().unwrap();
        assert!(store
            .find_by_content_hash("core", "semantic", &m.content_hash)
            .unwrap()
            .is_none());
    }

    #[test]
    fn outbox_enqueue_pending_mark() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let store = SqliteRelationalStore::new(db.clone());
        let mem_id = new_id();

        let mut tx = db.begin().unwrap();
        store
            .enqueue_outbox(
                &mut tx,
                &OutboxEntry::upsert(mem_id, IndexTarget::LanceDb, "h1"),
            )
            .unwrap();
        tx.commit().unwrap();

        let pending = store.pending_outbox(IndexTarget::LanceDb, 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].memory_id, mem_id);

        let mut tx = db.begin().unwrap();
        store
            .mark_outbox(&mut tx, pending[0].id, OutboxStatus::Done, 1)
            .unwrap();
        tx.commit().unwrap();
        assert!(store
            .pending_outbox(IndexTarget::LanceDb, 10)
            .unwrap()
            .is_empty());
    }
}
