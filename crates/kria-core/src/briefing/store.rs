//! Persistence for the briefing configuration (single-row JSON in `kria.db`).

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};

use super::config::BriefingConfig;

pub struct BriefingStore {
    conn: Mutex<Connection>,
}

impl BriefingStore {
    pub fn open(db_path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS briefing_config (
                id   INTEGER PRIMARY KEY CHECK (id = 1),
                json TEXT NOT NULL
            );",
        )?;
        Ok(())
    }

    /// Get the saved config, or the default when none is stored / parse fails.
    pub fn get(&self) -> BriefingConfig {
        let conn = self.conn.lock().unwrap();
        let raw: Option<String> = conn
            .query_row(
                "SELECT json FROM briefing_config WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .ok();
        match raw.and_then(|j| serde_json::from_str::<BriefingConfig>(&j).ok()) {
            Some(cfg) => cfg,
            None => BriefingConfig::default(),
        }
    }

    /// Persist the config (upsert single row).
    pub fn set(&self, cfg: &BriefingConfig) -> anyhow::Result<()> {
        let json = serde_json::to_string(cfg)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO briefing_config (id, json) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            params![json],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_returns_default() {
        let s = BriefingStore::open_in_memory().unwrap();
        assert_eq!(s.get().sections.len(), 4);
    }

    #[test]
    fn set_then_get_roundtrip() {
        let s = BriefingStore::open_in_memory().unwrap();
        let mut cfg = BriefingConfig::default();
        cfg.sections.truncate(1);
        cfg.sections[0].query = Some("subject:invoice".into());
        cfg.schedule.auto = true;
        s.set(&cfg).unwrap();
        let got = s.get();
        assert_eq!(got.sections.len(), 1);
        assert_eq!(got.sections[0].query.as_deref(), Some("subject:invoice"));
        assert!(got.schedule.auto);
    }
}
