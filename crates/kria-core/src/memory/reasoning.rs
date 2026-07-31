//! Reasoning Memory (memory-upgrade Phase 2, Priority 2).
//!
//! Records reasoning **traces** so past reasoning becomes reusable knowledge:
//! reasoning chains (with a known success/failure outcome), hypotheses, and
//! counterexamples, keyed by task class + session. Enables replay (a session's
//! reasoning in order), per-task history, hallucination tracking (failed chains
//! + counterexamples), and analytics. Backed by the single authority
//! [`Database`] (no parallel store).
//!
//! **Pending F1.5/F2 governed-writer cutover.** [`CommandCandidate::reasoning_trace`](
//! crate::memory::authority::CommandCandidate::reasoning_trace) is the typed
//! command-candidate scaffolding (task F1.5.1) this store's trace writes will
//! route through once a concrete `TxSemanticStore` builder persists the
//! reasoning-trace semantic row (F2). This store remains the live persistence
//! path until then — see the ledger in [`crate::memory::model::legacy_mapping`].

use std::sync::Arc;

use rusqlite::params;
use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::ids::new_id;
use crate::memory::planning::normalize_task_label;

/// A kind of reasoning trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceKind {
    /// A completed reasoning chain (may carry an outcome).
    Chain,
    /// A proposed hypothesis (unverified).
    Hypothesis,
    /// A counterexample / refutation (evidence a conclusion was wrong).
    Counterexample,
}

impl TraceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TraceKind::Chain => "chain",
            TraceKind::Hypothesis => "hypothesis",
            TraceKind::Counterexample => "counterexample",
        }
    }
    fn from_str(s: &str) -> TraceKind {
        match s {
            "hypothesis" => TraceKind::Hypothesis,
            "counterexample" => TraceKind::Counterexample,
            _ => TraceKind::Chain,
        }
    }
}

/// A stored reasoning trace.
#[derive(Clone, Debug, PartialEq)]
pub struct ReasoningTrace {
    pub id: Uuid,
    pub session_id: Option<String>,
    pub task_label: String,
    pub kind: TraceKind,
    pub content: String,
    pub confidence: f64,
    pub success: Option<bool>,
    pub created_at: String,
}

/// Reasoning-memory analytics (hallucination tracking + confidence).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReasoningAnalytics {
    pub chains: i64,
    pub hypotheses: i64,
    pub counterexamples: i64,
    pub failed_chains: i64,
    pub avg_confidence: f64,
}

impl ReasoningAnalytics {
    /// Fraction of reasoning that went wrong (failed chains + counterexamples
    /// over all chains) — a hallucination/error indicator in [0,1].
    pub fn hallucination_rate(&self) -> f64 {
        if self.chains == 0 {
            return 0.0;
        }
        ((self.failed_chains + self.counterexamples) as f64 / self.chains as f64).min(1.0)
    }
}

/// Reasoning Memory engine over the authority database.
#[derive(Clone)]
pub struct ReasoningStore {
    db: Arc<Database>,
}

impl ReasoningStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    fn insert(
        &self,
        session: Option<&str>,
        task: &str,
        kind: TraceKind,
        content: &str,
        confidence: f64,
        success: Option<bool>,
    ) -> MemoryResult<Uuid> {
        let id = new_id();
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO reasoning_traces(id, session_id, task_label, kind, content, \
                 confidence, success, created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    id.to_string(),
                    session,
                    normalize_task_label(task),
                    kind.as_str(),
                    content,
                    confidence.clamp(0.0, 1.0),
                    success.map(|s| if s { 1_i64 } else { 0_i64 }),
                    chrono::Utc::now().to_rfc3339(),
                ],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()?;
        Ok(id)
    }

    /// Record a completed reasoning chain with a known outcome.
    pub fn record_chain(
        &self,
        session: Option<&str>,
        task: &str,
        content: &str,
        confidence: f64,
        success: bool,
    ) -> MemoryResult<Uuid> {
        self.insert(
            session,
            task,
            TraceKind::Chain,
            content,
            confidence,
            Some(success),
        )
    }

    /// Record a hypothesis (unverified reasoning).
    pub fn record_hypothesis(
        &self,
        session: Option<&str>,
        task: &str,
        content: &str,
        confidence: f64,
    ) -> MemoryResult<Uuid> {
        self.insert(
            session,
            task,
            TraceKind::Hypothesis,
            content,
            confidence,
            None,
        )
    }

    /// Record a counterexample / refutation (hallucination signal).
    pub fn record_counterexample(
        &self,
        session: Option<&str>,
        task: &str,
        content: &str,
    ) -> MemoryResult<Uuid> {
        self.insert(session, task, TraceKind::Counterexample, content, 0.0, None)
    }

    /// Replay a session's reasoning traces in chronological order.
    pub fn replay(&self, session: &str) -> MemoryResult<Vec<ReasoningTrace>> {
        self.query(
            "SELECT id, session_id, task_label, kind, content, confidence, success, created_at \
             FROM reasoning_traces WHERE session_id = ?1 ORDER BY created_at ASC",
            params![session],
        )
    }

    /// Past reasoning for a task class, newest first (bounded).
    pub fn history_for_task(&self, task: &str, limit: usize) -> MemoryResult<Vec<ReasoningTrace>> {
        let task_label = normalize_task_label(task);
        self.query(
            "SELECT id, session_id, task_label, kind, content, confidence, success, created_at \
             FROM reasoning_traces WHERE task_label = ?1 ORDER BY created_at DESC LIMIT ?2",
            params![task_label, limit as i64],
        )
    }

    /// A grounding block of prior successful reasoning + known counterexamples
    /// for a task, or `None` when there is no relevant history.
    pub fn reasoning_context(&self, task: &str, limit: usize) -> MemoryResult<Option<String>> {
        let history = self.history_for_task(task, limit * 3)?;
        let mut good = Vec::new();
        let mut avoid = Vec::new();
        for t in history {
            match t.kind {
                TraceKind::Chain if t.success == Some(true) => {
                    good.push(format!("- {}", t.content.trim()));
                }
                TraceKind::Counterexample => avoid.push(format!("- {}", t.content.trim())),
                TraceKind::Chain if t.success == Some(false) => {
                    avoid.push(format!("- {}", t.content.trim()))
                }
                _ => {}
            }
            if good.len() >= limit && avoid.len() >= limit {
                break;
            }
        }
        good.truncate(limit);
        avoid.truncate(limit);
        if good.is_empty() && avoid.is_empty() {
            return Ok(None);
        }
        let mut out = String::from("Prior reasoning for similar tasks:");
        if !good.is_empty() {
            out.push_str("\nApproaches that worked:\n");
            out.push_str(&good.join("\n"));
        }
        if !avoid.is_empty() {
            out.push_str("\nApproaches to avoid (failed / refuted):\n");
            out.push_str(&avoid.join("\n"));
        }
        Ok(Some(out))
    }

    /// Reasoning analytics (hallucination tracking + average confidence).
    pub fn analytics(&self) -> MemoryResult<ReasoningAnalytics> {
        self.db.with_read(|conn| {
            let mut a = ReasoningAnalytics::default();
            let mut stmt = conn
                .prepare("SELECT kind, COUNT(*) FROM reasoning_traces GROUP BY kind")
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            for (kind, count) in rows {
                match TraceKind::from_str(&kind) {
                    TraceKind::Chain => a.chains = count,
                    TraceKind::Hypothesis => a.hypotheses = count,
                    TraceKind::Counterexample => a.counterexamples = count,
                }
            }
            a.failed_chains = conn
                .query_row(
                    "SELECT COUNT(*) FROM reasoning_traces WHERE kind='chain' AND success=0",
                    [],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            a.avg_confidence = conn
                .query_row(
                    "SELECT COALESCE(AVG(confidence),0.0) FROM reasoning_traces WHERE kind='chain'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            Ok(a)
        })
    }

    fn query(&self, sql: &str, p: impl rusqlite::Params) -> MemoryResult<Vec<ReasoningTrace>> {
        self.db.with_read(|conn| {
            let mut stmt = conn.prepare(sql).map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(p, row_to_trace)
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(rows)
        })
    }
}

fn row_to_trace(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReasoningTrace> {
    let id: String = row.get(0)?;
    let kind: String = row.get(3)?;
    let success: Option<i64> = row.get(6)?;
    Ok(ReasoningTrace {
        id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
        session_id: row.get(1)?,
        task_label: row.get(2)?,
        kind: TraceKind::from_str(&kind),
        content: row.get(4)?,
        confidence: row.get(5)?,
        success: success.map(|s| s != 0),
        created_at: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> ReasoningStore {
        ReasoningStore::new(Arc::new(Database::open_in_memory().unwrap()))
    }

    #[test]
    fn records_replays_and_analytics() {
        let rs = store();
        rs.record_chain(Some("s1"), "solve X", "tried A then B, worked", 0.8, true)
            .unwrap();
        rs.record_chain(Some("s1"), "solve X", "tried C, wrong", 0.4, false)
            .unwrap();
        rs.record_hypothesis(Some("s1"), "solve X", "maybe D works", 0.5)
            .unwrap();
        rs.record_counterexample(Some("s1"), "solve X", "D fails when input empty")
            .unwrap();

        let replay = rs.replay("s1").unwrap();
        assert_eq!(replay.len(), 4);
        assert_eq!(replay[0].content, "tried A then B, worked"); // chronological

        let a = rs.analytics().unwrap();
        assert_eq!(a.chains, 2);
        assert_eq!(a.failed_chains, 1);
        assert_eq!(a.counterexamples, 1);
        assert_eq!(a.hypotheses, 1);
        // hallucination rate = (failed_chains + counterexamples)/chains = 2/2 = 1.0
        assert!((a.hallucination_rate() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn reasoning_context_separates_good_and_avoid() {
        let rs = store();
        rs.record_chain(None, "task T", "good approach", 0.9, true)
            .unwrap();
        rs.record_counterexample(None, "Task T", "bad approach")
            .unwrap(); // case-insensitive
        let ctx = rs.reasoning_context("task t", 5).unwrap().unwrap();
        assert!(ctx.contains("good approach"));
        assert!(ctx.contains("bad approach"));
        assert!(ctx.contains("worked"));
        assert!(ctx.contains("avoid"));
    }

    #[test]
    fn empty_history_has_no_context() {
        let rs = store();
        assert!(rs.reasoning_context("nothing", 5).unwrap().is_none());
    }
}
