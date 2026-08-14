//! Deterministic authority lifecycle operations (task 1.1.7, F1.1 finalizer).
//!
//! This module establishes the **single-authority lifecycle contract** for the
//! SQLite authority: there is exactly one way to bring a v2 authority into
//! existence (a deterministic, fully-migrated open) and exactly one sanctioned
//! way to re-establish it from scratch (a hard reset). It provides:
//!
//! * [`Database::fresh_create`] — create a brand-new, fully-migrated v2 authority
//!   from empty (deterministic: same schema_version, all v2 tables present,
//!   `authority_meta` singleton seeded).
//! * [`Database::hard_reset`] — destroy the existing authority data and recreate
//!   a fresh, fully-migrated v2 authority at the same path. Data loss is
//!   intentional and acceptable (dev-context: single-dev, pre-production).
//! * [`Database::reconciliation_report`] / [`ReconciliationReport`] — a
//!   deterministic, serializable account of the legacy authority-competing rows
//!   that a hard reset would discard, plus the current v2 authority counts.
//!
//! ## Single-authority guarantee (task 1.1.7 item #4 — scoping decision)
//!
//! The migration runner ([`super::migrations::run`]) applies an **ordered,
//! additive** migration set and records each step in the `schema_version`
//! ledger. It contains no dual-run / dual-write machinery and never creates or
//! maintains a *second writable v2 authority*: a single `open` deterministically
//! yields exactly one `authority_meta` singleton (id = 1). That single
//! deterministic open — plus [`hard_reset`](Database::hard_reset) to rebuild —
//! is the ONLY sanctioned way to (re)establish authority. There is therefore no
//! silent dual-write path to remove here.
//!
//! SCOPING DECISION (task 1.1.7 vs F1.9.3): the v2 schema currently *coexists*
//! with the legacy v1 schema (legacy `events`, `memories`, `memory_facts`,
//! `shred_keys`, … from migrations 0001–0010) in the same DB file. That
//! coexistence is intentional and temporary. The full legacy-writer cutover —
//! deleting legacy tables and their Rust writers — is a LATER task (**F1.9.3**),
//! NOT this one: other kria-core modules still read/write the legacy tables, so
//! removing them now would break the crate. This task therefore establishes the
//! single-authority + hard-reset contract and reports honestly on the legacy
//! rows a reset discards, WITHOUT deleting any legacy schema or writers.

use std::path::Path;

use rusqlite::Connection;

use super::Database;
use crate::error::{MemoryResult, StorageError};

/// The `:memory:` sentinel path used by [`Database::open_in_memory`].
const IN_MEMORY_PATH: &str = ":memory:";

/// Legacy authority-competing tables (v1 schema, migrations 0001–0010) whose
/// rows a hard reset discards. Each is counted defensively (guarded by an
/// existence check) so the report works on a fresh v2-only DB where these may
/// be absent AND on the current coexistence DB where they exist.
///
/// This is the reporting surface for what a reset loses; it does NOT delete
/// these tables (that is F1.9.3 — see the module-level scoping decision).
const LEGACY_TABLES: &[&str] = &[
    "events",       // legacy immutable event log (0001) → superseded by events_v2
    "memories",     // legacy derived durable memories (0001)
    "memory_facts", // legacy runtime-compat facts (0005)
    "world_facts",  // named in the F1.1 design; absent in current schema → 0
    "shred_keys",   // legacy crypto-shred catalog (0001) → superseded by shred_keys_v2
];

/// v2 authority tables reported alongside the legacy counts for an honest
/// before/after picture of a reset.
const V2_TABLES: &[&str] = &[
    "events_v2",     // v2 immutable event log (0012)
    "shred_keys_v2", // v2 crypto-shred reference catalog (0014)
    "sources",       // v2 source base rows (0014)
];

/// A deterministic account of the rows a [`hard_reset`](Database::hard_reset)
/// would discard: counts of legacy authority-competing tables plus the current
/// v2 authority tables. Every field is a plain row count; absent tables report
/// `0`.
///
/// Serializable so callers (CLI / diagnostics) can surface exactly what a reset
/// destroys before performing one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReconciliationReport {
    /// Rows in the legacy `events` immutable log (0001).
    pub legacy_events: u64,
    /// Rows in the legacy `memories` table (0001).
    pub legacy_memories: u64,
    /// Rows in the legacy `memory_facts` table (0005).
    pub legacy_memory_facts: u64,
    /// Rows in a `world_facts` table if present (named in the F1.1 design; not
    /// in the current schema, so normally `0`).
    pub world_facts: u64,
    /// Rows in the legacy `shred_keys` catalog (0001).
    pub legacy_shred_keys: u64,
    /// Rows in the v2 `events_v2` immutable log (0012).
    pub v2_events: u64,
    /// Rows in the v2 `shred_keys_v2` reference catalog (0014).
    pub v2_shred_keys: u64,
    /// Rows in the v2 `sources` base table (0014).
    pub v2_sources: u64,
}

impl ReconciliationReport {
    /// Total rows across the legacy authority-competing tables — the volume a
    /// hard reset discards.
    pub fn legacy_total(&self) -> u64 {
        self.legacy_events
            .saturating_add(self.legacy_memories)
            .saturating_add(self.legacy_memory_facts)
            .saturating_add(self.world_facts)
            .saturating_add(self.legacy_shred_keys)
    }
}

/// Whether `path` denotes the in-memory sentinel authority.
fn is_in_memory(path: &Path) -> bool {
    path.as_os_str() == IN_MEMORY_PATH
}

/// Remove a sidecar file, treating "not found" as success (idempotent). Any
/// other IO error propagates.
fn remove_if_present(path: &Path) -> MemoryResult<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(StorageError::Io(e).into()),
    }
}

/// Whether `name` exists as a table in the connected database.
fn table_exists(conn: &Connection, name: &str) -> MemoryResult<bool> {
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |r| r.get(0),
        )
        .map_err(StorageError::Sqlite)?;
    Ok(n == 1)
}

/// Count rows in `name`, or `0` if the table does not exist (fresh v2-only DB).
fn count_if_exists(conn: &Connection, name: &str) -> MemoryResult<u64> {
    if !table_exists(conn, name)? {
        return Ok(0);
    }
    // Table name comes from a fixed const allow-list (never user input), so the
    // interpolation here cannot be an injection vector.
    let sql = format!("SELECT COUNT(*) FROM {name}");
    let count: i64 = conn
        .query_row(&sql, [], |r| r.get(0))
        .map_err(StorageError::Sqlite)?;
    Ok(count as u64)
}

impl Database {
    /// Deterministically create a brand-new, fully-migrated v2 authority at
    /// `path` from empty.
    ///
    /// The result is deterministic: `schema_version` equals
    /// [`migrations::latest_version`](super::migrations::latest_version), every
    /// v2 table is present, and the `authority_meta` singleton (id = 1) is
    /// seeded. This is the single sanctioned "create from nothing" entry point.
    ///
    /// Refuses to run if a file-backed authority already exists at `path` (or a
    /// `-wal`/`-shm` sidecar is present): "fresh create" means *from empty*. Use
    /// [`hard_reset`](Database::hard_reset) to rebuild over existing data. For
    /// the `:memory:` sentinel this always yields a fresh in-memory authority.
    pub fn fresh_create(path: impl AsRef<Path>) -> MemoryResult<Self> {
        let path = path.as_ref();
        if is_in_memory(path) {
            return Self::open_in_memory();
        }
        let wal = sidecar(path, "-wal");
        let shm = sidecar(path, "-shm");
        if path.exists() || wal.exists() || shm.exists() {
            return Err(StorageError::Serde(format!(
                "fresh_create: an authority already exists at {}; use hard_reset to rebuild",
                path.display()
            ))
            .into());
        }
        Self::open(path)
    }

    /// Destroy the authority data at `path` and recreate a fresh, fully-migrated
    /// v2 authority there, returning the new handle.
    ///
    /// Data loss is intentional and acceptable (dev-context: single-dev,
    /// pre-production — no backup/rollback ceremony). The implementation removes
    /// the main DB file plus its `-wal`/`-shm` sidecars (missing sidecars are not
    /// an error) and re-runs [`open`](Database::open), which re-applies all
    /// migrations to yield a deterministic fresh v2 authority.
    ///
    /// Single-writer model: **the caller MUST drop any existing [`Database`]
    /// handle to `path` before calling this** (associated function, not a
    /// method, precisely so the old handle can be dropped first). A lingering
    /// handle keeps the old file inode open on unix and would observe stale
    /// state. For the `:memory:` sentinel this simply returns a fresh in-memory
    /// authority.
    pub fn hard_reset(path: impl AsRef<Path>) -> MemoryResult<Self> {
        let path = path.as_ref();
        if is_in_memory(path) {
            return Self::open_in_memory();
        }
        remove_if_present(path)?;
        remove_if_present(&sidecar(path, "-wal"))?;
        remove_if_present(&sidecar(path, "-shm"))?;
        Self::open(path)
    }

    /// Produce a deterministic [`ReconciliationReport`] of the legacy
    /// authority-competing rows a [`hard_reset`](Database::hard_reset) would
    /// discard, alongside the current v2 authority counts.
    ///
    /// Each table is guarded by an existence check, so this works both on a
    /// fresh v2-only DB (legacy tables absent → `0`) and on the current
    /// coexistence DB (both legacy and v2 tables present). It reads and does not
    /// mutate; it never deletes legacy data (that is F1.9.3).
    pub fn reconciliation_report(&self) -> MemoryResult<ReconciliationReport> {
        self.with_read(|conn| {
            // Fixed allow-lists keep the field mapping explicit and auditable.
            debug_assert_eq!(LEGACY_TABLES.len(), 5);
            debug_assert_eq!(V2_TABLES.len(), 3);
            Ok(ReconciliationReport {
                legacy_events: count_if_exists(conn, "events")?,
                legacy_memories: count_if_exists(conn, "memories")?,
                legacy_memory_facts: count_if_exists(conn, "memory_facts")?,
                world_facts: count_if_exists(conn, "world_facts")?,
                legacy_shred_keys: count_if_exists(conn, "shred_keys")?,
                v2_events: count_if_exists(conn, "events_v2")?,
                v2_shred_keys: count_if_exists(conn, "shred_keys_v2")?,
                v2_sources: count_if_exists(conn, "sources")?,
            })
        })
    }
}

/// Build a sidecar path by appending `suffix` to the file name (e.g. `-wal`).
fn sidecar(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    std::path::PathBuf::from(os)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations;
    use rusqlite::Connection;
    use tempfile::tempdir;

    /// All v2 authority tables that a fully-migrated open must materialize.
    const REQUIRED_V2_TABLES: &[&str] = &[
        "schema_versions",
        "authority_meta",
        "events_v2",
        "graph_revisions",
        "graph_changes",
        "audit_records",
        "idempotency_results",
        "derived_outbox",
        "derived_manifests",
        "recovery_snapshots",
        "shred_keys_v2",
        "sources",
        "interchange_imports",
    ];

    /// Assert the deterministic fresh-v2 schema state on a connection: latest
    /// schema_version, all v2 tables present, exactly one seeded authority_meta
    /// singleton (id = 1, schema_epoch = 2).
    fn assert_fresh_v2_state(db: &Database) {
        assert_eq!(
            db.schema_version(),
            migrations::latest_version(),
            "schema_version must equal the latest migration"
        );
        let conn = db.write();
        for t in REQUIRED_V2_TABLES {
            assert!(
                table_exists(&conn, t).unwrap(),
                "v2 table {t} must be present after a fresh create"
            );
        }
        let (id, schema_epoch): (i64, i64) = conn
            .query_row("SELECT id, schema_epoch FROM authority_meta", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(id, 1, "authority_meta singleton id must be 1");
        assert_eq!(schema_epoch, 2, "schema_epoch must be the v2 epoch");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM authority_meta", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "exactly one authority_meta singleton");
    }

    #[test]
    fn fresh_create_is_deterministic() {
        // (a) fresh-create: a nonexistent path yields the latest schema_version,
        // all v2 tables, and a seeded singleton — and doing it twice (in two
        // fresh dirs) produces identical schema state.
        let dir1 = tempdir().unwrap();
        let db1 = Database::fresh_create(dir1.path().join("mem.db")).unwrap();
        assert_fresh_v2_state(&db1);

        let dir2 = tempdir().unwrap();
        let db2 = Database::fresh_create(dir2.path().join("mem.db")).unwrap();
        assert_fresh_v2_state(&db2);

        assert_eq!(
            db1.schema_version(),
            db2.schema_version(),
            "two fresh creates must yield identical schema_version"
        );
    }

    #[test]
    fn open_on_nonexistent_path_yields_full_v2() {
        // The documented guarantee: a plain open on a nonexistent path is a
        // deterministic fully-migrated v2 authority (fresh_create's foundation).
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("mem.db")).unwrap();
        assert_fresh_v2_state(&db);
    }

    #[test]
    fn fresh_create_refuses_existing_authority() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("mem.db");
        let db = Database::fresh_create(&path).unwrap();
        drop(db);
        // Second fresh_create over the now-existing file must be refused.
        let err = Database::fresh_create(&path);
        assert!(
            err.is_err(),
            "fresh_create must refuse an already-existing authority"
        );
    }

    #[test]
    fn hard_reset_destroys_data_and_rebuilds_fresh_v2() {
        // (b) hard_reset: write rows into a v2 table, reset, and assert the fresh
        // DB is empty with a valid seeded singleton and the latest schema.
        let dir = tempdir().unwrap();
        let path = dir.path().join("mem.db");

        let db = Database::open(&path).unwrap();
        {
            let conn = db.write();
            // A v2 `sources` row (base table with a source_kind CHECK plus the
            // NOT NULL policy columns).
            conn.execute(
                "INSERT INTO sources(
                     id, source_kind, namespace, owner_id, scope, sensitivity, policy_version)
                 VALUES ('s1', 'native', 'ns', 'owner-1', 'private', 0, 'p1')",
                [],
            )
            .unwrap();
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM sources", [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 1, "row must be present before reset");
        }
        // Single-writer model: drop the old handle before resetting.
        drop(db);

        let fresh = Database::hard_reset(&path).unwrap();
        assert_fresh_v2_state(&fresh);
        let n: i64 = fresh
            .with_read(|c| {
                Ok(
                    c.query_row("SELECT COUNT(*) FROM sources", [], |r| r.get(0))
                        .map_err(StorageError::Sqlite)?,
                )
            })
            .unwrap();
        assert_eq!(n, 0, "hard_reset must discard all prior rows");
    }

    #[test]
    fn hard_reset_ok_when_sidecars_absent() {
        // hard_reset on a path with no -wal/-shm sidecars must not error.
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.db");
        assert!(!path.exists());
        let db = Database::hard_reset(&path).unwrap();
        assert_fresh_v2_state(&db);
    }

    #[test]
    fn hard_reset_in_memory_is_fresh() {
        // The :memory: sentinel yields a fresh in-memory authority.
        let db = Database::hard_reset(":memory:").unwrap();
        assert_fresh_v2_state(&db);
    }

    /// Insert a legacy `events` row (v1 schema, migration 0001).
    fn insert_legacy_event(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO events(id, hlc, ts_utc, tz_offset_min, event_type, source, payload, checksum)
             VALUES (?1,'00','2026-01-01T00:00:00Z',0,'observation','user','{}','h')",
            [id],
        )
        .unwrap();
    }

    /// Insert a v2 `events_v2` row (migration 0012).
    fn insert_v2_event(conn: &Connection, id: &str, hlc: &str) {
        conn.execute(
            "INSERT INTO events_v2(
                 id, phase, hlc, ts_utc, tz_offset_min, event_type,
                 source_kind, source_id, actor_id,
                 namespace, owner_id, scope, sensitivity, policy_version,
                 payload_cipher, payload_plain, payload_encoding, payload_checksum,
                 schema_version)
             VALUES (?1, 'start', ?2, '2026-01-01T00:00:00Z', 0, 'observation',
                     'user', 'src-1', 'actor-1',
                     'ns', 'owner-1', 'private', 0, 'p1',
                     NULL, '{}', 'utf8', 'chk', 1)",
            [id, hlc],
        )
        .unwrap();
    }

    #[test]
    fn reconciliation_report_reflects_coexistence_rows() {
        // (c) On a coexistence DB (all 0001–0015 migrations applied), the report
        // reflects both a legacy and a v2 row.
        let db = Database::open_in_memory().unwrap();
        {
            let conn = db.write();
            insert_legacy_event(&conn, "legacy-1");
            insert_v2_event(&conn, "v2-1", "hlc-1");
        }
        let report = db.reconciliation_report().unwrap();
        assert_eq!(report.legacy_events, 1, "one legacy events row");
        assert_eq!(report.v2_events, 1, "one v2 events_v2 row");
        assert_eq!(report.world_facts, 0, "world_facts table is absent → 0");
        assert_eq!(
            report.legacy_total(),
            1,
            "legacy total counts the one event"
        );
    }

    #[test]
    fn reconciliation_report_is_deterministic() {
        // The same DB state yields the same report on repeated calls.
        let db = Database::open_in_memory().unwrap();
        {
            let conn = db.write();
            insert_legacy_event(&conn, "legacy-1");
        }
        let r1 = db.reconciliation_report().unwrap();
        let r2 = db.reconciliation_report().unwrap();
        assert_eq!(r1, r2, "reconciliation_report must be deterministic");
    }

    #[test]
    fn reconciliation_report_serde_round_trip() {
        // The report struct round-trips through serde unchanged.
        let db = Database::open_in_memory().unwrap();
        {
            let conn = db.write();
            insert_legacy_event(&conn, "legacy-1");
            insert_v2_event(&conn, "v2-1", "hlc-1");
        }
        let report = db.reconciliation_report().unwrap();
        let json = serde_json::to_string(&report).unwrap();
        let decoded: ReconciliationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report, decoded, "serde round-trip must be identity");
    }

    #[test]
    fn fresh_v2_authority_has_single_meta_singleton() {
        // Task 1.1.7 item #4: a fresh open produces exactly one v2 authority
        // meta singleton — the single-authority contract, no dual-write path.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM authority_meta", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "exactly one authority_meta singleton (id=1)");
    }
}
