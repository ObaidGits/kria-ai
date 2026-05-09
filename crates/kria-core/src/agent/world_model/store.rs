//! World Model Store — SQLite-backed fact storage with conflict resolution.
//!
//! # Conflict Resolution Algorithm
//!
//! ```text
//! upsert(subject, predicate, object, confidence, source, evidence):
//!   existing = SELECT FROM world_facts WHERE subject=? AND predicate=?
//!   if existing is None:
//!     INSERT new fact → ConflictResolution::Inserted
//!   else if existing.object == object:
//!     // Same fact, new evidence — merge confidence
//!     new_conf = 1 - (1 - existing.conf) * (1 - confidence)
//!     UPDATE world_facts SET confidence=new_conf, evidence=merged
//!     → ConflictResolution::Merged
//!   else:
//!     // Contradiction — archive old, insert new
//!     INSERT old INTO world_facts_archive (deprecated_by=new_id)
//!     DELETE old FROM world_facts
//!     INSERT new fact
//!     → ConflictResolution::Overwritten
//! ```

use chrono::Utc;
use rusqlite::params;
use std::collections::HashMap;
use std::sync::Mutex;

use super::types::{ConflictResolution, FactSource, WorldFact, WorldModelStats};

/// SQLite-backed World Model with deterministic conflict resolution.
pub struct WorldModelStore {
    conn: Mutex<rusqlite::Connection>,
    /// How fast confidence decays without re-verification (per hour).
    decay_rate_per_hour: f64,
}

impl WorldModelStore {
    /// Open (or create) the World Model tables in an existing SQLite connection.
    pub fn open(conn: rusqlite::Connection) -> anyhow::Result<Self> {
        let store = Self {
            conn: Mutex::new(conn),
            decay_rate_per_hour: 0.05,
        };
        store.migrate()?;
        Ok(store)
    }

    /// Create from a path (standalone DB for testing).
    pub fn open_path(path: &std::path::Path) -> anyhow::Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        Self::open(conn)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS world_facts (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                subject        TEXT NOT NULL,
                predicate      TEXT NOT NULL,
                object         TEXT NOT NULL,
                confidence     REAL NOT NULL DEFAULT 0.5,
                evidence       TEXT NOT NULL DEFAULT '[]',
                source         TEXT NOT NULL DEFAULT 'inferred',
                last_verified  TEXT NOT NULL DEFAULT (datetime('now')),
                created_at     TEXT NOT NULL DEFAULT (datetime('now')),
                access_count   INTEGER NOT NULL DEFAULT 0,
                UNIQUE(subject, predicate)
            );
            CREATE INDEX IF NOT EXISTS idx_wf_subj ON world_facts(subject);
            CREATE INDEX IF NOT EXISTS idx_wf_pred ON world_facts(predicate);

            CREATE TABLE IF NOT EXISTS world_facts_archive (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                subject        TEXT NOT NULL,
                predicate      TEXT NOT NULL,
                object         TEXT NOT NULL,
                confidence     REAL NOT NULL,
                evidence       TEXT NOT NULL DEFAULT '[]',
                source         TEXT NOT NULL,
                last_verified  TEXT NOT NULL,
                created_at     TEXT NOT NULL,
                archived_at    TEXT NOT NULL DEFAULT (datetime('now')),
                deprecated_by  INTEGER,
                archive_reason TEXT NOT NULL DEFAULT 'contradicted'
            );
            CREATE INDEX IF NOT EXISTS idx_wfa_subj ON world_facts_archive(subject);

            CREATE VIRTUAL TABLE IF NOT EXISTS world_facts_fts USING fts5(
                subject, predicate, object,
                content=world_facts, content_rowid=id
            );
            ",
        )?;
        Ok(())
    }

    /// Insert or update a fact with conflict resolution.
    pub fn upsert(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        confidence: f64,
        source: FactSource,
        evidence: &str,
    ) -> anyhow::Result<ConflictResolution> {
        let conn = self.conn.lock().unwrap();

        // Check for existing fact with same (subject, predicate)
        let existing = conn.query_row(
            "SELECT id, object, confidence, evidence FROM world_facts WHERE subject = ?1 AND predicate = ?2",
            params![subject, predicate],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        );

        match existing {
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // No conflict — insert new fact
                let now = Utc::now().to_rfc3339();
                let evidence_json = serde_json::to_string(&vec![evidence.to_string()])
                    .unwrap_or_else(|_| format!("[\"{}\"]", evidence));
                conn.execute(
                    "INSERT INTO world_facts (subject, predicate, object, confidence, evidence, source, last_verified, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                    params![subject, predicate, object, confidence, evidence_json, source.to_string(), now],
                )?;
                let id = conn.last_insert_rowid();

                // Update FTS
                conn.execute(
                    "INSERT INTO world_facts_fts(rowid, subject, predicate, object) VALUES (?1, ?2, ?3, ?4)",
                    params![id, subject, predicate, object],
                )?;

                Ok(ConflictResolution::Inserted { id })
            }
            Err(e) => Err(e.into()),
            Ok((existing_id, existing_object, existing_conf, existing_evidence)) => {
                if existing_object == object {
                    // Same fact, new evidence — merge confidence
                    // Bayesian update: P(A|B) = 1 - (1-P(A))*(1-P(B))
                    let new_conf = 1.0 - (1.0 - existing_conf) * (1.0 - confidence);

                    // Merge evidence arrays
                    let mut ev_vec: Vec<String> = serde_json::from_str(&existing_evidence)
                        .unwrap_or_default();
                    if !ev_vec.contains(&evidence.to_string()) {
                        ev_vec.push(evidence.to_string());
                    }
                    let merged_evidence = serde_json::to_string(&ev_vec).unwrap_or_else(|_| "[]".into());

                    let now = Utc::now().to_rfc3339();
                    conn.execute(
                        "UPDATE world_facts SET confidence = ?1, evidence = ?2, last_verified = ?3, source = ?4, access_count = access_count + 1
                         WHERE id = ?5",
                        params![new_conf, merged_evidence, now, source.to_string(), existing_id],
                    )?;

                    Ok(ConflictResolution::Merged {
                        id: existing_id,
                        new_confidence: new_conf,
                    })
                } else {
                    // Contradiction — archive old, insert new
                    let now = Utc::now().to_rfc3339();

                    // Archive the old fact
                    conn.execute(
                        "INSERT INTO world_facts_archive (subject, predicate, object, confidence, evidence, source, last_verified, created_at, deprecated_by, archive_reason)
                         SELECT subject, predicate, object, confidence, evidence, source, last_verified, created_at, 0, 'contradicted'
                         FROM world_facts WHERE id = ?1",
                        params![existing_id],
                    )?;
                    let archived_id = conn.last_insert_rowid();

                    // Delete old fact
                    conn.execute("DELETE FROM world_facts WHERE id = ?1", params![existing_id])?;

                    // Update FTS
                    conn.execute(
                        "DELETE FROM world_facts_fts WHERE rowid = ?1",
                        params![existing_id],
                    )?;

                    // Insert new fact
                    let evidence_json = serde_json::to_string(&vec![evidence.to_string()])
                        .unwrap_or_else(|_| format!("[\"{}\"]", evidence));
                    conn.execute(
                        "INSERT INTO world_facts (subject, predicate, object, confidence, evidence, source, last_verified, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                        params![subject, predicate, object, confidence, evidence_json, source.to_string(), now],
                    )?;
                    let new_id = conn.last_insert_rowid();

                    // Update deprecated_by in archive
                    conn.execute(
                        "UPDATE world_facts_archive SET deprecated_by = ?1 WHERE id = ?2",
                        params![new_id, archived_id],
                    )?;

                    // Update FTS for new fact
                    conn.execute(
                        "INSERT INTO world_facts_fts(rowid, subject, predicate, object) VALUES (?1, ?2, ?3, ?4)",
                        params![new_id, subject, predicate, object],
                    )?;

                    Ok(ConflictResolution::Overwritten {
                        new_id,
                        archived_id,
                    })
                }
            }
        }
    }

    /// Query a fact by subject and predicate.
    pub fn query(&self, subject: &str, predicate: &str) -> anyhow::Result<Option<WorldFact>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT id, subject, predicate, object, confidence, evidence, source, last_verified, created_at, access_count
             FROM world_facts WHERE subject = ?1 AND predicate = ?2",
            params![subject, predicate],
            |row| {
                Ok(WorldFact {
                    id: Some(row.get(0)?),
                    subject: row.get(1)?,
                    predicate: row.get(2)?,
                    object: row.get(3)?,
                    confidence: row.get(4)?,
                    evidence: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
                    source: row.get::<_, String>(6)?.parse().unwrap_or(FactSource::Inferred),
                    last_verified: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    access_count: row.get(9)?,
                })
            },
        );

        match result {
            Ok(fact) => Ok(Some(fact)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Query all facts for a subject.
    pub fn query_subject(&self, subject: &str) -> anyhow::Result<Vec<WorldFact>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, subject, predicate, object, confidence, evidence, source, last_verified, created_at, access_count
             FROM world_facts WHERE subject = ?1 ORDER BY confidence DESC",
        )?;
        let facts = stmt
            .query_map(params![subject], |row| {
                Ok(WorldFact {
                    id: Some(row.get(0)?),
                    subject: row.get(1)?,
                    predicate: row.get(2)?,
                    object: row.get(3)?,
                    confidence: row.get(4)?,
                    evidence: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
                    source: row.get::<_, String>(6)?.parse().unwrap_or(FactSource::Inferred),
                    last_verified: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    access_count: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(facts)
    }

    /// Full-text search across facts.
    pub fn search(&self, query: &str) -> anyhow::Result<Vec<WorldFact>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT f.id, f.subject, f.predicate, f.object, f.confidence, f.evidence, f.source, f.last_verified, f.created_at, f.access_count
             FROM world_facts f
             JOIN world_facts_fts fts ON f.id = fts.rowid
             WHERE world_facts_fts MATCH ?1
             ORDER BY f.confidence DESC LIMIT 20",
        )?;
        let facts = stmt
            .query_map(params![query], |row| {
                Ok(WorldFact {
                    id: Some(row.get(0)?),
                    subject: row.get(1)?,
                    predicate: row.get(2)?,
                    object: row.get(3)?,
                    confidence: row.get(4)?,
                    evidence: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
                    source: row.get::<_, String>(6)?.parse().unwrap_or(FactSource::Inferred),
                    last_verified: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    access_count: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(facts)
    }

    /// Apply staleness decay and archive facts below threshold.
    pub fn decay_and_archive(&self, threshold: f64) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now();

        // Get all facts
        let mut stmt = conn.prepare(
            "SELECT id, confidence, last_verified FROM world_facts",
        )?;
        let rows: Vec<(i64, f64, String)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut archived_count = 0i64;
        for (id, conf, last_verified_str) in &rows {
            let last_verified = chrono::DateTime::parse_from_rfc3339(last_verified_str)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let hours = (now - last_verified).num_hours() as f64;
            let decayed_conf = conf * (-self.decay_rate_per_hour * hours).exp();

            if decayed_conf < threshold {
                // Archive and delete
                conn.execute(
                    "INSERT INTO world_facts_archive (subject, predicate, object, confidence, evidence, source, last_verified, created_at, archive_reason)
                     SELECT subject, predicate, object, confidence, evidence, source, last_verified, created_at, 'stale'
                     FROM world_facts WHERE id = ?1",
                    params![id],
                )?;
                conn.execute("DELETE FROM world_facts WHERE id = ?1", params![id])?;
                conn.execute("DELETE FROM world_facts_fts WHERE rowid = ?1", params![id])?;
                archived_count += 1;
            } else if (decayed_conf - conf).abs() > 0.001 {
                // Update decayed confidence
                conn.execute(
                    "UPDATE world_facts SET confidence = ?1 WHERE id = ?2",
                    params![decayed_conf, id],
                )?;
            }
        }

        Ok(archived_count)
    }

    /// Get aggregate statistics.
    pub fn stats(&self) -> anyhow::Result<WorldModelStats> {
        let conn = self.conn.lock().unwrap();

        let total: i64 = conn.query_row("SELECT COUNT(*) FROM world_facts", [], |r| r.get(0))?;
        let archived: i64 = conn.query_row("SELECT COUNT(*) FROM world_facts_archive", [], |r| r.get(0))?;
        let avg_conf: f64 = conn.query_row("SELECT COALESCE(AVG(confidence), 0.0) FROM world_facts", [], |r| r.get(0))?;
        let stale: i64 = conn.query_row(
            "SELECT COUNT(*) FROM world_facts WHERE confidence < 0.1",
            [], |r| r.get(0),
        )?;

        let mut by_source = HashMap::new();
        let mut src_stmt = conn.prepare("SELECT source, COUNT(*) FROM world_facts GROUP BY source")?;
        let src_rows = src_stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in src_rows {
            let (src, cnt) = row?;
            by_source.insert(src, cnt);
        }

        Ok(WorldModelStats {
            total_facts: total,
            archived_facts: archived,
            facts_by_source: by_source,
            avg_confidence: avg_conf,
            stale_facts: stale,
        })
    }

    /// Delete a fact by id.
    pub fn delete(&self, id: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM world_facts WHERE id = ?1", params![id])?;
        conn.execute("DELETE FROM world_facts_fts WHERE rowid = ?1", params![id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn test_store() -> WorldModelStore {
        let tmp = NamedTempFile::new().unwrap();
        WorldModelStore::open_path(tmp.path()).unwrap()
    }

    #[test]
    fn insert_new_fact() {
        let store = test_store();
        let res = store.upsert("VM1", "runs", "Ubuntu 24.04", 0.9, FactSource::Detected, "ssh uname -a").unwrap();
        assert!(matches!(res, ConflictResolution::Inserted { .. }));

        let fact = store.query("VM1", "runs").unwrap().unwrap();
        assert_eq!(fact.object, "Ubuntu 24.04");
        assert!((fact.confidence - 0.9).abs() < 0.001);
    }

    #[test]
    fn merge_same_fact_merges_evidence() {
        let store = test_store();
        store.upsert("VM1", "runs", "Ubuntu 24.04", 0.9, FactSource::Detected, "ssh uname -a").unwrap();
        let res = store.upsert("VM1", "runs", "Ubuntu 24.04", 0.8, FactSource::Detected, "cat /etc/os-release").unwrap();

        assert!(matches!(res, ConflictResolution::Merged { .. }));
        let fact = store.query("VM1", "runs").unwrap().unwrap();
        // Bayesian: 1 - (1-0.9)*(1-0.8) = 1 - 0.02 = 0.98
        assert!((fact.confidence - 0.98).abs() < 0.01);
        assert_eq!(fact.evidence.len(), 2);
    }

    #[test]
    fn contradict_old_fact_archives_it() {
        let store = test_store();
        store.upsert("VM1", "runs", "Ubuntu 22.04", 0.9, FactSource::Detected, "old").unwrap();
        let res = store.upsert("VM1", "runs", "Ubuntu 24.04", 0.95, FactSource::Detected, "new").unwrap();

        assert!(matches!(res, ConflictResolution::Overwritten { .. }));

        // New fact is active
        let fact = store.query("VM1", "runs").unwrap().unwrap();
        assert_eq!(fact.object, "Ubuntu 24.04");

        // Old fact is archived
        let stats = store.stats().unwrap();
        assert_eq!(stats.archived_facts, 1);
    }

    #[test]
    fn decay_archives_stale_facts() {
        let store = test_store();
        // Insert a fact with low confidence that will decay below threshold
        store.upsert("test", "is", "stale", 0.11, FactSource::Inferred, "old").unwrap();

        // Manually set last_verified to 48 hours ago
        {
            let conn = store.conn.lock().unwrap();
            let old = (Utc::now() - chrono::Duration::hours(48)).to_rfc3339();
            conn.execute("UPDATE world_facts SET last_verified = ?1 WHERE subject = 'test'", params![old]).unwrap();
        }

        let archived = store.decay_and_archive(0.1).unwrap();
        assert_eq!(archived, 1);
        assert!(store.query("test", "is").unwrap().is_none());
    }

    #[test]
    fn full_text_search() {
        let store = test_store();
        store.upsert("VM1", "runs_service", "nginx web server", 0.9, FactSource::Detected, "ps aux").unwrap();
        store.upsert("VM2", "runs_service", "postgres database", 0.8, FactSource::Detected, "ps aux").unwrap();

        let results = store.search("nginx").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].object, "nginx web server");
    }

    #[test]
    fn query_subject_returns_all_predicates() {
        let store = test_store();
        store.upsert("VM1", "runs", "Ubuntu", 0.9, FactSource::Detected, "").unwrap();
        store.upsert("VM1", "has_ip", "192.168.1.1", 0.95, FactSource::Detected, "").unwrap();
        store.upsert("VM1", "has_ram", "8GB", 0.8, FactSource::Detected, "").unwrap();

        let facts = store.query_subject("VM1").unwrap();
        assert_eq!(facts.len(), 3);
    }
}
