//! Config storage backends (settings-config-revamp, Task 4).
//!
//! The **user layer** of configuration can be persisted either as the legacy
//! whole-file TOML (`~/.kria/config.toml`, via `service::TomlFilePersist`) or as
//! **field-level rows** in `kria.db` (`SqliteConfigStore`). The backend is
//! selected by `KRIA_CONFIG_BACKEND` (default `toml`).
//!
//! This module provides the SQLite store (field-level `put`/`delete`/`all` +
//! `config_version` via `PRAGMA user_version`). Wiring the store into
//! `ConfigService`'s layered read/resolve (`code < default.toml < DB < env`) is
//! finalized alongside the precedence rework (Task 7); until then the store is
//! flag-gated and inert (default backend = `toml`), so legacy behaviour holds
//! byte-for-byte (Req 13.3 / Property 10).

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};

/// Current config-DB schema version (drives additive migrations in Task 5).
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// Selected user-layer storage backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigBackend {
    Toml,
    Sqlite,
}

impl ConfigBackend {
    /// Read the backend from `KRIA_CONFIG_BACKEND`. Default is now **Sqlite**
    /// (settings-config-revamp cutover): user config lives in `kria.db`, with a
    /// one-time migration of any legacy `~/.kria/config.toml`. Set
    /// `KRIA_CONFIG_BACKEND=toml` to opt back into the legacy file backend.
    pub fn from_env() -> Self {
        match std::env::var("KRIA_CONFIG_BACKEND")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "toml" => ConfigBackend::Toml,
            _ => ConfigBackend::Sqlite,
        }
    }
}

/// One persisted config field (a `(section, key)` row).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigRow {
    pub section: String,
    pub key: String,
    pub value_json: String,
    pub source: String,
    pub updated_at: String,
}

/// One atomic field-level mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigMutation {
    Put {
        section: String,
        key: String,
        value_json: String,
        source: String,
    },
    Delete {
        section: String,
        key: String,
    },
}

/// Field-level user-layer persistence.
pub trait ConfigStore: Send + Sync {
    /// Atomically apply all mutations or leave the store unchanged.
    fn apply_batch(&self, mutations: &[ConfigMutation]) -> Result<(), String>;
    /// Upsert one field. `value_json` is the serialized JSON value.
    fn put(&self, section: &str, key: &str, value_json: &str, source: &str) -> Result<(), String> {
        self.apply_batch(&[ConfigMutation::Put {
            section: section.to_string(),
            key: key.to_string(),
            value_json: value_json.to_string(),
            source: source.to_string(),
        }])
    }
    /// Remove one field (revert to baseline/default for it).
    fn delete(&self, section: &str, key: &str) -> Result<(), String> {
        self.apply_batch(&[ConfigMutation::Delete {
            section: section.to_string(),
            key: key.to_string(),
        }])
    }
    /// Read the entire user layer (all overridden fields).
    fn all(&self) -> Result<Vec<ConfigRow>, String>;
    /// The DB schema version (`PRAGMA user_version`).
    fn config_version(&self) -> u32;
}

/// SQLite-backed field-level config store over `kria.db` (WAL).
pub struct SqliteConfigStore {
    conn: Mutex<Connection>,
}

impl SqliteConfigStore {
    /// Open (or create) the store at `db_path` and initialize the schema.
    pub fn open(db_path: &Path) -> Result<Self, String> {
        let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
        Self::init(conn)
    }

    /// Ephemeral in-memory store (tests).
    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self, String> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS config (
                 section    TEXT NOT NULL,
                 key        TEXT NOT NULL,
                 value_json TEXT NOT NULL,
                 source     TEXT NOT NULL DEFAULT 'ui',
                 updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                 PRIMARY KEY (section, key)
             );
             CREATE TABLE IF NOT EXISTS config_meta ( config_version INTEGER NOT NULL );",
        )
        .map_err(|e| e.to_string())?;

        // Set the schema version via PRAGMA user_version (migration marker, Task 5).
        let current: u32 = conn
            .query_row("SELECT * FROM pragma_user_version", [], |r| r.get(0))
            .unwrap_or(0);
        if current < CONFIG_SCHEMA_VERSION {
            // pragma_update requires the value inline; safe (internal constant).
            let _ = conn.pragma_update(None, "user_version", CONFIG_SCHEMA_VERSION);
        }

        // Seed a single config_meta row for human/debug visibility.
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM config_meta", [], |r| r.get(0))
            .unwrap_or(0);
        if rows == 0 {
            let _ = conn.execute(
                "INSERT INTO config_meta (config_version) VALUES (?1)",
                params![CONFIG_SCHEMA_VERSION],
            );
        }

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl ConfigStore for SqliteConfigStore {
    fn apply_batch(&self, mutations: &[ConfigMutation]) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for mutation in mutations {
            match mutation {
                ConfigMutation::Put {
                    section,
                    key,
                    value_json,
                    source,
                } => {
                    tx.execute(
                        "INSERT INTO config (section, key, value_json, source, updated_at)
                         VALUES (?1, ?2, ?3, ?4, datetime('now'))
                         ON CONFLICT(section, key) DO UPDATE SET
                             value_json = excluded.value_json,
                             source     = excluded.source,
                             updated_at = datetime('now')",
                        params![section, key, value_json, source],
                    )
                    .map_err(|e| e.to_string())?;
                }
                ConfigMutation::Delete { section, key } => {
                    tx.execute(
                        "DELETE FROM config WHERE section = ?1 AND key = ?2",
                        params![section, key],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
        }
        tx.commit().map_err(|e| e.to_string())
    }

    fn all(&self) -> Result<Vec<ConfigRow>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT section, key, value_json, source, updated_at FROM config ORDER BY section, key")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ConfigRow {
                    section: r.get(0)?,
                    key: r.get(1)?,
                    value_json: r.get(2)?,
                    source: r.get(3)?,
                    updated_at: r.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    fn config_version(&self) -> u32 {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return 0,
        };
        conn.query_row("SELECT * FROM pragma_user_version", [], |r| r.get(0))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_is_set() {
        let store = SqliteConfigStore::open_in_memory().unwrap();
        assert_eq!(store.config_version(), CONFIG_SCHEMA_VERSION);
    }

    #[test]
    fn put_get_roundtrip() {
        let store = SqliteConfigStore::open_in_memory().unwrap();
        store.put("ui", "theme", "\"dark\"", "ui").unwrap();
        let all = store.all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].section, "ui");
        assert_eq!(all[0].key, "theme");
        assert_eq!(all[0].value_json, "\"dark\"");
        assert_eq!(all[0].source, "ui");
    }

    #[test]
    fn put_is_field_level_isolated() {
        // Changing one field must NOT touch other rows (the core anti-drift
        // property — Req 1.3 / Property 3).
        let store = SqliteConfigStore::open_in_memory().unwrap();
        store.put("ui", "theme", "\"dark\"", "ui").unwrap();
        store.put("ui", "font_scale", "1.5", "ui").unwrap();
        let before = store.all().unwrap();
        let theme_before = before.iter().find(|r| r.key == "theme").unwrap().clone();

        // Update a DIFFERENT field.
        store.put("voice", "enabled", "true", "prompt").unwrap();

        let after = store.all().unwrap();
        let theme_after = after.iter().find(|r| r.key == "theme").unwrap().clone();
        // The untouched row is byte-identical (value + source + updated_at).
        assert_eq!(theme_before, theme_after);
        assert_eq!(after.len(), 3);
    }

    #[test]
    fn upsert_updates_in_place() {
        let store = SqliteConfigStore::open_in_memory().unwrap();
        store.put("ui", "theme", "\"light\"", "ui").unwrap();
        store.put("ui", "theme", "\"dark\"", "prompt").unwrap();
        let all = store.all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].value_json, "\"dark\"");
        assert_eq!(all[0].source, "prompt");
    }

    #[test]
    fn delete_removes_override() {
        let store = SqliteConfigStore::open_in_memory().unwrap();
        store.put("ui", "theme", "\"dark\"", "ui").unwrap();
        store.delete("ui", "theme").unwrap();
        assert!(store.all().unwrap().is_empty());
    }

    #[test]
    fn batch_rolls_back_when_later_mutation_fails() {
        let store = SqliteConfigStore::open_in_memory().unwrap();
        store
            .conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER reject_failed_config
                 BEFORE INSERT ON config
                 WHEN NEW.key = 'reject_me'
                 BEGIN
                     SELECT RAISE(ABORT, 'injected config write failure');
                 END;",
            )
            .unwrap();

        let result = store.apply_batch(&[
            ConfigMutation::Put {
                section: "ui".into(),
                key: "theme".into(),
                value_json: "\"dark\"".into(),
                source: "ui".into(),
            },
            ConfigMutation::Put {
                section: "ui".into(),
                key: "reject_me".into(),
                value_json: "true".into(),
                source: "ui".into(),
            },
        ]);

        assert!(result.is_err());
        assert!(store.all().unwrap().is_empty());
    }

    #[test]
    fn backend_defaults_to_sqlite_after_cutover() {
        // Cutover: unset ⇒ Sqlite; explicit "toml" ⇒ Toml escape hatch.
        std::env::remove_var("KRIA_CONFIG_BACKEND");
        assert_eq!(ConfigBackend::from_env(), ConfigBackend::Sqlite);
        std::env::set_var("KRIA_CONFIG_BACKEND", "toml");
        assert_eq!(ConfigBackend::from_env(), ConfigBackend::Toml);
        std::env::remove_var("KRIA_CONFIG_BACKEND");
    }
}
