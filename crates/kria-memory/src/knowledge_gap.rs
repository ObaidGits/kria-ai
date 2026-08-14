//! Knowledge Gap Engine (memory-upgrade design §36 / task 30).
//!
//! Records "what KRIA doesn't know" — queries that returned nothing useful — so
//! gaps surface in the health report and can feed Research-mode proactive
//! extraction.
//!
//! **Pending F1.5/F2 governed-writer cutover.** [`CommandCandidate::knowledge_gap`](
//! crate::authority::CommandCandidate::knowledge_gap) is the typed
//! command-candidate scaffolding (task F1.5.1) this engine's gap writes will
//! route through once a concrete `TxSemanticStore` builder persists the
//! knowledge-gap semantic row (F2). This engine remains the live persistence
//! path until then — see the ledger in [`crate::model::legacy_mapping`].

use std::sync::Arc;

use rusqlite::params;
use uuid::Uuid;

use crate::db::Database;
use crate::error::{MemoryResult, StorageError};
use crate::ids::{new_id, normalized_content_hash};

/// A recorded knowledge gap.
#[derive(Clone, Debug, PartialEq)]
pub struct KnowledgeGap {
    pub id: Uuid,
    pub query: String,
    pub domain: Option<String>,
    pub times_missed: u32,
    pub resolved: bool,
}

pub struct KnowledgeGapEngine {
    db: Arc<Database>,
}

impl KnowledgeGapEngine {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Record a missed query. Repeated misses of the same normalized query
    /// increment `times_missed` (a deterministic id keyed on the normalized query).
    pub fn record_miss(&self, query: &str, domain: Option<&str>) -> MemoryResult<()> {
        // Deterministic id from the normalized query so repeats collapse.
        let gap_id = uuid_from_hash(&normalized_content_hash(query));
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO knowledge_gaps(id, query, domain, times_missed, last_missed_at, resolved) \
                 VALUES(?1,?2,?3,1,?4,0) \
                 ON CONFLICT(id) DO UPDATE SET times_missed = times_missed + 1, \
                 last_missed_at = excluded.last_missed_at",
                params![
                    gap_id.to_string(),
                    query,
                    domain,
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// Mark a gap resolved (e.g. after the knowledge was learned).
    pub fn resolve(&self, query: &str) -> MemoryResult<()> {
        let gap_id = uuid_from_hash(&normalized_content_hash(query));
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "UPDATE knowledge_gaps SET resolved = 1 WHERE id = ?1",
                params![gap_id.to_string()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// Top unresolved gaps by miss count (for the health report / Research mode).
    pub fn top_gaps(&self, limit: usize) -> MemoryResult<Vec<KnowledgeGap>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, query, domain, times_missed, resolved FROM knowledge_gaps \
                     WHERE resolved = 0 ORDER BY times_missed DESC LIMIT ?1",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![limit as i64], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                })
                .map_err(StorageError::Sqlite)?;
            let mut out = Vec::new();
            for row in rows {
                let (id, query, domain, times, resolved) = row.map_err(StorageError::Sqlite)?;
                out.push(KnowledgeGap {
                    id: Uuid::parse_str(&id).unwrap_or_else(|_| new_id()),
                    query,
                    domain,
                    times_missed: times.max(0) as u32,
                    resolved: resolved != 0,
                });
            }
            Ok(out)
        })
    }
}

/// Derive a stable UUID from a hex hash (first 16 bytes).
fn uuid_from_hash(hex: &str) -> Uuid {
    let mut bytes = [0u8; 16];
    let h = hex.as_bytes();
    for (i, b) in bytes.iter_mut().enumerate() {
        let hi = h.get(i * 2).copied().unwrap_or(b'0');
        let lo = h.get(i * 2 + 1).copied().unwrap_or(b'0');
        *b = (hexval(hi) << 4) | hexval(lo);
    }
    Uuid::from_bytes(bytes)
}
fn hexval(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_miss_increments() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let kg = KnowledgeGapEngine::new(db.clone());
        kg.record_miss("what is the user's cat's name", Some("personal"))
            .unwrap();
        kg.record_miss("what is the user's cat's name", Some("personal"))
            .unwrap();
        let gaps = kg.top_gaps(10).unwrap();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].times_missed, 2);

        kg.resolve("what is the user's cat's name").unwrap();
        assert!(kg.top_gaps(10).unwrap().is_empty());
    }
}
