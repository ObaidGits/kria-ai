//! SQLite FTS5-backed [`SearchStore`] (memory-upgrade design §16, D-2).
//!
//! P1 keyword floor. Writes on the memory path happen **inside the authority
//! transaction** (design §47.1: the FTS row commits with the memory row, so FTS
//! is not an outbox target in P1) via [`index_fts_in_tx`]/[`delete_fts_in_tx`].
//! The async [`SearchStore`] trait serves queries, reconciliation deletes, and
//! `all_ids`; it becomes the out-of-txn write path only when Tantivy arrives (P2).

use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::db::{AuthorityTx, Database};
use crate::error::{MemoryResult, StorageError};
use crate::types::{ScopeFilter, SearchHit};

use super::ports::SearchStore;

/// Turn arbitrary user text into a safe FTS5 MATCH expression. Each alphanumeric
/// token is quoted (neutralizing FTS operators) and OR-joined for recall (this
/// is the keyword floor). Returns `None` when there is nothing to search.
pub fn fts5_query(raw: &str) -> Option<String> {
    let terms: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

/// Insert/replace an FTS row inside the authority transaction (P1 write path).
pub fn index_fts_in_tx(
    tx: &mut AuthorityTx<'_>,
    id: Uuid,
    content: &str,
    namespace: &str,
) -> MemoryResult<()> {
    // FTS5 has no UPSERT; delete-then-insert keeps it idempotent.
    tx.conn()
        .execute(
            "DELETE FROM memories_fts WHERE memory_id = ?1",
            params![id.to_string()],
        )
        .map_err(StorageError::Sqlite)?;
    tx.conn()
        .execute(
            "INSERT INTO memories_fts(content, memory_id, namespace) VALUES(?1,?2,?3)",
            params![content, id.to_string(), namespace],
        )
        .map_err(StorageError::Sqlite)?;
    Ok(())
}

/// Delete FTS rows inside the authority transaction (P1 delete/forget path).
pub fn delete_fts_in_tx(tx: &mut AuthorityTx<'_>, ids: &[Uuid]) -> MemoryResult<()> {
    for id in ids {
        tx.conn()
            .execute(
                "DELETE FROM memories_fts WHERE memory_id = ?1",
                params![id.to_string()],
            )
            .map_err(StorageError::Sqlite)?;
    }
    Ok(())
}

pub struct SqliteSearchStore {
    db: Arc<Database>,
}

impl SqliteSearchStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SearchStore for SqliteSearchStore {
    /// Out-of-txn index (reconciliation / rebuild). The P1 memory write path
    /// uses [`index_fts_in_tx`] instead so the FTS row commits with the memory.
    async fn index(&self, id: Uuid, content: &str, namespace: &str) -> MemoryResult<()> {
        let mut tx = self.db.begin()?;
        index_fts_in_tx(&mut tx, id, content, namespace)?;
        tx.commit()
    }

    async fn query(
        &self,
        query: &str,
        k: usize,
        filter: &ScopeFilter,
    ) -> MemoryResult<Vec<SearchHit>> {
        let Some(match_expr) = fts5_query(query) else {
            return Ok(Vec::new());
        };
        let ns_filter = filter.namespaces.clone();
        self.db.with_read(move |conn: &Connection| {
            // bm25() returns lower-is-better; negate so higher = better for fusion.
            let (sql, single_ns) = if ns_filter.len() == 1 {
                (
                    "SELECT memory_id, bm25(memories_fts) AS s FROM memories_fts \
                     WHERE memories_fts MATCH ?1 AND namespace = ?3 ORDER BY s ASC LIMIT ?2"
                        .to_string(),
                    Some(ns_filter[0].clone()),
                )
            } else {
                (
                    "SELECT memory_id, bm25(memories_fts) AS s FROM memories_fts \
                     WHERE memories_fts MATCH ?1 ORDER BY s ASC LIMIT ?2"
                        .to_string(),
                    None,
                )
            };
            let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
            let mapper = |r: &rusqlite::Row<'_>| -> rusqlite::Result<(String, f64)> {
                Ok((r.get(0)?, r.get(1)?))
            };
            let rows = if let Some(ns) = single_ns {
                stmt.query_map(params![match_expr, k as i64, ns], mapper)
            } else {
                stmt.query_map(params![match_expr, k as i64], mapper)
            }
            .map_err(StorageError::Sqlite)?;
            let mut hits = Vec::new();
            for row in rows {
                let (id, bm25) = row.map_err(StorageError::Sqlite)?;
                hits.push(SearchHit {
                    id: Uuid::parse_str(&id)
                        .map_err(|e| StorageError::Serde(format!("bad uuid: {e}")))?,
                    score: -(bm25 as f32),
                });
            }
            Ok(hits)
        })
    }

    async fn delete(&self, ids: &[Uuid]) -> MemoryResult<()> {
        let mut tx = self.db.begin()?;
        delete_fts_in_tx(&mut tx, ids)?;
        tx.commit()
    }

    async fn all_ids(&self) -> MemoryResult<Vec<Uuid>> {
        self.db.with_read(|conn: &Connection| {
            let mut stmt = conn
                .prepare("SELECT memory_id FROM memories_fts")
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(StorageError::Sqlite)?;
            let mut out = Vec::new();
            for r in rows {
                let s = r.map_err(StorageError::Sqlite)?;
                out.push(
                    Uuid::parse_str(&s)
                        .map_err(|e| StorageError::Serde(format!("bad uuid: {e}")))?,
                );
            }
            Ok(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizer_quotes_and_drops_operators() {
        assert_eq!(
            fts5_query("dark mode"),
            Some("\"dark\" OR \"mode\"".to_string())
        );
        assert_eq!(fts5_query("  \"*(^ "), None);
        // An FTS operator like NEAR is neutralized by quoting.
        assert_eq!(
            fts5_query("a NEAR b"),
            Some("\"a\" OR \"NEAR\" OR \"b\"".to_string())
        );
    }

    #[tokio::test]
    async fn index_query_delete_roundtrip() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let ss = SqliteSearchStore::new(db.clone());
        let id1 = Uuid::now_v7();
        let id2 = Uuid::now_v7();
        ss.index(id1, "the user prefers dark mode themes", "core")
            .await
            .unwrap();
        ss.index(id2, "kria runs entirely on the local laptop", "core")
            .await
            .unwrap();

        let hits = ss.query("dark", 10, &ScopeFilter::default()).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, id1);

        let all = ss.all_ids().await.unwrap();
        assert_eq!(all.len(), 2);

        ss.delete(&[id1]).await.unwrap();
        let hits = ss.query("dark", 10, &ScopeFilter::default()).await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn namespace_filter_scopes_results() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let ss = SqliteSearchStore::new(db.clone());
        ss.index(Uuid::now_v7(), "shared knowledge fact", "core")
            .await
            .unwrap();
        ss.index(Uuid::now_v7(), "shared knowledge fact", "plugin/x")
            .await
            .unwrap();
        let filter = ScopeFilter {
            namespaces: vec!["plugin/x".into()],
            ..Default::default()
        };
        let hits = ss.query("knowledge", 10, &filter).await.unwrap();
        assert_eq!(hits.len(), 1);
    }
}
