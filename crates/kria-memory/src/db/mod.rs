//! SQLite authority connection management (memory-upgrade design §9/§14, tasks 3–4).
//!
//! The authority is the single source of truth (L2). This module owns:
//! * the **single serialized write connection** (invariant L10: only the atomic
//!   commit holds the writer),
//! * a **WAL read pool** so readers never block the writer,
//! * the **L14 guard** refusing a network-filesystem authority,
//! * migration application on open.

pub mod encoding;
pub mod lifecycle;
pub mod migrations;

pub use lifecycle::ReconciliationReport;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use arc_swap::ArcSwap;
use rusqlite::Connection;

use crate::error::{MemoryResult, StorageError};

/// Default read-pool size. Desktop scale; readers are cheap under WAL.
const DEFAULT_READ_POOL: usize = 4;

/// The SQLite authority: one write connection + a pool of read connections.
///
/// Cloneable handles are not provided; wrap in `Arc<Database>` to share. The
/// write connection is serialized behind a `Mutex` (single writer, L10); the
/// read pool round-robins across `Mutex<Connection>` slots (WAL → concurrent).
pub struct Database {
    path: PathBuf,
    write: Mutex<Connection>,
    /// WAL read pool, swappable so `restore_from` can atomically rebuild it
    /// (fresh connections drop any pages cached from the pre-restore DB — H3).
    read_pool: ArcSwap<Vec<Mutex<Connection>>>,
    /// Target read-pool size, so a rebuild recreates the same number of readers.
    read_pool_size: usize,
    next_reader: AtomicUsize,
    schema_version: u32,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("path", &self.path)
            .field("schema_version", &self.schema_version)
            .field("read_pool", &self.read_pool.load().len())
            .finish()
    }
}

impl Database {
    /// Open (creating if needed) the authority at `path`, run migrations, and
    /// build the read pool. Enforces the L14 local-filesystem guard.
    pub fn open(path: impl AsRef<Path>) -> MemoryResult<Self> {
        Self::open_with_pool(path, DEFAULT_READ_POOL)
    }

    /// Open an in-memory authority (tests only). Shares one connection across a
    /// named in-memory DB so reads see writes.
    pub fn open_in_memory() -> MemoryResult<Self> {
        // A private in-memory DB per connection would not share state, so the
        // read pool must reuse the same connection semantics. For tests we use
        // a single connection acting as both writer and (serialized) reader.
        let conn = Connection::open_in_memory().map_err(StorageError::Sqlite)?;
        configure(&conn, true)?;
        let version = migrations::run(&conn)?;
        Ok(Self {
            path: PathBuf::from(":memory:"),
            write: Mutex::new(conn),
            read_pool: ArcSwap::from_pointee(Vec::new()), // empty → reads use the write conn
            read_pool_size: 0,
            next_reader: AtomicUsize::new(0),
            schema_version: version,
        })
    }

    fn open_with_pool(path: impl AsRef<Path>, pool: usize) -> MemoryResult<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(StorageError::Io)?;
            }
        }
        // L14: refuse a network-filesystem authority (corruption risk).
        guard_local_filesystem(&path)?;

        let writer = Connection::open(&path).map_err(StorageError::Sqlite)?;
        configure(&writer, false)?;
        let schema_version = migrations::run(&writer)?;

        let mut read_pool = Vec::with_capacity(pool);
        for _ in 0..pool {
            let rc = Connection::open(&path).map_err(StorageError::Sqlite)?;
            configure(&rc, false)?;
            read_pool.push(Mutex::new(rc));
        }

        tracing::info!(
            path = %path.display(),
            schema_version,
            read_pool = read_pool.len(),
            "memory authority opened"
        );
        Ok(Self {
            path,
            write: Mutex::new(writer),
            read_pool: ArcSwap::from_pointee(read_pool),
            read_pool_size: pool,
            next_reader: AtomicUsize::new(0),
            schema_version,
        })
    }

    /// The applied schema version.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Write a consistent standalone copy of the authority to `dest` (P7 backup).
    /// The heavy `VACUUM INTO` runs on a **read** connection so it does NOT hold
    /// the single write lock for the whole copy (H3) — concurrent writes proceed
    /// against the live DB while the snapshot is taken. Only the brief WAL
    /// checkpoint touches the writer. Returns the backup size in bytes.
    pub fn backup_to(&self, dest: impl AsRef<Path>) -> MemoryResult<u64> {
        let dest = dest.as_ref();
        if let Some(parent) = dest.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(StorageError::Io)?;
            }
        }
        // A pre-existing target would make VACUUM INTO fail; overwrite is fine
        // (dev-context: data loss acceptable, no backup ceremony).
        let _ = std::fs::remove_file(dest);
        // Brief writer touch: fold the WAL into the main DB so the snapshot is
        // complete, then release the write lock before the (potentially large)
        // VACUUM copy.
        {
            let conn = self.write();
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(FULL);");
        }
        // VACUUM INTO on a pooled read connection (no write lock held).
        self.with_read(|conn| {
            conn.execute("VACUUM INTO ?1", rusqlite::params![dest.to_string_lossy()])
                .map_err(StorageError::Sqlite)?;
            Ok(())
        })?;
        Ok(std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0))
    }

    /// Open a fresh read connection against the authority path (used to rebuild
    /// the read pool after a restore).
    fn open_reader(&self) -> MemoryResult<Mutex<Connection>> {
        let rc = Connection::open(&self.path).map_err(StorageError::Sqlite)?;
        configure(&rc, false)?;
        Ok(Mutex::new(rc))
    }

    /// Restore the authority in-place from a backup file `src` (P7 restore).
    /// Uses SQLite's online backup API to overwrite the live main database, then
    /// **atomically rebuilds the read pool** with fresh connections so pooled
    /// readers cannot serve pages cached from the pre-restore database (H3). No
    /// restart is required; reads after this call observe the restored state.
    pub fn restore_from(&self, src: impl AsRef<Path>) -> MemoryResult<()> {
        use rusqlite::backup::Backup;
        use rusqlite::OpenFlags;

        let src = src.as_ref();
        if !src.exists() {
            return Err(StorageError::Serde(format!(
                "restore source not found: {}",
                src.display()
            ))
            .into());
        }
        let src_conn = Connection::open_with_flags(
            src,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .map_err(StorageError::Sqlite)?;

        {
            let mut dst = self.write();
            let backup = Backup::new(&src_conn, &mut dst).map_err(StorageError::Sqlite)?;
            backup
                .run_to_completion(256, std::time::Duration::from_millis(5), None)
                .map_err(StorageError::Sqlite)?;
            drop(backup);
            let _ = dst.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }

        // Rebuild + atomically swap the read pool so no reader retains stale
        // cached pages. In-memory DBs (size 0) share the write conn — nothing to
        // rebuild.
        if self.read_pool_size > 0 {
            let mut fresh = Vec::with_capacity(self.read_pool_size);
            for _ in 0..self.read_pool_size {
                fresh.push(self.open_reader()?);
            }
            self.read_pool.store(Arc::new(fresh));
        }
        Ok(())
    }

    /// The authority path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Acquire the single write connection (serialized — L10). Hold only for the
    /// atomic commit; do not perform long reads under this lock.
    pub fn write(&self) -> MutexGuard<'_, Connection> {
        self.write.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Run a closure against a read connection from the WAL pool (readers never
    /// block the writer — L10). Falls back to the write connection for the
    /// in-memory test database.
    pub fn with_read<T>(&self, f: impl FnOnce(&Connection) -> MemoryResult<T>) -> MemoryResult<T> {
        let pool = self.read_pool.load();
        if pool.is_empty() {
            let conn = self.write();
            return f(&conn);
        }
        let idx = self.next_reader.fetch_add(1, Ordering::Relaxed) % pool.len();
        let conn = pool[idx].lock().unwrap_or_else(|p| p.into_inner());
        f(&conn)
    }

    /// Begin an authority transaction (`BEGIN IMMEDIATE`), holding the single
    /// write lock for its duration (L2/L10). The transaction rolls back on drop
    /// unless [`AuthorityTx::commit`] is called.
    pub fn begin(&self) -> MemoryResult<AuthorityTx<'_>> {
        let guard = self.write();
        guard
            .execute_batch("BEGIN IMMEDIATE;")
            .map_err(StorageError::Sqlite)?;
        Ok(AuthorityTx {
            conn: guard,
            committed: false,
        })
    }

    /// Run SQLite's integrity check (design §30 startup integrity).
    pub fn quick_check(&self) -> MemoryResult<bool> {
        let conn = self.write();
        let result: String = conn
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(StorageError::Sqlite)?;
        Ok(result == "ok")
    }
}

/// A single authority transaction over the serialized write connection.
///
/// Holds the write `MutexGuard` for its lifetime (invariant L10: only the atomic
/// commit holds the writer). All authority-transaction store operations
/// (`EventStore`/`RelationalStore`/`GraphStore`) take `&mut AuthorityTx` so
/// events + derived memory + graph + outbox commit atomically together (L2).
pub struct AuthorityTx<'a> {
    conn: MutexGuard<'a, Connection>,
    committed: bool,
}

impl AuthorityTx<'_> {
    /// The underlying connection for issuing statements within the transaction.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Commit the transaction. Consumes `self`.
    pub fn commit(mut self) -> MemoryResult<()> {
        self.conn
            .execute_batch("COMMIT;")
            .map_err(StorageError::Sqlite)?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for AuthorityTx<'_> {
    fn drop(&mut self) {
        if !self.committed {
            // Best-effort rollback; a poisoned/failed rollback is logged, not
            // panicked, so a drop during unwinding stays safe.
            if let Err(e) = self.conn.execute_batch("ROLLBACK;") {
                tracing::warn!(error = %e, "AuthorityTx rollback on drop failed");
            }
        }
    }
}

/// Apply **and assert** the standard pragmas on every authority connection
/// (design §4 preamble + §4.4): WAL journaling, foreign-key enforcement, a busy
/// timeout for transient cross-connection contention, and `synchronous=FULL`.
///
/// The authority mandates `synchronous=FULL` (not `NORMAL`): a committed
/// transaction must survive an OS crash / power loss, closing the durability gap
/// flagged in the F0 schema inventory (MGR-033). `FULL` on the read-pool
/// connections is harmless, so a single `configure` covers writer and readers.
///
/// Each pragma is **read back after being set** and a mismatch is a hard error
/// ([`StorageError::PragmaAssertion`]): a silently-ignored pragma would violate a
/// durability or integrity invariant, so the authority refuses to open rather
/// than run mis-configured. Required SQLite capabilities (JSON1) are asserted
/// too ([`assert_capabilities`]).
///
/// `in_memory` relaxes only the WAL assertion: `:memory:` databases cannot use a
/// WAL and SQLite reports `memory` as their journal mode.
fn configure(conn: &Connection, in_memory: bool) -> MemoryResult<()> {
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(StorageError::Sqlite)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(StorageError::Sqlite)?;
    conn.pragma_update(None, "busy_timeout", 5000)
        .map_err(StorageError::Sqlite)?;
    conn.pragma_update(None, "synchronous", "FULL")
        .map_err(StorageError::Sqlite)?;

    assert_pragmas(conn, in_memory)?;
    assert_capabilities(conn)?;
    Ok(())
}

/// Read back the pragmas applied in [`configure`] and reject the connection if
/// any did not take effect (design §4.4 startup checks). A set pragma that SQLite
/// silently ignores would otherwise leave the authority without WAL, without
/// FK enforcement, or without FULL durability.
fn assert_pragmas(conn: &Connection, in_memory: bool) -> MemoryResult<()> {
    // journal_mode: `wal` on a file-backed authority; `:memory:` reports `memory`.
    let journal: String = conn
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .map_err(StorageError::Sqlite)?;
    let journal_lc = journal.to_ascii_lowercase();
    let journal_ok = journal_lc == "wal" || (in_memory && journal_lc == "memory");
    if !journal_ok {
        return Err(StorageError::PragmaAssertion(format!(
            "journal_mode = {journal} (expected wal{})",
            if in_memory { " or memory" } else { "" }
        ))
        .into());
    }

    // foreign_keys: must be enforced (1).
    let foreign_keys: i64 = conn
        .pragma_query_value(None, "foreign_keys", |r| r.get(0))
        .map_err(StorageError::Sqlite)?;
    if foreign_keys != 1 {
        return Err(StorageError::PragmaAssertion(format!(
            "foreign_keys = {foreign_keys} (expected 1)"
        ))
        .into());
    }

    // busy_timeout: at least the configured 5000 ms.
    let busy_timeout: i64 = conn
        .pragma_query_value(None, "busy_timeout", |r| r.get(0))
        .map_err(StorageError::Sqlite)?;
    if busy_timeout < 5000 {
        return Err(StorageError::PragmaAssertion(format!(
            "busy_timeout = {busy_timeout} (expected >= 5000)"
        ))
        .into());
    }

    // synchronous: FULL == 2 (authority durability mandate).
    let synchronous: i64 = conn
        .pragma_query_value(None, "synchronous", |r| r.get(0))
        .map_err(StorageError::Sqlite)?;
    if synchronous != 2 {
        return Err(StorageError::PragmaAssertion(format!(
            "synchronous = {synchronous} (expected 2 = FULL)"
        ))
        .into());
    }
    Ok(())
}

/// Assert required SQLite capabilities are compiled into the linked library
/// (design §4). The authority relies on JSON1 (`json` / `json_valid`) for payload
/// validation; rusqlite's bundled SQLite ships it, and this probe guards that
/// dependency so a mis-built binary fails loudly at open instead of at first use.
fn assert_capabilities(conn: &Connection) -> MemoryResult<()> {
    // `json_valid('{}')` returns 1 iff JSON1 is available and the argument parses.
    let valid: i64 = conn
        .query_row("SELECT json_valid('{}')", [], |r| r.get(0))
        .map_err(|e| StorageError::CapabilityMissing(format!("JSON1 probe failed: {e}")))?;
    if valid != 1 {
        return Err(StorageError::CapabilityMissing(
            "json_valid('{}') did not return 1".to_string(),
        )
        .into());
    }
    // A round-trip through `json()` confirms the function is usable, not just
    // that `json_valid` links.
    let normalized: String = conn
        .query_row("SELECT json('{}')", [], |r| r.get(0))
        .map_err(|e| StorageError::CapabilityMissing(format!("json() probe failed: {e}")))?;
    if normalized != "{}" {
        return Err(StorageError::CapabilityMissing(format!(
            "json('{{}}') returned {normalized:?} (expected \"{{}}\")"
        ))
        .into());
    }
    Ok(())
}

/// L14 guard: the authority must live on a local filesystem, never a network
/// mount (NFS/SMB/CIFS), where SQLite locking is unreliable and corruption is
/// possible (architecture §38.1). Best-effort on Linux via `statfs` magic; a
/// no-op on other platforms and for in-memory DBs.
#[cfg(target_os = "linux")]
fn guard_local_filesystem(path: &Path) -> MemoryResult<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    // Check the nearest existing ancestor (the DB file may not exist yet).
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        match probe.parent() {
            Some(p) if !p.as_os_str().is_empty() => probe = p.to_path_buf(),
            _ => return Ok(()), // nothing to probe; allow
        }
    }
    let c = match CString::new(probe.as_os_str().as_bytes()) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    // SAFETY: `statfs` reads into a zeroed struct; we only read `f_type`.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statfs(c.as_ptr(), &mut stat) };
    if rc != 0 {
        return Ok(()); // can't determine → allow (best-effort)
    }
    // Known network filesystem magic numbers.
    const NFS_SUPER_MAGIC: i64 = 0x6969;
    const SMB_SUPER_MAGIC: i64 = 0x517B;
    const CIFS_MAGIC_NUMBER: i64 = 0xFF534D42u32 as i64;
    const SMB2_MAGIC_NUMBER: i64 = 0xFE534D42u32 as i64;
    let fs_type = stat.f_type as i64;
    if matches!(
        fs_type,
        NFS_SUPER_MAGIC | SMB_SUPER_MAGIC | CIFS_MAGIC_NUMBER | SMB2_MAGIC_NUMBER
    ) {
        return Err(StorageError::NetworkFilesystem(format!(
            "{} (fs magic {:#x})",
            path.display(),
            fs_type
        ))
        .into());
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn guard_local_filesystem(_path: &Path) -> MemoryResult<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn opens_and_runs_migrations() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("mem.db")).unwrap();
        assert_eq!(db.schema_version(), migrations::latest_version());
        assert!(db.quick_check().unwrap());
    }

    #[test]
    fn file_backed_open_asserts_pragmas() {
        // Task 1.1.6: a file-backed authority connection must open with WAL,
        // FK enforcement, synchronous=FULL (2), and busy_timeout>=5000, verified
        // by reading the pragmas back on both the writer and a pooled reader.
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("mem.db")).unwrap();

        let check = |conn: &Connection| {
            let journal: String = conn
                .pragma_query_value(None, "journal_mode", |r| r.get(0))
                .unwrap();
            assert_eq!(
                journal.to_ascii_lowercase(),
                "wal",
                "journal_mode must be wal"
            );
            let fk: i64 = conn
                .pragma_query_value(None, "foreign_keys", |r| r.get(0))
                .unwrap();
            assert_eq!(fk, 1, "foreign_keys must be enforced");
            let sync: i64 = conn
                .pragma_query_value(None, "synchronous", |r| r.get(0))
                .unwrap();
            assert_eq!(sync, 2, "synchronous must be FULL (2)");
            let busy: i64 = conn
                .pragma_query_value(None, "busy_timeout", |r| r.get(0))
                .unwrap();
            assert!(busy >= 5000, "busy_timeout must be >= 5000, got {busy}");
        };

        // Writer connection.
        check(&db.write());
        // Pooled reader connection.
        db.with_read(|conn| {
            check(conn);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn json1_capability_available_on_authority() {
        // Task 1.1.6: JSON1 must be available; a json_valid/json query succeeds.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let valid: i64 = conn
            .query_row("SELECT json_valid('{\"k\":1}')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(valid, 1, "json_valid must return 1 for valid JSON");
        let round: String = conn
            .query_row("SELECT json(' { \"k\" : 1 } ')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(round, "{\"k\":1}", "json() must normalize/compact JSON");
    }

    #[test]
    fn reopen_is_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("mem.db");
        let v1 = Database::open(&path).unwrap().schema_version();
        let v2 = Database::open(&path).unwrap().schema_version();
        assert_eq!(v1, v2);
    }

    #[test]
    fn events_are_immutable() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        conn.execute(
            "INSERT INTO events(id, hlc, ts_utc, tz_offset_min, event_type, source, payload, checksum)
             VALUES ('e1','00','2026-01-01T00:00:00Z',0,'observation','user','{}','h')",
            [],
        )
        .unwrap();
        // UPDATE and DELETE must be aborted by the L1 triggers.
        let upd = conn.execute("UPDATE events SET payload='{\"x\":1}' WHERE id='e1'", []);
        assert!(upd.is_err(), "UPDATE on events must be rejected (L1)");
        let del = conn.execute("DELETE FROM events WHERE id='e1'", []);
        assert!(del.is_err(), "DELETE on events must be rejected (L1)");
    }

    /// Query whether a table exists in `sqlite_master`.
    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            == 1
    }

    #[test]
    fn v2_meta_tables_exist_and_singleton_seeded() {
        // (a) both v2 meta tables exist after open; (b) the authority_meta
        // singleton (id=1) is seeded with the v2 epoch.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        assert!(
            table_exists(&conn, "schema_versions"),
            "schema_versions (plural) table must exist"
        );
        assert!(
            table_exists(&conn, "authority_meta"),
            "authority_meta table must exist"
        );

        let (id, graph_revision, event_hlc, schema_epoch): (i64, i64, String, i64) = conn
            .query_row(
                "SELECT id, graph_revision, event_hlc, schema_epoch FROM authority_meta",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(id, 1, "singleton id must be 1");
        assert_eq!(graph_revision, 0, "graph_revision seeds at 0");
        assert_eq!(event_hlc, "", "event_hlc seeds empty");
        assert_eq!(schema_epoch, 2, "schema_epoch is the v2 epoch");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM authority_meta", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "exactly one singleton row");
    }

    #[test]
    fn authority_meta_rejects_extra_row() {
        // (c) INSERT of a second authority_meta row (id=2) is rejected.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let res = conn.execute(
            "INSERT INTO authority_meta(id, graph_revision, event_hlc, schema_epoch)
             VALUES (2, 0, '', 2)",
            [],
        );
        assert!(
            res.is_err(),
            "extra authority_meta row (id<>1) must be rejected"
        );
    }

    #[test]
    fn authority_meta_rejects_delete_of_singleton() {
        // (d) DELETE of the singleton is rejected.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let res = conn.execute("DELETE FROM authority_meta WHERE id = 1", []);
        assert!(
            res.is_err(),
            "deleting the authority_meta singleton must be rejected"
        );
    }

    #[test]
    fn schema_versions_rows_are_immutable() {
        // (e) UPDATE and DELETE on schema_versions are rejected.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        conn.execute(
            "INSERT INTO schema_versions(version, name, checksum, applied_at)
             VALUES (1, 'init', 'deadbeef', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        let upd = conn.execute(
            "UPDATE schema_versions SET checksum='changed' WHERE version=1",
            [],
        );
        assert!(upd.is_err(), "UPDATE on schema_versions must be rejected");
        let del = conn.execute("DELETE FROM schema_versions WHERE version=1", []);
        assert!(del.is_err(), "DELETE on schema_versions must be rejected");
    }

    // ── v2 events_v2 immutable event log (design §4.1, task 1.1.2) ──

    /// A valid `events_v2` insert with all NOT NULL columns populated. The
    /// payload representation is selected by `payload_plain` / `payload_cipher`
    /// so individual tests can exercise the exactly-one CHECK. `phase` is
    /// overridable so the invalid-phase case can reuse this builder.
    fn events_v2_insert(
        conn: &Connection,
        id: &str,
        hlc: &str,
        phase: &str,
        payload_plain: Option<&str>,
        payload_cipher: Option<&[u8]>,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO events_v2(
                 id, phase, hlc, ts_utc, tz_offset_min, event_type,
                 source_kind, source_id, actor_id,
                 namespace, owner_id, scope, sensitivity, policy_version,
                 payload_cipher, payload_plain, payload_encoding, payload_checksum,
                 schema_version)
             VALUES (?1, ?2, ?3, '2026-01-01T00:00:00Z', 0, 'observation',
                     'user', 'src-1', 'actor-1',
                     'ns', 'owner-1', 'private', 0, 'p1',
                     ?4, ?5, 'utf8', 'chk', 1)",
            rusqlite::params![id, phase, hlc, payload_cipher, payload_plain],
        )
    }

    #[test]
    fn events_v2_table_exists() {
        // (a) events_v2 exists after migrations run.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        assert!(
            table_exists(&conn, "events_v2"),
            "events_v2 table must exist"
        );
    }

    #[test]
    fn events_v2_valid_plain_payload_insert_succeeds() {
        // (b) payload_plain set + payload_cipher NULL → accepted.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let n = events_v2_insert(&conn, "e1", "h1", "start", Some("{}"), None).unwrap();
        assert_eq!(n, 1, "valid plain-payload insert must succeed");
    }

    #[test]
    fn events_v2_rejects_both_payloads() {
        // (c) both payload columns non-null → rejected by exactly-one CHECK.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let res = events_v2_insert(&conn, "e1", "h1", "start", Some("{}"), Some(b"x"));
        assert!(
            res.is_err(),
            "insert with BOTH payload columns must be rejected"
        );
    }

    #[test]
    fn events_v2_rejects_neither_payload() {
        // (d) neither payload column set → rejected by exactly-one CHECK.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let res = events_v2_insert(&conn, "e1", "h1", "start", None, None);
        assert!(
            res.is_err(),
            "insert with NEITHER payload column must be rejected"
        );
    }

    #[test]
    fn events_v2_rejects_invalid_phase() {
        // (e) invalid phase value → rejected by the phase CHECK.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let res = events_v2_insert(&conn, "e1", "h1", "bogus", Some("{}"), None);
        assert!(res.is_err(), "invalid phase value must be rejected");
    }

    #[test]
    fn events_v2_are_immutable() {
        // (f) UPDATE and DELETE on an events_v2 row are rejected by triggers.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        events_v2_insert(&conn, "e1", "h1", "start", Some("{}"), None).unwrap();
        let upd = conn.execute("UPDATE events_v2 SET outcome='x' WHERE id='e1'", []);
        assert!(upd.is_err(), "UPDATE on events_v2 must be rejected (L1)");
        let del = conn.execute("DELETE FROM events_v2 WHERE id='e1'", []);
        assert!(del.is_err(), "DELETE on events_v2 must be rejected (L1)");
    }

    #[test]
    fn events_v2_rejects_duplicate_hlc() {
        // (g) duplicate hlc → rejected by the inline UNIQUE constraint.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        events_v2_insert(&conn, "e1", "dup", "start", Some("{}"), None).unwrap();
        let res = events_v2_insert(&conn, "e2", "dup", "completion", Some("{}"), None);
        assert!(res.is_err(), "duplicate hlc must be rejected (UNIQUE)");
    }

    // ── v2 revisions / idempotency / audit (design §4.1, task 1.1.3) ──

    /// Insert a valid `graph_revisions` row. `revision`/`base_revision` are
    /// overridable so the contiguity CHECK can be exercised directly.
    fn graph_revision_insert(
        conn: &Connection,
        revision: i64,
        base_revision: i64,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO graph_revisions(
                 revision, base_revision, tx_id, committed_at,
                 actor_id, policy_hash, change_count)
             VALUES (?1, ?2, ?3, '2026-01-01T00:00:00Z', 'actor-1', 'ph', 0)",
            rusqlite::params![revision, base_revision, format!("tx-{revision}")],
        )
    }

    #[test]
    fn revisions_audit_tables_exist() {
        // (a) all four v2 tables from 0013 exist after migrations run.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        for t in [
            "idempotency_results",
            "graph_revisions",
            "graph_changes",
            "audit_records",
        ] {
            assert!(table_exists(&conn, t), "{t} table must exist");
        }
    }

    #[test]
    fn idempotency_results_rejects_duplicate_key() {
        // (b) composite PK (caller_partition, idempotency_key) rejects a dup.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let insert = |ch: &str| {
            conn.execute(
                "INSERT INTO idempotency_results(
                     caller_partition, idempotency_key, command_hash,
                     result_json, created_at)
                 VALUES ('cp-1', 'key-1', ?1, '{}', '2026-01-01T00:00:00Z')",
                rusqlite::params![ch],
            )
        };
        assert_eq!(insert("h1").unwrap(), 1, "first insert must succeed");
        let res = insert("h2");
        assert!(
            res.is_err(),
            "duplicate (caller_partition, idempotency_key) must be rejected (PK)"
        );
    }

    #[test]
    fn graph_revisions_enforce_contiguous_base() {
        // (c) base_revision != revision-1 rejected; contiguous insert succeeds.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let bad = graph_revision_insert(&conn, 1, 5);
        assert!(
            bad.is_err(),
            "base_revision != revision-1 must be rejected (CHECK)"
        );
        let ok = graph_revision_insert(&conn, 1, 0).unwrap();
        assert_eq!(ok, 1, "contiguous revision=1, base=0 must succeed");
    }

    #[test]
    fn graph_revisions_are_immutable() {
        // (d) UPDATE and DELETE on graph_revisions are rejected by triggers.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        graph_revision_insert(&conn, 1, 0).unwrap();
        let upd = conn.execute(
            "UPDATE graph_revisions SET actor_id='x' WHERE revision=1",
            [],
        );
        assert!(
            upd.is_err(),
            "UPDATE on graph_revisions must be rejected (L1)"
        );
        let del = conn.execute("DELETE FROM graph_revisions WHERE revision=1", []);
        assert!(
            del.is_err(),
            "DELETE on graph_revisions must be rejected (L1)"
        );
    }

    #[test]
    fn graph_changes_constrain_kind_and_are_immutable() {
        // (e) invalid change_kind rejected, valid accepted (parent revision
        // inserted first for the FK); UPDATE/DELETE rejected by triggers.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        graph_revision_insert(&conn, 1, 0).unwrap();

        let insert = |ordinal: i64, kind: &str| {
            conn.execute(
                "INSERT INTO graph_changes(
                     revision, ordinal, record_kind, record_id,
                     change_kind, policy_partition)
                 VALUES (1, ?1, 'memory', 'rec-1', ?2, 'pp')",
                rusqlite::params![ordinal, kind],
            )
        };
        let bad = insert(0, "bogus");
        assert!(bad.is_err(), "invalid change_kind must be rejected (CHECK)");
        assert_eq!(
            insert(1, "insert").unwrap(),
            1,
            "valid change_kind must succeed"
        );

        let upd = conn.execute(
            "UPDATE graph_changes SET after_hash='x' WHERE revision=1 AND ordinal=1",
            [],
        );
        assert!(
            upd.is_err(),
            "UPDATE on graph_changes must be rejected (L1)"
        );
        let del = conn.execute(
            "DELETE FROM graph_changes WHERE revision=1 AND ordinal=1",
            [],
        );
        assert!(
            del.is_err(),
            "DELETE on graph_changes must be rejected (L1)"
        );
    }

    #[test]
    fn audit_records_constrain_disposition_and_are_immutable() {
        // (f) invalid disposition rejected, valid accepted; UPDATE/DELETE
        // rejected by triggers.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let insert = |id: &str, disp: &str| {
            conn.execute(
                "INSERT INTO audit_records(id, command_kind, disposition, created_at)
                 VALUES (?1, 'cmd', ?2, '2026-01-01T00:00:00Z')",
                rusqlite::params![id, disp],
            )
        };
        let bad = insert("a1", "bogus");
        assert!(bad.is_err(), "invalid disposition must be rejected (CHECK)");
        assert_eq!(
            insert("a2", "accepted").unwrap(),
            1,
            "valid disposition must succeed"
        );

        let upd = conn.execute(
            "UPDATE audit_records SET command_kind='x' WHERE id='a2'",
            [],
        );
        assert!(
            upd.is_err(),
            "UPDATE on audit_records must be rejected (L1)"
        );
        let del = conn.execute("DELETE FROM audit_records WHERE id='a2'", []);
        assert!(
            del.is_err(),
            "DELETE on audit_records must be rejected (L1)"
        );
    }

    // ── v2 outbox / manifests / recovery / shred / sources (task 1.1.4) ──

    /// Insert a `derived_outbox` row. `model_partition` is overridable so the
    /// semantic-uniqueness COALESCE behaviour can be exercised.
    fn derived_outbox_insert(
        conn: &Connection,
        model_partition: Option<&str>,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO derived_outbox(
                 target, op, record_kind, record_id, content_hash,
                 model_partition, created_at)
             VALUES ('fts', 'upsert', 'memory', 'rec-1', 'ch-1',
                     ?1, '2026-01-01T00:00:00Z')",
            rusqlite::params![model_partition],
        )
    }

    #[test]
    fn v2_outbox_manifest_shred_source_tables_exist() {
        // (a) all new 1.1.4 tables exist after migrations run.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        for t in [
            "derived_outbox",
            "derived_manifests",
            "recovery_snapshots",
            "shred_keys_v2",
            "sources",
            "interchange_imports",
        ] {
            assert!(table_exists(&conn, t), "{t} table must exist");
        }
    }

    #[test]
    fn derived_outbox_semantic_unique_index() {
        // (b) two identical NULL-model rows rejected by the semantic UNIQUE
        // index; NULL vs 'p' model_partition are allowed (COALESCE distinct).
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        assert_eq!(
            derived_outbox_insert(&conn, None).unwrap(),
            1,
            "first NULL-model row must succeed"
        );
        let dup = derived_outbox_insert(&conn, None);
        assert!(
            dup.is_err(),
            "second identical NULL-model row must be rejected (COALESCE('')→dup)"
        );
        // Differ only by model_partition = 'p' → allowed (distinct under COALESCE).
        assert_eq!(
            derived_outbox_insert(&conn, Some("p")).unwrap(),
            1,
            "row differing only by model_partition='p' must be allowed"
        );
    }

    #[test]
    fn derived_outbox_retry_state_defaults() {
        // Retry/dead-letter state defaults: attempts=0, status='pending'.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        derived_outbox_insert(&conn, None).unwrap();
        let (attempts, status): (i64, String) = conn
            .query_row(
                "SELECT attempts, status FROM derived_outbox WHERE record_id='rec-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(attempts, 0, "attempts defaults to 0");
        assert_eq!(status, "pending", "status defaults to 'pending'");
    }

    #[test]
    fn shred_keys_status_check_and_terminal_destroyed() {
        // (c) invalid status rejected; active→destroyed allowed; destroyed→active
        // REJECTED by the terminal trigger (no secret bytes are ever stored).
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let insert = |subject: &str, status: &str| {
            conn.execute(
                "INSERT INTO shred_keys_v2(subject_id, key_version, key_ref, status)
                 VALUES (?1, 1, 'keyring://ref', ?2)",
                rusqlite::params![subject, status],
            )
        };
        // Invalid status → CHECK rejects.
        assert!(
            insert("s-bad", "bogus").is_err(),
            "invalid shred_keys status must be rejected (CHECK)"
        );
        // Valid active insert.
        assert_eq!(insert("s1", "active").unwrap(), 1, "active insert succeeds");
        // active → destroyed allowed.
        let to_destroyed = conn.execute(
            "UPDATE shred_keys_v2 SET status='destroyed' WHERE subject_id='s1' AND key_version=1",
            [],
        );
        assert!(to_destroyed.is_ok(), "active→destroyed must be allowed");
        // destroyed → active REJECTED by the terminal trigger.
        let resurrect = conn.execute(
            "UPDATE shred_keys_v2 SET status='active' WHERE subject_id='s1' AND key_version=1",
            [],
        );
        assert!(
            resurrect.is_err(),
            "destroyed→active must be rejected (destroyed is terminal)"
        );
    }

    #[test]
    fn sources_source_kind_check() {
        // (d) invalid source_kind rejected; a valid one accepted (with the
        // required NOT NULL policy columns populated).
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let insert = |id: &str, kind: &str| {
            conn.execute(
                "INSERT INTO sources(
                     id, source_kind, namespace, owner_id, scope,
                     sensitivity, policy_version)
                 VALUES (?1, ?2, 'ns', 'owner-1', 'private', 0, 'p1')",
                rusqlite::params![id, kind],
            )
        };
        assert!(
            insert("src-bad", "bogus").is_err(),
            "invalid source_kind must be rejected (CHECK)"
        );
        assert_eq!(
            insert("src-ok", "openclaw").unwrap(),
            1,
            "valid source_kind must be accepted"
        );
    }

    #[test]
    fn recovery_snapshots_null_verified_at_inserts() {
        // (e) a recovery_snapshots row with NULL verified_at inserts fine (no
        // validity claim until verification sets verified_at).
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let n = conn
            .execute(
                "INSERT INTO recovery_snapshots(
                     id, path_ref, schema_version, revision, checksum, verified_at)
                 VALUES ('snap-1', '/tmp/snap', 14, 0, 'ck', NULL)",
                [],
            )
            .unwrap();
        assert_eq!(n, 1, "NULL verified_at snapshot must insert");
    }

    // ── v2 secondary indexes (design §4.1, task 1.1.5) ──

    /// Query whether an index exists in `sqlite_master`.
    fn index_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
            [name],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            == 1
    }

    /// Insert an `events_v2` row with explicit source-identity columns so the
    /// partial UNIQUE (source_kind, source_id, source_event_id) can be
    /// exercised. `source_event_id` is nullable to hit the partial predicate.
    fn events_v2_insert_src(
        conn: &Connection,
        id: &str,
        hlc: &str,
        source_kind: &str,
        source_id: &str,
        source_event_id: Option<&str>,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO events_v2(
                 id, source_event_id, phase, hlc, ts_utc, tz_offset_min, event_type,
                 source_kind, source_id, actor_id,
                 namespace, owner_id, scope, sensitivity, policy_version,
                 payload_plain, payload_encoding, payload_checksum, schema_version)
             VALUES (?1, ?2, 'start', ?3, '2026-01-01T00:00:00Z', 0, 'observation',
                     ?4, ?5, 'actor-1',
                     'ns', 'owner-1', 'private', 0, 'p1',
                     '{}', 'utf8', 'chk', 1)",
            rusqlite::params![id, source_event_id, hlc, source_kind, source_id],
        )
    }

    #[test]
    fn v2_secondary_indexes_exist() {
        // (a) a representative set of the 1.1.5 indexes exists after migrations.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        for idx in [
            "uq_events_v2_source_identity",
            "idx_events_v2_policy",
            "idx_derived_outbox_pending",
            "idx_sources_policy",
            "uq_interchange_imports_idem",
        ] {
            assert!(index_exists(&conn, idx), "{idx} index must exist");
        }
    }

    #[test]
    fn events_v2_source_identity_partial_unique_enforced() {
        // (b) the partial UNIQUE rejects duplicate NON-NULL source identities
        // but permits multiple rows whose source_event_id is NULL (excluded by
        // the WHERE source_event_id IS NOT NULL predicate).
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        // First non-null source identity — accepted.
        assert_eq!(
            events_v2_insert_src(&conn, "e1", "h1", "mcp", "src-1", Some("se-1")).unwrap(),
            1,
            "first non-null source identity must insert"
        );
        // Same (source_kind, source_id, source_event_id) — rejected.
        let dup = events_v2_insert_src(&conn, "e2", "h2", "mcp", "src-1", Some("se-1"));
        assert!(
            dup.is_err(),
            "duplicate non-null source identity must be rejected (partial UNIQUE)"
        );

        // Two rows with NULL source_event_id but otherwise identical source
        // fields — BOTH allowed (partial predicate excludes NULLs).
        assert_eq!(
            events_v2_insert_src(&conn, "e3", "h3", "mcp", "src-1", None).unwrap(),
            1,
            "first NULL source_event_id row must insert"
        );
        assert_eq!(
            events_v2_insert_src(&conn, "e4", "h4", "mcp", "src-1", None).unwrap(),
            1,
            "second NULL source_event_id row must also insert (partial excludes NULL)"
        );
    }

    #[test]
    fn interchange_imports_idempotency_partial_unique_enforced() {
        // (c) uq_interchange_imports_idem rejects duplicate non-null
        // idempotency_key but allows multiple NULL keys.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let insert = |id: &str, idem: Option<&str>| {
            conn.execute(
                "INSERT INTO interchange_imports(id, idempotency_key, created_at)
                 VALUES (?1, ?2, '2026-01-01T00:00:00Z')",
                rusqlite::params![id, idem],
            )
        };

        assert_eq!(
            insert("imp-1", Some("k1")).unwrap(),
            1,
            "first non-null idempotency_key must insert"
        );
        assert!(
            insert("imp-2", Some("k1")).is_err(),
            "duplicate non-null idempotency_key must be rejected (partial UNIQUE)"
        );
        // Multiple NULL idempotency_key rows — all allowed.
        assert_eq!(
            insert("imp-3", None).unwrap(),
            1,
            "first NULL idempotency_key row must insert"
        );
        assert_eq!(
            insert("imp-4", None).unwrap(),
            1,
            "second NULL idempotency_key row must also insert (partial excludes NULL)"
        );
    }

    #[test]
    fn read_pool_sees_writes() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("mem.db")).unwrap();
        {
            let conn = db.write();
            conn.execute(
                "INSERT INTO events(id, hlc, ts_utc, tz_offset_min, event_type, source, payload, checksum)
                 VALUES ('e1','00','2026-01-01T00:00:00Z',0,'observation','user','{}','h')",
                [],
            )
            .unwrap();
        }
        let count: i64 = db
            .with_read(|c| {
                Ok(c.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
                    .map_err(StorageError::Sqlite)?)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    fn insert_event(db: &Database, id: &str) {
        let conn = db.write();
        conn.execute(
            "INSERT INTO events(id, hlc, ts_utc, tz_offset_min, event_type, source, payload, checksum)
             VALUES (?1,'00','2026-01-01T00:00:00Z',0,'observation','user','{}','h')",
            [id],
        )
        .unwrap();
    }

    fn read_event_count(db: &Database) -> i64 {
        db.with_read(|c| {
            Ok(c.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
                .map_err(StorageError::Sqlite)?)
        })
        .unwrap()
    }

    #[test]
    fn backup_restore_rebuilds_read_pool_no_stale_reads() {
        // H3: after restore, the read pool must be rebuilt so pooled readers do
        // NOT serve pages cached from the pre-restore database.
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("a.db");
        let backup = dir.path().join("b.db");
        let db = Database::open(&db_path).unwrap();

        insert_event(&db, "e1");
        // Warm every read-pool connection so each caches the 1-row state.
        for _ in 0..(db.read_pool_size * 2 + 2) {
            assert_eq!(read_event_count(&db), 1);
        }

        let bytes = db.backup_to(&backup).unwrap();
        assert!(bytes > 0);

        // Mutate after the backup.
        insert_event(&db, "e2");
        assert_eq!(read_event_count(&db), 2);

        // Restore → every reader must now observe the 1-row snapshot.
        db.restore_from(&backup).unwrap();
        for _ in 0..(db.read_pool_size * 2 + 2) {
            assert_eq!(
                read_event_count(&db),
                1,
                "no reader may serve a stale pre-restore page"
            );
        }
    }

    // ── v2 cognitive records + semantic base (design §4.2/§4.3, task 2.1.1) ──

    /// Seed a single valid `events_v2` row (`id='ev1'`) so FK targets resolve
    /// for the cognitive-record inserts below.
    fn seed_event(conn: &Connection) {
        events_v2_insert(conn, "ev1", "hlc-ev1", "observation", Some("{}"), None).unwrap();
    }

    /// Insert a valid canonical entity (`id='en1'`) referencing `ev1`.
    fn seed_entity(conn: &Connection) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO entities_v2(
                 id, namespace, owner_id, scope, sensitivity, source_id,
                 policy_version, created_event_id, created_at)
             VALUES ('en1','ns','o','private',0,'src','p1','ev1','2026-01-01T00:00:00Z')",
            [],
        )
    }

    #[test]
    fn cognitive_tables_exist() {
        // (a) every 0017 table exists after migrations run.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        for t in [
            "records",
            "entities_v2",
            "aliases",
            "mentions",
            "evidence_v2",
            "episodes_v2",
            "goals_v2",
            "goal_progress",
            "consolidation_runs",
            "tool_observations",
            "retrieval_traces",
            "retrieval_trace_items",
            "feedback",
        ] {
            assert!(table_exists(&conn, t), "table {t} must exist after 0017");
        }
        // Deferred sources indexes were added in 0017.
        for ix in [
            "idx_sources_identity",
            "idx_sources_version",
            "idx_sources_policy",
            "idx_sources_lifecycle",
            "idx_aliases_identity",
            "idx_records_policy",
        ] {
            assert!(index_exists(&conn, ix), "index {ix} must exist after 0017");
        }
    }

    #[test]
    fn records_valid_insert_succeeds() {
        // (b) a well-formed record with a single plaintext payload is accepted.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        seed_event(&conn);
        let n = conn
            .execute(
                "INSERT INTO records(
                     id, record_kind, schema_version, content,
                     namespace, owner_id, scope, sensitivity, source_id, policy_version,
                     created_event_id, created_at, estimated_tokens)
                 VALUES ('r1','memory',1,'hi',
                         'ns','o','private',0,'src','p1',
                         'ev1','2026-01-01T00:00:00Z',3)",
                [],
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn records_rejects_unknown_kind() {
        // (c) record_kind outside the CHECK set is rejected.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        seed_event(&conn);
        let res = conn.execute(
            "INSERT INTO records(
                 id, record_kind, schema_version, content,
                 namespace, owner_id, scope, sensitivity, source_id, policy_version,
                 created_event_id, created_at)
             VALUES ('r1','fact',1,'hi',
                     'ns','o','private',0,'src','p1','ev1','2026-01-01T00:00:00Z')",
            [],
        );
        assert!(res.is_err(), "unknown record_kind must be rejected");
    }

    #[test]
    fn records_reject_both_and_neither_payload() {
        // (d) payload exclusivity CHECK: both non-null and both null are rejected.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        seed_event(&conn);
        let both = conn.execute(
            "INSERT INTO records(
                 id, record_kind, schema_version, content, content_cipher,
                 namespace, owner_id, scope, sensitivity, source_id, policy_version,
                 created_event_id, created_at)
             VALUES ('r1','memory',1,'hi',x'01',
                     'ns','o','private',0,'src','p1','ev1','2026-01-01T00:00:00Z')",
            [],
        );
        assert!(both.is_err(), "both payload columns must be rejected");
        let neither = conn.execute(
            "INSERT INTO records(
                 id, record_kind, schema_version,
                 namespace, owner_id, scope, sensitivity, source_id, policy_version,
                 created_event_id, created_at)
             VALUES ('r2','memory',1,
                     'ns','o','private',0,'src','p1','ev1','2026-01-01T00:00:00Z')",
            [],
        );
        assert!(neither.is_err(), "neither payload column must be rejected");
    }

    #[test]
    fn records_reject_inverted_interval_and_negative_tokens() {
        // (e) inverted valid interval and negative estimated_tokens are rejected.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        seed_event(&conn);
        let inverted = conn.execute(
            "INSERT INTO records(
                 id, record_kind, schema_version, content, valid_from, valid_until,
                 namespace, owner_id, scope, sensitivity, source_id, policy_version,
                 created_event_id, created_at)
             VALUES ('r1','memory',1,'hi','2026-06-01T00:00:00Z','2026-01-01T00:00:00Z',
                     'ns','o','private',0,'src','p1','ev1','2026-01-01T00:00:00Z')",
            [],
        );
        assert!(
            inverted.is_err(),
            "inverted valid interval must be rejected"
        );
        let negative = conn.execute(
            "INSERT INTO records(
                 id, record_kind, schema_version, content, estimated_tokens,
                 namespace, owner_id, scope, sensitivity, source_id, policy_version,
                 created_event_id, created_at)
             VALUES ('r2','memory',1,'hi',-1,
                     'ns','o','private',0,'src','p1','ev1','2026-01-01T00:00:00Z')",
            [],
        );
        assert!(
            negative.is_err(),
            "negative estimated_tokens must be rejected"
        );
    }

    #[test]
    fn aliases_enforce_unique_identity() {
        // (f) UNIQUE (normalized_alias, alias_type, namespace, scope).
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        seed_event(&conn);
        seed_entity(&conn).unwrap();
        let insert = |id: &str| {
            conn.execute(
                "INSERT INTO aliases(
                     id, entity_id, alias, normalized_alias, alias_type,
                     namespace, owner_id, scope, sensitivity, source_id, policy_version)
                 VALUES (?1,'en1','Ada','ada','name','ns','o','private',0,'src','p1')",
                rusqlite::params![id],
            )
        };
        insert("a1").unwrap();
        assert!(
            insert("a2").is_err(),
            "duplicate (normalized_alias,alias_type,namespace,scope) must be rejected"
        );
    }

    #[test]
    fn mentions_reject_inverted_span_and_bad_locator_json() {
        // (g) span order CHECK + json_valid guard on locator_json.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        seed_event(&conn);
        seed_entity(&conn).unwrap();
        let inverted = conn.execute(
            "INSERT INTO mentions(
                 id, entity_id, span_start, span_end,
                 namespace, owner_id, scope, sensitivity, source_id, policy_version)
             VALUES ('m1','en1',10,2,'ns','o','private',0,'src','p1')",
            [],
        );
        assert!(inverted.is_err(), "inverted span must be rejected");
        let bad_json = conn.execute(
            "INSERT INTO mentions(
                 id, entity_id, locator_json,
                 namespace, owner_id, scope, sensitivity, source_id, policy_version)
             VALUES ('m2','en1','not json','ns','o','private',0,'src','p1')",
            [],
        );
        assert!(bad_json.is_err(), "invalid locator_json must be rejected");
    }

    #[test]
    fn evidence_rejects_bad_polarity() {
        // (h) polarity CHECK(supports/contradicts).
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let res = conn.execute(
            "INSERT INTO evidence_v2(
                 id, polarity, namespace, owner_id, scope, sensitivity, source_id, policy_version)
             VALUES ('e1','maybe','ns','o','private',0,'src','p1')",
            [],
        );
        assert!(res.is_err(), "invalid polarity must be rejected");
    }

    #[test]
    fn goals_reject_bad_status_and_priority() {
        // (i) status CHECK set + priority 0..10.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let bad_status = conn.execute(
            "INSERT INTO goals_v2(
                 id, status, namespace, owner_id, scope, sensitivity, source_id, policy_version)
             VALUES ('g1','failed','ns','o','private',0,'src','p1')",
            [],
        );
        assert!(
            bad_status.is_err(),
            "legacy 'failed' status must be rejected"
        );
        let bad_priority = conn.execute(
            "INSERT INTO goals_v2(
                 id, status, priority,
                 namespace, owner_id, scope, sensitivity, source_id, policy_version)
             VALUES ('g2','active',11,'ns','o','private',0,'src','p1')",
            [],
        );
        assert!(bad_priority.is_err(), "priority 11 must be rejected");
    }

    #[test]
    fn goal_progress_is_append_only() {
        // (j) UPDATE and DELETE on goal_progress are rejected by triggers.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        conn.execute(
            "INSERT INTO goals_v2(
                 id, status, namespace, owner_id, scope, sensitivity, source_id, policy_version)
             VALUES ('g1','active','ns','o','private',0,'src','p1')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO goal_progress(id, goal_id, state) VALUES ('gp1','g1','started')",
            [],
        )
        .unwrap();
        let upd = conn.execute("UPDATE goal_progress SET state='x' WHERE id='gp1'", []);
        assert!(
            upd.is_err(),
            "UPDATE on goal_progress must be rejected (L1)"
        );
        let del = conn.execute("DELETE FROM goal_progress WHERE id='gp1'", []);
        assert!(
            del.is_err(),
            "DELETE on goal_progress must be rejected (L1)"
        );
    }

    #[test]
    fn consolidation_runs_enforce_unique_identity() {
        // (k) UNIQUE (algorithm, version, input_set_hash, level).
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let insert = |id: &str| {
            conn.execute(
                "INSERT INTO consolidation_runs(id, algorithm, version, input_set_hash, level)
                 VALUES (?1,'a','1','h','summary')",
                rusqlite::params![id],
            )
        };
        insert("c1").unwrap();
        assert!(
            insert("c2").is_err(),
            "duplicate (algorithm,version,input_set_hash,level) must be rejected"
        );
    }

    #[test]
    fn tool_observations_enforce_unique_invocation() {
        // (l) unique invocation completion.
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let insert = |id: &str| {
            conn.execute(
                "INSERT INTO tool_observations(
                     id, invocation_id,
                     namespace, owner_id, scope, sensitivity, source_id, policy_version)
                 VALUES (?1,'inv1','ns','o','private',0,'src','p1')",
                rusqlite::params![id],
            )
        };
        insert("t1").unwrap();
        assert!(
            insert("t2").is_err(),
            "duplicate invocation_id must be rejected"
        );
    }
}
