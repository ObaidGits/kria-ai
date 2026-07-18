//! Additive-only schema migrations (memory-upgrade design §31.2, Issue 18).
//!
//! Migrations never drop or rename columns; they only add. Each migration is
//! applied once inside a transaction and recorded in `schema_version` with a
//! BLAKE3 checksum of its script. A newer binary can always read an older DB;
//! a downgrade that would require dropping schema is refused.

use rusqlite::Connection;

use crate::memory::error::{MemoryResult, MigrationError, StorageError};
use crate::memory::ids::blake3_hex;

/// A single ordered migration step.
struct Migration {
    version: u32,
    sql: &'static str,
}

/// The ordered migration set. Append new entries; never edit an applied one
/// (edits are caught by the checksum guard).
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("schema/0001_init.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("schema/0002_vectors.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("schema/0003_library.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("schema/0004_conversation.sql"),
    },
    Migration {
        version: 5,
        sql: include_str!("schema/0005_runtime_compat.sql"),
    },
    Migration {
        version: 6,
        sql: include_str!("schema/0006_goals.sql"),
    },
    Migration {
        version: 7,
        sql: include_str!("schema/0007_plans.sql"),
    },
    Migration {
        version: 8,
        sql: include_str!("schema/0008_reasoning.sql"),
    },
    Migration {
        version: 9,
        sql: include_str!("schema/0009_retrieval_weights.sql"),
    },
    Migration {
        version: 10,
        sql: include_str!("schema/0010_causal.sql"),
    },
];

/// The highest schema version this binary knows about.
pub fn latest_version() -> u32 {
    MIGRATIONS.iter().map(|m| m.version).max().unwrap_or(0)
}

/// Apply all pending migrations to `conn`. Idempotent: already-applied
/// migrations are skipped. Returns the resulting schema version.
pub fn run(conn: &Connection) -> MemoryResult<u32> {
    // Bootstrap the version table so we can query applied versions.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL,
            checksum TEXT NOT NULL
        );",
    )
    .map_err(StorageError::Sqlite)?;

    let applied = applied_version(conn)?;
    let target = latest_version();

    // Refuse a silent downgrade: the DB is newer than this binary understands.
    if applied > target {
        return Err(MigrationError::SchemaTooOld {
            found: applied,
            required: target,
        }
        .into());
    }

    for m in MIGRATIONS {
        if m.version <= applied {
            verify_checksum(conn, m)?;
            continue;
        }
        apply_one(conn, m)?;
    }

    Ok(target)
}

/// Highest applied version, or 0 if none.
fn applied_version(conn: &Connection) -> MemoryResult<u32> {
    let v: Option<i64> = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .map_err(StorageError::Sqlite)?;
    Ok(v.unwrap_or(0) as u32)
}

/// Apply a single migration inside a transaction and record it.
fn apply_one(conn: &Connection, m: &Migration) -> MemoryResult<()> {
    let checksum = blake3_hex(m.sql.as_bytes());
    conn.execute_batch("BEGIN;").map_err(StorageError::Sqlite)?;
    let result = (|| -> Result<(), StorageError> {
        conn.execute_batch(m.sql).map_err(StorageError::Sqlite)?;
        conn.execute(
            "INSERT INTO schema_version(version, applied_at, checksum) VALUES (?1, ?2, ?3)",
            rusqlite::params![m.version as i64, chrono::Utc::now().to_rfc3339(), checksum],
        )
        .map_err(StorageError::Sqlite)?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT;")
                .map_err(StorageError::Sqlite)?;
            tracing::info!(version = m.version, "applied memory schema migration");
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(e.into())
        }
    }
}

/// Guard against an applied migration being edited after the fact.
fn verify_checksum(conn: &Connection, m: &Migration) -> MemoryResult<()> {
    let recorded: Option<String> = conn
        .query_row(
            "SELECT checksum FROM schema_version WHERE version = ?1",
            [m.version as i64],
            |r| r.get(0),
        )
        .map_err(StorageError::Sqlite)?;
    let expected = blake3_hex(m.sql.as_bytes());
    match recorded {
        Some(c) if c == expected => Ok(()),
        Some(_) => Err(MigrationError::Script(format!(
            "migration {} checksum mismatch: script changed after being applied",
            m.version
        ))
        .into()),
        None => Ok(()),
    }
}
