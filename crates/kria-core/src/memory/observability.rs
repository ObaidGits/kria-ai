//! Observability: explain + health report + metrics (memory-upgrade design §28, L6).
//!
//! Every memory is explainable (L6): `explain_memory` reconstructs a memory's
//! provenance chain, contradictions, Memory Worth, and access history;
//! `memory_health_report` summarizes the bank for the "what KRIA believes about
//! you" surface. Read-only.

use std::sync::Arc;

use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};

/// Provenance + status explanation for a single memory (L6).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemoryExplanation {
    pub id: Uuid,
    pub content: String,
    pub memory_type: String,
    pub state: String,
    pub confidence: f32,
    pub importance: f32,
    pub source_event_tag: Option<String>,
    pub derived_from: Vec<Uuid>,
    pub contradicts: Vec<Uuid>,
    pub worth_success: u32,
    pub worth_failure: u32,
    pub worth_samples: u32,
    pub access_count: u64,
    pub staleness_class: String,
    pub superseded_by: Option<Uuid>,
}

/// Aggregate health / "what KRIA believes about you" report (design §28).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemoryHealthReport {
    pub total_active: i64,
    pub total_archived: i64,
    pub total_superseded: i64,
    pub total_forgotten: i64,
    pub by_type: Vec<(String, i64)>,
    pub by_staleness: Vec<(String, i64)>,
    pub avg_confidence: f64,
    pub unresolved_contradictions: i64,
    pub knowledge_gaps: i64,
    pub enrichment_backlog: i64,
    pub outbox_pending: i64,
}

pub struct Observability {
    db: Arc<Database>,
}

impl Observability {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Explain a memory: provenance chain + status (L6). `None` if not found.
    pub fn explain_memory(&self, id: Uuid) -> MemoryResult<Option<MemoryExplanation>> {
        self.db.with_read(|conn| {
            let base: Option<(
                String,
                String,
                String,
                f64,
                f64,
                String,
                u32,
                u32,
                u32,
                i64,
                String,
                Option<String>,
                String,
            )> = conn
                .query_row(
                    "SELECT m.content, m.memory_type, m.state, m.confidence, m.importance, \
                     e.source, m.memory_worth_success, m.memory_worth_failure, \
                     m.memory_worth_samples, m.access_count, m.staleness_class, m.superseded_by, \
                     m.source_event_id \
                     FROM memories m JOIN events e ON m.source_event_id = e.id WHERE m.id = ?1",
                    params![id.to_string()],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get::<_, i64>(6)? as u32,
                            r.get::<_, i64>(7)? as u32,
                            r.get::<_, i64>(8)? as u32,
                            r.get::<_, i64>(9)?,
                            r.get(10)?,
                            r.get::<_, Option<String>>(11)?,
                            r.get(12)?,
                        ))
                    },
                )
                .optional()
                .map_err(StorageError::Sqlite)?;

            let Some((
                content,
                mtype,
                state,
                conf,
                imp,
                src,
                ws,
                wf,
                wsm,
                ac,
                stale,
                superseded,
                _sev,
            )) = base
            else {
                return Ok(None);
            };

            let derived_from = collect_ids(
                conn,
                "SELECT child_id FROM memory_derived_from WHERE parent_id = ?1",
                &id,
            )?;
            let contradicts = collect_ids(
                conn,
                "SELECT b_id FROM memory_contradicts WHERE a_id = ?1",
                &id,
            )?;

            Ok(Some(MemoryExplanation {
                id,
                content,
                memory_type: mtype,
                state,
                confidence: conf as f32,
                importance: imp as f32,
                source_event_tag: Some(src),
                derived_from,
                contradicts,
                worth_success: ws,
                worth_failure: wf,
                worth_samples: wsm,
                access_count: ac.max(0) as u64,
                staleness_class: stale,
                superseded_by: superseded.and_then(|s| Uuid::parse_str(&s).ok()),
            }))
        })
    }

    /// Build the aggregate health report (design §28).
    pub fn health_report(&self) -> MemoryResult<MemoryHealthReport> {
        self.db.with_read(|conn| {
            let count = |sql: &str| -> Result<i64, StorageError> {
                conn.query_row(sql, [], |r| r.get(0)).map_err(StorageError::Sqlite)
            };
            let mut report = MemoryHealthReport {
                total_active: count("SELECT COUNT(*) FROM memories WHERE state='active'")?,
                total_archived: count("SELECT COUNT(*) FROM memories WHERE state='archived'")?,
                total_superseded: count("SELECT COUNT(*) FROM memories WHERE state='superseded'")?,
                total_forgotten: count("SELECT COUNT(*) FROM memories WHERE state='forgotten'")?,
                unresolved_contradictions: count("SELECT COUNT(*) FROM memory_contradicts")?,
                knowledge_gaps: count("SELECT COUNT(*) FROM knowledge_gaps WHERE resolved=0")?,
                enrichment_backlog: count("SELECT COUNT(*) FROM enrichment_deadletter")?,
                outbox_pending: count("SELECT COUNT(*) FROM embedding_outbox WHERE status='pending'")?,
                avg_confidence: conn
                    .query_row(
                        "SELECT COALESCE(AVG(confidence),0) FROM memories WHERE state='active'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(StorageError::Sqlite)?,
                by_type: Vec::new(),
                by_staleness: Vec::new(),
            };

            report.by_type = group_counts(
                conn,
                "SELECT memory_type, COUNT(*) FROM memories WHERE state='active' GROUP BY memory_type",
            )?;
            report.by_staleness = group_counts(
                conn,
                "SELECT staleness_class, COUNT(*) FROM memories WHERE state='active' GROUP BY staleness_class",
            )?;
            Ok(report)
        })
    }
}

fn collect_ids(
    conn: &rusqlite::Connection,
    sql: &str,
    key: &Uuid,
) -> Result<Vec<Uuid>, StorageError> {
    let mut stmt = conn.prepare(sql).map_err(StorageError::Sqlite)?;
    let rows = stmt
        .query_map(params![key.to_string()], |r| r.get::<_, String>(0))
        .map_err(StorageError::Sqlite)?;
    let mut out = Vec::new();
    for r in rows {
        if let Ok(u) = Uuid::parse_str(&r.map_err(StorageError::Sqlite)?) {
            out.push(u);
        }
    }
    Ok(out)
}

fn group_counts(
    conn: &rusqlite::Connection,
    sql: &str,
) -> Result<Vec<(String, i64)>, StorageError> {
    let mut stmt = conn.prepare(sql).map_err(StorageError::Sqlite)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(StorageError::Sqlite)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(StorageError::Sqlite)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::stores::ports::{EventStore, RelationalStore};
    use crate::memory::stores::{SqliteEventStore, SqliteRelationalStore};
    use crate::memory::types::{
        Event, EventType, Memory, MemoryState, MemoryType, MemoryWorth, Modality, Scope,
        Sensitivity, Source, StalenessClass,
    };

    fn seed(db: &Arc<Database>) -> Uuid {
        let events = SqliteEventStore::new(db.clone());
        let rel = SqliteRelationalStore::new(db.clone());
        let ev = Event {
            id: crate::memory::ids::new_id(),
            hlc: crate::memory::ids::HlcGenerator::new().now(),
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
            id: crate::memory::ids::new_id(),
            content: "the user prefers dark mode".into(),
            memory_type: MemoryType::Semantic,
            compression_level: 0,
            source_event_id: ev.id,
            namespace: "core".into(),
            owner_id: "user".into(),
            device_id: "d".into(),
            scope: Scope::Global,
            confidence: 0.9,
            importance: 6.0,
            access_count: 3,
            decay_score: 1.0,
            staleness_class: StalenessClass::Slow,
            sensitivity: Sensitivity::Private,
            state: MemoryState::Active,
            created_at: now,
            last_accessed: Some(now),
            valid_from: now,
            valid_until: None,
            embedding_id: None,
            embedding_model_version: None,
            estimated_tokens: 3,
            content_hash: "h".into(),
            shred_key_id: None,
            verify_against: None,
            superseded_by: None,
            episode_id: None,
            goal_context_id: None,
            worth: MemoryWorth {
                success: 5,
                failure: 1,
                samples: 6,
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
    fn explain_memory_returns_provenance() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let id = seed(&db);
        let obs = Observability::new(db.clone());
        let e = obs.explain_memory(id).unwrap().unwrap();
        assert_eq!(e.memory_type, "semantic");
        assert_eq!(e.state, "active");
        assert_eq!(e.worth_samples, 6);
        assert_eq!(e.source_event_tag.as_deref(), Some("user"));
        assert!(obs.explain_memory(Uuid::now_v7()).unwrap().is_none());
    }

    #[test]
    fn health_report_aggregates() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        seed(&db);
        let obs = Observability::new(db.clone());
        let r = obs.health_report().unwrap();
        assert_eq!(r.total_active, 1);
        assert!(r.avg_confidence > 0.8);
        assert_eq!(r.by_type, vec![("semantic".to_string(), 1)]);
        assert_eq!(r.knowledge_gaps, 0);
    }
}
