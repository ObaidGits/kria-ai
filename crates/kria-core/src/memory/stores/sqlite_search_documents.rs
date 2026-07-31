//! `search_documents` projection store — F3.2 / task 3.2.1.
//!
//! `search_documents` is the **authority-derived projection** that the FTS5
//! external-content table (task 3.2.2) indexes over (design §4.4).  It holds
//! one row per searchable item and is the single source of truth for what the
//! FTS5 index will see.
//!
//! ## Design invariants
//! * This is a **rebuildable derived projection** (§A8): it may be truncated and
//!   rebuilt at any time without losing authority data.
//! * Deleted/Forgotten records stay in the table with their `truth_state` set —
//!   FTS5 queries pre-filter by `truth_state` to exclude them without requiring
//!   a full rebuild.
//! * `content_hash` (SHA-256 of the source record's searchable content) drives
//!   dedup: the projection builder skips the write if the hash is unchanged.
//! * `revision` tracks which graph revision was current when the row was written,
//!   supporting cursor-based incremental rebuilds.
//! * Policy columns (`namespace`, `owner_id`, `scope`, `sensitivity`) are present
//!   on every row; FTS5 queries use them for mandatory preselection.
//! * User text is never interpolated into SQL — all parameters go through the
//!   `rusqlite::params!` / `?N` binding mechanism.

use std::sync::Arc;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};

// ─── SearchDocument ───────────────────────────────────────────────────────────

/// One projection row for the `search_documents` table.
///
/// Matches the schema created by migration `0027_search_documents.sql`.
/// `record_kind` is the discriminator; `record_id` is the stable UUID text.
/// `aliases`, `source_text`, and `relation_text` are `None` for record kinds
/// that do not carry those fields (FTS5 treats NULL as an empty column).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchDocument {
    // ── Identity ────────────────────────────────────────────────────────────
    /// Discriminator — one of `memory`, `summary`, `skill`, `rule`, `entity`,
    /// `source`, `goal`, `relationship`.
    pub record_kind: String,
    /// Stable UUID (canonical lower-case text).
    pub record_id: String,

    // ── Searchable fields ────────────────────────────────────────────────────
    /// Primary searchable name / label.
    pub title: Option<String>,
    /// Main searchable content / description.
    pub body: Option<String>,
    /// Space/comma-joined aliases — populated for `entity` rows.
    pub aliases: Option<String>,
    /// Source name/description/kind metadata — populated for `source` rows.
    pub source_text: Option<String>,
    /// Relation label(s) — populated for `relationship` rows.
    pub relation_text: Option<String>,

    // ── Policy columns (design §4.1) ─────────────────────────────────────────
    pub namespace: String,
    pub owner_id: String,
    pub scope: String,
    /// 0 = Public, 1 = Internal, 2 = Private, 3 = Secret.
    pub sensitivity: i64,

    // ── Truth / time ─────────────────────────────────────────────────────────
    /// e.g. `"Current"`, `"Stale"`, `"Unverified"`, `"Superseded"`,
    /// `"Forgotten"`, `"Deleted"`.
    pub truth_state: String,
    /// RFC3339 UTC or `None` — start of the valid interval (inclusive).
    pub valid_from: Option<String>,
    /// RFC3339 UTC or `None` — end of the valid interval (exclusive).
    pub valid_until: Option<String>,

    // ── Projection integrity ──────────────────────────────────────────────────
    /// SHA-256 (hex) of the source record's searchable content.
    /// Used for dedup (skip upsert when unchanged) and rebuild comparison.
    pub content_hash: String,
    /// The graph revision when this projection row was written.
    pub revision: i64,
}

// ─── upsert_search_document ───────────────────────────────────────────────────

/// Upsert one [`SearchDocument`] row into `search_documents`.
///
/// Uses `ON CONFLICT(record_kind, record_id) DO UPDATE SET …` so the row is
/// fully replaced when an updated projection is written (e.g. content changed,
/// truth state transitioned, or revision bumped).  This mirrors the
/// `mem_vectors_v2` upsert pattern (task 3.1.2).
pub fn upsert_search_document(conn: &Connection, doc: &SearchDocument) -> MemoryResult<()> {
    conn.execute(
        "INSERT INTO search_documents (
             record_kind, record_id,
             title, body, aliases, source_text, relation_text,
             namespace, owner_id, scope, sensitivity,
             truth_state, valid_from, valid_until,
             content_hash, revision
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
         ON CONFLICT(record_kind, record_id) DO UPDATE SET
             title         = excluded.title,
             body          = excluded.body,
             aliases       = excluded.aliases,
             source_text   = excluded.source_text,
             relation_text = excluded.relation_text,
             namespace     = excluded.namespace,
             owner_id      = excluded.owner_id,
             scope         = excluded.scope,
             sensitivity   = excluded.sensitivity,
             truth_state   = excluded.truth_state,
             valid_from    = excluded.valid_from,
             valid_until   = excluded.valid_until,
             content_hash  = excluded.content_hash,
             revision      = excluded.revision",
        params![
            doc.record_kind,
            doc.record_id,
            doc.title,
            doc.body,
            doc.aliases,
            doc.source_text,
            doc.relation_text,
            doc.namespace,
            doc.owner_id,
            doc.scope,
            doc.sensitivity,
            doc.truth_state,
            doc.valid_from,
            doc.valid_until,
            doc.content_hash,
            doc.revision,
        ],
    )
    .map_err(StorageError::Sqlite)?;
    Ok(())
}

// ─── delete_search_document ───────────────────────────────────────────────────

/// Delete one projection row from `search_documents` by `(record_kind, record_id)`.
///
/// A no-op (returns `Ok(())`) when the row does not exist — idempotent by
/// design for the outbox/rebuild path.
pub fn delete_search_document(
    conn: &Connection,
    record_kind: &str,
    record_id: &str,
) -> MemoryResult<()> {
    conn.execute(
        "DELETE FROM search_documents WHERE record_kind = ?1 AND record_id = ?2",
        params![record_kind, record_id],
    )
    .map_err(StorageError::Sqlite)?;
    Ok(())
}

// ─── count_search_documents ───────────────────────────────────────────────────

/// Return the number of `search_documents` rows whose `namespace` equals
/// `partition`.  Intended for rebuild verification ("are there N rows?").
///
/// `partition` maps to the `namespace` column — it is the coarsest policy
/// partition used by rebuild and integrity checks.
pub fn count_search_documents(conn: &Connection, partition: &str) -> MemoryResult<i64> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM search_documents WHERE namespace = ?1",
            params![partition],
            |r| r.get(0),
        )
        .map_err(StorageError::Sqlite)?;
    Ok(count)
}

// ─── SqliteSearchDocumentStore ────────────────────────────────────────────────

/// Thin wrapper that provides owned-`Arc<Database>` access to the three
/// projection-layer helpers above.  Higher-level rebuild and outbox code can
/// hold this type and invoke the operations without borrowing a `Connection`
/// directly.
pub struct SqliteSearchDocumentStore {
    db: Arc<Database>,
}

impl SqliteSearchDocumentStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Upsert one [`SearchDocument`] row (see [`upsert_search_document`]).
    pub fn upsert(&self, doc: &SearchDocument) -> MemoryResult<()> {
        let conn = self.db.write();
        upsert_search_document(&conn, doc)
    }

    /// Delete one projection row (see [`delete_search_document`]).
    pub fn delete(&self, record_kind: &str, record_id: &str) -> MemoryResult<()> {
        let conn = self.db.write();
        delete_search_document(&conn, record_kind, record_id)
    }

    /// Count rows for a namespace partition (see [`count_search_documents`]).
    pub fn count(&self, partition: &str) -> MemoryResult<i64> {
        self.db
            .with_read(|conn| count_search_documents(conn, partition))
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal but fully-populated `SearchDocument` for a given kind.
    fn make_doc(kind: &str, id: &str) -> SearchDocument {
        SearchDocument {
            record_kind: kind.to_string(),
            record_id: id.to_string(),
            title: Some(format!("{kind} title")),
            body: Some(format!("{kind} body text")),
            aliases: if kind == "entity" {
                Some("alias1, alias2".to_string())
            } else {
                None
            },
            source_text: if kind == "source" {
                Some("source description kind=native".to_string())
            } else {
                None
            },
            relation_text: if kind == "relationship" {
                Some("derived_from".to_string())
            } else {
                None
            },
            namespace: "core".to_string(),
            owner_id: "user-001".to_string(),
            scope: "default".to_string(),
            sensitivity: 0,
            truth_state: "Current".to_string(),
            valid_from: Some("2024-01-01T00:00:00Z".to_string()),
            valid_until: None,
            content_hash: format!("sha256-{kind}-{id}"),
            revision: 1,
        }
    }

    fn open_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    // ── 1. Insert-then-query round-trip ──────────────────────────────────────

    #[test]
    fn insert_then_query_round_trip() {
        let db = open_db();
        let store = SqliteSearchDocumentStore::new(db.clone());

        let doc = make_doc("memory", "11111111-1111-1111-1111-111111111111");
        store.upsert(&doc).unwrap();

        // Read back the row and verify every field is persisted correctly.
        db.with_read(|conn| {
            let row: SearchDocument = conn
                .query_row(
                    "SELECT record_kind, record_id,
                            title, body, aliases, source_text, relation_text,
                            namespace, owner_id, scope, sensitivity,
                            truth_state, valid_from, valid_until,
                            content_hash, revision
                     FROM search_documents
                     WHERE record_kind = 'memory'
                       AND record_id   = '11111111-1111-1111-1111-111111111111'",
                    [],
                    |r| {
                        Ok(SearchDocument {
                            record_kind: r.get(0)?,
                            record_id: r.get(1)?,
                            title: r.get(2)?,
                            body: r.get(3)?,
                            aliases: r.get(4)?,
                            source_text: r.get(5)?,
                            relation_text: r.get(6)?,
                            namespace: r.get(7)?,
                            owner_id: r.get(8)?,
                            scope: r.get(9)?,
                            sensitivity: r.get(10)?,
                            truth_state: r.get(11)?,
                            valid_from: r.get(12)?,
                            valid_until: r.get(13)?,
                            content_hash: r.get(14)?,
                            revision: r.get(15)?,
                        })
                    },
                )
                .map_err(StorageError::Sqlite)?;

            assert_eq!(row, doc, "every field must round-trip without mutation");
            Ok(())
        })
        .unwrap();
    }

    // ── 2. All optional fields (aliases, source_text, relation_text) persist ─

    #[test]
    fn all_optional_fields_persist_correctly() {
        let db = open_db();
        let store = SqliteSearchDocumentStore::new(db.clone());

        // Entity: aliases populated, source_text/relation_text NULL.
        let entity = make_doc("entity", "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
        assert!(entity.aliases.is_some());
        assert!(entity.source_text.is_none());
        assert!(entity.relation_text.is_none());
        store.upsert(&entity).unwrap();

        // Source: source_text populated, aliases/relation_text NULL.
        let source = make_doc("source", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
        assert!(source.source_text.is_some());
        store.upsert(&source).unwrap();

        // Relationship: relation_text populated.
        let rel = make_doc("relationship", "cccccccc-cccc-cccc-cccc-cccccccccccc");
        assert!(rel.relation_text.is_some());
        store.upsert(&rel).unwrap();

        // Verify aliases round-trip for entity.
        db.with_read(|conn| {
            let aliases: Option<String> = conn
                .query_row(
                    "SELECT aliases FROM search_documents WHERE record_kind='entity' AND record_id='aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            assert_eq!(aliases.as_deref(), Some("alias1, alias2"));
            Ok(())
        })
        .unwrap();
    }

    // ── 3. Policy index hit: sensitivity=3 row is queryable with exact filter ─

    #[test]
    fn policy_index_sensitivity_filter() {
        let db = open_db();
        let store = SqliteSearchDocumentStore::new(db.clone());

        let mut secret_doc = make_doc("skill", "dddddddd-dddd-dddd-dddd-dddddddddddd");
        secret_doc.sensitivity = 3;
        secret_doc.namespace = "core".to_string();
        secret_doc.scope = "default".to_string();
        store.upsert(&secret_doc).unwrap();

        // A sensitivity=3 row is returned when queried with the exact value.
        db.with_read(|conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM search_documents
                     WHERE namespace = 'core'
                       AND scope = 'default'
                       AND sensitivity = 3
                       AND truth_state = 'Current'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            assert_eq!(
                count, 1,
                "secret (sensitivity=3) row must be reachable via policy index"
            );
            Ok(())
        })
        .unwrap();

        // A query capped at sensitivity <= 2 must NOT return the secret row.
        db.with_read(|conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM search_documents
                     WHERE namespace = 'core'
                       AND scope = 'default'
                       AND sensitivity <= 2
                       AND truth_state = 'Current'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            assert_eq!(
                count, 0,
                "secret row must be invisible when sensitivity <= 2"
            );
            Ok(())
        })
        .unwrap();
    }

    // ── 4. Dedup: upsert same (kind, id) with new content_hash replaces row ──

    #[test]
    fn upsert_same_key_replaces_row() {
        let db = open_db();
        let store = SqliteSearchDocumentStore::new(db.clone());

        let id = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let mut doc = make_doc("rule", id);
        doc.content_hash = "sha256-original".to_string();
        doc.revision = 10;
        store.upsert(&doc).unwrap();

        // Now upsert the same (record_kind, record_id) with updated content.
        doc.title = Some("updated rule title".to_string());
        doc.body = Some("updated rule body".to_string());
        doc.content_hash = "sha256-updated".to_string();
        doc.revision = 11;
        store.upsert(&doc).unwrap();

        // Verify exactly one row exists and it has the new values.
        db.with_read(|conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM search_documents WHERE record_kind='rule' AND record_id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            assert_eq!(count, 1, "upsert must not create a second row");

            let (hash, rev, title): (String, i64, Option<String>) = conn
                .query_row(
                    "SELECT content_hash, revision, title FROM search_documents
                     WHERE record_kind='rule' AND record_id=?1",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(StorageError::Sqlite)?;
            assert_eq!(hash, "sha256-updated");
            assert_eq!(rev, 11);
            assert_eq!(title.as_deref(), Some("updated rule title"));
            Ok(())
        })
        .unwrap();
    }

    // ── 5. Delete removes the row; double-delete is a no-op ─────────────────

    #[test]
    fn delete_removes_row_and_is_idempotent() {
        let db = open_db();
        let store = SqliteSearchDocumentStore::new(db.clone());

        let id = "ffffffff-ffff-ffff-ffff-ffffffffffff";
        store.upsert(&make_doc("goal", id)).unwrap();

        let count_before: i64 = db
            .with_read(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM search_documents WHERE record_kind='goal'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)
                .map_err(Into::into)
            })
            .unwrap();
        assert_eq!(count_before, 1);

        store.delete("goal", id).unwrap();

        let count_after: i64 = db
            .with_read(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM search_documents WHERE record_kind='goal'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)
                .map_err(Into::into)
            })
            .unwrap();
        assert_eq!(count_after, 0);

        // Second delete must not error.
        store.delete("goal", id).unwrap();
    }

    // ── 6. count_search_documents aggregates by namespace ───────────────────

    #[test]
    fn count_aggregates_by_namespace() {
        let db = open_db();
        let store = SqliteSearchDocumentStore::new(db.clone());

        let mut doc_a = make_doc("memory", "00000000-0000-0000-0000-000000000001");
        doc_a.namespace = "ns-a".to_string();
        store.upsert(&doc_a).unwrap();

        let mut doc_b = make_doc("memory", "00000000-0000-0000-0000-000000000002");
        doc_b.namespace = "ns-b".to_string();
        store.upsert(&doc_b).unwrap();

        let mut doc_b2 = make_doc("summary", "00000000-0000-0000-0000-000000000003");
        doc_b2.namespace = "ns-b".to_string();
        store.upsert(&doc_b2).unwrap();

        let count_a = db
            .with_read(|conn| count_search_documents(conn, "ns-a"))
            .unwrap();
        assert_eq!(count_a, 1);

        let count_b = db
            .with_read(|conn| count_search_documents(conn, "ns-b"))
            .unwrap();
        assert_eq!(count_b, 2);

        let count_missing = db
            .with_read(|conn| count_search_documents(conn, "ns-nope"))
            .unwrap();
        assert_eq!(count_missing, 0);
    }

    // ── 7. Schema rejects invalid sensitivity ────────────────────────────────

    #[test]
    fn schema_rejects_invalid_sensitivity() {
        let db = open_db();
        let conn = db.write();
        let result = conn.execute(
            "INSERT INTO search_documents (
                 record_kind, record_id, namespace, owner_id, scope,
                 sensitivity, truth_state, content_hash, revision
             ) VALUES ('memory','bad-sens-id','core','u','default',4,'Current','h',1)",
            [],
        );
        assert!(
            result.is_err(),
            "sensitivity=4 must violate the CHECK constraint"
        );
    }

    // ── 8. Schema rejects negative revision ─────────────────────────────────

    #[test]
    fn schema_rejects_negative_revision() {
        let db = open_db();
        let conn = db.write();
        let result = conn.execute(
            "INSERT INTO search_documents (
                 record_kind, record_id, namespace, owner_id, scope,
                 sensitivity, truth_state, content_hash, revision
             ) VALUES ('memory','bad-rev-id','core','u','default',0,'Current','h',-1)",
            [],
        );
        assert!(
            result.is_err(),
            "revision=-1 must violate the CHECK constraint"
        );
    }

    // ── 9. Schema rejects inverted valid time interval ───────────────────────

    #[test]
    fn schema_rejects_inverted_valid_interval() {
        let db = open_db();
        let conn = db.write();
        let result = conn.execute(
            "INSERT INTO search_documents (
                 record_kind, record_id, namespace, owner_id, scope,
                 sensitivity, truth_state, content_hash, revision,
                 valid_from, valid_until
             ) VALUES ('memory','bad-time-id','core','u','default',0,'Current','h',1,
                       '2025-01-10T00:00:00Z','2025-01-01T00:00:00Z')",
            [],
        );
        assert!(
            result.is_err(),
            "valid_until < valid_from must violate the CHECK constraint"
        );
    }

    // ── 10. Multiple kinds co-exist; deletion is kind-scoped ────────────────

    #[test]
    fn multiple_kinds_coexist_deletion_is_kind_scoped() {
        let db = open_db();
        let store = SqliteSearchDocumentStore::new(db.clone());

        let id = "12345678-1234-1234-1234-123456789abc";
        store.upsert(&make_doc("memory", id)).unwrap();
        store.upsert(&make_doc("entity", id)).unwrap();

        // Deleting 'memory' with that id must NOT touch the 'entity' row.
        store.delete("memory", id).unwrap();

        let remaining: i64 = db
            .with_read(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM search_documents WHERE record_id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)
                .map_err(Into::into)
            })
            .unwrap();
        assert_eq!(remaining, 1, "only the 'memory' row should be deleted");
    }

    // ── 11. Deleted/Forgotten rows are retained with truth_state ─────────────

    #[test]
    fn deleted_rows_retained_with_truth_state() {
        let db = open_db();
        let store = SqliteSearchDocumentStore::new(db.clone());

        let id = "deadbeef-dead-beef-dead-beefdeadbeef";
        let mut doc = make_doc("memory", id);
        doc.truth_state = "Current".to_string();
        store.upsert(&doc).unwrap();

        // Transition to Deleted via upsert (no physical delete — just truth_state).
        doc.truth_state = "Deleted".to_string();
        doc.revision = 2;
        store.upsert(&doc).unwrap();

        db.with_read(|conn| {
            let ts: String = conn
                .query_row(
                    "SELECT truth_state FROM search_documents WHERE record_kind='memory' AND record_id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            assert_eq!(ts, "Deleted", "truth_state transition must persist via upsert");
            Ok(())
        })
        .unwrap();
    }
}

// ─── FTS5 search helper ───────────────────────────────────────────────────────

/// Policy prefilter options for [`search_documents_fts_query`].
///
/// All fields default to "unfiltered" (`None`) so callers opt-in to each
/// dimension they need.  When a field is `Some`, the corresponding
/// `UNINDEXED` column in `search_documents_fts` is matched exactly (namespace,
/// scope) or with `<=` (sensitivity).
#[derive(Debug, Clone, Default)]
pub struct Fts5SearchQuery {
    /// Restrict to a specific namespace (e.g. `"core"` or `"plugin/x"`).
    pub namespace: Option<String>,
    /// Restrict to a specific scope (e.g. `"default"`).
    pub scope: Option<String>,
    /// Restrict to rows where `sensitivity <= max_sensitivity`.
    pub max_sensitivity: Option<i64>,
    /// Restrict to a specific truth state (e.g. `"Current"`).
    /// When `None` the caller is responsible for applying truth filtering
    /// (e.g. exclude Deleted/Forgotten rows via a higher-level gate).
    pub truth_state: Option<String>,
    /// Maximum number of results to return (defaults to 25).
    pub limit: Option<usize>,
}

// ─── PolicySummary ────────────────────────────────────────────────────────────

/// Policy provenance included in every [`SearchDocumentHit`].
///
/// Conveys the access-control context for the result so callers can apply
/// further policy gates without re-querying.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicySummary {
    /// Coarsest policy partition (e.g. `"core"` or `"plugin/x"`).
    pub namespace: String,
    /// Scope within the namespace (e.g. `"default"` or `"session-xyz"`).
    pub scope: String,
    /// 0 = Public, 1 = Internal, 2 = Private, 3 = Secret.
    pub sensitivity: i64,
}

// ─── TotalSemantics ───────────────────────────────────────────────────────────

/// Honest representation of the total number of matching documents for an FTS5
/// query result set.
///
/// FTS5 does not provide a cheap exact count when results are capped by a
/// `LIMIT`.  Instead of lying with a fixed number or omitting the total, we
/// communicate what we actually know:
///
/// * [`Exact`](TotalSemantics::Exact) — returned fewer rows than `limit`; the
///   result set is complete so we know the precise total.
/// * [`AtLeast`](TotalSemantics::AtLeast) — returned exactly `limit` rows;
///   there may be more matches in the corpus that were truncated.
/// * [`Estimate`](TotalSemantics::Estimate) — reserved for callers that
///   compute an approximate count via a secondary query or statistics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TotalSemantics {
    /// We know the exact count — the result set was not truncated.
    Exact(u64),
    /// The result set was capped at the limit; there are at least this many
    /// matching documents.
    AtLeast(u64),
    /// A statistical estimate — approximate, not guaranteed.
    Estimate(u64),
}

// ─── FtsSearchResult ─────────────────────────────────────────────────────────

/// Full result of a [`search_documents_fts_query`] call.
///
/// Bundles the ranked hit list with honest total semantics and the strategy
/// profile version, so callers have everything needed for result presentation
/// without additional round-trips.
#[derive(Debug, Clone, PartialEq)]
pub struct FtsSearchResult {
    /// Ordered list of matching documents, best match first.
    pub hits: Vec<SearchDocumentHit>,
    /// Honest representation of how many documents matched the query.
    pub total_semantics: TotalSemantics,
    /// Strategy profile identifier — currently `"fts5-v1"`.
    pub profile_version: String,
}

// ─── SearchDocumentHit ────────────────────────────────────────────────────────

/// One hit returned by [`search_documents_fts_query`].
#[derive(Debug, Clone, PartialEq)]
pub struct SearchDocumentHit {
    /// Discriminator from `record_kind` UNINDEXED column.
    pub record_kind: String,
    /// Stable UUID text from `record_id` UNINDEXED column.
    pub record_id: String,
    /// BM25 score as returned by SQLite (lower-is-better raw, negated here to
    /// higher-is-better). Relative within the result set only — not an absolute
    /// relevance score.
    ///
    /// Retained for backwards compatibility. `relative_score` carries the same
    /// value with a clearer name.
    pub bm25_score: f32,
    /// Negated BM25 score — higher is better.  Relative within this result set
    /// only; not an absolute relevance score.
    pub relative_score: f32,
    /// Human-readable explanation of the ranking strategy.
    ///
    /// Format: `"BM25 relevance score (relative); strategy: fts5; profile: v1"`.
    pub rank_rationale: String,
    /// Truth state at the time the FTS row was written (UNINDEXED).
    pub truth_state: String,
    /// Namespace (UNINDEXED).
    pub namespace: String,
    /// Scope (UNINDEXED).
    pub scope: String,
    /// Sensitivity level 0..3 (UNINDEXED).
    pub sensitivity: i64,
    /// Graph revision when the projection row was written (UNINDEXED).
    pub revision: i64,
    /// The primary FTS5 column that contributed to this match.
    ///
    /// Determined by inspecting which of the indexed columns (`title`, `body`,
    /// `aliases`, `source_text`, `relation_text`) are non-null and non-empty in
    /// the matched row.  The column that appears first in the FTS5 definition
    /// order is selected as the primary match field.  `None` when all indexed
    /// columns are empty (should not occur in a well-formed projection).
    pub matched_field: Option<String>,
    /// Policy provenance for this hit.
    pub policy_summary: PolicySummary,
    /// UI navigation target for this hit.
    ///
    /// | `record_kind`  | target pattern                          |
    /// |----------------|-----------------------------------------|
    /// | `entity`       | `knowledge/entity/{record_id}`          |
    /// | `memory`, `summary`, `skill`, `rule` | `knowledge/record/{record_id}` |
    /// | `source`       | `sources/{record_id}`                   |
    /// | `goal`         | `goals/{record_id}`                     |
    /// | `relationship` | `knowledge/relationship/{record_id}`    |
    pub navigation_target: Option<String>,
}

// ─── navigation_target helper ─────────────────────────────────────────────────

/// Derive the UI navigation path for a given `record_kind` and `record_id`.
///
/// Returns `None` only for unknown `record_kind` values — all currently
/// defined kinds produce a target.
fn navigation_target(record_kind: &str, record_id: &str) -> Option<String> {
    match record_kind {
        "entity" => Some(format!("knowledge/entity/{record_id}")),
        "memory" | "summary" | "skill" | "rule" => Some(format!("knowledge/record/{record_id}")),
        "source" => Some(format!("sources/{record_id}")),
        "goal" => Some(format!("goals/{record_id}")),
        "relationship" => Some(format!("knowledge/relationship/{record_id}")),
        _ => None,
    }
}

/// Determine the primary matched FTS5 column from a row's indexed column values.
///
/// Checks `title`, `body`, `aliases`, `source_text`, `relation_text` in that
/// order and returns the name of the first non-empty column.
fn primary_matched_field(
    title: Option<&str>,
    body: Option<&str>,
    aliases: Option<&str>,
    source_text: Option<&str>,
    relation_text: Option<&str>,
) -> Option<String> {
    if title.map_or(false, |s| !s.is_empty()) {
        return Some("title".to_string());
    }
    if body.map_or(false, |s| !s.is_empty()) {
        return Some("body".to_string());
    }
    if aliases.map_or(false, |s| !s.is_empty()) {
        return Some("aliases".to_string());
    }
    if source_text.map_or(false, |s| !s.is_empty()) {
        return Some("source_text".to_string());
    }
    if relation_text.map_or(false, |s| !s.is_empty()) {
        return Some("relation_text".to_string());
    }
    None
}

/// Query `search_documents_fts` using a safe FTS5 MATCH expression built from
/// `raw_query` via [`compile_fts5_query`].
///
/// # Arguments
/// * `conn`      — a read (or write) connection to the authority database.
/// * `raw_query` — arbitrary user text; sanitized by [`compile_fts5_query`] before
///                 use in the SQL MATCH expression.  No user text is ever
///                 interpolated into SQL.
/// * `filter`    — optional policy / truth / limit constraints.
///
/// # Returns
/// An [`FtsSearchResult`] containing:
/// * `hits` — ordered `Vec<SearchDocumentHit>` sorted by descending BM25
///   relevance (best match first), capped at `filter.limit` (default 25).
///   Returns an empty vec when `raw_query` contains no searchable tokens.
/// * `total_semantics` — [`TotalSemantics::Exact`] when fewer results than
///   `limit` were returned; [`TotalSemantics::AtLeast`] when the cap was hit.
/// * `profile_version` — `"fts5-v1"`.
///
/// # Invariant
/// FTS5 is never the semantic authority — results are candidates only.  Callers
/// MUST re-apply authorization and truth-state gates before surfacing results.
pub fn search_documents_fts_query(
    conn: &Connection,
    raw_query: &str,
    filter: &Fts5SearchQuery,
) -> MemoryResult<FtsSearchResult> {
    use super::fts5_query::compile_fts5_query;
    use crate::memory::error::StorageError;

    let compiled = match compile_fts5_query(raw_query) {
        Ok(c) => c,
        Err(super::fts5_query::QueryCompileError::EmptyQuery) => {
            return Ok(FtsSearchResult {
                hits: Vec::new(),
                total_semantics: TotalSemantics::Exact(0),
                profile_version: "fts5-v1".to_string(),
            });
        }
        Err(e) => {
            return Err(StorageError::Search(e.to_string()).into());
        }
    };
    let match_expr = compiled.match_expr;

    let limit = filter.limit.unwrap_or(25) as i64;

    // Build the WHERE clause and the bind-parameter list dynamically.
    //
    // rusqlite rejects queries where the actual number of bound parameters
    // differs from the number of `?N` placeholders in the SQL, so we can't
    // pre-allocate 6 slots and skip the ones that are None.  Instead we
    // construct both the SQL fragment AND the param list together, keeping
    // them in sync, then pass them via `query_map` with a `Vec<Box<dyn
    // rusqlite::types::ToSql>>`.
    //
    // Parameter slots:
    //   ?1 — FTS5 MATCH expression (always present)
    //   ?2 — LIMIT value            (always present)
    //   ?3 … — filter values in the order they are added below.

    let mut where_parts: Vec<String> = Vec::new();
    // We collect owned `ToSql` values as boxed trait objects.
    let mut extra_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    let mut next_param = 3usize; // ?1 and ?2 are always MATCH and LIMIT

    if let Some(ns) = &filter.namespace {
        where_parts.push(format!("namespace = ?{next_param}"));
        extra_params.push(Box::new(ns.clone()));
        next_param += 1;
    }
    if let Some(sc) = &filter.scope {
        where_parts.push(format!("scope = ?{next_param}"));
        extra_params.push(Box::new(sc.clone()));
        next_param += 1;
    }
    if let Some(sens) = filter.max_sensitivity {
        where_parts.push(format!("sensitivity <= ?{next_param}"));
        extra_params.push(Box::new(sens));
        next_param += 1;
    }
    if let Some(ts) = &filter.truth_state {
        where_parts.push(format!("truth_state = ?{next_param}"));
        extra_params.push(Box::new(ts.clone()));
        // next_param += 1; — no further params, omit to suppress dead_code lint
    }

    let where_sql = if where_parts.is_empty() {
        String::new()
    } else {
        format!(" AND {}", where_parts.join(" AND "))
    };

    // bm25() returns negative values (lower = better); negate so that higher
    // float = better, consistent with the existing SqliteSearchStore convention.
    //
    // We also select the indexed content columns so we can determine
    // `matched_field` without a second query.
    let sql = format!(
        "SELECT record_kind, record_id, -bm25(search_documents_fts) AS score, \
                truth_state, namespace, scope, sensitivity, revision, \
                title, body, aliases, source_text, relation_text \
         FROM search_documents_fts \
         WHERE search_documents_fts MATCH ?1{where_sql} \
         ORDER BY score DESC \
         LIMIT ?2"
    );

    let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;

    // Build the final flat parameter slice: [match_expr, limit, extra…].
    // We use `params_from_iter` which accepts any iterator of `&dyn ToSql`.
    let mut all_params: Vec<&dyn rusqlite::types::ToSql> = Vec::new();
    all_params.push(&match_expr);
    all_params.push(&limit);
    for p in &extra_params {
        all_params.push(p.as_ref());
    }

    let rows = stmt
        .query_map(all_params.as_slice(), |r| {
            let record_kind: String = r.get(0)?;
            let record_id: String = r.get(1)?;
            let score: f32 = r.get::<_, f64>(2)? as f32;
            let truth_state: String = r.get(3)?;
            let namespace: String = r.get(4)?;
            let scope: String = r.get(5)?;
            let sensitivity: i64 = r.get(6)?;
            let revision: i64 = r.get(7)?;
            let title: Option<String> = r.get(8)?;
            let body: Option<String> = r.get(9)?;
            let aliases: Option<String> = r.get(10)?;
            let source_text: Option<String> = r.get(11)?;
            let relation_text: Option<String> = r.get(12)?;

            let matched_field = primary_matched_field(
                title.as_deref(),
                body.as_deref(),
                aliases.as_deref(),
                source_text.as_deref(),
                relation_text.as_deref(),
            );
            let nav = navigation_target(&record_kind, &record_id);
            let policy = PolicySummary {
                namespace: namespace.clone(),
                scope: scope.clone(),
                sensitivity,
            };

            Ok(SearchDocumentHit {
                record_kind,
                record_id,
                bm25_score: score,
                relative_score: score,
                rank_rationale: "BM25 relevance score (relative); strategy: fts5; profile: v1"
                    .to_string(),
                truth_state,
                namespace,
                scope,
                sensitivity,
                revision,
                matched_field,
                policy_summary: policy,
                navigation_target: nav,
            })
        })
        .map_err(StorageError::Sqlite)?;

    let mut hits = Vec::new();
    for row in rows {
        hits.push(row.map_err(StorageError::Sqlite)?);
    }

    let hit_count = hits.len() as u64;
    let total_semantics = if hit_count < limit as u64 {
        TotalSemantics::Exact(hit_count)
    } else {
        TotalSemantics::AtLeast(hit_count)
    };

    Ok(FtsSearchResult {
        hits,
        total_semantics,
        profile_version: "fts5-v1".to_string(),
    })
}

// ─── FTS5 tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod fts5_tests {
    use super::*;

    /// Insert a minimal `search_documents` row via the upsert helper.  The
    /// INSERT trigger on `search_documents` will automatically populate the FTS5
    /// index.
    fn insert_doc(
        conn: &Connection,
        kind: &str,
        id: &str,
        title: Option<&str>,
        body: Option<&str>,
        aliases: Option<&str>,
        namespace: &str,
        scope: &str,
        sensitivity: i64,
        truth_state: &str,
        revision: i64,
    ) {
        let doc = SearchDocument {
            record_kind: kind.to_string(),
            record_id: id.to_string(),
            title: title.map(str::to_string),
            body: body.map(str::to_string),
            aliases: aliases.map(str::to_string),
            source_text: None,
            relation_text: None,
            namespace: namespace.to_string(),
            owner_id: "user-001".to_string(),
            scope: scope.to_string(),
            sensitivity,
            truth_state: truth_state.to_string(),
            valid_from: None,
            valid_until: None,
            content_hash: format!("h-{kind}-{id}"),
            revision,
        };
        upsert_search_document(conn, &doc).unwrap();
    }

    fn open_db() -> std::sync::Arc<Database> {
        std::sync::Arc::new(Database::open_in_memory().unwrap())
    }

    // ── 1. Insert populates FTS5 — basic round-trip ──────────────────────────

    #[test]
    fn insert_populates_fts_index() {
        let db = open_db();
        let conn = db.write();

        insert_doc(
            &conn,
            "memory",
            "00000000-0000-0000-0000-000000000001",
            Some("dark mode preference"),
            Some("the user prefers dark mode for all applications"),
            None,
            "core",
            "default",
            0,
            "Current",
            1,
        );

        let result =
            search_documents_fts_query(&conn, "dark mode", &Fts5SearchQuery::default()).unwrap();
        let hits = &result.hits;

        assert_eq!(hits.len(), 1, "FTS5 must find the inserted row");
        assert_eq!(hits[0].record_kind, "memory");
        assert_eq!(hits[0].record_id, "00000000-0000-0000-0000-000000000001");
        assert!(
            hits[0].bm25_score > 0.0,
            "BM25 score must be positive (negated)"
        );
    }

    // ── 2. Update — old FTS row removed, new one searchable ──────────────────

    #[test]
    fn update_replaces_fts_entry() {
        let db = open_db();
        let conn = db.write();

        let id = "00000000-0000-0000-0000-000000000002";

        // Insert with original title.
        insert_doc(
            &conn,
            "memory",
            id,
            Some("original title about cats"),
            Some("cats are nice pets"),
            None,
            "core",
            "default",
            0,
            "Current",
            1,
        );

        // Verify the original term is searchable.
        let r = search_documents_fts_query(&conn, "cats", &Fts5SearchQuery::default()).unwrap();
        assert_eq!(r.hits.len(), 1, "original 'cats' term must be found");

        // Update to completely different content via upsert.
        let updated = SearchDocument {
            record_kind: "memory".to_string(),
            record_id: id.to_string(),
            title: Some("updated title about dogs".to_string()),
            body: Some("dogs are loyal animals".to_string()),
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
            content_hash: "h-memory-updated".to_string(),
            revision: 2,
        };
        upsert_search_document(&conn, &updated).unwrap();

        // Old term must no longer be found.
        let cats_r =
            search_documents_fts_query(&conn, "cats", &Fts5SearchQuery::default()).unwrap();
        assert!(
            cats_r.hits.is_empty(),
            "old FTS term 'cats' must not be found after update"
        );

        // New term must be found.
        let dogs_r =
            search_documents_fts_query(&conn, "dogs", &Fts5SearchQuery::default()).unwrap();
        assert_eq!(
            dogs_r.hits.len(),
            1,
            "new term 'dogs' must be searchable after update"
        );
        assert_eq!(dogs_r.hits[0].record_id, id);
    }

    // ── 3. Delete — FTS row is gone ──────────────────────────────────────────

    #[test]
    fn delete_removes_fts_entry() {
        let db = open_db();
        let conn = db.write();

        let id = "00000000-0000-0000-0000-000000000003";

        insert_doc(
            &conn,
            "memory",
            id,
            Some("deletable memory about pigeons"),
            None,
            None,
            "core",
            "default",
            0,
            "Current",
            1,
        );

        // Confirm it's found before deletion.
        let before =
            search_documents_fts_query(&conn, "pigeons", &Fts5SearchQuery::default()).unwrap();
        assert_eq!(before.hits.len(), 1, "must be searchable before delete");

        // Physically delete from search_documents (trigger fires).
        conn.execute(
            "DELETE FROM search_documents WHERE record_kind='memory' AND record_id=?1",
            rusqlite::params![id],
        )
        .unwrap();

        // Must not be found after deletion.
        let after =
            search_documents_fts_query(&conn, "pigeons", &Fts5SearchQuery::default()).unwrap();
        assert!(
            after.hits.is_empty(),
            "FTS row must be removed when search_documents row is deleted"
        );
    }

    // ── 4. Policy filter — namespace scoping ──────────────────────────────────

    #[test]
    fn namespace_filter_scopes_fts_results() {
        let db = open_db();
        let conn = db.write();

        // Two rows with the same body but different namespaces.
        insert_doc(
            &conn,
            "memory",
            "00000000-0000-0000-0000-000000000010",
            Some("shared knowledge fact"),
            Some("this fact is in namespace core"),
            None,
            "core",
            "default",
            0,
            "Current",
            1,
        );
        insert_doc(
            &conn,
            "memory",
            "00000000-0000-0000-0000-000000000011",
            Some("shared knowledge fact"),
            Some("this fact is in namespace plugin"),
            None,
            "plugin/x",
            "default",
            0,
            "Current",
            1,
        );

        // Without namespace filter: both results returned.
        let all =
            search_documents_fts_query(&conn, "knowledge", &Fts5SearchQuery::default()).unwrap();
        assert_eq!(
            all.hits.len(),
            2,
            "both rows must be found without namespace filter"
        );

        // With namespace="plugin/x": only the plugin row.
        let filtered = search_documents_fts_query(
            &conn,
            "knowledge",
            &Fts5SearchQuery {
                namespace: Some("plugin/x".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            filtered.hits.len(),
            1,
            "namespace filter must return only the matching row"
        );
        assert_eq!(filtered.hits[0].namespace, "plugin/x");
    }

    // ── 5. Unicode / diacritics — "café" found by "cafe" ─────────────────────

    #[test]
    fn unicode_diacritics_normalized() {
        let db = open_db();
        let conn = db.write();

        insert_doc(
            &conn,
            "memory",
            "00000000-0000-0000-0000-000000000020",
            Some("café recommendation"),
            Some("the user likes café au lait every morning"),
            None,
            "core",
            "default",
            0,
            "Current",
            1,
        );

        // Searching without diacritic must find the accented content.
        let r = search_documents_fts_query(&conn, "cafe", &Fts5SearchQuery::default()).unwrap();
        assert_eq!(
            r.hits.len(),
            1,
            "unicode61 remove_diacritics 2 must normalize 'café' → 'cafe'"
        );
    }

    // ── 6. Prefix matching — "mem" finds "memory" ────────────────────────────

    #[test]
    fn prefix_matching_works() {
        let db = open_db();
        let conn = db.write();

        insert_doc(
            &conn,
            "memory",
            "00000000-0000-0000-0000-000000000030",
            Some("memory graph production redesign"),
            Some("the memory system stores cognitive records"),
            None,
            "core",
            "default",
            0,
            "Current",
            1,
        );

        // 3-char prefix "mem" must match "memory" and "memories" via prefix index.
        // The fts5_query helper does not add a '*' suffix because it uses exact
        // quoted terms.  Prefix matching in FTS5 uses the `term*` syntax.
        // To test prefix, we use the raw fts5 query syntax directly without
        // going through the search helper, which uses OR-quoted tokens.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM search_documents_fts WHERE search_documents_fts MATCH 'mem*'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "prefix 'mem*' must match 'memory' via prefix index"
        );
    }

    // ── 7. UNINDEXED columns are returned in hits ────────────────────────────

    #[test]
    fn unindexed_columns_in_hits() {
        let db = open_db();
        let conn = db.write();

        insert_doc(
            &conn,
            "entity",
            "00000000-0000-0000-0000-000000000040",
            Some("entity label test"),
            Some("entity body text"),
            Some("alias-a alias-b"),
            "ns-entity",
            "private-scope",
            2,
            "Stale",
            7,
        );

        let result =
            search_documents_fts_query(&conn, "entity", &Fts5SearchQuery::default()).unwrap();

        assert_eq!(result.hits.len(), 1);
        let h = &result.hits[0];
        assert_eq!(h.record_kind, "entity");
        assert_eq!(h.record_id, "00000000-0000-0000-0000-000000000040");
        assert_eq!(h.truth_state, "Stale");
        assert_eq!(h.namespace, "ns-entity");
        assert_eq!(h.scope, "private-scope");
        assert_eq!(h.sensitivity, 2);
        assert_eq!(h.revision, 7);
    }

    // ── 8. Empty query returns no results (not an error) ────────────────────

    #[test]
    fn empty_query_returns_empty_vec() {
        let db = open_db();
        let conn = db.write();

        insert_doc(
            &conn,
            "memory",
            "00000000-0000-0000-0000-000000000050",
            Some("some content"),
            None,
            None,
            "core",
            "default",
            0,
            "Current",
            1,
        );

        let r = search_documents_fts_query(&conn, "   ", &Fts5SearchQuery::default()).unwrap();
        assert!(
            r.hits.is_empty(),
            "whitespace-only query must return empty vec"
        );

        let r2 = search_documents_fts_query(&conn, "", &Fts5SearchQuery::default()).unwrap();
        assert!(r2.hits.is_empty(), "empty query must return empty vec");
    }

    // ── 9. sensitivity filter ────────────────────────────────────────────────

    #[test]
    fn sensitivity_filter_excludes_high_sensitivity_rows() {
        let db = open_db();
        let conn = db.write();

        // sensitivity=1 row (should pass max_sensitivity=1 filter)
        insert_doc(
            &conn,
            "memory",
            "00000000-0000-0000-0000-000000000060",
            Some("internal document"),
            Some("internal content for sensitivity test"),
            None,
            "core",
            "default",
            1,
            "Current",
            1,
        );
        // sensitivity=3 row (should fail max_sensitivity=1 filter)
        insert_doc(
            &conn,
            "memory",
            "00000000-0000-0000-0000-000000000061",
            Some("secret document"),
            Some("secret content for sensitivity test"),
            None,
            "core",
            "default",
            3,
            "Current",
            1,
        );

        let all =
            search_documents_fts_query(&conn, "sensitivity", &Fts5SearchQuery::default()).unwrap();
        assert_eq!(all.hits.len(), 2, "both rows must be found without filter");

        let filtered = search_documents_fts_query(
            &conn,
            "sensitivity",
            &Fts5SearchQuery {
                max_sensitivity: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            filtered.hits.len(),
            1,
            "only sensitivity<=1 rows should be returned"
        );
        assert_eq!(filtered.hits[0].sensitivity, 1);
    }

    // ── 10. truth_state filter ───────────────────────────────────────────────

    #[test]
    fn truth_state_filter_excludes_non_matching_rows() {
        let db = open_db();
        let conn = db.write();

        insert_doc(
            &conn,
            "memory",
            "00000000-0000-0000-0000-000000000070",
            Some("current fact"),
            Some("this is a current fact about retrieval"),
            None,
            "core",
            "default",
            0,
            "Current",
            1,
        );
        insert_doc(
            &conn,
            "memory",
            "00000000-0000-0000-0000-000000000071",
            Some("deleted fact"),
            Some("this is a deleted fact about retrieval"),
            None,
            "core",
            "default",
            0,
            "Deleted",
            2,
        );

        let current_only = search_documents_fts_query(
            &conn,
            "retrieval",
            &Fts5SearchQuery {
                truth_state: Some("Current".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(
            current_only.hits.len(),
            1,
            "truth_state filter must exclude Deleted row"
        );
        assert_eq!(current_only.hits[0].truth_state, "Current");
    }

    // ── 11. aliases column is indexed ────────────────────────────────────────

    #[test]
    fn aliases_column_is_searchable() {
        let db = open_db();
        let conn = db.write();

        insert_doc(
            &conn,
            "entity",
            "00000000-0000-0000-0000-000000000080",
            Some("entity name"),
            None,
            Some("Linus Torvalds linux-creator kernel-dev"),
            "core",
            "default",
            0,
            "Current",
            1,
        );

        let r =
            search_documents_fts_query(&conn, "kernel-dev", &Fts5SearchQuery::default()).unwrap();
        assert_eq!(r.hits.len(), 1, "alias text must be indexed and searchable");
    }

    // ── 12. result limit is honoured ─────────────────────────────────────────

    #[test]
    fn result_limit_is_honoured() {
        let db = open_db();
        let conn = db.write();

        for i in 0u32..10 {
            insert_doc(
                &conn,
                "memory",
                &format!("00000000-0000-0000-0000-{i:012}"),
                Some("limit test row common word"),
                Some("body containing the word limittest for this row"),
                None,
                "core",
                "default",
                0,
                "Current",
                i as i64 + 1,
            );
        }

        let limited = search_documents_fts_query(
            &conn,
            "limittest",
            &Fts5SearchQuery {
                limit: Some(3),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(limited.hits.len(), 3, "limit must cap the result set");
    }

    // ── 13. FTS5 table exists after migration ────────────────────────────────

    #[test]
    fn fts5_table_exists_after_migration() {
        let db = open_db();
        let conn = db.write();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='search_documents_fts'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "search_documents_fts virtual table must exist after migration"
        );
    }

    // ── 14. matched_field is non-None when title contains the search term ────

    #[test]
    fn matched_field_set_for_title_match() {
        let db = open_db();
        let conn = db.write();

        insert_doc(
            &conn,
            "memory",
            "00000000-0000-0000-0000-000000000090",
            Some("uniquetitleterm for matching"),
            Some("body does not contain it"),
            None,
            "core",
            "default",
            0,
            "Current",
            1,
        );

        let r = search_documents_fts_query(&conn, "uniquetitleterm", &Fts5SearchQuery::default())
            .unwrap();
        assert_eq!(r.hits.len(), 1);
        let h = &r.hits[0];
        assert!(
            h.matched_field.is_some(),
            "matched_field must be Some when a column contains the search term"
        );
        // Title is the first checked column, so it should be selected.
        assert_eq!(
            h.matched_field.as_deref(),
            Some("title"),
            "matched_field must be 'title' when title contains the term"
        );
    }

    // ── 15. navigation_target is correct for each record_kind ───────────────

    #[test]
    fn navigation_target_correct_for_all_kinds() {
        let cases: &[(&str, &str)] = &[
            ("entity", "knowledge/entity/"),
            ("memory", "knowledge/record/"),
            ("summary", "knowledge/record/"),
            ("skill", "knowledge/record/"),
            ("rule", "knowledge/record/"),
            ("source", "sources/"),
            ("goal", "goals/"),
            ("relationship", "knowledge/relationship/"),
        ];

        let db = open_db();
        let conn = db.write();
        let id = "aaaabbbb-cccc-dddd-eeee-ffffffffffff";

        for (kind, expected_prefix) in cases {
            // Insert a fresh doc for this kind (delete first to avoid PK conflicts).
            conn.execute(
                "DELETE FROM search_documents WHERE record_kind=?1 AND record_id=?2",
                rusqlite::params![kind, id],
            )
            .unwrap();

            let doc = SearchDocument {
                record_kind: kind.to_string(),
                record_id: id.to_string(),
                title: Some(format!("navtest {kind} navunique")),
                body: None,
                aliases: None,
                source_text: None,
                relation_text: None,
                namespace: "core".to_string(),
                owner_id: "u".to_string(),
                scope: "default".to_string(),
                sensitivity: 0,
                truth_state: "Current".to_string(),
                valid_from: None,
                valid_until: None,
                content_hash: format!("h-{kind}"),
                revision: 1,
            };
            upsert_search_document(&conn, &doc).unwrap();

            let r = search_documents_fts_query(&conn, "navunique", &Fts5SearchQuery::default())
                .unwrap();
            // There could be multiple hits from previous loop iterations if IDs differ,
            // so find the hit for this kind.
            let hit = r
                .hits
                .iter()
                .find(|h| h.record_kind == *kind)
                .unwrap_or_else(|| panic!("no hit for kind={kind}"));
            let nav = hit
                .navigation_target
                .as_deref()
                .unwrap_or_else(|| panic!("navigation_target is None for kind={kind}"));
            assert!(
                nav.starts_with(expected_prefix),
                "kind={kind}: expected prefix '{expected_prefix}' but got '{nav}'"
            );
            assert!(
                nav.ends_with(id),
                "kind={kind}: navigation_target must end with record_id, got '{nav}'"
            );
        }
    }

    // ── 16. TotalSemantics::AtLeast when results equal the limit ─────────────

    #[test]
    fn total_semantics_at_least_when_at_limit() {
        let db = open_db();
        let conn = db.write();

        // Insert exactly 5 rows that all match "atleasttest".
        for i in 0u32..5 {
            insert_doc(
                &conn,
                "memory",
                &format!("11111111-1111-1111-1111-{i:012}"),
                Some("atleasttest common term"),
                None,
                None,
                "core",
                "default",
                0,
                "Current",
                i as i64 + 1,
            );
        }

        // Limit=5 exactly — we get 5 rows back, so we cannot guarantee there are no more.
        let r = search_documents_fts_query(
            &conn,
            "atleasttest",
            &Fts5SearchQuery {
                limit: Some(5),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(r.hits.len(), 5, "should return exactly 5 hits");
        assert_eq!(
            r.total_semantics,
            TotalSemantics::AtLeast(5),
            "when hit count == limit, semantics must be AtLeast"
        );
    }

    // ── 17. TotalSemantics::Exact when fewer results than limit ──────────────

    #[test]
    fn total_semantics_exact_when_below_limit() {
        let db = open_db();
        let conn = db.write();

        insert_doc(
            &conn,
            "memory",
            "22222222-2222-2222-2222-222222222222",
            Some("exactsemantics unique term"),
            None,
            None,
            "core",
            "default",
            0,
            "Current",
            1,
        );

        // limit=25 (default) but only 1 row matches.
        let r = search_documents_fts_query(&conn, "exactsemantics", &Fts5SearchQuery::default())
            .unwrap();

        assert_eq!(r.hits.len(), 1);
        assert_eq!(
            r.total_semantics,
            TotalSemantics::Exact(1),
            "when hit count < limit, semantics must be Exact"
        );
    }

    // ── 18. rank_rationale is non-empty and mentions BM25 ────────────────────

    #[test]
    fn rank_rationale_mentions_bm25() {
        let db = open_db();
        let conn = db.write();

        insert_doc(
            &conn,
            "memory",
            "33333333-3333-3333-3333-333333333333",
            Some("bm25rationale unique search term"),
            None,
            None,
            "core",
            "default",
            0,
            "Current",
            1,
        );

        let r = search_documents_fts_query(&conn, "bm25rationale", &Fts5SearchQuery::default())
            .unwrap();
        assert_eq!(r.hits.len(), 1);
        let rationale = &r.hits[0].rank_rationale;
        assert!(!rationale.is_empty(), "rank_rationale must not be empty");
        assert!(
            rationale.to_lowercase().contains("bm25"),
            "rank_rationale must mention BM25, got: '{rationale}'"
        );
    }

    // ── 19. policy_summary fields match the inserted row ─────────────────────

    #[test]
    fn policy_summary_matches_inserted_row() {
        let db = open_db();
        let conn = db.write();

        let doc = SearchDocument {
            record_kind: "skill".to_string(),
            record_id: "44444444-4444-4444-4444-444444444444".to_string(),
            title: Some("policytest unique term skillrow".to_string()),
            body: None,
            aliases: None,
            source_text: None,
            relation_text: None,
            namespace: "plugin/acme".to_string(),
            owner_id: "u".to_string(),
            scope: "workspace-42".to_string(),
            sensitivity: 2,
            truth_state: "Current".to_string(),
            valid_from: None,
            valid_until: None,
            content_hash: "h-skill-policy".to_string(),
            revision: 3,
        };
        upsert_search_document(&conn, &doc).unwrap();

        let r =
            search_documents_fts_query(&conn, "policytest", &Fts5SearchQuery::default()).unwrap();
        assert_eq!(r.hits.len(), 1);
        let ps = &r.hits[0].policy_summary;
        assert_eq!(ps.namespace, "plugin/acme");
        assert_eq!(ps.scope, "workspace-42");
        assert_eq!(ps.sensitivity, 2);
    }
}
