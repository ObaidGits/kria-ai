//! FTS5 delete/reconcile/temp-generation rebuild and membership hash for the
//! `search_documents` / `search_documents_fts` projection pair (F3.2 / task 3.2.5).
//!
//! ## Key functions
//! * [`compute_fts_membership_hash`] — deterministic SHA-256 over sorted
//!   `(record_kind, record_id)` tuples in `search_documents` (mirrors the
//!   vector partition's `compute_membership_hash` contract in §4.4).
//! * [`reconcile_fts_index`] — detect and repair missing / orphan FTS5 entries
//!   without a full rebuild.
//! * [`rebuild_fts_from_stream`] — temp-generation rebuild: clear, re-populate
//!   from an authority record stream, run `FTS5 'rebuild'`, write a
//!   `derived_manifests` row.
//! * [`rebuild_fts_for_kind`] — delete all `search_documents` rows for a single
//!   `record_kind` (FTS5 DELETE triggers keep the index in sync automatically).

use rusqlite::params;
use sha2::{Digest, Sha256};

use crate::db::Database;
use crate::error::{MemoryResult, StorageError};
use crate::stores::sqlite_search_documents::{upsert_search_document, SearchDocument};

// ─── compute_fts_membership_hash ─────────────────────────────────────────────

/// Compute the deterministic SHA-256 membership hash for `search_documents`.
///
/// Queries every `(record_kind || ':' || record_id)` tuple ordered by
/// `record_kind ASC, record_id ASC`, concatenates with `'\n'` as delimiter,
/// and returns the hex SHA-256 digest.
///
/// Empty table → SHA-256 of the empty byte string:
/// `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
pub fn compute_fts_membership_hash(conn: &rusqlite::Connection) -> MemoryResult<String> {
    let mut stmt = conn
        .prepare(
            "SELECT (record_kind || ':' || record_id) \
             FROM search_documents \
             ORDER BY record_kind ASC, record_id ASC",
        )
        .map_err(StorageError::Sqlite)?;

    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(StorageError::Sqlite)?;

    let mut hasher = Sha256::new();
    let mut first = true;
    for row in rows {
        let key = row.map_err(StorageError::Sqlite)?;
        if !first {
            hasher.update(b"\n");
        }
        hasher.update(key.as_bytes());
        first = false;
    }

    Ok(hex::encode(hasher.finalize()))
}

// ─── ReconciliationReport ─────────────────────────────────────────────────────

/// Summary of an FTS reconciliation pass performed by [`reconcile_fts_index`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconciliationReport {
    /// Rows present in `search_documents` that had no corresponding FTS5 entry
    /// before reconciliation.
    pub missing_fts_rows: usize,
    /// FTS5 index entries that pointed to non-existent `search_documents` rows
    /// before reconciliation (cleaned by FTS5 `'rebuild'` when orphans exist).
    pub orphan_fts_rows: usize,
    /// Number of `search_documents` rows whose FTS entry was re-populated.
    pub repopulated: usize,
    /// Number of orphan FTS entries cleaned (by triggering a full FTS rebuild).
    pub cleaned: usize,
}

// ─── reconcile_fts_index ─────────────────────────────────────────────────────

/// Detect and repair inconsistencies between `search_documents` and the
/// external-content FTS5 index `search_documents_fts`.
///
/// ## What is reconciled
/// For an external-content FTS5 table, the *text content* is stored in
/// `search_documents`; the FTS5 index stores only term→rowid mappings.
/// Inconsistencies arise when rows are inserted into `search_documents`
/// *bypassing* the FTS5 triggers (e.g. direct SQL during a bulk import or
/// a repair path).
///
/// 1. **Missing FTS rows** — rowids in `search_documents` that have no
///    matching entry in `search_documents_fts(rowid)`.  These are re-indexed
///    by running `INSERT INTO search_documents_fts(search_documents_fts)
///    VALUES('rebuild')` which reconstructs the full index from the content
///    table.
/// 2. **Orphan FTS rows** — FTS5 entries whose rowid no longer exists in
///    `search_documents`.  These are also cleaned by the `'rebuild'` command
///    since it rebuilds from the authoritative content table, discarding any
///    stale entries.
///
/// ## Notes on the rebuild command
/// For FTS5 external-content tables, `INSERT INTO t(t) VALUES('rebuild')` is
/// the canonical SQLite mechanism for reconstructing the entire inverted index
/// from the content table.  It is idempotent and safe to run at any time.
pub fn reconcile_fts_index(conn: &rusqlite::Connection) -> MemoryResult<ReconciliationReport> {
    // Count total rows in search_documents (the authoritative content table).
    let sd_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM search_documents", [], |r| r.get(0))
        .map_err(StorageError::Sqlite)?;

    // Count FTS5 index entries via the `search_documents_fts_docsize` shadow
    // table, which stores one row per indexed document (rowid → byte-size
    // of each indexed column). This is the actual FTS inverted-index entry
    // count, independent of the content table, so it correctly reflects the
    // FTS index state even when rows have been deleted from the index using
    // the FTS5 delete protocol without removing them from the content table.
    let fts_indexed_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM search_documents_fts_docsize",
            [],
            |r| r.get(0),
        )
        .map_err(StorageError::Sqlite)?;

    // Determine divergence.
    let missing_fts_rows = if sd_count > fts_indexed_count {
        (sd_count - fts_indexed_count) as usize
    } else {
        0
    };
    let orphan_fts_rows = if fts_indexed_count > sd_count {
        (fts_indexed_count - sd_count) as usize
    } else {
        0
    };

    let needs_rebuild = missing_fts_rows > 0 || orphan_fts_rows > 0;

    if needs_rebuild {
        // Reconstruct the entire FTS5 inverted index from search_documents.
        conn.execute_batch(
            "INSERT INTO search_documents_fts(search_documents_fts) VALUES('rebuild');",
        )
        .map_err(StorageError::Sqlite)?;
    }

    Ok(ReconciliationReport {
        missing_fts_rows,
        orphan_fts_rows,
        repopulated: if needs_rebuild { sd_count as usize } else { 0 },
        cleaned: if needs_rebuild { orphan_fts_rows } else { 0 },
    })
}

// ─── FtsRebuildOutcome ────────────────────────────────────────────────────────

/// Result of a [`rebuild_fts_from_stream`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FtsRebuildOutcome {
    /// Rebuild completed and the FTS5 index was atomically reconstructed.
    Activated {
        /// Number of rows in `search_documents` after the rebuild.
        member_count: i64,
        /// Deterministic SHA-256 membership hash over all `(record_kind, record_id)` tuples.
        membership_hash: String,
    },
    /// Rebuild was interrupted mid-stream (the record iterator returned an error).
    /// The last successfully upserted `"record_kind:record_id"` composite key, if any.
    Interrupted { last_kind_id: Option<String> },
}

// ─── FtsRebuildRecord ─────────────────────────────────────────────────────────

/// A single record ready to be upserted into `search_documents` during an FTS
/// stream rebuild.  Mirrors [`RebuildRecord`] in `sqlite_vector_rebuild.rs` but
/// carries all [`SearchDocument`] fields instead of a vector blob.
pub struct FtsRebuildRecord {
    pub record_kind: String,
    pub record_id: String,
    pub title: Option<String>,
    pub body: Option<String>,
    pub aliases: Option<String>,
    pub source_text: Option<String>,
    pub relation_text: Option<String>,
    pub namespace: String,
    pub owner_id: String,
    pub scope: String,
    pub sensitivity: i64,
    pub truth_state: String,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub content_hash: String,
    pub revision: i64,
}

impl FtsRebuildRecord {
    /// Convert this record into a [`SearchDocument`] for upsert.
    fn into_search_document(self) -> SearchDocument {
        SearchDocument {
            record_kind: self.record_kind,
            record_id: self.record_id,
            title: self.title,
            body: self.body,
            aliases: self.aliases,
            source_text: self.source_text,
            relation_text: self.relation_text,
            namespace: self.namespace,
            owner_id: self.owner_id,
            scope: self.scope,
            sensitivity: self.sensitivity,
            truth_state: self.truth_state,
            valid_from: self.valid_from,
            valid_until: self.valid_until,
            content_hash: self.content_hash,
            revision: self.revision,
        }
    }
}

// ─── rebuild_fts_from_stream ──────────────────────────────────────────────────

/// Build the `search_documents` projection and reconstruct the FTS5 index from
/// an authority record stream.
///
/// ## Flow
/// 1. Delete all existing rows from `search_documents` (this fires the FTS5
///    DELETE triggers, removing all FTS entries cleanly first, or we can
///    rely on the final `'rebuild'` which will replace everything).
///    To avoid trigger overhead on large rebuilds, we DELETE + then rebuild.
/// 2. Upsert each record from `records` into `search_documents`.  On stream
///    error: return [`FtsRebuildOutcome::Interrupted`].
/// 3. After stream completion: run `INSERT INTO search_documents_fts(
///    search_documents_fts) VALUES('rebuild')` to reconstruct the FTS5
///    inverted index from `search_documents`.
/// 4. Compute the membership hash and member count.
/// 5. Write a `derived_manifests` row for `target='fts'`.
/// 6. Return [`FtsRebuildOutcome::Activated`].
///
/// ## Interruption
/// If `records` yields `Err(_)` mid-stream, the already-upserted rows remain
/// in `search_documents` (partially populated).  The caller may retry by
/// calling this function again with the full stream — `search_documents` will
/// be cleared and re-populated on the next attempt.
pub fn rebuild_fts_from_stream(
    db: &Database,
    authority_revision: Option<i64>,
    model_id: &str,
    records: impl Iterator<Item = MemoryResult<FtsRebuildRecord>>,
) -> MemoryResult<FtsRebuildOutcome> {
    // 1. Clear search_documents (triggers clean the FTS index entry by entry).
    //    For large corpuses this is fine; the final 'rebuild' will also fix any
    //    residual inconsistency.
    {
        let conn = db.write();
        conn.execute_batch("DELETE FROM search_documents;")
            .map_err(StorageError::Sqlite)?;
    }

    // 2. Stream records into search_documents via upsert.
    let mut last_kind_id: Option<String> = None;

    for record_result in records {
        let record = match record_result {
            Ok(r) => r,
            Err(_e) => {
                return Ok(FtsRebuildOutcome::Interrupted {
                    last_kind_id: last_kind_id.clone(),
                });
            }
        };

        let kind_id = format!("{}:{}", record.record_kind, record.record_id);
        let doc = record.into_search_document();

        {
            let conn = db.write();
            if let Err(_e) = upsert_search_document(&conn, &doc) {
                return Ok(FtsRebuildOutcome::Interrupted {
                    last_kind_id: last_kind_id.clone(),
                });
            }
        }

        last_kind_id = Some(kind_id);
    }

    // 3. Rebuild the FTS5 inverted index from the now-populated content table.
    {
        let conn = db.write();
        conn.execute_batch(
            "INSERT INTO search_documents_fts(search_documents_fts) VALUES('rebuild');",
        )
        .map_err(StorageError::Sqlite)?;
    }

    // 4. Compute membership hash and member count.
    let (membership_hash, member_count) = {
        let conn = db.write();
        let hash = compute_fts_membership_hash(&conn)?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM search_documents", [], |r| r.get(0))
            .map_err(StorageError::Sqlite)?;
        (hash, count)
    };

    // 5. Write derived_manifests row for target='fts'.
    {
        let conn = db.write();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR REPLACE INTO derived_manifests \
             (target, version, authority_revision, member_count, membership_hash, \
              algorithm_version, model_version, completed_at, status) \
             VALUES ('fts', 'fts5-v1', ?1, ?2, ?3, 'fts5-v1', ?4, ?5, 'active')",
            params![
                authority_revision,
                member_count,
                membership_hash,
                model_id,
                now,
            ],
        )
        .map_err(StorageError::Sqlite)?;
    }

    Ok(FtsRebuildOutcome::Activated {
        member_count,
        membership_hash,
    })
}

// ─── rebuild_fts_for_kind ─────────────────────────────────────────────────────

/// Delete all `search_documents` rows with the given `record_kind`.
///
/// The FTS5 DELETE triggers (`trg_sd_fts_delete`) fire automatically for each
/// deleted row, keeping `search_documents_fts` in sync without a manual FTS
/// operation.
///
/// This is a scoped purge — the caller then upserts replacement rows via
/// [`upsert_search_document`] to repopulate the kind.  Useful when a single
/// record kind needs a targeted re-projection (e.g. after an authority
/// migration that only touched `'entity'` rows).
///
/// Returns `Ok(())` whether or not any rows existed (idempotent).
pub fn rebuild_fts_for_kind(conn: &rusqlite::Connection, record_kind: &str) -> MemoryResult<()> {
    conn.execute(
        "DELETE FROM search_documents WHERE record_kind = ?1",
        params![record_kind],
    )
    .map_err(StorageError::Sqlite)?;
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::db::Database;
    use crate::error::StorageError;
    use crate::stores::sqlite_search_documents::{upsert_search_document, SearchDocument};

    /// SHA-256 of empty string.
    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb924\
         27ae41e4649b934ca495991b7852b855";

    fn open_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    /// Build a minimal [`SearchDocument`] for a given kind and id.
    fn make_doc(kind: &str, id: &str) -> SearchDocument {
        SearchDocument {
            record_kind: kind.to_string(),
            record_id: id.to_string(),
            title: Some(format!("{kind} title")),
            body: Some(format!("{kind} body text")),
            aliases: None,
            source_text: None,
            relation_text: None,
            namespace: "core".to_string(),
            owner_id: "user-001".to_string(),
            scope: "default".to_string(),
            sensitivity: 0,
            truth_state: "Current".to_string(),
            valid_from: None,
            valid_until: None,
            content_hash: format!("sha256-{kind}-{id}"),
            revision: 1,
        }
    }

    /// Build an [`FtsRebuildRecord`] from a kind and id.
    fn make_rebuild_record(kind: &str, id: &str) -> FtsRebuildRecord {
        FtsRebuildRecord {
            record_kind: kind.to_string(),
            record_id: id.to_string(),
            title: Some(format!("{kind} title {id}")),
            body: Some(format!("{kind} body content for {id}")),
            aliases: None,
            source_text: None,
            relation_text: None,
            namespace: "core".to_string(),
            owner_id: "user-001".to_string(),
            scope: "default".to_string(),
            sensitivity: 0,
            truth_state: "Current".to_string(),
            valid_from: None,
            valid_until: None,
            content_hash: format!("hash-{kind}-{id}"),
            revision: 1,
        }
    }

    // ── compute_fts_membership_hash ──────────────────────────────────────────

    #[test]
    fn membership_hash_empty_table_is_known_sha256() {
        let db = open_db();
        let conn = db.write();
        let hash = compute_fts_membership_hash(&conn).unwrap();
        assert_eq!(
            hash, EMPTY_SHA256,
            "empty search_documents must produce SHA-256 of empty string"
        );
    }

    #[test]
    fn membership_hash_one_row_is_non_empty() {
        let db = open_db();
        let conn = db.write();
        upsert_search_document(
            &conn,
            &make_doc("memory", "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        let hash = compute_fts_membership_hash(&conn).unwrap();
        assert_ne!(
            hash, EMPTY_SHA256,
            "one-row table must produce a non-empty-string hash"
        );
        assert_eq!(hash.len(), 64, "SHA-256 hex must be 64 chars");
    }

    #[test]
    fn membership_hash_is_order_independent() {
        // Two databases with the same rows inserted in different orders must
        // produce the same hash (ORDER BY record_kind, record_id ASC guarantees this).
        let db1 = open_db();
        let db2 = open_db();

        let rows = [
            ("memory", "cccccccc-cccc-cccc-cccc-cccccccccccc"),
            ("entity", "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
            ("summary", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        ];

        // db1: insert in c, a, b order.
        {
            let conn = db1.write();
            for (kind, id) in &rows {
                upsert_search_document(&conn, &make_doc(kind, id)).unwrap();
            }
        }
        // db2: insert in a, b, c order.
        {
            let conn = db2.write();
            for (kind, id) in rows.iter().rev() {
                upsert_search_document(&conn, &make_doc(kind, id)).unwrap();
            }
        }

        let hash1 = {
            let conn = db1.write();
            compute_fts_membership_hash(&conn).unwrap()
        };
        let hash2 = {
            let conn = db2.write();
            compute_fts_membership_hash(&conn).unwrap()
        };

        assert_eq!(hash1, hash2, "membership hash must be order-independent");
        assert_ne!(
            hash1, EMPTY_SHA256,
            "non-empty table must differ from empty hash"
        );
    }

    // ── reconcile_fts_index ──────────────────────────────────────────────────

    /// Insert a row into `search_documents` bypassing the FTS5 triggers so that
    /// `search_documents_fts` does not have a corresponding entry.
    /// This simulates what would happen if rows were bulk-loaded via a direct
    /// SQL copy that bypassed the trigger mechanism.
    fn insert_sd_bypass_fts(conn: &rusqlite::Connection, kind: &str, id: &str) {
        // Disable triggers temporarily by going through a direct INSERT that
        // deliberately does NOT use the upsert helper (which does fire triggers).
        // We use a raw INSERT to simulate the scenario.
        // In SQLite, triggers on INSERT always fire unless we disable them.
        // The standard way to test "missing FTS row" is to:
        //   1. Insert via upsert (triggers fire, FTS is populated)
        //   2. Delete from FTS5 using the delete command directly (bypassing triggers)
        //      to simulate a missing FTS entry.
        // This simulates an FTS entry being missing.
        conn.execute(
            "INSERT INTO search_documents_fts(search_documents_fts, rowid, \
             title, body, aliases, source_text, relation_text, \
             record_kind, record_id, namespace, scope, sensitivity, truth_state, revision) \
             VALUES('delete', \
               (SELECT rowid FROM search_documents WHERE record_kind=?1 AND record_id=?2), \
               (SELECT title FROM search_documents WHERE record_kind=?1 AND record_id=?2), \
               (SELECT body FROM search_documents WHERE record_kind=?1 AND record_id=?2), \
               NULL, NULL, NULL, ?1, ?2, 'core', 'default', 0, 'Current', 1)",
            params![kind, id],
        )
        .unwrap();
    }

    #[test]
    fn reconcile_repopulates_missing_fts_entries() {
        let db = open_db();

        // Insert a doc via normal upsert (triggers fire → FTS gets the entry).
        {
            let conn = db.write();
            upsert_search_document(
                &conn,
                &make_doc("memory", "11111111-1111-1111-1111-111111111111"),
            )
            .unwrap();
            upsert_search_document(
                &conn,
                &make_doc("entity", "22222222-2222-2222-2222-222222222222"),
            )
            .unwrap();
        }

        // Remove one FTS entry to simulate a missing FTS row.
        {
            let conn = db.write();
            insert_sd_bypass_fts(&conn, "memory", "11111111-1111-1111-1111-111111111111");
        }

        // Now search_documents has 2 rows but FTS has only 1.
        // reconcile_fts_index should detect 1 missing and rebuild.
        let report = {
            let conn = db.write();
            reconcile_fts_index(&conn).unwrap()
        };

        assert_eq!(report.missing_fts_rows, 1, "one missing FTS row expected");
        assert_eq!(report.repopulated, 2, "full rebuild repopulates all 2 rows");

        // After reconciliation, both rows should be searchable via FTS.
        let conn = db.write();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM search_documents_fts WHERE search_documents_fts MATCH 'title'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 2,
            "both rows must be searchable after reconciliation"
        );
    }

    #[test]
    fn reconcile_no_op_when_fts_is_in_sync() {
        let db = open_db();
        {
            let conn = db.write();
            upsert_search_document(
                &conn,
                &make_doc("memory", "aaaaaaaa-0000-0000-0000-000000000000"),
            )
            .unwrap();
        }
        let report = db.with_read(|conn| reconcile_fts_index(conn)).unwrap();
        assert_eq!(report.missing_fts_rows, 0);
        assert_eq!(report.orphan_fts_rows, 0);
        assert_eq!(report.repopulated, 0);
        assert_eq!(report.cleaned, 0);
    }

    // ── rebuild_fts_from_stream ──────────────────────────────────────────────

    #[test]
    fn rebuild_stream_inserts_records_and_fts_is_searchable() {
        let db = open_db();

        let records: Vec<MemoryResult<FtsRebuildRecord>> = vec![
            Ok(make_rebuild_record("memory", "r-00000001")),
            Ok(make_rebuild_record("memory", "r-00000002")),
            Ok(make_rebuild_record("entity", "r-00000003")),
            Ok(make_rebuild_record("goal", "r-00000004")),
            Ok(make_rebuild_record("summary", "r-00000005")),
        ];

        let outcome =
            rebuild_fts_from_stream(&db, Some(42), "fts5-rebuild-model", records.into_iter())
                .unwrap();

        match outcome {
            FtsRebuildOutcome::Activated {
                member_count,
                membership_hash,
            } => {
                assert_eq!(member_count, 5, "all 5 records must be in search_documents");
                assert_ne!(membership_hash, EMPTY_SHA256);
                assert_eq!(membership_hash.len(), 64);
            }
            FtsRebuildOutcome::Interrupted { .. } => {
                panic!("expected Activated, got Interrupted");
            }
        }

        // Verify FTS5 is searchable (search for common word in all titles).
        let conn = db.write();
        let fts_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM search_documents_fts WHERE search_documents_fts MATCH 'title'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            fts_count, 5,
            "all 5 records must be searchable via FTS5 after rebuild"
        );

        // Verify derived_manifests row was written.
        let (mcount, mhash, status): (i64, String, String) = conn
            .query_row(
                "SELECT member_count, membership_hash, status \
                 FROM derived_manifests WHERE target = 'fts' AND version = 'fts5-v1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(mcount, 5);
        assert!(!mhash.is_empty());
        assert_eq!(status, "active");
    }

    #[test]
    fn rebuild_stream_interrupted_returns_interrupted_outcome() {
        let db = open_db();

        let records: Vec<MemoryResult<FtsRebuildRecord>> = vec![
            Ok(make_rebuild_record("memory", "ok-record-1")),
            Err(StorageError::Serde("simulated stream failure".into()).into()),
        ];

        let outcome =
            rebuild_fts_from_stream(&db, Some(1), "fts5-rebuild-model", records.into_iter())
                .unwrap();

        match outcome {
            FtsRebuildOutcome::Interrupted { last_kind_id } => {
                // The last successfully upserted record should be recorded.
                assert_eq!(
                    last_kind_id.as_deref(),
                    Some("memory:ok-record-1"),
                    "last_kind_id must be the last successful record before interruption"
                );
            }
            FtsRebuildOutcome::Activated { .. } => {
                panic!("expected Interrupted, got Activated");
            }
        }
    }

    #[test]
    fn rebuild_stream_empty_produces_empty_hash() {
        let db = open_db();

        let outcome =
            rebuild_fts_from_stream(&db, None, "fts5-rebuild-model", std::iter::empty()).unwrap();

        match outcome {
            FtsRebuildOutcome::Activated {
                member_count,
                membership_hash,
            } => {
                assert_eq!(member_count, 0);
                assert_eq!(membership_hash, EMPTY_SHA256);
            }
            FtsRebuildOutcome::Interrupted { .. } => panic!("expected Activated"),
        }
    }

    // ── rebuild_fts_for_kind ─────────────────────────────────────────────────

    #[test]
    fn rebuild_for_kind_deletes_only_that_kind() {
        let db = open_db();
        {
            let conn = db.write();
            upsert_search_document(&conn, &make_doc("memory", "mem-00000001")).unwrap();
            upsert_search_document(&conn, &make_doc("memory", "mem-00000002")).unwrap();
            upsert_search_document(&conn, &make_doc("entity", "ent-00000001")).unwrap();
        }

        // Delete only 'memory' kind.
        {
            let conn = db.write();
            rebuild_fts_for_kind(&conn, "memory").unwrap();
        }

        // Only 'entity' row should remain in search_documents.
        let conn = db.write();
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM search_documents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 1, "only the entity row should remain");

        let kind: String = conn
            .query_row("SELECT record_kind FROM search_documents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kind, "entity");

        // FTS5 should also reflect the deletion (triggers fired).
        let fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM search_documents_fts", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(fts_count, 1, "FTS5 should have 1 entry after kind deletion");
    }

    #[test]
    fn rebuild_for_kind_is_idempotent_on_empty_kind() {
        let db = open_db();
        // Deleting a kind that has no rows must succeed without error.
        let conn = db.write();
        rebuild_fts_for_kind(&conn, "nonexistent_kind").unwrap();
    }
}
