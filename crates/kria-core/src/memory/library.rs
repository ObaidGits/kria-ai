//! Library manager (memory-upgrade design §8.8/§14, task 31).
//!
//! A personal knowledge library separate from experiential memory (documents
//! don't decay). Streamed/dedup'd ingestion, adaptive chunking, per-item
//! provenance (`library:{item}:chunk:{idx}`), versioning, and per-item cascade
//! delete. Extracted facts flow through the Write Policy like any other write
//! (L3); this module owns the item/chunk records + filesystem originals.
//!
//! **Pending F1.5/F2 governed-writer cutover.** [`CommandCandidate::library_chunk`](
//! crate::memory::authority::CommandCandidate::library_chunk) is the typed
//! command-candidate scaffolding (task F1.5.1) chunk ingestion will route
//! through once a concrete `TxSemanticStore` builder persists a `records` v2
//! row per chunk (F2). Item/chunk bookkeeping here (`library_items`/
//! `library_chunks`, SHA dedup, versioning, cascade delete) remains the live
//! path until then — see the ledger in [`crate::memory::model::legacy_mapping`].

use std::sync::Arc;

use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::ids::new_id;

/// Adaptive chunk sizes (design §14): dense text vs code.
const CHUNK_TEXT: usize = 512;
const CHUNK_CODE: usize = 1024;

/// A library item record.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryItem {
    pub id: Uuid,
    pub sha256: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub version: u32,
    pub prev_version_id: Option<Uuid>,
    pub path: String,
}

/// A chunk of a library item.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryChunk {
    pub id: Uuid,
    pub item_id: Uuid,
    pub chunk_index: u32,
    pub text: String,
}

/// Split `content` into chunks. Code (heuristic: has `{`/`;`/`fn `/`def `) uses a
/// larger window; prose uses the smaller one. Splits on char boundaries.
pub fn adaptive_chunk(content: &str) -> Vec<String> {
    let is_code = content.contains("fn ")
        || content.contains("def ")
        || content.contains(';')
        || content.contains('{');
    let size = if is_code { CHUNK_CODE } else { CHUNK_TEXT };
    let chars: Vec<char> = content.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    chars
        .chunks(size)
        .map(|c| c.iter().collect::<String>())
        .collect()
}

/// Decode a stored UUID string from row column `col`, surfacing corruption as a
/// hard error rather than silently fabricating a fresh id (integrity — never
/// mask a corrupted `library_items.id`, which would break delete/cascade by id).
fn row_uuid(s: String, col: usize) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(&s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, Box::new(e))
    })
}

/// SHA-256 hex of bytes (exact-dedup key).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

pub struct Library {
    db: Arc<Database>,
}

impl Library {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Ingest a document. Content is provided as text (large-file streaming is a
    /// caller concern; the chunker works on a slice). Returns the item id and
    /// its chunk count. Exact duplicates (by SHA-256) are not re-ingested.
    pub fn ingest(
        &self,
        title: Option<&str>,
        author: Option<&str>,
        path: &str,
        content: &str,
    ) -> MemoryResult<(Uuid, usize, bool)> {
        let sha = sha256_hex(content.as_bytes());

        // Exact-dedup: return the existing item if the same bytes were ingested.
        // `false` = not newly created (callers skip re-submitting chunks).
        if let Some(existing) = self.find_by_sha(&sha)? {
            let n = self.chunk_count(existing)?;
            return Ok((existing, n, false));
        }

        let chunks = adaptive_chunk(content);
        let item_id = new_id();
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO library_items(id, sha256, title, author, version, prev_version_id, \
                 path, ingested_at) VALUES(?1,?2,?3,?4,1,NULL,?5,?6)",
                params![
                    item_id.to_string(),
                    sha,
                    title,
                    author,
                    path,
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .map_err(StorageError::Sqlite)?;
        for (idx, text) in chunks.iter().enumerate() {
            tx.conn()
                .execute(
                    "INSERT INTO library_chunks(id, item_id, chunk_index, text, modality) \
                     VALUES(?1,?2,?3,?4,'text')",
                    params![new_id().to_string(), item_id.to_string(), idx as i64, text],
                )
                .map_err(StorageError::Sqlite)?;
        }
        tx.commit()?;
        Ok((item_id, chunks.len(), true))
    }

    /// Ingest a new version of an existing item, linked to the previous version
    /// (never loses the old version — design §14).
    pub fn ingest_version(
        &self,
        prev_item_id: Uuid,
        path: &str,
        content: &str,
    ) -> MemoryResult<Uuid> {
        let prev = self
            .get_item(prev_item_id)?
            .ok_or_else(|| StorageError::Serde("prev item not found".into()))?;
        let sha = sha256_hex(content.as_bytes());
        let chunks = adaptive_chunk(content);
        let item_id = new_id();
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO library_items(id, sha256, title, author, version, prev_version_id, \
                 path, ingested_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    item_id.to_string(),
                    sha,
                    prev.title,
                    prev.author,
                    (prev.version + 1) as i64,
                    prev_item_id.to_string(),
                    path,
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .map_err(StorageError::Sqlite)?;
        for (idx, text) in chunks.iter().enumerate() {
            tx.conn()
                .execute(
                    "INSERT INTO library_chunks(id, item_id, chunk_index, text, modality) \
                     VALUES(?1,?2,?3,?4,'text')",
                    params![new_id().to_string(), item_id.to_string(), idx as i64, text],
                )
                .map_err(StorageError::Sqlite)?;
        }
        tx.commit()?;
        Ok(item_id)
    }

    /// Per-item cascade delete: chunks + the item (design R8). Memories sourced
    /// from this item are handled by `Lifecycle::hard_delete` with a
    /// `SourcePrefix("library:{item}")` scope.
    pub fn delete_item(&self, item_id: Uuid) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "DELETE FROM library_chunks WHERE item_id = ?1",
                params![item_id.to_string()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.conn()
            .execute(
                "DELETE FROM library_items WHERE id = ?1",
                params![item_id.to_string()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// Provenance tag for a chunk (design §46: `library:{item}:chunk:{idx}`).
    pub fn provenance_tag(item_id: Uuid, chunk_index: u32) -> String {
        format!("library:{item_id}:chunk:{chunk_index}")
    }

    fn find_by_sha(&self, sha: &str) -> MemoryResult<Option<Uuid>> {
        self.db.with_read(|conn| {
            let id: Option<String> = conn
                .query_row(
                    "SELECT id FROM library_items WHERE sha256 = ?1 ORDER BY version DESC LIMIT 1",
                    params![sha],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StorageError::Sqlite)?;
            Ok(id.and_then(|s| Uuid::parse_str(&s).ok()))
        })
    }

    fn chunk_count(&self, item_id: Uuid) -> MemoryResult<usize> {
        self.db.with_read(|conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM library_chunks WHERE item_id = ?1",
                    params![item_id.to_string()],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            Ok(n.max(0) as usize)
        })
    }

    /// List all library items (latest first) with their chunk counts — backs the
    /// `list_knowledge_base` tool + the Memory UI Library view.
    pub fn list_items(&self) -> MemoryResult<Vec<(LibraryItem, usize)>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT i.id, i.sha256, i.title, i.author, i.version, i.prev_version_id, \
                     i.path, (SELECT COUNT(*) FROM library_chunks c WHERE c.item_id = i.id) \
                     FROM library_items i ORDER BY i.ingested_at DESC",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map([], |r| {
                    let item = LibraryItem {
                        id: row_uuid(r.get::<_, String>(0)?, 0)?,
                        sha256: r.get(1)?,
                        title: r.get(2)?,
                        author: r.get(3)?,
                        version: r.get::<_, i64>(4)?.max(0) as u32,
                        prev_version_id: r
                            .get::<_, Option<String>>(5)?
                            .and_then(|s| Uuid::parse_str(&s).ok()),
                        path: r.get(6)?,
                    };
                    let chunks = r.get::<_, i64>(7)?.max(0) as usize;
                    Ok((item, chunks))
                })
                .map_err(StorageError::Sqlite)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(StorageError::Sqlite)?);
            }
            Ok(out)
        })
    }

    /// Fetch an item record.
    pub fn get_item(&self, item_id: Uuid) -> MemoryResult<Option<LibraryItem>> {
        self.db.with_read(|conn| {
            conn.query_row(
                "SELECT id, sha256, title, author, version, prev_version_id, path \
                 FROM library_items WHERE id = ?1",
                params![item_id.to_string()],
                |r| {
                    Ok(LibraryItem {
                        id: row_uuid(r.get::<_, String>(0)?, 0)?,
                        sha256: r.get(1)?,
                        title: r.get(2)?,
                        author: r.get(3)?,
                        version: r.get::<_, i64>(4)?.max(0) as u32,
                        prev_version_id: r
                            .get::<_, Option<String>>(5)?
                            .and_then(|s| Uuid::parse_str(&s).ok()),
                        path: r.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(StorageError::Sqlite)
            .map_err(Into::into)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_chunking_sizes() {
        let prose = "a".repeat(1300);
        assert_eq!(adaptive_chunk(&prose).len(), 3); // 512 * 3 covers 1300
        let code = format!("fn main() {{}}\n{}", "x".repeat(2100));
        // code uses 1024 window → ceil(~2113/1024) = 3
        assert_eq!(adaptive_chunk(&code).len(), 3);
        assert!(adaptive_chunk("").is_empty());
    }

    #[test]
    fn ingest_dedups_and_versions() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let lib = Library::new(db.clone());
        let (id1, n1, created1) = lib
            .ingest(Some("Doc"), None, "/docs/a.md", "hello world content")
            .unwrap();
        assert!(n1 >= 1);
        assert!(created1, "first ingest creates the item");
        // Same content → dedup to the same item (not newly created).
        let (id2, _, created2) = lib
            .ingest(Some("Doc"), None, "/docs/a.md", "hello world content")
            .unwrap();
        assert_eq!(id1, id2);
        assert!(!created2, "re-ingest of identical bytes dedups");

        // New version.
        let v2 = lib
            .ingest_version(id1, "/docs/a.md", "hello world content v2")
            .unwrap();
        let item = lib.get_item(v2).unwrap().unwrap();
        assert_eq!(item.version, 2);
        assert_eq!(item.prev_version_id, Some(id1));
    }

    #[test]
    fn corrupt_item_id_surfaces_error_not_fabricated() {
        // A corrupted library_items.id must error on read, never be silently
        // replaced with a fresh uuid (which would break delete/cascade by id).
        let db = Arc::new(Database::open_in_memory().unwrap());
        let lib = Library::new(db.clone());
        // Insert an item row with a corrupt (non-uuid) id directly (no chunks →
        // no FK dependents), simulating on-disk corruption.
        db.write()
            .execute(
                "INSERT INTO library_items(id, sha256, title, author, version, prev_version_id, \
                 path, ingested_at) VALUES('not-a-uuid','deadbeef',NULL,NULL,1,NULL,'/x',?1)",
                params![chrono::Utc::now().to_rfc3339()],
            )
            .unwrap();
        assert!(
            lib.list_items().is_err(),
            "corrupt id must surface an error"
        );
    }

    #[test]
    fn per_item_cascade_delete() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let lib = Library::new(db.clone());
        let (id, _, _) = lib
            .ingest(None, None, "/x", "some document text to chunk")
            .unwrap();
        lib.delete_item(id).unwrap();
        assert!(lib.get_item(id).unwrap().is_none());
        let chunks: i64 = db
            .with_read(|c| {
                Ok(
                    c.query_row("SELECT COUNT(*) FROM library_chunks", [], |r| r.get(0))
                        .map_err(StorageError::Sqlite)?,
                )
            })
            .unwrap();
        assert_eq!(chunks, 0);
    }

    #[test]
    fn provenance_tag_format() {
        let id = Uuid::now_v7();
        assert_eq!(
            Library::provenance_tag(id, 3),
            format!("library:{id}:chunk:3")
        );
    }
}
