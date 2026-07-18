//! Research-level cognitive memory (memory-upgrade Phase 2, Priority C).
//!
//! Advanced retrieval/analytics over the EXISTING authority substrate — no new
//! storage engines:
//!
//! * **Temporal memory** — timeline reconstruction and time-bounded retrieval
//!   over the `memories` table (`created_at`), plus recency-weighted decay.
//! * **Meta-memory** — memory about memories: confidence / worth distribution
//!   and evolution across the store.
//! * **Uncertainty & confidence propagation** — pure combinators for carrying
//!   confidence through multi-hop reasoning and accumulating evidence
//!   (noisy-OR), used by the reasoner and graph traversal.

use std::sync::Arc;

use rusqlite::params;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};

/// A timeline entry (temporal reconstruction).
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineEntry {
    pub id: String,
    pub content: String,
    pub memory_type: String,
    pub created_at: String,
    pub confidence: f64,
}

/// Meta-memory: aggregate statistics about the memory store itself.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MetaMemory {
    pub active: i64,
    pub archived: i64,
    pub superseded: i64,
    pub avg_confidence: f64,
    pub avg_worth: f64,
}

/// Research-memory engine over the authority database.
#[derive(Clone)]
pub struct ResearchMemory {
    db: Arc<Database>,
}

impl ResearchMemory {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Reconstruct the most recent `limit` active memories in chronological
    /// order (temporal memory / historical reconstruction).
    pub fn timeline(&self, limit: usize) -> MemoryResult<Vec<TimelineEntry>> {
        let mut rows = self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, content, memory_type, created_at, confidence FROM memories \
                     WHERE state = 'active' ORDER BY created_at DESC LIMIT ?1",
                )
                .map_err(StorageError::Sqlite)?;
            let out = stmt
                .query_map(params![limit as i64], |r| {
                    Ok(TimelineEntry {
                        id: r.get(0)?,
                        content: r.get(1)?,
                        memory_type: r.get(2)?,
                        created_at: r.get(3)?,
                        confidence: r.get(4)?,
                    })
                })
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(out)
        })?;
        rows.reverse(); // chronological (oldest first)
        Ok(rows)
    }

    /// Active memories created within `[since_rfc3339, until_rfc3339]`
    /// (time-bounded temporal retrieval), chronological.
    pub fn in_window(
        &self,
        since_rfc3339: &str,
        until_rfc3339: &str,
    ) -> MemoryResult<Vec<TimelineEntry>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, content, memory_type, created_at, confidence FROM memories \
                     WHERE state = 'active' AND created_at >= ?1 AND created_at <= ?2 \
                     ORDER BY created_at ASC",
                )
                .map_err(StorageError::Sqlite)?;
            let out = stmt
                .query_map(params![since_rfc3339, until_rfc3339], |r| {
                    Ok(TimelineEntry {
                        id: r.get(0)?,
                        content: r.get(1)?,
                        memory_type: r.get(2)?,
                        created_at: r.get(3)?,
                        confidence: r.get(4)?,
                    })
                })
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(out)
        })
    }

    /// Meta-memory snapshot: state distribution + average confidence/worth.
    pub fn meta(&self) -> MemoryResult<MetaMemory> {
        self.db.with_read(|conn| {
            let mut m = MetaMemory::default();
            let mut stmt = conn
                .prepare("SELECT state, COUNT(*) FROM memories GROUP BY state")
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            for (state, count) in rows {
                match state.as_str() {
                    "active" | "promoted" => m.active += count,
                    "archived" => m.archived += count,
                    "superseded" => m.superseded += count,
                    _ => {}
                }
            }
            m.avg_confidence = conn
                .query_row(
                    "SELECT COALESCE(AVG(confidence),0.0) FROM memories WHERE state='active'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            // Average Laplace-smoothed worth over sampled memories.
            m.avg_worth = conn
                .query_row(
                    "SELECT COALESCE(AVG(CAST(memory_worth_success + 1 AS REAL) / \
                     CAST(memory_worth_success + memory_worth_failure + 2 AS REAL)), 0.5) \
                     FROM memories WHERE state='active'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            Ok(m)
        })
    }
}

/// Propagate confidence through a multi-hop reasoning/traversal chain. Chained
/// inference cannot increase certainty, so this is the product of per-hop
/// confidences with a small floor to avoid collapsing to zero on long chains
/// (uncertainty propagation). Empty chain → 1.0 (no inference, no loss).
pub fn propagate_confidence(chain: &[f64]) -> f64 {
    let mut c = 1.0f64;
    for &hop in chain {
        c *= hop.clamp(0.0, 1.0);
    }
    c.max(0.01)
}

/// Accumulate independent supporting evidence via noisy-OR: combined belief
/// `1 - Π(1 - eᵢ)`. More independent evidence increases confidence, but never
/// beyond 1.0 (evidence accumulation / confidence propagation).
pub fn combine_evidence(evidence: &[f64]) -> f64 {
    let mut not_any = 1.0f64;
    for &e in evidence {
        not_any *= 1.0 - e.clamp(0.0, 1.0);
    }
    (1.0 - not_any).clamp(0.0, 1.0)
}

/// Resolve two conflicting confidences from opposing sources into a net belief
/// in [0,1] (0.5 = balanced conflict). `support` argues for; `refute` against.
pub fn resolve_conflict(support: f64, refute: f64) -> f64 {
    let s = support.clamp(0.0, 1.0);
    let r = refute.clamp(0.0, 1.0);
    if s + r == 0.0 {
        return 0.5;
    }
    s / (s + r)
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
    use uuid::Uuid;

    fn seed(db: &Arc<Database>, content: &str, hash: &str, confidence: f32) {
        let events = SqliteEventStore::new(db.clone());
        let rel = SqliteRelationalStore::new(db.clone());
        let ev = Event {
            id: crate::memory::ids::new_id(),
            hlc: crate::memory::ids::HlcGenerator::new().now(),
            ts_utc: chrono::Utc::now(),
            tz_offset_min: 0,
            event_type: EventType::UserMessage,
            source: Source::User,
            session_id: Some(Uuid::now_v7()),
            parent_event_id: None,
            shred_key_id: None,
            payload: serde_json::json!({}),
            encrypted: false,
            checksum: "c".into(),
        };
        let now = chrono::Utc::now();
        let m = Memory {
            id: crate::memory::ids::new_id(),
            content: content.into(),
            memory_type: MemoryType::Semantic,
            compression_level: 0,
            source_event_id: ev.id,
            namespace: "core".into(),
            owner_id: "user".into(),
            device_id: "d".into(),
            scope: Scope::Global,
            confidence,
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
            shred_key_id: None,
            verify_against: None,
            superseded_by: None,
            episode_id: None,
            goal_context_id: None,
            worth: MemoryWorth::default(),
            modality: Modality::Text,
            preference_pair_id: None,
            training_eligible: false,
        };
        let mut tx = db.begin().unwrap();
        events.append(&mut tx, &ev).unwrap();
        rel.upsert_memory(&mut tx, &m).unwrap();
        tx.commit().unwrap();
    }

    #[test]
    fn timeline_and_meta() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        seed(&db, "first fact", "h1", 0.8);
        seed(&db, "second fact", "h2", 0.6);
        let rm = ResearchMemory::new(db.clone());
        let tl = rm.timeline(10).unwrap();
        assert_eq!(tl.len(), 2);
        let meta = rm.meta().unwrap();
        assert_eq!(meta.active, 2);
        assert!((meta.avg_confidence - 0.7).abs() < 0.05);
        assert!((meta.avg_worth - 0.5).abs() < 1e-6); // no worth samples yet
    }

    #[test]
    fn uncertainty_propagation_math() {
        // Product over hops, floored.
        assert!((propagate_confidence(&[0.9, 0.8, 0.5]) - 0.36).abs() < 1e-6);
        assert_eq!(propagate_confidence(&[]), 1.0);
        assert_eq!(propagate_confidence(&[0.0]), 0.01); // floor

        // Noisy-OR evidence accumulation.
        assert!((combine_evidence(&[0.5, 0.5]) - 0.75).abs() < 1e-6);
        assert_eq!(combine_evidence(&[]), 0.0);

        // Conflict resolution.
        assert!((resolve_conflict(0.8, 0.2) - 0.8).abs() < 1e-6);
        assert_eq!(resolve_conflict(0.0, 0.0), 0.5);
    }
}
