//! [`SqliteCapabilityKnowledge`] — the durable Capability Knowledge Base (CKB).
//!
//! The authoritative *learned* layer (spec R1): per `(provider_id, capability_id)`
//! it persists identity, kind/family, a descriptor snapshot + content hash,
//! provenance, usage (successes/total), last outcome (latency + failure
//! explanation), health, and first-seen/last-used timestamps; plus a separate
//! **Decision Records** table (spec R16) powering explainability + learning.
//!
//! Durable across restarts (SQLite, mirrors [`GrantStore`]) and **concurrency-safe**
//! (single `Mutex<Connection>`, transactional writes) — spec R24.1. Storage is
//! isolated behind the [`CapabilityKnowledge`] trait so the future global Memory
//! redesign can re-home it without touching callers (spec R22); `schema_version()`
//! + `snapshot`/`restore` support the migration (spec R22.2).
//!
//! The federated index remains a derived, rebuildable view; the CKB is the
//! source of learned truth (spec R1.2).

use std::sync::Mutex;

use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension};

use super::{CapabilityKnowledge, DecisionRecord, ExecutionPath, GoalClass};
use crate::capability::descriptor::CapabilityDescriptor;
use crate::capability::error::CapError;

/// Current CKB schema version (spec R22 migration negotiation).
pub const CKB_SCHEMA_VERSION: u32 = 1;

/// SQLite-backed Capability Knowledge Base.
pub struct SqliteCapabilityKnowledge {
    conn: Mutex<Connection>,
}

impl SqliteCapabilityKnowledge {
    /// Open (or create) a durable CKB at `path` (e.g. `~/.kria/cpp_knowledge.db`).
    pub fn open(path: &std::path::Path) -> Result<Self, CapError> {
        let conn = Connection::open(path).map_err(|e| CapError::Io(format!("ckb db open: {e}")))?;
        Self::from_conn(conn)
    }

    /// In-memory CKB (tests).
    pub fn in_memory() -> Result<Self, CapError> {
        let conn =
            Connection::open_in_memory().map_err(|e| CapError::Io(format!("ckb db mem: {e}")))?;
        Self::from_conn(conn)
    }

    fn from_conn(conn: Connection) -> Result<Self, CapError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cpp_knowledge (
                provider_id     TEXT NOT NULL,
                capability_id   TEXT NOT NULL,
                kind            TEXT NOT NULL,
                family          TEXT NOT NULL,
                name            TEXT NOT NULL,
                descriptor_hash TEXT NOT NULL,
                descriptor_json TEXT NOT NULL,
                state           TEXT NOT NULL DEFAULT 'enabled',
                provenance      TEXT NOT NULL DEFAULT 'installed',
                successes       INTEGER NOT NULL DEFAULT 0,
                total           INTEGER NOT NULL DEFAULT 0,
                last_latency_ms INTEGER,
                last_failure    TEXT,
                health          TEXT NOT NULL DEFAULT 'unknown',
                first_seen      TEXT NOT NULL,
                last_used       TEXT,
                PRIMARY KEY (provider_id, capability_id)
            );
            CREATE TABLE IF NOT EXISTS cpp_decisions (
                id            TEXT PRIMARY KEY,
                goal          TEXT NOT NULL,
                goal_class    TEXT NOT NULL,
                candidates_json TEXT NOT NULL,
                chosen_json   TEXT,
                rejected_json TEXT NOT NULL,
                path          TEXT NOT NULL,
                confidence    REAL NOT NULL,
                policy_version INTEGER NOT NULL,
                created_at    TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_cpp_decisions_created
                ON cpp_decisions(created_at);
            CREATE TABLE IF NOT EXISTS cpp_benchmarks (
                provider_id   TEXT NOT NULL,
                capability_id TEXT NOT NULL,
                success       INTEGER NOT NULL,
                latency_ms    INTEGER NOT NULL,
                score         REAL NOT NULL,
                created_at    TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_cpp_benchmarks_cap
                ON cpp_benchmarks(provider_id, capability_id);
            CREATE TABLE IF NOT EXISTS cpp_proposals (
                id            TEXT PRIMARY KEY,
                kind          TEXT NOT NULL,
                provider_id   TEXT NOT NULL,
                capability_id TEXT NOT NULL,
                replacement_json TEXT,
                rationale     TEXT NOT NULL,
                confidence    REAL NOT NULL,
                requires_approval INTEGER NOT NULL,
                status        TEXT NOT NULL,
                policy_version INTEGER NOT NULL,
                created_at    TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_cpp_proposals_status
                ON cpp_proposals(status, created_at);
            CREATE TABLE IF NOT EXISTS cpp_jobs (
                id            TEXT PRIMARY KEY,
                provider_id   TEXT NOT NULL,
                capability_id TEXT NOT NULL,
                args_json     TEXT NOT NULL,
                priority      INTEGER NOT NULL DEFAULT 0,
                state         TEXT NOT NULL,
                attempts      INTEGER NOT NULL DEFAULT 0,
                correlation_id TEXT NOT NULL,
                created_at    TEXT NOT NULL,
                updated_at    TEXT NOT NULL,
                last_error    TEXT,
                result_json   TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_cpp_jobs_state
                ON cpp_jobs(state, priority, created_at);",
        )
        .map_err(|e| CapError::Io(format!("ckb migrate: {e}")))?;
        // Additive migration for older DBs: track consecutive failures (chronic-
        // failure health signal, R6.1). Ignore error if the column already exists.
        let _ = conn.execute(
            "ALTER TABLE cpp_knowledge ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0",
            [],
        );
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Stable content hash of a descriptor (change detection for reconcile, R3.6).
    fn descriptor_hash(json: &str) -> String {
        blake3::hash(json.as_bytes()).to_hex().to_string()
    }

    fn now_rfc3339() -> String {
        chrono::Utc::now().to_rfc3339()
    }
}

#[async_trait]
impl CapabilityKnowledge for SqliteCapabilityKnowledge {
    async fn record_install(&self, d: &CapabilityDescriptor) -> Result<(), CapError> {
        let kind = format!("{:?}", super::infer_kind(d));
        let family = format!("{:?}", super::infer_family(d));
        let json = serde_json::to_string(d)
            .map_err(|e| CapError::Descriptor(format!("serialize descriptor: {e}")))?;
        let hash = Self::descriptor_hash(&json);
        let provenance = d
            .extensions
            .get("provenance")
            .and_then(|v| v.as_str())
            .unwrap_or("installed")
            .to_string();
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        // Preserve learned stats on re-install; refresh descriptor + hash.
        conn.execute(
            "INSERT INTO cpp_knowledge
                (provider_id, capability_id, kind, family, name, descriptor_hash,
                 descriptor_json, provenance, first_seen)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(provider_id, capability_id) DO UPDATE SET
                kind=excluded.kind, family=excluded.family, name=excluded.name,
                descriptor_hash=excluded.descriptor_hash,
                descriptor_json=excluded.descriptor_json",
            rusqlite::params![
                d.provider_id,
                d.capability_id,
                kind,
                family,
                d.name,
                hash,
                json,
                provenance,
                Self::now_rfc3339(),
            ],
        )
        .map_err(|e| CapError::Io(format!("ckb record_install: {e}")))?;
        Ok(())
    }

    async fn record_outcome(
        &self,
        provider_id: &str,
        capability_id: &str,
        ok: bool,
        latency_ms: Option<u64>,
        failure: Option<&str>,
    ) -> Result<(), CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        let health = if ok { "healthy" } else { "degraded" };
        // Maintain consecutive_failures: reset to 0 on success, increment on fail.
        let changed = conn
            .execute(
                "UPDATE cpp_knowledge SET
                    successes = successes + ?3,
                    total = total + 1,
                    last_latency_ms = ?4,
                    last_failure = ?5,
                    health = ?6,
                    last_used = ?7,
                    consecutive_failures = CASE WHEN ?3 = 1 THEN 0
                                                ELSE consecutive_failures + 1 END
                 WHERE provider_id = ?1 AND capability_id = ?2",
                rusqlite::params![
                    provider_id,
                    capability_id,
                    ok as i64,
                    latency_ms.map(|v| v as i64),
                    failure,
                    health,
                    Self::now_rfc3339(),
                ],
            )
            .map_err(|e| CapError::Io(format!("ckb record_outcome: {e}")))?;
        // An outcome for an unknown capability is not fatal — a native tool may
        // have no install row yet. Record a minimal row so learning starts.
        if changed == 0 {
            conn.execute(
                "INSERT OR IGNORE INTO cpp_knowledge
                    (provider_id, capability_id, kind, family, name, descriptor_hash,
                     descriptor_json, provenance, successes, total, last_latency_ms,
                     last_failure, health, first_seen, last_used)
                 VALUES (?1,?2,'Other','Other',?2,'','{}','native',?3,1,?4,?5,?6,?7,?7)",
                rusqlite::params![
                    provider_id,
                    capability_id,
                    ok as i64,
                    latency_ms.map(|v| v as i64),
                    failure,
                    health,
                    Self::now_rfc3339(),
                ],
            )
            .map_err(|e| CapError::Io(format!("ckb record_outcome insert: {e}")))?;
        }
        Ok(())
    }

    async fn record_decision(&self, decision: &DecisionRecord) -> Result<(), CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        conn.execute(
            "INSERT OR REPLACE INTO cpp_decisions
                (id, goal, goal_class, candidates_json, chosen_json, rejected_json,
                 path, confidence, policy_version, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            rusqlite::params![
                decision.id,
                decision.goal,
                goal_class_str(&decision.goal_class),
                serde_json::to_string(&decision.candidates).unwrap_or_else(|_| "[]".into()),
                decision
                    .chosen
                    .as_ref()
                    .map(|c| serde_json::to_string(c).unwrap_or_default()),
                serde_json::to_string(&decision.rejected).unwrap_or_else(|_| "[]".into()),
                path_str(&decision.path),
                decision.confidence as f64,
                decision.policy_version as i64,
                decision.created_at,
            ],
        )
        .map_err(|e| CapError::Io(format!("ckb record_decision: {e}")))?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<CapabilityDescriptor>, CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT descriptor_json FROM cpp_knowledge
                 WHERE state != 'archived' AND descriptor_json != '{}'
                 ORDER BY name",
            )
            .map_err(|e| CapError::Io(format!("ckb list prepare: {e}")))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| CapError::Io(format!("ckb list query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            let json = row.map_err(|e| CapError::Io(format!("ckb list row: {e}")))?;
            if let Ok(d) = serde_json::from_str::<CapabilityDescriptor>(&json) {
                out.push(d);
            }
        }
        Ok(out)
    }

    async fn success_rate(&self, provider_id: &str, capability_id: &str) -> f32 {
        let Ok(conn) = self.conn.lock() else {
            return 0.5;
        };
        let row: Result<(i64, i64), _> = conn.query_row(
            "SELECT successes, total FROM cpp_knowledge
             WHERE provider_id = ?1 AND capability_id = ?2",
            rusqlite::params![provider_id, capability_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        );
        match row {
            Ok((s, t)) if t > 0 => s as f32 / t as f32,
            // Unobserved ⇒ neutral 0.5 (neither boost nor penalize), matches index.
            _ => 0.5,
        }
    }

    async fn purge(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        conn.execute(
            "DELETE FROM cpp_knowledge WHERE provider_id = ?1 AND capability_id = ?2",
            rusqlite::params![provider_id, capability_id],
        )
        .map_err(|e| CapError::Io(format!("ckb purge: {e}")))?;
        Ok(())
    }

    async fn set_state(
        &self,
        provider_id: &str,
        capability_id: &str,
        state: &str,
    ) -> Result<(), CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        conn.execute(
            "UPDATE cpp_knowledge SET state = ?3 WHERE provider_id = ?1 AND capability_id = ?2",
            rusqlite::params![provider_id, capability_id, state],
        )
        .map_err(|e| CapError::Io(format!("ckb set_state: {e}")))?;
        Ok(())
    }

    fn schema_version(&self) -> u32 {
        CKB_SCHEMA_VERSION
    }
}

#[async_trait]
impl super::evolution::EvolutionStore for SqliteCapabilityKnowledge {
    async fn health_snapshots(&self) -> Result<Vec<super::health::CapabilityHealth>, CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT provider_id, capability_id, family, total, successes,
                        consecutive_failures, last_latency_ms, last_failure, state
                 FROM cpp_knowledge WHERE state != 'archived'",
            )
            .map_err(|e| CapError::Io(format!("ckb health prepare: {e}")))?;
        let rows = stmt
            .query_map([], |r| {
                let state: String = r.get(8)?;
                Ok(super::health::CapabilityHealth {
                    provider_id: r.get(0)?,
                    capability_id: r.get(1)?,
                    family: r.get(2)?,
                    total: r.get::<_, i64>(3)? as u64,
                    successes: r.get::<_, i64>(4)? as u64,
                    consecutive_failures: r.get::<_, i64>(5)? as u32,
                    last_latency_ms: r.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                    last_failure: r.get(7)?,
                    quarantined: state == "quarantined",
                    status: super::health::HealthStatus::Unknown,
                })
            })
            .map_err(|e| CapError::Io(format!("ckb health query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| CapError::Io(format!("ckb health row: {e}")))?);
        }
        Ok(out)
    }

    async fn record_benchmark(
        &self,
        provider_id: &str,
        capability_id: &str,
        success: bool,
        latency_ms: u64,
        score: f32,
    ) -> Result<(), CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        conn.execute(
            "INSERT INTO cpp_benchmarks
                (provider_id, capability_id, success, latency_ms, score, created_at)
             VALUES (?1,?2,?3,?4,?5,?6)",
            rusqlite::params![
                provider_id,
                capability_id,
                success as i64,
                latency_ms as i64,
                score as f64,
                Self::now_rfc3339(),
            ],
        )
        .map_err(|e| CapError::Io(format!("ckb record_benchmark: {e}")))?;
        Ok(())
    }

    async fn benchmark_score(&self, provider_id: &str, capability_id: &str) -> Option<f32> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT AVG(score) FROM cpp_benchmarks
             WHERE provider_id = ?1 AND capability_id = ?2",
            rusqlite::params![provider_id, capability_id],
            |r| r.get::<_, Option<f64>>(0),
        )
        .ok()
        .flatten()
        .map(|v| v as f32)
    }

    async fn record_proposal(
        &self,
        p: &super::evolution::EvolutionProposal,
    ) -> Result<(), CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        conn.execute(
            "INSERT OR REPLACE INTO cpp_proposals
                (id, kind, provider_id, capability_id, replacement_json, rationale,
                 confidence, requires_approval, status, policy_version, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            rusqlite::params![
                p.id,
                p.kind.as_str(),
                p.provider_id,
                p.capability_id,
                p.replacement
                    .as_ref()
                    .map(|r| serde_json::to_string(r).unwrap_or_default()),
                p.rationale,
                p.confidence as f64,
                p.requires_approval as i64,
                p.status.as_str(),
                p.policy_version as i64,
                p.created_at,
            ],
        )
        .map_err(|e| CapError::Io(format!("ckb record_proposal: {e}")))?;
        Ok(())
    }

    async fn list_proposals(
        &self,
        status: Option<super::evolution::ProposalStatus>,
    ) -> Result<Vec<super::evolution::EvolutionProposal>, CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        let (sql, filter): (&str, Option<String>) = match status {
            Some(s) => (
                "SELECT id,kind,provider_id,capability_id,replacement_json,rationale,confidence,
                        requires_approval,status,policy_version,created_at
                 FROM cpp_proposals WHERE status = ?1 ORDER BY created_at DESC",
                Some(s.as_str().to_string()),
            ),
            None => (
                "SELECT id,kind,provider_id,capability_id,replacement_json,rationale,confidence,
                        requires_approval,status,policy_version,created_at
                 FROM cpp_proposals ORDER BY created_at DESC",
                None,
            ),
        };
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| CapError::Io(format!("ckb proposals prepare: {e}")))?;
        let map_row = |r: &rusqlite::Row| -> rusqlite::Result<super::evolution::EvolutionProposal> {
            let kind_s: String = r.get(1)?;
            let repl_json: Option<String> = r.get(4)?;
            let status_s: String = r.get(8)?;
            Ok(super::evolution::EvolutionProposal {
                id: r.get(0)?,
                kind: parse_proposal_kind(&kind_s),
                provider_id: r.get(2)?,
                capability_id: r.get(3)?,
                replacement: repl_json.and_then(|j| serde_json::from_str(&j).ok()),
                rationale: r.get(5)?,
                confidence: r.get::<_, f64>(6)? as f32,
                requires_approval: r.get::<_, i64>(7)? != 0,
                status: parse_proposal_status(&status_s),
                policy_version: r.get::<_, i64>(9)? as u32,
                created_at: r.get(10)?,
            })
        };
        let mut out = Vec::new();
        if let Some(f) = filter {
            let rows = stmt
                .query_map(rusqlite::params![f], map_row)
                .map_err(|e| CapError::Io(format!("ckb proposals query: {e}")))?;
            for row in rows {
                out.push(row.map_err(|e| CapError::Io(format!("ckb proposals row: {e}")))?);
            }
        } else {
            let rows = stmt
                .query_map([], map_row)
                .map_err(|e| CapError::Io(format!("ckb proposals query: {e}")))?;
            for row in rows {
                out.push(row.map_err(|e| CapError::Io(format!("ckb proposals row: {e}")))?);
            }
        }
        Ok(out)
    }

    async fn set_proposal_status(
        &self,
        id: &str,
        status: super::evolution::ProposalStatus,
    ) -> Result<(), CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        conn.execute(
            "UPDATE cpp_proposals SET status = ?2 WHERE id = ?1",
            rusqlite::params![id, status.as_str()],
        )
        .map_err(|e| CapError::Io(format!("ckb set_proposal_status: {e}")))?;
        Ok(())
    }

    async fn get_proposal(
        &self,
        id: &str,
    ) -> Result<Option<super::evolution::EvolutionProposal>, CapError> {
        Ok(self
            .list_proposals(None)
            .await?
            .into_iter()
            .find(|p| p.id == id))
    }
}

fn parse_proposal_kind(s: &str) -> super::evolution::ProposalKind {
    use super::evolution::ProposalKind::*;
    match s {
        "upgrade" => Upgrade,
        "replace" => Replace,
        "repair" => Repair,
        _ => Retire,
    }
}

fn parse_proposal_status(s: &str) -> super::evolution::ProposalStatus {
    use super::evolution::ProposalStatus::*;
    match s {
        "pending" => Pending,
        "approved" => Approved,
        "applied" => Applied,
        "rejected" => Rejected,
        _ => Undone,
    }
}

fn goal_class_str(c: &GoalClass) -> String {
    match c {
        GoalClass::Other(s) => format!("other:{s}"),
        other => format!("{other:?}").to_lowercase(),
    }
}

fn path_str(p: &ExecutionPath) -> String {
    format!("{p:?}").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::CapabilityDescriptor;

    fn desc(provider: &str, cap: &str, name: &str) -> CapabilityDescriptor {
        CapabilityDescriptor::minimal(
            provider,
            cap,
            name,
            "",
            serde_json::json!({"type":"object"}),
        )
    }

    #[tokio::test]
    async fn install_list_and_grounding() {
        let ckb = SqliteCapabilityKnowledge::in_memory().unwrap();
        ckb.record_install(&desc("openclaw", "oc_ip_info", "IP Info"))
            .await
            .unwrap();
        ckb.record_install(&desc("openclaw", "oc_html_to_text", "HTML to Text"))
            .await
            .unwrap();
        let installed = ckb.list_installed().await.unwrap();
        assert_eq!(installed.len(), 2);
        // Grounding: the set matches exactly what was installed (no hallucination).
        let names: Vec<_> = installed.iter().map(|d| d.name.clone()).collect();
        assert!(names.contains(&"IP Info".to_string()));
        assert!(names.contains(&"HTML to Text".to_string()));
    }

    #[tokio::test]
    async fn outcomes_drive_success_rate() {
        let ckb = SqliteCapabilityKnowledge::in_memory().unwrap();
        ckb.record_install(&desc("openclaw", "oc_ip_info", "IP Info"))
            .await
            .unwrap();
        assert_eq!(ckb.success_rate("openclaw", "oc_ip_info").await, 0.5); // unobserved
        ckb.record_outcome("openclaw", "oc_ip_info", true, Some(120), None)
            .await
            .unwrap();
        ckb.record_outcome("openclaw", "oc_ip_info", true, Some(90), None)
            .await
            .unwrap();
        ckb.record_outcome("openclaw", "oc_ip_info", false, None, Some("timeout"))
            .await
            .unwrap();
        let rate = ckb.success_rate("openclaw", "oc_ip_info").await;
        assert!((rate - 2.0 / 3.0).abs() < 1e-6, "rate={rate}");
    }

    #[tokio::test]
    async fn outcome_for_unknown_native_tool_starts_learning() {
        let ckb = SqliteCapabilityKnowledge::in_memory().unwrap();
        ckb.record_outcome("native", "calculate", true, Some(2), None)
            .await
            .unwrap();
        assert_eq!(ckb.success_rate("native", "calculate").await, 1.0);
    }

    #[tokio::test]
    async fn purge_removes_all_knowledge() {
        let ckb = SqliteCapabilityKnowledge::in_memory().unwrap();
        ckb.record_install(&desc("openclaw", "oc_ip_info", "IP Info"))
            .await
            .unwrap();
        ckb.purge("openclaw", "oc_ip_info").await.unwrap();
        assert!(ckb.list_installed().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn durable_across_reopen() {
        let dir = std::env::temp_dir().join(format!("kria_ckb_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cpp_knowledge.db");
        {
            let ckb = SqliteCapabilityKnowledge::open(&path).unwrap();
            ckb.record_install(&desc("openclaw", "oc_ip_info", "IP Info"))
                .await
                .unwrap();
            ckb.record_outcome("openclaw", "oc_ip_info", true, Some(50), None)
                .await
                .unwrap();
        }
        // Reopen (simulate restart): learned knowledge persists (spec R1.2 / Property 10).
        let ckb2 = SqliteCapabilityKnowledge::open(&path).unwrap();
        assert_eq!(ckb2.list_installed().await.unwrap().len(), 1);
        assert_eq!(ckb2.success_rate("openclaw", "oc_ip_info").await, 1.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn decision_records_persist() {
        let ckb = SqliteCapabilityKnowledge::in_memory().unwrap();
        let dr = DecisionRecord {
            id: uuid::Uuid::new_v4().to_string(),
            goal: "what is my public ip".into(),
            goal_class: GoalClass::Information,
            candidates: vec![("native".into(), "get_public_ip".into(), 0.91)],
            chosen: Some(("native".into(), "get_public_ip".into())),
            rejected: vec![(
                "openclaw".into(),
                "oc_ip_info".into(),
                "native sufficient".into(),
            )],
            path: ExecutionPath::Native,
            confidence: 0.91,
            policy_version: super::super::REASONING_POLICY_VERSION,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        ckb.record_decision(&dr).await.unwrap();
        assert_eq!(ckb.schema_version(), CKB_SCHEMA_VERSION);
    }
}

/// Wave 11 — durable job persistence on the CKB (spec R28.1). Jobs survive
/// restart and are resumable; no parallel store.
#[async_trait::async_trait]
impl super::jobs::JobStore for SqliteCapabilityKnowledge {
    async fn put_job(&self, job: &super::jobs::Job) -> Result<(), CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        conn.execute(
            "INSERT OR REPLACE INTO cpp_jobs
                (id, provider_id, capability_id, args_json, priority, state, attempts,
                 correlation_id, created_at, updated_at, last_error, result_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            rusqlite::params![
                job.id,
                job.provider_id,
                job.capability_id,
                job.args_json,
                job.priority,
                job.state.as_str(),
                job.attempts,
                job.correlation_id,
                job.created_at,
                job.updated_at,
                job.last_error,
                job.result_json,
            ],
        )
        .map_err(|e| CapError::Io(format!("ckb put_job: {e}")))?;
        Ok(())
    }

    async fn get_job(&self, id: &str) -> Result<Option<super::jobs::Job>, CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        let job = conn
            .query_row(
                "SELECT id, provider_id, capability_id, args_json, priority, state, attempts,
                        correlation_id, created_at, updated_at, last_error, result_json
                 FROM cpp_jobs WHERE id = ?1",
                rusqlite::params![id],
                Self::row_to_job,
            )
            .optional()
            .map_err(|e| CapError::Io(format!("ckb get_job: {e}")))?;
        Ok(job)
    }

    async fn list_jobs(&self, limit: usize) -> Result<Vec<super::jobs::Job>, CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, provider_id, capability_id, args_json, priority, state, attempts,
                        correlation_id, created_at, updated_at, last_error, result_json
                 FROM cpp_jobs ORDER BY created_at DESC LIMIT ?1",
            )
            .map_err(|e| CapError::Io(format!("ckb list_jobs prepare: {e}")))?;
        let rows = stmt
            .query_map(rusqlite::params![limit as i64], Self::row_to_job)
            .map_err(|e| CapError::Io(format!("ckb list_jobs query: {e}")))?;
        let mut out = Vec::new();
        for r in rows.flatten() {
            out.push(r);
        }
        Ok(out)
    }

    async fn list_active(&self) -> Result<Vec<super::jobs::Job>, CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, provider_id, capability_id, args_json, priority, state, attempts,
                        correlation_id, created_at, updated_at, last_error, result_json
                 FROM cpp_jobs
                 WHERE state NOT IN ('completed','failed','cancelled','rolled_back')
                 ORDER BY priority DESC, created_at ASC",
            )
            .map_err(|e| CapError::Io(format!("ckb list_active prepare: {e}")))?;
        let rows = stmt
            .query_map([], Self::row_to_job)
            .map_err(|e| CapError::Io(format!("ckb list_active query: {e}")))?;
        let mut out = Vec::new();
        for r in rows.flatten() {
            out.push(r);
        }
        Ok(out)
    }

    async fn set_state(
        &self,
        id: &str,
        state: super::jobs::JobState,
        attempts: u32,
        last_error: Option<&str>,
        result_json: Option<&str>,
    ) -> Result<(), CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        conn.execute(
            "UPDATE cpp_jobs
             SET state = ?2, attempts = ?3, updated_at = ?4,
                 last_error = COALESCE(?5, last_error),
                 result_json = COALESCE(?6, result_json)
             WHERE id = ?1",
            rusqlite::params![
                id,
                state.as_str(),
                attempts,
                Self::now_rfc3339(),
                last_error,
                result_json,
            ],
        )
        .map_err(|e| CapError::Io(format!("ckb set_state(job): {e}")))?;
        Ok(())
    }
}

impl SqliteCapabilityKnowledge {
    /// Map a `cpp_jobs` row to a [`super::jobs::Job`].
    fn row_to_job(r: &rusqlite::Row) -> rusqlite::Result<super::jobs::Job> {
        let state_s: String = r.get(5)?;
        Ok(super::jobs::Job {
            id: r.get(0)?,
            provider_id: r.get(1)?,
            capability_id: r.get(2)?,
            args_json: r.get(3)?,
            priority: r.get(4)?,
            state: super::jobs::JobState::parse(&state_s).unwrap_or(super::jobs::JobState::Failed),
            attempts: r.get(6)?,
            correlation_id: r.get(7)?,
            created_at: r.get(8)?,
            updated_at: r.get(9)?,
            last_error: r.get(10)?,
            result_json: r.get(11)?,
        })
    }
}

/// Wave 13 (spec R22.2) — CKB→Memory **migration primitives**: a reversible
/// export/import of the learned layer so the future global Memory redesign can
/// dual-write / shadow-read and cut over reversibly. `snapshot` exports the full
/// `cpp_knowledge` table (identity + learned stats + health + provenance);
/// `restore` re-imports it into any CKB (preserving learned stats). A
/// snapshot→restore roundtrip is byte-stable for the learned layer.
impl SqliteCapabilityKnowledge {
    /// Export the learned layer as a versioned JSON document (migration payload).
    pub fn snapshot(&self) -> Result<serde_json::Value, CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT provider_id, capability_id, kind, family, name, descriptor_hash,
                        descriptor_json, state, provenance, successes, total,
                        last_latency_ms, last_failure, health, first_seen, last_used,
                        consecutive_failures
                 FROM cpp_knowledge ORDER BY provider_id, capability_id",
            )
            .map_err(|e| CapError::Io(format!("ckb snapshot prepare: {e}")))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(serde_json::json!({
                    "provider_id": r.get::<_, String>(0)?,
                    "capability_id": r.get::<_, String>(1)?,
                    "kind": r.get::<_, String>(2)?,
                    "family": r.get::<_, String>(3)?,
                    "name": r.get::<_, String>(4)?,
                    "descriptor_hash": r.get::<_, String>(5)?,
                    "descriptor_json": r.get::<_, String>(6)?,
                    "state": r.get::<_, String>(7)?,
                    "provenance": r.get::<_, String>(8)?,
                    "successes": r.get::<_, i64>(9)?,
                    "total": r.get::<_, i64>(10)?,
                    "last_latency_ms": r.get::<_, Option<i64>>(11)?,
                    "last_failure": r.get::<_, Option<String>>(12)?,
                    "health": r.get::<_, String>(13)?,
                    "first_seen": r.get::<_, String>(14)?,
                    "last_used": r.get::<_, Option<String>>(15)?,
                    "consecutive_failures": r.get::<_, i64>(16)?,
                }))
            })
            .map_err(|e| CapError::Io(format!("ckb snapshot query: {e}")))?;
        let mut items = Vec::new();
        for row in rows.flatten() {
            items.push(row);
        }
        Ok(serde_json::json!({
            "schema_version": CKB_SCHEMA_VERSION,
            "knowledge": items,
        }))
    }

    /// Re-import a [`Self::snapshot`] payload (reversible cut-over). Preserves the
    /// learned stats. Rejects a snapshot from an incompatible schema version
    /// (honest — never silently drops/corrupts learning).
    pub fn restore(&self, snapshot: &serde_json::Value) -> Result<usize, CapError> {
        let version = snapshot
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        if version != CKB_SCHEMA_VERSION {
            return Err(CapError::Io(format!(
                "ckb restore: incompatible snapshot schema {version} (expected {CKB_SCHEMA_VERSION})"
            )));
        }
        let items = snapshot
            .get("knowledge")
            .and_then(|v| v.as_array())
            .ok_or_else(|| CapError::Io("ckb restore: missing 'knowledge' array".into()))?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("ckb lock: {e}")))?;
        let mut restored = 0usize;
        for it in items {
            conn.execute(
                "INSERT OR REPLACE INTO cpp_knowledge
                    (provider_id, capability_id, kind, family, name, descriptor_hash,
                     descriptor_json, state, provenance, successes, total,
                     last_latency_ms, last_failure, health, first_seen, last_used,
                     consecutive_failures)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
                rusqlite::params![
                    it.get("provider_id").and_then(|v| v.as_str()).unwrap_or(""),
                    it.get("capability_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    it.get("kind").and_then(|v| v.as_str()).unwrap_or("Other"),
                    it.get("family").and_then(|v| v.as_str()).unwrap_or("Other"),
                    it.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                    it.get("descriptor_hash")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    it.get("descriptor_json")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}"),
                    it.get("state")
                        .and_then(|v| v.as_str())
                        .unwrap_or("enabled"),
                    it.get("provenance")
                        .and_then(|v| v.as_str())
                        .unwrap_or("installed"),
                    it.get("successes").and_then(|v| v.as_i64()).unwrap_or(0),
                    it.get("total").and_then(|v| v.as_i64()).unwrap_or(0),
                    it.get("last_latency_ms").and_then(|v| v.as_i64()),
                    it.get("last_failure").and_then(|v| v.as_str()),
                    it.get("health")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown"),
                    it.get("first_seen").and_then(|v| v.as_str()).unwrap_or(""),
                    it.get("last_used").and_then(|v| v.as_str()),
                    it.get("consecutive_failures")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0),
                ],
            )
            .map_err(|e| CapError::Io(format!("ckb restore insert: {e}")))?;
            restored += 1;
        }
        Ok(restored)
    }
}
