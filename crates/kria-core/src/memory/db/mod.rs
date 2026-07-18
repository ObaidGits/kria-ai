//! SQLite authority connection management (memory-upgrade design §9/§14, tasks 3–4).
//!
//! The authority is the single source of truth (L2). This module owns:
//! * the **single serialized write connection** (invariant L10: only the atomic
//!   commit holds the writer),
//! * a **WAL read pool** so readers never block the writer,
//! * the **L14 guard** refusing a network-filesystem authority,
//! * migration application on open.

pub mod migrations;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use arc_swap::ArcSwap;
use rusqlite::Connection;

use crate::memory::error::{MemoryResult, StorageError};

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
        configure(&conn)?;
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
        configure(&writer)?;
        let schema_version = migrations::run(&writer)?;

        let mut read_pool = Vec::with_capacity(pool);
        for _ in 0..pool {
            let rc = Connection::open(&path).map_err(StorageError::Sqlite)?;
            configure(&rc)?;
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
        configure(&rc)?;
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

/// Apply the standard pragmas to every connection (design §9): WAL, FK
/// enforcement, a busy timeout for transient cross-connection contention, and
/// `synchronous=NORMAL` (safe under WAL).
fn configure(conn: &Connection) -> MemoryResult<()> {
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(StorageError::Sqlite)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(StorageError::Sqlite)?;
    conn.pragma_update(None, "busy_timeout", 5000)
        .map_err(StorageError::Sqlite)?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(StorageError::Sqlite)?;
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
}
