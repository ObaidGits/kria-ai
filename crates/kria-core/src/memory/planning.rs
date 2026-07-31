//! Planning Memory (memory-upgrade Phase 2, Priority 1).
//!
//! Records the outcomes of plans — ordered tool/step sequences executed for a
//! task — per normalized **task class**, so the planner learns which approaches
//! succeed and which fail. Plan worth uses the same min-sample-gated scoring as
//! Memory-Worth (`success / (success+failure)`, gated below a sample floor), so
//! recommendations only kick in once there is evidence. Backed by the single
//! authority [`Database`] (no parallel store).
//!
//! The planner queries [`PlanStore::recommend`] before a cycle and injects the
//! historically best sequence as a hint; every executed sequence is recorded via
//! [`PlanStore::record_outcome`], closing the planning learning loop.
//!
//! **Pending F1.5/F2 governed-writer cutover.** [`CommandCandidate::plan_outcome`](
//! crate::memory::authority::CommandCandidate::plan_outcome) is the typed
//! command-candidate scaffolding (task F1.5.1) this store's outcome writes will
//! route through once a concrete `TxSemanticStore` builder persists the
//! plan-outcome semantic row (F2). This store remains the live persistence
//! path until then — see the ledger in [`crate::memory::model::legacy_mapping`].

use std::sync::Arc;

use rusqlite::params;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::ids::blake3_hex;

/// Minimum samples before a plan's worth is trusted for recommendation.
const MIN_SAMPLES: u32 = 2;

/// A recorded plan and its accumulated outcome worth.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanRecord {
    pub signature: String,
    pub task_label: String,
    pub steps: Vec<String>,
    pub success: u32,
    pub failure: u32,
    pub samples: u32,
    pub confidence: f64,
}

impl PlanRecord {
    /// Laplace-smoothed success ratio in [0,1]. Neutral (0.5) with no samples.
    pub fn worth(&self) -> f64 {
        let s = self.success as f64;
        let f = self.failure as f64;
        (s + 1.0) / (s + f + 2.0)
    }

    /// Whether this plan has enough evidence to be recommended.
    pub fn is_trusted(&self) -> bool {
        self.samples >= MIN_SAMPLES
    }
}

/// Normalize an arbitrary task description into a stable task-class label
/// (lowercase, collapsed whitespace, capped) so similar requests share history.
pub fn normalize_task_label(task: &str) -> String {
    let lowered = task.to_lowercase();
    let collapsed = lowered.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(160).collect()
}

/// Planning Memory engine over the authority database.
#[derive(Clone)]
pub struct PlanStore {
    db: Arc<Database>,
}

impl PlanStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    fn signature(task_label: &str, steps: &[String]) -> String {
        let joined = format!("{task_label}\u{1f}{}", steps.join("\u{1e}"));
        blake3_hex(joined.as_bytes())
    }

    /// Record the outcome of executing `steps` for `task`. Increments the plan's
    /// success/failure/samples counters (upsert). Empty step lists are ignored.
    pub fn record_outcome(&self, task: &str, steps: &[String], success: bool) -> MemoryResult<()> {
        if steps.is_empty() {
            return Ok(());
        }
        let task_label = normalize_task_label(task);
        let signature = Self::signature(&task_label, steps);
        let steps_json = serde_json::to_string(steps).unwrap_or_else(|_| "[]".to_string());
        let now = chrono::Utc::now().to_rfc3339();
        let (succ, fail) = if success {
            (1_i64, 0_i64)
        } else {
            (0_i64, 1_i64)
        };
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO plans(signature, task_label, steps, success, failure, samples, \
                 confidence, created_at, last_used) \
                 VALUES(?1,?2,?3,?4,?5,1,0.5,?6,?6) \
                 ON CONFLICT(signature) DO UPDATE SET \
                 success = success + ?4, failure = failure + ?5, samples = samples + 1, \
                 last_used = ?6, \
                 confidence = MAX(0.0, MIN(1.0, \
                    CAST(success + ?4 AS REAL) / CAST(success + ?4 + failure + ?5 AS REAL)))",
                params![signature, task_label, steps_json, succ, fail, now],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// All recorded plans for a task class, best worth first.
    pub fn plans_for(&self, task: &str) -> MemoryResult<Vec<PlanRecord>> {
        let task_label = normalize_task_label(task);
        let mut plans = self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT signature, task_label, steps, success, failure, samples, confidence \
                     FROM plans WHERE task_label = ?1",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![task_label], row_to_plan)
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(rows)
        })?;
        plans.sort_by(|a, b| {
            b.worth()
                .partial_cmp(&a.worth())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.samples.cmp(&a.samples))
        });
        Ok(plans)
    }

    /// The best trusted plan for a task (enough samples + worth above neutral),
    /// or `None` when there is no confident recommendation yet.
    pub fn best_plan(&self, task: &str) -> MemoryResult<Option<PlanRecord>> {
        Ok(self
            .plans_for(task)?
            .into_iter()
            .find(|p| p.is_trusted() && p.worth() > 0.5))
    }

    /// A planner grounding hint describing the historically most successful
    /// approach for this task, or `None` if there is no confident history.
    pub fn recommend(&self, task: &str) -> MemoryResult<Option<String>> {
        let Some(best) = self.best_plan(task)? else {
            return Ok(None);
        };
        Ok(Some(format!(
            "Historically effective approach for this task (worth {:.0}%, {} samples): {}",
            best.worth() * 100.0,
            best.samples,
            best.steps.join(" → ")
        )))
    }

    /// Plans with enough evidence whose worth is below `worth_ceiling` — the
    /// chronic under-performers the Self-Improvement engine turns into
    /// optimization goals. Worst worth first.
    pub fn weak_plans(
        &self,
        min_samples: u32,
        worth_ceiling: f64,
    ) -> MemoryResult<Vec<PlanRecord>> {
        let mut all = self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT signature, task_label, steps, success, failure, samples, confidence \
                     FROM plans WHERE samples >= ?1",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![min_samples as i64], row_to_plan)
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(rows)
        })?;
        all.retain(|p| p.worth() < worth_ceiling);
        all.sort_by(|a, b| {
            a.worth()
                .partial_cmp(&b.worth())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(all)
    }

    /// Trusted, high-worth plans (enough samples + worth above `worth_floor`) —
    /// the repeated successes Dream Intelligence generalizes into reusable
    /// procedures. Best worth first.
    pub fn strong_plans(
        &self,
        min_samples: u32,
        worth_floor: f64,
    ) -> MemoryResult<Vec<PlanRecord>> {
        let mut all = self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT signature, task_label, steps, success, failure, samples, confidence \
                     FROM plans WHERE samples >= ?1",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![min_samples as i64], row_to_plan)
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(rows)
        })?;
        all.retain(|p| p.worth() >= worth_floor);
        all.sort_by(|a, b| {
            b.worth()
                .partial_cmp(&a.worth())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(all)
    }

    /// Plan-memory analytics for benchmarking/observability.
    pub fn analytics(&self) -> MemoryResult<PlanAnalytics> {
        self.db.with_read(|conn| {
            let (distinct, samples, succ): (i64, i64, i64) = conn
                .query_row(
                    "SELECT COUNT(*), COALESCE(SUM(samples),0), COALESCE(SUM(success),0) FROM plans",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(StorageError::Sqlite)?;
            Ok(PlanAnalytics {
                distinct_plans: distinct,
                total_executions: samples,
                total_successes: succ,
            })
        })
    }
}

/// Aggregate planning analytics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanAnalytics {
    pub distinct_plans: i64,
    pub total_executions: i64,
    pub total_successes: i64,
}

impl PlanAnalytics {
    /// Overall plan success rate in [0,1].
    pub fn success_rate(&self) -> f64 {
        if self.total_executions == 0 {
            0.0
        } else {
            self.total_successes as f64 / self.total_executions as f64
        }
    }
}

fn row_to_plan(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlanRecord> {
    let steps_json: String = row.get(2)?;
    let steps: Vec<String> = serde_json::from_str(&steps_json).unwrap_or_default();
    Ok(PlanRecord {
        signature: row.get(0)?,
        task_label: row.get(1)?,
        steps,
        success: row.get::<_, i64>(3)?.max(0) as u32,
        failure: row.get::<_, i64>(4)?.max(0) as u32,
        samples: row.get::<_, i64>(5)?.max(0) as u32,
        confidence: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> PlanStore {
        PlanStore::new(Arc::new(Database::open_in_memory().unwrap()))
    }

    #[test]
    fn records_and_accumulates_worth() {
        let ps = store();
        let steps = vec!["search_files".to_string(), "read_file".to_string()];
        ps.record_outcome("find the config", &steps, true).unwrap();
        ps.record_outcome("find the config", &steps, true).unwrap();
        let plans = ps.plans_for("Find the Config").unwrap(); // case-insensitive
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].samples, 2);
        assert_eq!(plans[0].success, 2);
        assert!(plans[0].worth() > 0.5);
    }

    #[test]
    fn best_plan_prefers_higher_worth() {
        let ps = store();
        let good = vec!["tool_a".to_string()];
        let bad = vec!["tool_b".to_string()];
        // good: 3/3 success; bad: 0/3 success.
        for _ in 0..3 {
            ps.record_outcome("do the task", &good, true).unwrap();
            ps.record_outcome("do the task", &bad, false).unwrap();
        }
        let best = ps.best_plan("do the task").unwrap().unwrap();
        assert_eq!(best.steps, good);
        assert!(ps
            .recommend("do the task")
            .unwrap()
            .unwrap()
            .contains("tool_a"));
    }

    #[test]
    fn untrusted_plan_not_recommended() {
        let ps = store();
        // Single sample → below MIN_SAMPLES → no confident recommendation.
        ps.record_outcome("rare task", &["x".to_string()], true)
            .unwrap();
        assert!(ps.best_plan("rare task").unwrap().is_none());
        assert!(ps.recommend("rare task").unwrap().is_none());
    }

    #[test]
    fn analytics_track_success_rate() {
        let ps = store();
        ps.record_outcome("t", &["a".to_string()], true).unwrap();
        ps.record_outcome("t", &["a".to_string()], true).unwrap();
        ps.record_outcome("t", &["b".to_string()], false).unwrap();
        let a = ps.analytics().unwrap();
        assert_eq!(a.total_executions, 3);
        assert_eq!(a.total_successes, 2);
        assert!((a.success_rate() - 2.0 / 3.0).abs() < 1e-9);
    }

    /// Regression benchmark: after learning, the recommended plan's worth must
    /// exceed the failed alternative's — guards against ranking regressions.
    #[test]
    fn ranking_regression_guard() {
        let ps = store();
        let winner = vec!["plan_win".to_string()];
        let loser = vec!["plan_lose".to_string()];
        for _ in 0..5 {
            ps.record_outcome("bench task", &winner, true).unwrap();
            ps.record_outcome("bench task", &loser, false).unwrap();
        }
        let ranked = ps.plans_for("bench task").unwrap();
        assert_eq!(ranked[0].steps, winner);
        assert!(ranked[0].worth() > ranked[1].worth());
    }
}
