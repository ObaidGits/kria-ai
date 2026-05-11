//! Failure Analyzer Store — SQLite-backed failure pattern persistence.
//!
//! Failure patterns are stored in SQLite and survive process restarts.
//! The store supports:
//! - Recording new failure patterns
//! - Matching commands against known failure patterns
//! - Tracking occurrence counts and confidence

use chrono::Utc;
use rusqlite::params;
use std::sync::Mutex;

use super::types::{FailureContext, FailurePattern, RootCause};

/// SQLite-backed failure pattern store.
pub struct FailureAnalyzerStore {
    conn: Mutex<rusqlite::Connection>,
}

impl FailureAnalyzerStore {
    /// Open (or create) the failure analyzer tables.
    pub fn open(conn: rusqlite::Connection) -> anyhow::Result<Self> {
        let store = Self {
            conn: Mutex::new(conn),
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
            CREATE TABLE IF NOT EXISTS failure_patterns (
                id                   INTEGER PRIMARY KEY AUTOINCREMENT,
                goal                 TEXT NOT NULL,
                failed_binary        TEXT NOT NULL,
                failed_arg           TEXT,
                root_cause_category  TEXT NOT NULL,
                stderr_signature     TEXT NOT NULL,
                occurrences          INTEGER NOT NULL DEFAULT 1,
                confidence           REAL NOT NULL DEFAULT 0.7,
                suggested_alternative TEXT,
                first_seen           TEXT NOT NULL DEFAULT (datetime('now')),
                last_seen            TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_fp_binary ON failure_patterns(failed_binary);
            CREATE INDEX IF NOT EXISTS idx_fp_category ON failure_patterns(root_cause_category);

            CREATE TABLE IF NOT EXISTS failure_log (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                goal            TEXT NOT NULL,
                failed_binary   TEXT NOT NULL,
                failed_arg      TEXT,
                exit_code       INTEGER NOT NULL,
                stderr          TEXT NOT NULL,
                stdout          TEXT NOT NULL,
                root_cause_json TEXT NOT NULL,
                timestamp       TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_fl_binary ON failure_log(failed_binary);
            ",
        )?;
        Ok(())
    }

    /// Record a failure — extracts root cause deterministically and stores the pattern.
    pub fn record_failure(&self, ctx: &FailureContext) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        // 1. Log the raw failure
        conn.execute(
            "INSERT INTO failure_log (goal, failed_binary, failed_arg, exit_code, stderr, stdout, root_cause_json, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                ctx.goal,
                ctx.failed_command.binary,
                ctx.failed_command.args.first().cloned().unwrap_or_default(),
                ctx.exit_code,
                ctx.stderr,
                ctx.stdout,
                serde_json::to_string(&ctx.root_cause).unwrap_or_default(),
                now,
            ],
        )?;

        let category = root_cause_category(&ctx.root_cause);
        let signature = stderr_signature(&ctx.stderr);

        // 2. Check if this pattern already exists
        let existing = conn.query_row(
            "SELECT id, occurrences FROM failure_patterns WHERE failed_binary = ?1 AND root_cause_category = ?2 AND stderr_signature = ?3",
            params![ctx.failed_command.binary, category, signature],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        );

        match existing {
            Ok((id, occurrences)) => {
                // Update existing pattern
                conn.execute(
                    "UPDATE failure_patterns SET occurrences = ?1, last_seen = ?2, confidence = MIN(0.95, confidence + 0.05) WHERE id = ?3",
                    params![occurrences + 1, now, id],
                )?;
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // Insert new pattern
                conn.execute(
                    "INSERT INTO failure_patterns (goal, failed_binary, failed_arg, root_cause_category, stderr_signature, occurrences, confidence, first_seen, last_seen)
                     VALUES (?1, ?2, ?3, ?4, ?5, 1, 0.7, ?6, ?6)",
                    params![
                        ctx.goal,
                        ctx.failed_command.binary,
                        ctx.failed_command.args.first().cloned().unwrap_or_default(),
                        category,
                        signature,
                        now,
                    ],
                )?;
            }
            Err(e) => return Err(e.into()),
        }

        Ok(())
    }

    /// Check if a command matches any known failure pattern.
    pub fn check_command(
        &self,
        binary: &str,
        arg: Option<&str>,
    ) -> anyhow::Result<Option<FailurePattern>> {
        let conn = self.conn.lock().unwrap();

        // Look for patterns matching this binary
        let mut stmt = conn.prepare(
            "SELECT id, goal, failed_binary, failed_arg, root_cause_category, stderr_signature,
                    occurrences, confidence, suggested_alternative, first_seen, last_seen
             FROM failure_patterns
             WHERE failed_binary = ?1 AND confidence > 0.5
             ORDER BY occurrences DESC, confidence DESC
             LIMIT 1",
        )?;

        let result = stmt.query_row(params![binary], |row| {
            Ok(FailurePattern {
                id: Some(row.get(0)?),
                goal: row.get(1)?,
                failed_binary: row.get(2)?,
                failed_arg: row.get(3)?,
                root_cause_category: row.get(4)?,
                stderr_signature: row.get(5)?,
                occurrences: row.get(6)?,
                confidence: row.get(7)?,
                suggested_alternative: row.get(8)?,
                first_seen: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(9)?)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                last_seen: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(10)?)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        });

        match result {
            Ok(pattern) => {
                // If we have a specific arg match, boost confidence
                if let Some(arg) = arg {
                    if pattern.failed_arg.as_deref() == Some(arg) {
                        return Ok(Some(pattern));
                    }
                }
                // Binary-only match is still useful
                Ok(Some(pattern))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get all failure patterns (for diagnostics/learning).
    pub fn all_patterns(&self) -> anyhow::Result<Vec<FailurePattern>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, goal, failed_binary, failed_arg, root_cause_category, stderr_signature,
                    occurrences, confidence, suggested_alternative, first_seen, last_seen
             FROM failure_patterns ORDER BY occurrences DESC",
        )?;
        let patterns = stmt
            .query_map([], |row| {
                Ok(FailurePattern {
                    id: Some(row.get(0)?),
                    goal: row.get(1)?,
                    failed_binary: row.get(2)?,
                    failed_arg: row.get(3)?,
                    root_cause_category: row.get(4)?,
                    stderr_signature: row.get(5)?,
                    occurrences: row.get(6)?,
                    confidence: row.get(7)?,
                    suggested_alternative: row.get(8)?,
                    first_seen: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(9)?)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    last_seen: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(10)?)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(patterns)
    }

    /// Update the suggested alternative for a pattern.
    pub fn set_alternative(&self, pattern_id: i64, alternative: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE failure_patterns SET suggested_alternative = ?1 WHERE id = ?2",
            params![alternative, pattern_id],
        )?;
        Ok(())
    }
}

/// Map a root cause to a category string for pattern matching.
fn root_cause_category(cause: &RootCause) -> String {
    match cause {
        RootCause::ExitCode { .. } => "exit_code".into(),
        RootCause::StderrPattern { category, .. } => category.clone(),
        RootCause::Timeout { .. } => "timeout".into(),
        RootCause::PermissionDenied { .. } => "permission_denied".into(),
        RootCause::ResourceExhausted { resource } => format!("resource_{}", resource),
        RootCause::ServiceNotRunning { .. } => "service_not_running".into(),
        RootCause::NetworkUnreachable { .. } => "network_unreachable".into(),
        RootCause::ConfigError { .. } => "config_error".into(),
        RootCause::Unknown { .. } => "unknown".into(),
    }
}

/// Extract a stable signature from stderr for pattern matching.
fn stderr_signature(stderr: &str) -> String {
    // Take first 100 chars, normalized
    let trimmed = stderr.trim();
    if trimmed.len() <= 100 {
        trimmed.to_string()
    } else {
        format!("{}...", &trimmed[..100])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::subprocess_executor::StructuredCommand;
    use tempfile::NamedTempFile;

    fn test_store() -> FailureAnalyzerStore {
        let tmp = NamedTempFile::new().unwrap();
        FailureAnalyzerStore::open_path(tmp.path()).unwrap()
    }

    fn make_cmd(binary: &str, args: &[&str]) -> StructuredCommand {
        StructuredCommand {
            binary: binary.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            target: "local".into(),
            timeout_secs: 30,
            working_dir: None,
            env_vars: None,
        }
    }

    #[test]
    fn record_and_retrieve_failure() {
        let store = test_store();
        let ctx = FailureContext {
            goal: "restart nginx".into(),
            failed_command: make_cmd("systemctl", &["restart", "nginx"]),
            exit_code: 1,
            stderr: "Job for nginx failed because the control process exited with error".into(),
            stdout: String::new(),
            root_cause: RootCause::ConfigError {
                file: None,
                detail: "control process exited with error".into(),
            },
            timestamp: Utc::now(),
        };
        store.record_failure(&ctx).unwrap();

        let pattern = store.check_command("systemctl", Some("restart")).unwrap();
        assert!(pattern.is_some());
        let p = pattern.unwrap();
        assert_eq!(p.occurrences, 1);
    }

    #[test]
    fn repeated_failure_increases_occurrences() {
        let store = test_store();
        let ctx = FailureContext {
            goal: "test".into(),
            failed_command: make_cmd("curl", &["http://localhost:9999"]),
            exit_code: 7,
            stderr: "ECONNREFUSED".into(),
            stdout: String::new(),
            root_cause: RootCause::NetworkUnreachable {
                target: "localhost".into(),
            },
            timestamp: Utc::now(),
        };
        store.record_failure(&ctx).unwrap();
        store.record_failure(&ctx).unwrap();
        store.record_failure(&ctx).unwrap();

        let pattern = store.check_command("curl", None).unwrap().unwrap();
        assert_eq!(pattern.occurrences, 3);
        // Confidence should increase with each occurrence
        assert!(pattern.confidence > 0.7);
    }

    #[test]
    fn no_match_for_unknown_binary() {
        let store = test_store();
        let result = store.check_command("nonexistent_tool", None).unwrap();
        assert!(result.is_none());
    }
}
