//! Quarantine Registry — Safety gate for dynamically generated tools.
//!
//! # Design: Tiered Promotion with SQLite Persistence
//!
//! New tools (compiled skills, dynamically discovered APIs) go here first.
//! They are NOT available for LLM use until promoted to the active registry.
//!
//! ## Promotion Rules
//!
//! | Risk Level | Promotion Path |
//! |------------|---------------|
//! | Green (read-only) | Auto-promote after N=3 successes |
//! | Yellow (write) | HITL approval required after N=3 successes |
//! | Red (destructive) | HITL approval + PIN required after N=3 |
//! | Black | Never promoted |
//!
//! ## Circuit Breaker
//!
//! If a quarantined tool fails 3 consecutive times, it is automatically
//! disabled. The tool can be re-enabled manually after review.
//!
//! ## Persistence
//!
//! All quarantine status is persisted in SQLite. Testing progress
//! survives process restarts.

use chrono::{DateTime, Utc};
use rusqlite::params;
use std::sync::Mutex;

use crate::safety::RiskLevel;

/// Status of a quarantined tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum QuarantineStatus {
    /// Accumulating test results (N < 3).
    Testing,
    /// Ready for HITL approval (yellow/red risk, N ≥ 3).
    PendingApproval,
    /// Promoted to active registry.
    Active,
    /// Disabled by circuit breaker (3 consecutive failures).
    Disabled,
    /// User explicitly rejected promotion.
    Rejected,
}

impl std::fmt::Display for QuarantineStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Testing => write!(f, "testing"),
            Self::PendingApproval => write!(f, "pending_approval"),
            Self::Active => write!(f, "active"),
            Self::Disabled => write!(f, "disabled"),
            Self::Rejected => write!(f, "rejected"),
        }
    }
}

impl std::str::FromStr for QuarantineStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "testing" => Ok(Self::Testing),
            "pending_approval" => Ok(Self::PendingApproval),
            "active" => Ok(Self::Active),
            "disabled" => Ok(Self::Disabled),
            "rejected" => Ok(Self::Rejected),
            _ => Err(format!("unknown quarantine status: {}", s)),
        }
    }
}

/// Source of a quarantined tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ToolSource {
    /// Compiled from successful plan by Skill Compiler.
    SkillCompiler,
    /// Discovered from API/CLI docs.
    DynamicDiscovery,
    /// Provided by MCP server.
    McpServer,
}

impl std::fmt::Display for ToolSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SkillCompiler => write!(f, "skill_compiler"),
            Self::DynamicDiscovery => write!(f, "dynamic_discovery"),
            Self::McpServer => write!(f, "mcp_server"),
        }
    }
}

impl std::str::FromStr for ToolSource {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "skill_compiler" => Ok(Self::SkillCompiler),
            "dynamic_discovery" => Ok(Self::DynamicDiscovery),
            "mcp_server" => Ok(Self::McpServer),
            _ => Err(format!("unknown tool source: {}", s)),
        }
    }
}

/// A quarantined tool record.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QuarantinedTool {
    /// SQLite row id.
    pub id: Option<i64>,
    /// Unique tool/skill name.
    pub name: String,
    /// Risk level (determines promotion path).
    pub risk_level: RiskLevel,
    /// Current quarantine status.
    pub status: QuarantineStatus,
    /// Where this tool came from.
    pub source: ToolSource,
    /// Number of successful test executions.
    pub success_count: i64,
    /// Number of consecutive failures (resets on success).
    pub consecutive_failures: i64,
    /// Total number of executions.
    pub total_executions: i64,
    /// When the tool was quarantined.
    pub created_at: DateTime<Utc>,
    /// When the tool was last tested.
    pub last_tested: DateTime<Utc>,
    /// Optional notes from HITL review.
    pub review_notes: Option<String>,
}

/// The Quarantine Registry — SQLite-backed safety gate.
pub struct QuarantineRegistry {
    conn: Mutex<rusqlite::Connection>,
    /// Number of successes required before promotion consideration.
    min_successes: i64,
    /// Number of consecutive failures before circuit breaker triggers.
    circuit_breaker_threshold: i64,
}

impl QuarantineRegistry {
    /// Create a new Quarantine Registry with SQLite persistence.
    pub fn open(conn: rusqlite::Connection) -> anyhow::Result<Self> {
        let registry = Self {
            conn: Mutex::new(conn),
            min_successes: 3,
            circuit_breaker_threshold: 3,
        };
        registry.migrate()?;
        Ok(registry)
    }

    pub fn open_path(path: &std::path::Path) -> anyhow::Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        Self::open(conn)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS quarantine (
                id                   INTEGER PRIMARY KEY AUTOINCREMENT,
                name                 TEXT NOT NULL UNIQUE,
                risk_level           TEXT NOT NULL DEFAULT 'yellow',
                status               TEXT NOT NULL DEFAULT 'testing',
                source               TEXT NOT NULL DEFAULT 'skill_compiler',
                success_count        INTEGER NOT NULL DEFAULT 0,
                consecutive_failures INTEGER NOT NULL DEFAULT 0,
                total_executions     INTEGER NOT NULL DEFAULT 0,
                created_at           TEXT NOT NULL DEFAULT (datetime('now')),
                last_tested          TEXT NOT NULL DEFAULT (datetime('now')),
                review_notes         TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_q_status ON quarantine(status);
            CREATE INDEX IF NOT EXISTS idx_q_name ON quarantine(name);

            CREATE TABLE IF NOT EXISTS quarantine_log (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                tool_name   TEXT NOT NULL,
                action      TEXT NOT NULL,
                details     TEXT,
                timestamp   TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_ql_name ON quarantine_log(tool_name);
            ",
        )?;
        Ok(())
    }

    /// Add a new tool to quarantine.
    pub fn quarantine(
        &self,
        name: &str,
        risk_level: RiskLevel,
        source: ToolSource,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        // Check if already quarantined
        let exists: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM quarantine WHERE name = ?1",
            params![name],
            |r| r.get(0),
        )?;

        if exists {
            return Ok(()); // Already quarantined
        }

        conn.execute(
            "INSERT INTO quarantine (name, risk_level, status, source, created_at, last_tested)
             VALUES (?1, ?2, 'testing', ?3, ?4, ?4)",
            params![name, risk_level.as_str(), source.to_string(), now],
        )?;

        log_action_inner(
            &conn,
            name,
            "quarantined",
            &format!("risk={:?}, source={}", risk_level, source),
        )?;

        Ok(())
    }

    /// Record a successful test execution.
    pub fn record_success(&self, name: &str) -> anyhow::Result<QuarantineStatus> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "UPDATE quarantine
             SET success_count = success_count + 1,
                 consecutive_failures = 0,
                 total_executions = total_executions + 1,
                 last_tested = ?1
             WHERE name = ?2",
            params![now, name],
        )?;

        // Check if ready for promotion
        let (success_count, risk_level_str): (i64, String) = conn.query_row(
            "SELECT success_count, risk_level FROM quarantine WHERE name = ?1",
            params![name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;

        if success_count >= self.min_successes {
            let risk_level: RiskLevel = match risk_level_str.to_uppercase().as_str() {
                "GREEN" => RiskLevel::Green,
                "YELLOW" => RiskLevel::Yellow,
                "RED" => RiskLevel::Red,
                "BLACK" => RiskLevel::Black,
                _ => RiskLevel::Yellow,
            };

            match risk_level {
                RiskLevel::Green => {
                    // Auto-promote read-only tools
                    conn.execute(
                        "UPDATE quarantine SET status = 'active' WHERE name = ?1",
                        params![name],
                    )?;
                    log_action_inner(
                        &conn,
                        name,
                        "auto_promoted",
                        "green risk, auto-promoted after 3 successes",
                    )?;
                    return Ok(QuarantineStatus::Active);
                }
                RiskLevel::Yellow | RiskLevel::Red => {
                    // Require HITL approval
                    conn.execute(
                        "UPDATE quarantine SET status = 'pending_approval' WHERE name = ?1",
                        params![name],
                    )?;
                    log_action_inner(
                        &conn,
                        name,
                        "pending_approval",
                        &format!("{:?} risk, HITL required", risk_level),
                    )?;
                    return Ok(QuarantineStatus::PendingApproval);
                }
                RiskLevel::Black => {
                    // Never promote
                    conn.execute(
                        "UPDATE quarantine SET status = 'rejected' WHERE name = ?1",
                        params![name],
                    )?;
                    log_action_inner(&conn, name, "rejected", "black risk, never promoted")?;
                    return Ok(QuarantineStatus::Rejected);
                }
            }
        }

        Ok(QuarantineStatus::Testing)
    }

    /// Record a failed test execution. Returns new status.
    pub fn record_failure(&self, name: &str) -> anyhow::Result<QuarantineStatus> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "UPDATE quarantine
             SET consecutive_failures = consecutive_failures + 1,
                 success_count = 0,
                 total_executions = total_executions + 1,
                 last_tested = ?1
             WHERE name = ?2",
            params![now, name],
        )?;

        // Check circuit breaker
        let failures: i64 = conn.query_row(
            "SELECT consecutive_failures FROM quarantine WHERE name = ?1",
            params![name],
            |r| r.get(0),
        )?;

        if failures >= self.circuit_breaker_threshold {
            conn.execute(
                "UPDATE quarantine SET status = 'disabled' WHERE name = ?1",
                params![name],
            )?;
            log_action_inner(
                &conn,
                name,
                "disabled",
                &format!("circuit breaker: {} consecutive failures", failures),
            )?;
            return Ok(QuarantineStatus::Disabled);
        }

        Ok(QuarantineStatus::Testing)
    }

    /// Approve a pending tool (HITL response).
    pub fn approve(&self, name: &str, notes: Option<&str>) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE quarantine SET status = 'active', review_notes = ?1 WHERE name = ?2 AND status = 'pending_approval'",
            params![notes.unwrap_or(""), name],
        )?;
        log_action_inner(&conn, name, "approved", notes.unwrap_or("HITL approved"))?;
        Ok(())
    }

    /// Reject a pending tool (HITL response).
    pub fn reject(&self, name: &str, notes: Option<&str>) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE quarantine SET status = 'rejected', review_notes = ?1 WHERE name = ?2 AND status = 'pending_approval'",
            params![notes.unwrap_or(""), name],
        )?;
        log_action_inner(&conn, name, "rejected", notes.unwrap_or("HITL rejected"))?;
        Ok(())
    }

    /// Re-enable a disabled tool (reset circuit breaker).
    pub fn reenable(&self, name: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE quarantine SET status = 'testing', consecutive_failures = 0, success_count = 0 WHERE name = ?1",
            params![name],
        )?;
        log_action_inner(&conn, name, "reenabled", "circuit breaker reset")?;
        Ok(())
    }

    /// Get all tools with a specific status.
    pub fn get_by_status(&self, status: QuarantineStatus) -> anyhow::Result<Vec<QuarantinedTool>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, risk_level, status, source, success_count, consecutive_failures,
                    total_executions, created_at, last_tested, review_notes
             FROM quarantine WHERE status = ?1 ORDER BY created_at DESC",
        )?;
        let tools = stmt
            .query_map(params![status.to_string()], |row| {
                Ok(QuarantinedTool {
                    id: Some(row.get(0)?),
                    name: row.get(1)?,
                    risk_level: match row.get::<_, String>(2)?.to_uppercase().as_str() {
                        "GREEN" => RiskLevel::Green,
                        "YELLOW" => RiskLevel::Yellow,
                        "RED" => RiskLevel::Red,
                        "BLACK" => RiskLevel::Black,
                        _ => RiskLevel::Yellow,
                    },
                    status: row
                        .get::<_, String>(3)?
                        .parse()
                        .unwrap_or(QuarantineStatus::Testing),
                    source: row
                        .get::<_, String>(4)?
                        .parse()
                        .unwrap_or(ToolSource::SkillCompiler),
                    success_count: row.get(5)?,
                    consecutive_failures: row.get(6)?,
                    total_executions: row.get(7)?,
                    created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    last_tested: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(9)?)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    review_notes: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tools)
    }

    /// Get all pending approval tools (for HITL UI).
    pub fn pending_approval(&self) -> anyhow::Result<Vec<QuarantinedTool>> {
        self.get_by_status(QuarantineStatus::PendingApproval)
    }

    /// Get all active tools (for router matching).
    pub fn active_tools(&self) -> anyhow::Result<Vec<QuarantinedTool>> {
        self.get_by_status(QuarantineStatus::Active)
    }

    /// Get all disabled tools (for diagnostics).
    pub fn disabled_tools(&self) -> anyhow::Result<Vec<QuarantinedTool>> {
        self.get_by_status(QuarantineStatus::Disabled)
    }

    /// Check if a tool is active (promoted).
    pub fn is_active(&self, name: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM quarantine WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "not_found".into());
        Ok(status == "active")
    }

    /// Get aggregate statistics.
    pub fn stats(&self) -> anyhow::Result<QuarantineStats> {
        let conn = self.conn.lock().unwrap();

        let testing: i64 = conn.query_row(
            "SELECT COUNT(*) FROM quarantine WHERE status = 'testing'",
            [],
            |r| r.get(0),
        )?;
        let pending: i64 = conn.query_row(
            "SELECT COUNT(*) FROM quarantine WHERE status = 'pending_approval'",
            [],
            |r| r.get(0),
        )?;
        let active: i64 = conn.query_row(
            "SELECT COUNT(*) FROM quarantine WHERE status = 'active'",
            [],
            |r| r.get(0),
        )?;
        let disabled: i64 = conn.query_row(
            "SELECT COUNT(*) FROM quarantine WHERE status = 'disabled'",
            [],
            |r| r.get(0),
        )?;
        let rejected: i64 = conn.query_row(
            "SELECT COUNT(*) FROM quarantine WHERE status = 'rejected'",
            [],
            |r| r.get(0),
        )?;

        Ok(QuarantineStats {
            testing,
            pending_approval: pending,
            active,
            disabled,
            rejected,
        })
    }

    // ── Private helpers ───────────────────────────────────────────────

    #[allow(dead_code)]
    fn log_action(&self, name: &str, action: &str, details: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        log_action_inner(&conn, name, action, details)
    }
}

/// Log an action to the quarantine audit trail. Takes an already-locked connection
/// to avoid Mutex re-entrance deadlocks.
fn log_action_inner(
    conn: &rusqlite::Connection,
    name: &str,
    action: &str,
    details: &str,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO quarantine_log (tool_name, action, details) VALUES (?1, ?2, ?3)",
        params![name, action, details],
    )?;
    Ok(())
}

/// Aggregate statistics for the Quarantine Registry.
#[derive(Debug, Clone, serde::Serialize)]
pub struct QuarantineStats {
    pub testing: i64,
    pub pending_approval: i64,
    pub active: i64,
    pub disabled: i64,
    pub rejected: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn test_registry() -> QuarantineRegistry {
        let tmp = NamedTempFile::new().unwrap();
        QuarantineRegistry::open_path(tmp.path()).unwrap()
    }

    #[test]
    fn quarantine_and_test() {
        let reg = test_registry();
        reg.quarantine("test_tool", RiskLevel::Yellow, ToolSource::SkillCompiler)
            .unwrap();

        // Record 3 successes
        let status = reg.record_success("test_tool").unwrap();
        assert_eq!(status, QuarantineStatus::Testing); // 1/3

        let status = reg.record_success("test_tool").unwrap();
        assert_eq!(status, QuarantineStatus::Testing); // 2/3

        let status = reg.record_success("test_tool").unwrap();
        assert_eq!(status, QuarantineStatus::PendingApproval); // 3/3, yellow = HITL
    }

    #[test]
    fn green_auto_promotes() {
        let reg = test_registry();
        reg.quarantine(
            "read_only_tool",
            RiskLevel::Green,
            ToolSource::SkillCompiler,
        )
        .unwrap();

        reg.record_success("read_only_tool").unwrap();
        reg.record_success("read_only_tool").unwrap();
        let status = reg.record_success("read_only_tool").unwrap();

        assert_eq!(status, QuarantineStatus::Active); // Green = auto-promote
        assert!(reg.is_active("read_only_tool").unwrap());
    }

    #[test]
    fn circuit_breaker_disables() {
        let reg = test_registry();
        reg.quarantine("failing_tool", RiskLevel::Yellow, ToolSource::SkillCompiler)
            .unwrap();

        reg.record_failure("failing_tool").unwrap();
        reg.record_failure("failing_tool").unwrap();
        let status = reg.record_failure("failing_tool").unwrap();

        assert_eq!(status, QuarantineStatus::Disabled);
    }

    #[test]
    fn success_resets_failure_streak() {
        let reg = test_registry();
        reg.quarantine("tool", RiskLevel::Yellow, ToolSource::SkillCompiler)
            .unwrap();

        reg.record_failure("tool").unwrap();
        reg.record_failure("tool").unwrap();
        reg.record_success("tool").unwrap(); // Reset!

        // Should still be testing (not disabled)
        let status = reg.record_failure("tool").unwrap();
        assert_eq!(status, QuarantineStatus::Testing);
    }

    #[test]
    fn hitl_approval_workflow() {
        let reg = test_registry();
        reg.quarantine("tool", RiskLevel::Yellow, ToolSource::SkillCompiler)
            .unwrap();

        for _ in 0..3 {
            reg.record_success("tool").unwrap();
        }

        let pending = reg.pending_approval().unwrap();
        assert_eq!(pending.len(), 1);

        reg.approve("tool", Some("Looks safe")).unwrap();
        assert!(reg.is_active("tool").unwrap());
    }

    #[test]
    fn hitl_rejection() {
        let reg = test_registry();
        reg.quarantine("tool", RiskLevel::Red, ToolSource::SkillCompiler)
            .unwrap();

        for _ in 0..3 {
            reg.record_success("tool").unwrap();
        }
        reg.reject("tool", Some("Too dangerous")).unwrap();

        let rejected = reg.get_by_status(QuarantineStatus::Rejected).unwrap();
        assert_eq!(rejected.len(), 1);
    }

    #[test]
    fn black_never_promotes() {
        let reg = test_registry();
        reg.quarantine("dangerous", RiskLevel::Black, ToolSource::SkillCompiler)
            .unwrap();

        for _ in 0..3 {
            reg.record_success("dangerous").unwrap();
        }

        let rejected = reg.get_by_status(QuarantineStatus::Rejected).unwrap();
        assert_eq!(rejected.len(), 1);
    }

    #[test]
    fn reenable_after_circuit_breaker() {
        let reg = test_registry();
        reg.quarantine("tool", RiskLevel::Yellow, ToolSource::SkillCompiler)
            .unwrap();

        for _ in 0..3 {
            reg.record_failure("tool").unwrap();
        }
        assert_eq!(reg.disabled_tools().unwrap().len(), 1);

        reg.reenable("tool").unwrap();
        assert_eq!(reg.disabled_tools().unwrap().len(), 0);
    }

    #[test]
    fn stats_accurate() {
        let reg = test_registry();
        reg.quarantine("t1", RiskLevel::Green, ToolSource::SkillCompiler)
            .unwrap();
        reg.quarantine("t2", RiskLevel::Yellow, ToolSource::DynamicDiscovery)
            .unwrap();
        for _ in 0..3 {
            reg.record_success("t1").unwrap();
        }

        let stats = reg.stats().unwrap();
        assert_eq!(stats.active, 1); // t1 auto-promoted
        assert_eq!(stats.testing, 1); // t2 still testing
    }

    #[test]
    fn persistence_survives_restart() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        // First session
        {
            let reg = QuarantineRegistry::open_path(&path).unwrap();
            reg.quarantine("persist_test", RiskLevel::Yellow, ToolSource::SkillCompiler)
                .unwrap();
            reg.record_success("persist_test").unwrap();
            reg.record_success("persist_test").unwrap();
        }

        // Second session (simulates restart)
        {
            let reg = QuarantineRegistry::open_path(&path).unwrap();
            let pending = reg.stats().unwrap();
            assert_eq!(pending.testing, 1); // Still testing (2/3)

            let status = reg.record_success("persist_test").unwrap();
            assert_eq!(status, QuarantineStatus::PendingApproval); // 3/3
        }
    }
}
