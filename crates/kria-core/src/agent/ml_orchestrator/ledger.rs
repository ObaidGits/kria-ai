// crates/kria-core/src/agent/ml_orchestrator/ledger.rs
//
// Actor-model JobLedger. Single-writer SQLite via MPSC channel.
// No contention. No locks. Ever.

use std::path::Path;
use tokio::sync::{mpsc, oneshot};
use rusqlite::Connection;

use super::types::PhaseArtifact;

/// Messages sent to the Ledger Actor.
pub enum LedgerMsg {
    CreateJob {
        job_id: String, plan_name: String, task_type: String,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    MarkStarted {
        job_id: String, cell_id: String, phase: String, pid: Option<u32>,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    MarkCompleted {
        job_id: String, cell_id: String, outputs: Vec<PhaseArtifact>,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    MarkFailed {
        job_id: String, cell_id: String, error: String,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    UpdateHeartbeat {
        job_id: String, cell_id: String, heartbeat_ts: f64,
        metrics: serde_json::Value,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    GetResumePoint {
        job_id: String,
        reply: oneshot::Sender<anyhow::Result<Option<String>>>,
    },
    GetPhaseOutputs {
        job_id: String, cell_id: String,
        reply: oneshot::Sender<anyhow::Result<Vec<PhaseArtifact>>>,
    },
    Shutdown,
}

/// Handle to the Ledger Actor. Cloneable — safe to share across async tasks.
#[derive(Clone)]
pub struct LedgerHandle {
    tx: mpsc::UnboundedSender<LedgerMsg>,
}

impl LedgerHandle {
    pub async fn create_job(&self, job_id: &str, plan_name: &str, task_type: &str) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(LedgerMsg::CreateJob {
            job_id: job_id.into(), plan_name: plan_name.into(),
            task_type: task_type.into(), reply: tx,
        }).map_err(|_| anyhow::anyhow!("ledger actor stopped"))?;
        rx.await.map_err(|_| anyhow::anyhow!("ledger actor dropped reply"))?
    }

    pub async fn mark_started(&self, job_id: &str, cell_id: &str, phase: &str, pid: Option<u32>) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(LedgerMsg::MarkStarted {
            job_id: job_id.into(), cell_id: cell_id.into(),
            phase: phase.into(), pid, reply: tx,
        }).map_err(|_| anyhow::anyhow!("ledger actor stopped"))?;
        rx.await.map_err(|_| anyhow::anyhow!("ledger actor dropped reply"))?
    }

    pub async fn mark_completed(&self, job_id: &str, cell_id: &str, outputs: Vec<PhaseArtifact>) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(LedgerMsg::MarkCompleted {
            job_id: job_id.into(), cell_id: cell_id.into(), outputs, reply: tx,
        }).map_err(|_| anyhow::anyhow!("ledger actor stopped"))?;
        rx.await.map_err(|_| anyhow::anyhow!("ledger actor dropped reply"))?
    }

    pub async fn mark_failed(&self, job_id: &str, cell_id: &str, error: &str) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(LedgerMsg::MarkFailed {
            job_id: job_id.into(), cell_id: cell_id.into(),
            error: error.into(), reply: tx,
        }).map_err(|_| anyhow::anyhow!("ledger actor stopped"))?;
        rx.await.map_err(|_| anyhow::anyhow!("ledger actor dropped reply"))?
    }

    pub async fn get_resume_point(&self, job_id: &str) -> anyhow::Result<Option<String>> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(LedgerMsg::GetResumePoint {
            job_id: job_id.into(), reply: tx,
        }).map_err(|_| anyhow::anyhow!("ledger actor stopped"))?;
        rx.await.map_err(|_| anyhow::anyhow!("ledger actor dropped reply"))?
    }

    pub async fn get_phase_outputs(&self, job_id: &str, cell_id: &str) -> anyhow::Result<Vec<PhaseArtifact>> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(LedgerMsg::GetPhaseOutputs {
            job_id: job_id.into(), cell_id: cell_id.into(), reply: tx,
        }).map_err(|_| anyhow::anyhow!("ledger actor stopped"))?;
        rx.await.map_err(|_| anyhow::anyhow!("ledger actor dropped reply"))?
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(LedgerMsg::Shutdown);
    }
}

/// The Ledger Actor — runs as a dedicated tokio task.
pub struct LedgerActor {
    rx: mpsc::UnboundedReceiver<LedgerMsg>,
    conn: Connection,
}

impl LedgerActor {
    pub fn spawn(db_path: &Path) -> anyhow::Result<LedgerHandle> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS ml_jobs (
                job_id TEXT PRIMARY KEY,
                plan_name TEXT NOT NULL,
                task_type TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'running'
            );
            CREATE TABLE IF NOT EXISTS ml_phases (
                job_id TEXT NOT NULL,
                cell_id TEXT NOT NULL,
                phase TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                started_at INTEGER,
                completed_at INTEGER,
                outputs_json TEXT,
                error TEXT,
                worker_pid INTEGER,
                attempt INTEGER NOT NULL DEFAULT 0,
                last_heartbeat_ts REAL,
                last_metrics_json TEXT,
                PRIMARY KEY (job_id, cell_id)
            );
        ")?;

        let (tx, rx) = mpsc::unbounded_channel();
        let actor = LedgerActor { rx, conn };

        tokio::spawn(async move { actor.run().await; });

        Ok(LedgerHandle { tx })
    }

    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                LedgerMsg::CreateJob { job_id, plan_name, task_type, reply } => {
                    let _ = reply.send(self.create_job(&job_id, &plan_name, &task_type));
                }
                LedgerMsg::MarkStarted { job_id, cell_id, phase, pid, reply } => {
                    let _ = reply.send(self.mark_started(&job_id, &cell_id, &phase, pid));
                }
                LedgerMsg::MarkCompleted { job_id, cell_id, outputs, reply } => {
                    let _ = reply.send(self.mark_completed(&job_id, &cell_id, &outputs));
                }
                LedgerMsg::MarkFailed { job_id, cell_id, error, reply } => {
                    let _ = reply.send(self.mark_failed(&job_id, &cell_id, &error));
                }
                LedgerMsg::UpdateHeartbeat { job_id, cell_id, heartbeat_ts, metrics, reply } => {
                    let _ = reply.send(self.update_heartbeat(&job_id, &cell_id, heartbeat_ts, &metrics));
                }
                LedgerMsg::GetResumePoint { job_id, reply } => {
                    let _ = reply.send(self.get_resume_point(&job_id));
                }
                LedgerMsg::GetPhaseOutputs { job_id, cell_id, reply } => {
                    let _ = reply.send(self.get_phase_outputs(&job_id, &cell_id));
                }
                LedgerMsg::Shutdown => break,
            }
        }
    }

    fn now() -> i64 {
        chrono::Utc::now().timestamp()
    }

    fn create_job(&self, job_id: &str, plan_name: &str, task_type: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO ml_jobs (job_id, plan_name, task_type, created_at, status)
             VALUES (?1, ?2, ?3, ?4, 'running')",
            rusqlite::params![job_id, plan_name, task_type, Self::now()],
        )?;
        Ok(())
    }

    fn mark_started(&self, job_id: &str, cell_id: &str, phase: &str, pid: Option<u32>) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO ml_phases (job_id, cell_id, phase, status, started_at, worker_pid, attempt)
             VALUES (?1, ?2, ?3, 'running', ?4, ?5, 1)
             ON CONFLICT(job_id, cell_id) DO UPDATE SET
                status = 'running',
                started_at = ?4,
                worker_pid = ?5,
                attempt = ml_phases.attempt + 1",
            rusqlite::params![job_id, cell_id, phase, Self::now(), pid],
        )?;
        Ok(())
    }

    fn mark_completed(&self, job_id: &str, cell_id: &str, outputs: &[PhaseArtifact]) -> anyhow::Result<()> {
        let json = serde_json::to_string(outputs)?;
        self.conn.execute(
            "UPDATE ml_phases SET status='completed', completed_at=?1, outputs_json=?2
             WHERE job_id=?3 AND cell_id=?4",
            rusqlite::params![Self::now(), json, job_id, cell_id],
        )?;
        Ok(())
    }

    fn mark_failed(&self, job_id: &str, cell_id: &str, error: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE ml_phases SET status='failed', error=?1 WHERE job_id=?2 AND cell_id=?3",
            rusqlite::params![error, job_id, cell_id],
        )?;
        Ok(())
    }

    fn update_heartbeat(&self, job_id: &str, cell_id: &str, ts: f64, metrics: &serde_json::Value) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE ml_phases SET last_heartbeat_ts=?1, last_metrics_json=?2
             WHERE job_id=?3 AND cell_id=?4",
            rusqlite::params![ts, serde_json::to_string(metrics)?, job_id, cell_id],
        )?;
        Ok(())
    }

    fn get_resume_point(&self, job_id: &str) -> anyhow::Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT cell_id FROM ml_phases WHERE job_id=?1 AND status != 'completed'
             ORDER BY rowid LIMIT 1"
        )?;
        let result = stmt.query_row(rusqlite::params![job_id], |row| {
            row.get::<_, String>(0)
        }).optional()?;
        Ok(result)
    }

    fn get_phase_outputs(&self, job_id: &str, cell_id: &str) -> anyhow::Result<Vec<PhaseArtifact>> {
        let mut stmt = self.conn.prepare(
            "SELECT outputs_json FROM ml_phases WHERE job_id=?1 AND cell_id=?2"
        )?;
        let json: Option<String> = stmt.query_row(rusqlite::params![job_id, cell_id], |row| {
            row.get(0)
        }).optional()?;
        match json {
            Some(j) if !j.is_empty() => Ok(serde_json::from_str(&j)?),
            _ => Ok(Vec::new()),
        }
    }
}

/// Extension trait for `rusqlite::Rows::query_row().optional()`.
trait OptionalExt<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
