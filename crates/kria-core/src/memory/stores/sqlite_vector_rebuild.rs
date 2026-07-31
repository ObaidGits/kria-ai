//! Authority outbox semantics, temporary-generation rebuild, membership hash,
//! and model migration cursor for the v2 vector partition (F3.1 / task 3.1.5).
//!
//! ## Key types
//! * [`compute_membership_hash`] — deterministic SHA-256 over sorted record_ids
//!   in a partition (design §4.4 / membership hash contract).
//! * [`VectorOutboxEntry`] / [`VectorOutboxProcessor`] — reads pending rows from
//!   `derived_outbox` for `target='vectors'`, drives upsert_v2 / delete_v2, and
//!   marks rows `status='applied'` on success or `status='dead_letter'` after
//!   MAX_ATTEMPTS retries.
//! * [`RebuildCursor`] — tracks one in-progress or interrupted rebuild in the
//!   `rebuild_cursor` table.
//! * [`rebuild_partition`] — full temporary-generation rebuild: creates a staging
//!   table, streams records, atomically activates, writes `derived_manifests`,
//!   and marks the cursor `'activated'`.  On interruption: marks `'interrupted'`
//!   and leaves the staging table for resume.

use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::stores::sqlite_vectors::{PartitionId, SqliteVectorStore, VectorPayloadV2};

// ─── constants ────────────────────────────────────────────────────────────────

/// Maximum delivery attempts before an outbox entry is moved to dead_letter.
pub const MAX_ATTEMPTS: u32 = 5;

/// The `target` value used in `derived_outbox` for vector index operations.
pub const OUTBOX_TARGET_VECTORS: &str = "vectors";

// ─── compute_membership_hash ─────────────────────────────────────────────────

/// Compute the deterministic SHA-256 membership hash for a partition.
///
/// Queries `mem_vectors_v2` for all `record_id` values in `partition_id`,
/// ordered lexicographically (`ORDER BY record_id ASC`), then concatenates
/// them with a newline delimiter and returns the hex SHA-256 digest.
///
/// An empty partition hashes the empty byte string → the well-known SHA-256 of
/// the empty string: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
pub fn compute_membership_hash(
    conn: &Connection,
    partition_id: &PartitionId,
) -> MemoryResult<String> {
    let mut stmt = conn
        .prepare(
            "SELECT record_id FROM mem_vectors_v2 \
             WHERE partition_id = ?1 \
             ORDER BY record_id ASC",
        )
        .map_err(StorageError::Sqlite)?;

    let rows = stmt
        .query_map(params![partition_id.as_str()], |r| r.get::<_, String>(0))
        .map_err(StorageError::Sqlite)?;

    let mut hasher = Sha256::new();
    let mut first = true;
    for row in rows {
        let record_id = row.map_err(StorageError::Sqlite)?;
        if !first {
            hasher.update(b"\n");
        }
        hasher.update(record_id.as_bytes());
        first = false;
    }

    Ok(hex::encode(hasher.finalize()))
}

// ─── VectorOutboxEntry ───────────────────────────────────────────────────────

/// A single pending row from `derived_outbox` with `target='vectors'`.
#[derive(Debug, Clone)]
pub struct VectorOutboxEntry {
    /// Row id in `derived_outbox`.
    pub id: i64,
    /// `'upsert'` or `'delete'`.
    pub op: String,
    /// The record kind (e.g. `'memory'`).
    pub record_kind: Option<String>,
    /// The record id string (UUID).
    pub record_id: Option<String>,
    /// The content hash that was embedded.
    pub content_hash: Option<String>,
    /// The partition this vector belongs to.
    pub model_partition: Option<String>,
    /// The authority revision that originated this entry.
    pub authority_revision: Option<i64>,
    /// How many times delivery has already been attempted.
    pub attempts: u32,
}

/// Fetch up to `limit` pending outbox entries for `target='vectors'`.
///
/// Only entries with `status='pending'` and whose backoff window has elapsed
/// (`next_attempt_at IS NULL OR next_attempt_at <= now`) are returned.
pub fn fetch_pending_vector_outbox(
    conn: &Connection,
    limit: usize,
) -> MemoryResult<Vec<VectorOutboxEntry>> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut stmt = conn
        .prepare(
            "SELECT id, op, record_kind, record_id, content_hash, \
                    model_partition, authority_revision, attempts \
             FROM derived_outbox \
             WHERE target = ?1 \
               AND status = 'pending' \
               AND (next_attempt_at IS NULL OR next_attempt_at <= ?2) \
             ORDER BY id ASC \
             LIMIT ?3",
        )
        .map_err(StorageError::Sqlite)?;

    let rows = stmt
        .query_map(params![OUTBOX_TARGET_VECTORS, now, limit as i64], |r| {
            Ok(VectorOutboxEntry {
                id: r.get(0)?,
                op: r.get(1)?,
                record_kind: r.get(2)?,
                record_id: r.get(3)?,
                content_hash: r.get(4)?,
                model_partition: r.get(5)?,
                authority_revision: r.get(6)?,
                attempts: r.get::<_, i64>(7)? as u32,
            })
        })
        .map_err(StorageError::Sqlite)?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(StorageError::Sqlite)?);
    }
    Ok(out)
}

/// Mark a `derived_outbox` row as applied (`status='applied'`) or move it to
/// `status='dead_letter'` after enough retries.
///
/// On success: sets `status='applied'`, increments `attempts`.
/// On transient failure (`error_code` is `Some`): increments `attempts`,
/// sets `error_code`, and schedules `next_attempt_at` with exponential backoff
/// (base 10 s, doubling per attempt, capped at 10 min).  If `attempts` reaches
/// [`MAX_ATTEMPTS`], sets `status='dead_letter'` instead.
pub fn mark_outbox_applied(conn: &Connection, id: i64, new_attempts: u32) -> MemoryResult<()> {
    conn.execute(
        "UPDATE derived_outbox \
         SET status = 'applied', attempts = ?2, next_attempt_at = NULL, error_code = NULL \
         WHERE id = ?1",
        params![id, new_attempts as i64],
    )
    .map_err(StorageError::Sqlite)?;
    Ok(())
}

/// Mark a `derived_outbox` row as failed, scheduling backoff or dead_letter.
pub fn mark_outbox_failed(
    conn: &Connection,
    id: i64,
    new_attempts: u32,
    error_code: &str,
) -> MemoryResult<()> {
    if new_attempts >= MAX_ATTEMPTS {
        conn.execute(
            "UPDATE derived_outbox \
             SET status = 'dead_letter', attempts = ?2, error_code = ?3 \
             WHERE id = ?1",
            params![id, new_attempts as i64, error_code],
        )
        .map_err(StorageError::Sqlite)?;
    } else {
        // Exponential backoff: 10s × 2^(attempts-1), capped at 600s.
        let delay_secs = (10u64 * (1u64 << (new_attempts.saturating_sub(1) as u64))).min(600);
        let next_attempt_at =
            (chrono::Utc::now() + chrono::Duration::seconds(delay_secs as i64)).to_rfc3339();
        conn.execute(
            "UPDATE derived_outbox \
             SET status = 'pending', attempts = ?2, next_attempt_at = ?3, error_code = ?4 \
             WHERE id = ?1",
            params![id, new_attempts as i64, next_attempt_at, error_code],
        )
        .map_err(StorageError::Sqlite)?;
    }
    Ok(())
}

// ─── VectorOutboxProcessor ───────────────────────────────────────────────────

/// Drives the `derived_outbox` relay for `target='vectors'`.
///
/// On each `process_batch` call:
/// 1. Fetches up to `batch_size` pending entries.
/// 2. For each `upsert` entry: calls `SqliteVectorStore::upsert_v2` with the
///    provided `build_payload` closure that resolves the full `VectorPayloadV2`
///    and vector bytes from the `record_id` and `content_hash`.
/// 3. For each `delete` entry: calls `SqliteVectorStore::delete_v2`.
/// 4. On success: marks the entry `applied`.  On error: increments attempts
///    and schedules backoff or promotes to `dead_letter`.
///
/// The processor does not own an async runtime — it is designed to be called
/// from an async task that drives it periodically (e.g. the authority reconciler).
pub struct VectorOutboxProcessor {
    db: Arc<Database>,
    store: Arc<SqliteVectorStore>,
}

impl VectorOutboxProcessor {
    pub fn new(db: Arc<Database>) -> Self {
        let store = Arc::new(SqliteVectorStore::new(db.clone()));
        Self { db, store }
    }

    /// Process up to `batch_size` pending outbox entries.
    ///
    /// `resolve_upsert` is called for each `op='upsert'` entry; it must return
    /// the `(vector, VectorPayloadV2)` to embed, or `None` if the record no
    /// longer exists (entry is marked applied without doing work).
    pub async fn process_batch<F, Fut>(
        &self,
        batch_size: usize,
        resolve_upsert: F,
    ) -> MemoryResult<ProcessBatchResult>
    where
        F: Fn(String, PartitionId) -> Fut,
        Fut: std::future::Future<Output = MemoryResult<Option<(Vec<f32>, VectorPayloadV2)>>>,
    {
        // Fetch pending entries (synchronous read).
        let entries = self
            .db
            .with_read(|conn| fetch_pending_vector_outbox(conn, batch_size))?;

        let mut result = ProcessBatchResult::default();

        for entry in entries {
            let outcome = self.process_one(&entry, &resolve_upsert).await;
            match outcome {
                Ok(ProcessedAs::Applied) => result.applied += 1,
                Ok(ProcessedAs::Skipped) => result.skipped += 1,
                Ok(ProcessedAs::DeadLetter) => result.dead_letter += 1,
                Err(e) => {
                    tracing::warn!(
                        outbox_id = entry.id,
                        error = %e,
                        "vector outbox entry processing error"
                    );
                    result.errors += 1;
                }
            }
        }

        Ok(result)
    }

    async fn process_one<F, Fut>(
        &self,
        entry: &VectorOutboxEntry,
        resolve_upsert: &F,
    ) -> MemoryResult<ProcessedAs>
    where
        F: Fn(String, PartitionId) -> Fut,
        Fut: std::future::Future<Output = MemoryResult<Option<(Vec<f32>, VectorPayloadV2)>>>,
    {
        let new_attempts = entry.attempts + 1;

        if entry.op == "delete" {
            // For delete: partition_id comes from model_partition.
            let partition_id = match &entry.model_partition {
                Some(p) => PartitionId::from_raw(p.clone()),
                None => {
                    // No partition → nothing to delete; mark applied.
                    let conn = self.db.write();
                    mark_outbox_applied(&conn, entry.id, new_attempts)?;
                    return Ok(ProcessedAs::Applied);
                }
            };
            let record_id = match &entry.record_id {
                Some(r) => r.clone(),
                None => {
                    let conn = self.db.write();
                    mark_outbox_applied(&conn, entry.id, new_attempts)?;
                    return Ok(ProcessedAs::Skipped);
                }
            };
            let uuid = Uuid::parse_str(&record_id)
                .map_err(|e| StorageError::Serde(format!("bad uuid in outbox: {e}")))?;

            match self.store.delete_v2(&partition_id, &[uuid]).await {
                Ok(()) => {
                    let conn = self.db.write();
                    mark_outbox_applied(&conn, entry.id, new_attempts)?;
                    Ok(ProcessedAs::Applied)
                }
                Err(e) => {
                    let conn = self.db.write();
                    let code = format!("delete_failed: {e}");
                    mark_outbox_failed(&conn, entry.id, new_attempts, &code)?;
                    if new_attempts >= MAX_ATTEMPTS {
                        Ok(ProcessedAs::DeadLetter)
                    } else {
                        Ok(ProcessedAs::Skipped)
                    }
                }
            }
        } else {
            // op == "upsert"
            let partition_id = match &entry.model_partition {
                Some(p) => PartitionId::from_raw(p.clone()),
                None => {
                    let conn = self.db.write();
                    mark_outbox_failed(&conn, entry.id, new_attempts, "missing_model_partition")?;
                    return Ok(if new_attempts >= MAX_ATTEMPTS {
                        ProcessedAs::DeadLetter
                    } else {
                        ProcessedAs::Skipped
                    });
                }
            };
            let record_id = match entry.record_id.clone() {
                Some(r) => r,
                None => {
                    let conn = self.db.write();
                    mark_outbox_failed(&conn, entry.id, new_attempts, "missing_record_id")?;
                    return Ok(ProcessedAs::Skipped);
                }
            };

            match resolve_upsert(record_id, partition_id).await? {
                None => {
                    // Record deleted before we could embed it — mark applied.
                    let conn = self.db.write();
                    mark_outbox_applied(&conn, entry.id, new_attempts)?;
                    Ok(ProcessedAs::Applied)
                }
                Some((vector, payload)) => {
                    let uuid = Uuid::parse_str(entry.record_id.as_deref().unwrap_or(""))
                        .map_err(|e| StorageError::Serde(format!("bad uuid: {e}")))?;
                    match self.store.upsert_v2(uuid, &vector, &payload).await {
                        Ok(()) => {
                            let conn = self.db.write();
                            mark_outbox_applied(&conn, entry.id, new_attempts)?;
                            Ok(ProcessedAs::Applied)
                        }
                        Err(e) => {
                            let conn = self.db.write();
                            let code = format!("upsert_failed: {e}");
                            mark_outbox_failed(&conn, entry.id, new_attempts, &code)?;
                            Ok(if new_attempts >= MAX_ATTEMPTS {
                                ProcessedAs::DeadLetter
                            } else {
                                ProcessedAs::Skipped
                            })
                        }
                    }
                }
            }
        }
    }
}

/// Outcome of processing a single outbox entry.
#[derive(Debug, PartialEq, Eq)]
enum ProcessedAs {
    Applied,
    Skipped,
    DeadLetter,
}

/// Summary returned by [`VectorOutboxProcessor::process_batch`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProcessBatchResult {
    pub applied: usize,
    pub skipped: usize,
    pub dead_letter: usize,
    pub errors: usize,
}

// ─── enqueue_vector_outbox ───────────────────────────────────────────────────

/// Enqueue a vector upsert or delete into `derived_outbox` within an existing
/// transaction.  Called by `AuthorityTx` write paths when a memory record is
/// written so the vector index stays in sync via the outbox relay.
///
/// Uses `INSERT OR IGNORE` against the semantic uniqueness index so duplicate
/// in-flight entries for the same `(target, op, record_kind, record_id,
/// content_hash, model_partition)` tuple are silently coalesced.
pub fn enqueue_vector_outbox(
    conn: &Connection,
    op: &str,
    record_kind: &str,
    record_id: &str,
    content_hash: &str,
    model_partition: Option<&str>,
    authority_revision: Option<i64>,
) -> MemoryResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR IGNORE INTO derived_outbox \
         (target, op, record_kind, record_id, content_hash, \
          model_partition, authority_revision, attempts, status, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 'pending', ?8)",
        params![
            OUTBOX_TARGET_VECTORS,
            op,
            record_kind,
            record_id,
            content_hash,
            model_partition,
            authority_revision,
            now,
        ],
    )
    .map_err(StorageError::Sqlite)?;
    Ok(())
}

// ─── RebuildCursor ───────────────────────────────────────────────────────────

/// A row in the `rebuild_cursor` table tracking one rebuild run.
#[derive(Debug, Clone)]
pub struct RebuildCursor {
    pub partition_id: String,
    pub run_id: String,
    /// Last `record_id` successfully staged; `None` if no rows processed yet.
    pub last_record_id: Option<String>,
    pub status: RebuildStatus,
    /// For model-migration rebuilds: the old partition being migrated FROM.
    pub migration_source_partition_id: Option<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Status of a rebuild run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebuildStatus {
    Running,
    Interrupted,
    Activated,
}

impl RebuildStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RebuildStatus::Running => "running",
            RebuildStatus::Interrupted => "interrupted",
            RebuildStatus::Activated => "activated",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "running" => Some(Self::Running),
            "interrupted" => Some(Self::Interrupted),
            "activated" => Some(Self::Activated),
            _ => None,
        }
    }
}

/// Insert or replace a `rebuild_cursor` row.
pub fn upsert_rebuild_cursor(conn: &Connection, cursor: &RebuildCursor) -> MemoryResult<()> {
    conn.execute(
        "INSERT INTO rebuild_cursor \
         (partition_id, run_id, last_record_id, status, \
          migration_source_partition_id, started_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(partition_id) DO UPDATE SET \
             run_id = excluded.run_id, \
             last_record_id = excluded.last_record_id, \
             status = excluded.status, \
             migration_source_partition_id = excluded.migration_source_partition_id, \
             started_at = excluded.started_at, \
             updated_at = excluded.updated_at",
        params![
            cursor.partition_id,
            cursor.run_id,
            cursor.last_record_id,
            cursor.status.as_str(),
            cursor.migration_source_partition_id,
            cursor.started_at.to_rfc3339(),
            cursor.updated_at.to_rfc3339(),
        ],
    )
    .map_err(StorageError::Sqlite)?;
    Ok(())
}

/// Load the current `rebuild_cursor` row for a partition, if any.
pub fn load_rebuild_cursor(
    conn: &Connection,
    partition_id: &str,
) -> MemoryResult<Option<RebuildCursor>> {
    let row = conn
        .query_row(
            "SELECT partition_id, run_id, last_record_id, status, \
                migration_source_partition_id, started_at, updated_at \
         FROM rebuild_cursor WHERE partition_id = ?1",
            params![partition_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            },
        )
        .optional()
        .map_err(StorageError::Sqlite)?;

    match row {
        None => Ok(None),
        Some((pid, run_id, last_rec, status_str, mig_src, started_str, updated_str)) => {
            let parse_ts = |s: &str| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .map_err(|e| StorageError::Serde(format!("bad timestamp {s:?}: {e}")))
            };
            let status = RebuildStatus::from_str(&status_str).ok_or_else(|| {
                StorageError::Serde(format!("unknown rebuild status: {status_str}"))
            })?;
            Ok(Some(RebuildCursor {
                partition_id: pid,
                run_id,
                last_record_id: last_rec,
                status,
                migration_source_partition_id: mig_src,
                started_at: parse_ts(&started_str)?,
                updated_at: parse_ts(&updated_str)?,
            }))
        }
    }
}

// ─── DerivedManifestRow ──────────────────────────────────────────────────────

/// A row to write into `derived_manifests` after a successful rebuild.
#[derive(Debug, Clone)]
pub struct DerivedManifestRow {
    /// The `partition_id` used as the `version` column.
    pub partition_id: String,
    /// The last authority revision included in this manifest.
    pub authority_revision: Option<i64>,
    /// Total rows in `mem_vectors_v2` for this partition.
    pub member_count: i64,
    /// Deterministic SHA-256 over sorted record_ids.
    pub membership_hash: String,
    /// Algorithm identifier, e.g. `'exact-cosine-f32le'`.
    pub algorithm: String,
    /// The `model_id` of the partition.
    pub model_version: String,
    /// RFC3339 UTC timestamp when the rebuild completed.
    pub completed_at: String,
}

/// Write a `derived_manifests` row for `target='vectors'`.
///
/// Uses `INSERT OR REPLACE` so a subsequent rebuild naturally supersedes
/// the previous active manifest for the same partition.
pub fn write_derived_manifest(conn: &Connection, row: &DerivedManifestRow) -> MemoryResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO derived_manifests \
         (target, version, authority_revision, member_count, membership_hash, \
          algorithm_version, model_version, completed_at, status) \
         VALUES ('vectors', ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active')",
        params![
            row.partition_id,
            row.authority_revision,
            row.member_count,
            row.membership_hash,
            row.algorithm,
            row.model_version,
            row.completed_at,
        ],
    )
    .map_err(StorageError::Sqlite)?;
    Ok(())
}

// ─── Rebuild record stream item ───────────────────────────────────────────────

/// A single record ready to be staged during a rebuild.
pub struct RebuildRecord {
    pub record_id: String,
    pub vector: Vec<f32>,
    pub content_hash: String,
    pub namespace: String,
    pub owner_id: String,
    pub scope: String,
    pub sensitivity: i64,
    pub truth_state: String,
    pub revision: i64,
}

// ─── rebuild_partition ───────────────────────────────────────────────────────

/// Result of a `rebuild_partition` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebuildOutcome {
    /// Rebuild completed and the partition was atomically activated.
    Activated {
        member_count: i64,
        membership_hash: String,
    },
    /// Rebuild was interrupted mid-stream.  The staging table and cursor row
    /// are left in place for resume.
    Interrupted { last_record_id: Option<String> },
}

/// Build or resume a temporary-generation rebuild of `partition_id`.
///
/// ## Flow
/// 1. Checks `rebuild_cursor` for an existing `run_id` to resume, or generates
///    a fresh `run_id` (UUID v4).
/// 2. Creates the staging table `mem_vectors_v2_gen_{run_id_hex}` if it doesn't
///    exist (same schema as `mem_vectors_v2`, no FK constraints so it can hold
///    partial data safely).
/// 3. Streams `records` into the staging table.  Each row is checkpointed into
///    `rebuild_cursor.last_record_id` every `checkpoint_every` rows.
/// 4. On completion: atomically activates the staging table by executing
///    `INSERT OR REPLACE INTO mem_vectors_v2 ... SELECT ... FROM staging` then
///    `DELETE FROM mem_vectors_v2 WHERE partition_id = ? AND record_id NOT IN
///    (SELECT record_id FROM staging)`, then drops the staging table.
/// 5. Writes a `derived_manifests` row and marks the cursor `'activated'`.
/// 6. On `records` returning `Err` or any mid-stream DB error: marks cursor
///    `'interrupted'`, leaves staging table, returns `RebuildOutcome::Interrupted`.
///
/// ### Model migration
/// When `migration_source_partition_id` is `Some`, the cursor row records the
/// old partition being migrated from.  The caller controls which records to
/// stream; this function just persists the migration metadata.
///
/// ### Resume
/// If an `'interrupted'` cursor exists for the same partition, the caller
/// should pass a `records` iterator that resumes from `cursor.last_record_id`
/// (filtering `record_id > last_record_id`).  This function will reuse the
/// same `run_id` and staging table.
pub fn rebuild_partition(
    db: &Database,
    partition_id: &PartitionId,
    authority_revision: Option<i64>,
    model_id: &str,
    migration_source_partition_id: Option<String>,
    checkpoint_every: usize,
    records: impl Iterator<Item = MemoryResult<RebuildRecord>>,
) -> MemoryResult<RebuildOutcome> {
    let pid = partition_id.as_str().to_string();
    let now = chrono::Utc::now();

    // 1. Determine run_id (resume if interrupted cursor exists).
    let (run_id, resume) = {
        let conn = db.write();
        let existing = load_rebuild_cursor(&conn, &pid)?;
        match existing {
            Some(c) if c.status == RebuildStatus::Interrupted => (c.run_id.clone(), true),
            _ => (Uuid::new_v4().to_string(), false),
        }
    };

    // Sanitize run_id for use in a table name (replace hyphens with underscores).
    let run_id_safe = run_id.replace('-', "_");
    let staging_table = format!("mem_vectors_v2_gen_{run_id_safe}");

    // 2. Create the staging table (idempotent — IF NOT EXISTS).
    {
        let conn = db.write();
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {staging_table} ( \
                 partition_id TEXT    NOT NULL, \
                 record_id    TEXT    NOT NULL, \
                 vector       BLOB    NOT NULL, \
                 content_hash TEXT    NOT NULL, \
                 namespace    TEXT    NOT NULL, \
                 owner_id     TEXT    NOT NULL, \
                 scope        TEXT    NOT NULL, \
                 sensitivity  INTEGER NOT NULL, \
                 truth_state  TEXT    NOT NULL, \
                 revision     INTEGER NOT NULL, \
                 PRIMARY KEY (partition_id, record_id) \
             );"
        ))
        .map_err(StorageError::Sqlite)?;
    }

    // Write initial cursor row.
    if !resume {
        let conn = db.write();
        upsert_rebuild_cursor(
            &conn,
            &RebuildCursor {
                partition_id: pid.clone(),
                run_id: run_id.clone(),
                last_record_id: None,
                status: RebuildStatus::Running,
                migration_source_partition_id: migration_source_partition_id.clone(),
                started_at: now,
                updated_at: now,
            },
        )?;
    } else {
        // Re-activate a previously interrupted cursor.
        let conn = db.write();
        conn.execute(
            "UPDATE rebuild_cursor SET status = 'running', updated_at = ?2 \
             WHERE partition_id = ?1",
            params![pid, now.to_rfc3339()],
        )
        .map_err(StorageError::Sqlite)?;
    }

    // 3. Stream records into staging table with periodic checkpointing.
    let mut last_record_id: Option<String> = None;
    let mut row_count: usize = 0;

    for record_result in records {
        let record = match record_result {
            Ok(r) => r,
            Err(_e) => {
                // Interrupted mid-stream.
                let conn = db.write();
                let _ = conn.execute(
                    "UPDATE rebuild_cursor SET status = 'interrupted', updated_at = ?2 \
                     WHERE partition_id = ?1",
                    params![pid, chrono::Utc::now().to_rfc3339()],
                );
                return Ok(RebuildOutcome::Interrupted {
                    last_record_id: last_record_id.clone(),
                });
            }
        };

        // Insert into staging.
        {
            let conn = db.write();
            let blob: Vec<u8> = record.vector.iter().flat_map(|f| f.to_le_bytes()).collect();
            let insert_result = conn.execute(
                &format!(
                    "INSERT OR REPLACE INTO {staging_table} \
                     (partition_id, record_id, vector, content_hash, \
                      namespace, owner_id, scope, sensitivity, truth_state, revision) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
                ),
                params![
                    pid,
                    record.record_id,
                    blob,
                    record.content_hash,
                    record.namespace,
                    record.owner_id,
                    record.scope,
                    record.sensitivity,
                    record.truth_state,
                    record.revision,
                ],
            );

            if let Err(_e) = insert_result {
                let conn2 = db.write();
                let _ = conn2.execute(
                    "UPDATE rebuild_cursor SET status = 'interrupted', updated_at = ?2 \
                     WHERE partition_id = ?1",
                    params![pid, chrono::Utc::now().to_rfc3339()],
                );
                return Ok(RebuildOutcome::Interrupted {
                    last_record_id: last_record_id.clone(),
                });
            }
        }

        last_record_id = Some(record.record_id.clone());
        row_count += 1;

        // Checkpoint.
        if checkpoint_every > 0 && row_count % checkpoint_every == 0 {
            let conn = db.write();
            let _ = conn.execute(
                "UPDATE rebuild_cursor SET last_record_id = ?2, updated_at = ?3 \
                 WHERE partition_id = ?1",
                params![pid, &last_record_id, chrono::Utc::now().to_rfc3339()],
            );
        }
    }

    // 4. Atomic activation: copy staging → main, delete stale rows, drop staging.
    {
        let conn = db.write();
        conn.execute_batch(&format!(
            "BEGIN;
             INSERT OR REPLACE INTO mem_vectors_v2
                 (partition_id, record_id, vector, content_hash,
                  namespace, owner_id, scope, sensitivity, truth_state, revision)
             SELECT partition_id, record_id, vector, content_hash,
                    namespace, owner_id, scope, sensitivity, truth_state, revision
             FROM {staging_table};
             DELETE FROM mem_vectors_v2
             WHERE partition_id = '{pid}'
               AND record_id NOT IN (SELECT record_id FROM {staging_table});
             DROP TABLE IF EXISTS {staging_table};
             COMMIT;"
        ))
        .map_err(StorageError::Sqlite)?;
    }

    // 5. Compute membership hash and member count from the now-active partition.
    let (membership_hash, member_count) = {
        let conn = db.write();
        let hash = compute_membership_hash(&conn, partition_id)?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM mem_vectors_v2 WHERE partition_id = ?1",
                params![pid],
                |r| r.get(0),
            )
            .map_err(StorageError::Sqlite)?;
        (hash, count)
    };

    // Write derived_manifests row.
    {
        let conn = db.write();
        write_derived_manifest(
            &conn,
            &DerivedManifestRow {
                partition_id: pid.clone(),
                authority_revision,
                member_count,
                membership_hash: membership_hash.clone(),
                algorithm: "exact-cosine-f32le".to_string(),
                model_version: model_id.to_string(),
                completed_at: chrono::Utc::now().to_rfc3339(),
            },
        )?;
    }

    // Mark cursor activated.
    {
        let conn = db.write();
        conn.execute(
            "UPDATE rebuild_cursor \
             SET status = 'activated', last_record_id = ?2, updated_at = ?3 \
             WHERE partition_id = ?1",
            params![pid, &last_record_id, chrono::Utc::now().to_rfc3339()],
        )
        .map_err(StorageError::Sqlite)?;
    }

    Ok(RebuildOutcome::Activated {
        member_count,
        membership_hash,
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::Database;
    use crate::memory::stores::manifest::EmbeddingPartitionManifest;
    use crate::memory::stores::sqlite_vectors::ensure_partition;
    use std::sync::Arc;

    fn open_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    fn canonical_manifest() -> EmbeddingPartitionManifest {
        EmbeddingPartitionManifest::canonical()
    }

    fn ensure_canonical_partition(db: &Arc<Database>) -> PartitionId {
        let m = canonical_manifest();
        let conn = db.write();
        ensure_partition(&conn, &m).unwrap()
    }

    /// Insert a vector row directly into mem_vectors_v2.
    fn insert_vector_row(conn: &Connection, partition_id: &str, record_id: &str) {
        let blob: Vec<u8> = vec![0u8; 1536];
        conn.execute(
            "INSERT INTO mem_vectors_v2 \
             (partition_id, record_id, vector, content_hash, \
              namespace, owner_id, scope, sensitivity, truth_state, revision) \
             VALUES (?1, ?2, ?3, 'hash', 'ns', 'owner', 'global', 0, 'Current', 1)",
            params![partition_id, record_id, blob],
        )
        .unwrap();
    }

    // ── compute_membership_hash ──────────────────────────────────────────────

    /// SHA-256 of empty string (no rows).
    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb924\
         27ae41e4649b934ca495991b7852b855";

    #[test]
    fn membership_hash_empty_partition_is_known() {
        let db = open_db();
        let pid = ensure_canonical_partition(&db);
        let conn = db.write();
        let hash = compute_membership_hash(&conn, &pid).unwrap();
        assert_eq!(hash, EMPTY_SHA256);
    }

    #[test]
    fn membership_hash_is_deterministic_regardless_of_insert_order() {
        // Insert 3 rows in two databases with different insert orders.
        // Both should produce the same membership hash.
        let db1 = open_db();
        let db2 = open_db();

        let pid1 = ensure_canonical_partition(&db1);
        let pid2 = ensure_canonical_partition(&db2);

        let ids = vec!["c-uuid", "a-uuid", "b-uuid"];
        // db1: insert in c, a, b order
        {
            let conn = db1.write();
            for id in &ids {
                insert_vector_row(&conn, pid1.as_str(), id);
            }
        }
        // db2: insert in a, b, c order
        {
            let conn = db2.write();
            for id in ["a-uuid", "b-uuid", "c-uuid"] {
                insert_vector_row(&conn, pid2.as_str(), id);
            }
        }

        let hash1 = {
            let conn = db1.write();
            compute_membership_hash(&conn, &pid1).unwrap()
        };
        let hash2 = {
            let conn = db2.write();
            compute_membership_hash(&conn, &pid2).unwrap()
        };

        assert_eq!(hash1, hash2, "membership hash must be order-independent");
        assert_ne!(
            hash1, EMPTY_SHA256,
            "non-empty partition must differ from empty"
        );
    }

    #[test]
    fn membership_hash_differs_for_different_record_sets() {
        let db = open_db();
        let pid = ensure_canonical_partition(&db);

        {
            let conn = db.write();
            insert_vector_row(&conn, pid.as_str(), "record-1");
        }
        let hash_one = {
            let conn = db.write();
            compute_membership_hash(&conn, &pid).unwrap()
        };
        {
            let conn = db.write();
            insert_vector_row(&conn, pid.as_str(), "record-2");
        }
        let hash_two = {
            let conn = db.write();
            compute_membership_hash(&conn, &pid).unwrap()
        };

        assert_ne!(hash_one, hash_two, "adding a record must change the hash");
    }

    // ── derived_outbox enqueue + fetch ───────────────────────────────────────

    #[test]
    fn enqueue_vector_outbox_idempotent() {
        let db = open_db();
        let conn = db.write();
        enqueue_vector_outbox(
            &conn,
            "upsert",
            "memory",
            "rec-1",
            "h1",
            Some("part-1"),
            Some(5),
        )
        .unwrap();
        // Second enqueue with same semantic key is silently ignored (INSERT OR IGNORE).
        enqueue_vector_outbox(
            &conn,
            "upsert",
            "memory",
            "rec-1",
            "h1",
            Some("part-1"),
            Some(6),
        )
        .unwrap();
        let entries = fetch_pending_vector_outbox(&conn, 100).unwrap();
        assert_eq!(entries.len(), 1, "duplicate semantic key must be coalesced");
        assert_eq!(
            entries[0].authority_revision,
            Some(5),
            "first-writer wins on collision"
        );
    }

    #[test]
    fn enqueue_fetch_and_mark_applied() {
        let db = open_db();
        {
            let conn = db.write();
            enqueue_vector_outbox(
                &conn,
                "upsert",
                "memory",
                "rec-apply",
                "h_apply",
                Some("p"),
                Some(1),
            )
            .unwrap();
        }
        let entries = {
            let conn = db.write();
            fetch_pending_vector_outbox(&conn, 10).unwrap()
        };
        assert_eq!(entries.len(), 1);
        let id = entries[0].id;
        {
            let conn = db.write();
            mark_outbox_applied(&conn, id, 1).unwrap();
        }
        // No longer pending.
        let pending = {
            let conn = db.write();
            fetch_pending_vector_outbox(&conn, 10).unwrap()
        };
        assert!(
            pending.is_empty(),
            "applied entry must not appear in pending"
        );
    }

    #[test]
    fn mark_outbox_failed_promotes_to_dead_letter_at_max_attempts() {
        let db = open_db();
        {
            let conn = db.write();
            enqueue_vector_outbox(&conn, "upsert", "memory", "rec-dl", "hdl", None, None).unwrap();
        }
        let entries = {
            let conn = db.write();
            fetch_pending_vector_outbox(&conn, 10).unwrap()
        };
        let id = entries[0].id;
        {
            let conn = db.write();
            mark_outbox_failed(&conn, id, MAX_ATTEMPTS, "embed_err").unwrap();
        }
        // Should appear in derived_outbox with status=dead_letter, not pending.
        let pending = {
            let conn = db.write();
            fetch_pending_vector_outbox(&conn, 10).unwrap()
        };
        assert!(pending.is_empty(), "dead_letter must not appear in pending");
        // Verify the status directly.
        let conn = db.write();
        let status: String = conn
            .query_row(
                "SELECT status FROM derived_outbox WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "dead_letter");
    }

    // ── rebuild_partition ────────────────────────────────────────────────────

    fn valid_384_vec() -> Vec<f32> {
        let mut v = vec![0.0f32; 384];
        v[0] = 1.0;
        v
    }

    fn make_rebuild_record(record_id: &str) -> RebuildRecord {
        RebuildRecord {
            record_id: record_id.to_string(),
            vector: valid_384_vec(),
            content_hash: format!("hash-{record_id}"),
            namespace: "ns".to_string(),
            owner_id: "owner".to_string(),
            scope: "global".to_string(),
            sensitivity: 0,
            truth_state: "Current".to_string(),
            revision: 1,
        }
    }

    #[test]
    fn rebuild_creates_staging_table_and_activates() {
        let db_arc = open_db();
        let db: &Database = &db_arc;
        let pid = ensure_canonical_partition(&db_arc);

        let records = vec![
            Ok(make_rebuild_record("r-1")),
            Ok(make_rebuild_record("r-2")),
            Ok(make_rebuild_record("r-3")),
        ];

        let outcome = rebuild_partition(
            db,
            &pid,
            Some(10),
            "Xenova/all-MiniLM-L6-v2",
            None,
            10,
            records.into_iter(),
        )
        .unwrap();

        match outcome {
            RebuildOutcome::Activated {
                member_count,
                membership_hash,
            } => {
                assert_eq!(member_count, 3);
                assert_ne!(membership_hash, EMPTY_SHA256);
            }
            RebuildOutcome::Interrupted { .. } => panic!("expected Activated"),
        }

        // Verify mem_vectors_v2 has exactly 3 rows.
        let conn = db_arc.write();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM mem_vectors_v2 WHERE partition_id = ?1",
                params![pid.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);

        // Verify cursor is 'activated'.
        let cursor = load_rebuild_cursor(&conn, pid.as_str()).unwrap().unwrap();
        assert_eq!(cursor.status, RebuildStatus::Activated);

        // Verify derived_manifests row exists.
        let mhash: String = conn
            .query_row(
                "SELECT membership_hash FROM derived_manifests \
                 WHERE target = 'vectors' AND version = ?1",
                params![pid.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!mhash.is_empty());
    }

    #[test]
    fn rebuild_interrupted_mid_stream_leaves_cursor_interrupted() {
        let db_arc = open_db();
        let db: &Database = &db_arc;
        let pid = ensure_canonical_partition(&db_arc);

        // Stream: one good record then an error.
        let records: Vec<MemoryResult<RebuildRecord>> = vec![
            Ok(make_rebuild_record("ok-1")),
            Err(StorageError::Serde("simulated read failure".into()).into()),
        ];

        let outcome = rebuild_partition(
            db,
            &pid,
            Some(10),
            "Xenova/all-MiniLM-L6-v2",
            None,
            10,
            records.into_iter(),
        )
        .unwrap();

        assert!(
            matches!(outcome, RebuildOutcome::Interrupted { .. }),
            "expected Interrupted, got {outcome:?}"
        );

        let conn = db_arc.write();
        let cursor = load_rebuild_cursor(&conn, pid.as_str()).unwrap().unwrap();
        assert_eq!(cursor.status, RebuildStatus::Interrupted);
    }

    #[test]
    fn rebuild_atomically_removes_stale_rows() {
        let db_arc = open_db();
        let db: &Database = &db_arc;
        let pid = ensure_canonical_partition(&db_arc);

        // First rebuild: 3 rows.
        rebuild_partition(
            db,
            &pid,
            Some(1),
            "Xenova/all-MiniLM-L6-v2",
            None,
            10,
            vec![
                Ok(make_rebuild_record("a")),
                Ok(make_rebuild_record("b")),
                Ok(make_rebuild_record("c")),
            ]
            .into_iter(),
        )
        .unwrap();

        // Second rebuild: only 2 rows — 'c' should be removed.
        rebuild_partition(
            db,
            &pid,
            Some(2),
            "Xenova/all-MiniLM-L6-v2",
            None,
            10,
            vec![Ok(make_rebuild_record("a")), Ok(make_rebuild_record("b"))].into_iter(),
        )
        .unwrap();

        let conn = db_arc.write();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM mem_vectors_v2 WHERE partition_id = ?1",
                params![pid.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2, "stale row 'c' must be removed during activation");
    }

    #[test]
    fn rebuild_stores_migration_source_partition_id() {
        let db_arc = open_db();
        let db: &Database = &db_arc;
        let pid = ensure_canonical_partition(&db_arc);

        rebuild_partition(
            db,
            &pid,
            Some(1),
            "Xenova/all-MiniLM-L6-v2",
            Some("old-partition-v1".to_string()),
            10,
            vec![Ok(make_rebuild_record("rec"))].into_iter(),
        )
        .unwrap();

        let conn = db_arc.write();
        let cursor = load_rebuild_cursor(&conn, pid.as_str()).unwrap().unwrap();
        assert_eq!(
            cursor.migration_source_partition_id,
            Some("old-partition-v1".to_string())
        );
    }

    // ── write_derived_manifest ───────────────────────────────────────────────

    #[test]
    fn write_derived_manifest_upserts_row() {
        let db = open_db();
        let conn = db.write();
        write_derived_manifest(
            &conn,
            &DerivedManifestRow {
                partition_id: "test-partition".to_string(),
                authority_revision: Some(42),
                member_count: 100,
                membership_hash: "deadbeef".to_string(),
                algorithm: "exact-cosine-f32le".to_string(),
                model_version: "model-v1".to_string(),
                completed_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();

        let (member_count, hash, status): (i64, String, String) = conn
            .query_row(
                "SELECT member_count, membership_hash, status \
                 FROM derived_manifests \
                 WHERE target = 'vectors' AND version = 'test-partition'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();

        assert_eq!(member_count, 100);
        assert_eq!(hash, "deadbeef");
        assert_eq!(status, "active");
    }
}
