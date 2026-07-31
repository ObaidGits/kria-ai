//! Goal Memory (memory-upgrade Phase 2, design §22/§45).
//!
//! Goals are first-class authority entities (the `goals` table from schema
//! 0001, extended with `parent_id` in 0006 for hierarchy). This engine owns
//! their lifecycle — creation, decomposition into sub-goals (goal graph),
//! status transitions, priority, confidence, and the planner/reasoner retrieval
//! surface — over the single authority [`Database`]. No parallel store:
//! `memories.goal_context_id` already references these rows, so goal-scoped
//! knowledge shares the one memory substrate.
//!
//! Priority is Memory-Worth-aware at read time: [`GoalStore::planner_context`]
//! and [`GoalStore::active_goals`] order by `(priority, confidence)` so the
//! planner naturally prefers high-priority, high-confidence goals.
//!
//! **Pending F1.5/F2 governed-writer cutover.** [`CommandCandidate::goal`](
//! crate::memory::authority::CommandCandidate::goal) is the typed
//! command-candidate scaffolding (task F1.5.1) this store's goal-creation
//! writes will route through once a concrete `TxSemanticStore` builder
//! persists a `goals_v2` row (F2; goal *status transitions* are a separate
//! preview-gated `Correct` command, F1.7). Until that builder exists, this
//! store remains the live persistence path — routing through the
//! [`AuthorityCommandBus`](crate::memory::authority::AuthorityCommandBus) today
//! would silently drop goal content, since the bus's only available semantic
//! store (`DeferredSemanticStore`) writes no concrete row. See the ledger in
//! [`crate::memory::model::legacy_mapping`].

use std::sync::Arc;

use rusqlite::params;
use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::ids::new_id;

/// Goal lifecycle status (superset of the schema default `candidate`).
///
/// **Superseded by** the canonical v2 [`crate::memory::model::GoalStatus`]
/// (a different closed set: no `failed`/`abandoned`; adds `conflicted`/`stale`/
/// `superseded`/`deleted`). Retained as the live `goals`-table status until the
/// F1.5 write cutover, which remaps `failed`/`abandoned` → `deleted`; see the
/// ledger in [`crate::memory::model::legacy_mapping`] (task F2.1.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoalStatus {
    /// Proposed, not yet actively pursued.
    Candidate,
    /// Actively being pursued.
    Active,
    /// Temporarily suspended (resumable).
    Paused,
    /// Achieved.
    Completed,
    /// Attempted and could not be achieved.
    Failed,
    /// Deliberately dropped.
    Abandoned,
}

impl GoalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            GoalStatus::Candidate => "candidate",
            GoalStatus::Active => "active",
            GoalStatus::Paused => "paused",
            GoalStatus::Completed => "completed",
            GoalStatus::Failed => "failed",
            GoalStatus::Abandoned => "abandoned",
        }
    }

    pub fn from_str(s: &str) -> GoalStatus {
        match s {
            "active" => GoalStatus::Active,
            "paused" => GoalStatus::Paused,
            "completed" => GoalStatus::Completed,
            "failed" => GoalStatus::Failed,
            "abandoned" => GoalStatus::Abandoned,
            _ => GoalStatus::Candidate,
        }
    }

    /// Whether this status is "open" (still relevant to planning).
    pub fn is_open(&self) -> bool {
        matches!(
            self,
            GoalStatus::Candidate | GoalStatus::Active | GoalStatus::Paused
        )
    }

    /// Whether this is a terminal status.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            GoalStatus::Completed | GoalStatus::Failed | GoalStatus::Abandoned
        )
    }
}

/// A goal record (mirror of the authority `goals` row + hierarchy).
///
/// **Superseded by** [`crate::memory::model::Goal`] (`goals_v2`) +
/// [`crate::memory::model::GoalProgress`] (canonical v2 goal). Retained as the
/// live goals persistence/read model until the F1.5 write cutover + F3
/// retrieval-on-v2; see the ledger in [`crate::memory::model::legacy_mapping`]
/// (task F2.1.6).
#[derive(Clone, Debug, PartialEq)]
pub struct Goal {
    pub id: Uuid,
    pub kind: String,
    pub title: String,
    pub status: GoalStatus,
    pub confidence: f64,
    pub priority: i64,
    pub resumption_context: Option<String>,
    pub parent_id: Option<Uuid>,
    pub created_at: String,
    pub last_progress_at: Option<String>,
}

/// A specification for creating a goal.
#[derive(Clone, Debug)]
pub struct NewGoal {
    pub kind: String,
    pub title: String,
    pub priority: i64,
    pub confidence: f64,
    pub parent_id: Option<Uuid>,
    pub resumption_context: Option<String>,
}

impl NewGoal {
    /// A user-declared goal at default priority/confidence.
    pub fn user(title: impl Into<String>) -> Self {
        Self {
            kind: "user".into(),
            title: title.into(),
            priority: 6,
            confidence: 0.6,
            parent_id: None,
            resumption_context: None,
        }
    }

    /// A system/self-improvement goal (e.g. from Active Learning).
    pub fn system(kind: impl Into<String>, title: impl Into<String>, priority: i64) -> Self {
        Self {
            kind: kind.into(),
            title: title.into(),
            priority: priority.clamp(0, 10),
            confidence: 0.5,
            parent_id: None,
            resumption_context: None,
        }
    }

    pub fn with_parent(mut self, parent: Uuid) -> Self {
        self.parent_id = Some(parent);
        self
    }
}

/// Goal Memory engine over the authority database.
#[derive(Clone)]
pub struct GoalStore {
    db: Arc<Database>,
}

impl GoalStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Create a goal (status `candidate`). Returns its id.
    pub fn create(&self, spec: NewGoal) -> MemoryResult<Uuid> {
        let id = new_id();
        let now = chrono::Utc::now().to_rfc3339();
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO goals(id, kind, title, status, confidence, priority, \
                 resumption_context, created_at, last_progress_at, parent_id) \
                 VALUES(?1,?2,?3,'candidate',?4,?5,?6,?7,NULL,?8)",
                params![
                    id.to_string(),
                    spec.kind,
                    spec.title,
                    spec.confidence.clamp(0.0, 1.0),
                    spec.priority.clamp(0, 10),
                    spec.resumption_context,
                    now,
                    spec.parent_id.map(|p| p.to_string()),
                ],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()?;
        Ok(id)
    }

    /// Decompose a parent goal into sub-goals (goal graph). Returns child ids.
    /// Sub-goals inherit the parent's priority minus one (so the parent's own
    /// direct progress still outranks its children by default).
    pub fn decompose(&self, parent: Uuid, subgoals: &[String]) -> MemoryResult<Vec<Uuid>> {
        let parent_goal = self.get(parent)?.ok_or_else(|| {
            crate::memory::error::MemoryError::Internal("decompose: parent goal not found".into())
        })?;
        let child_priority = (parent_goal.priority - 1).max(0);
        let mut ids = Vec::with_capacity(subgoals.len());
        for title in subgoals {
            let id = self.create(NewGoal {
                kind: parent_goal.kind.clone(),
                title: title.clone(),
                priority: child_priority,
                confidence: parent_goal.confidence,
                parent_id: Some(parent),
                resumption_context: None,
            })?;
            ids.push(id);
        }
        Ok(ids)
    }

    /// Fetch a single goal.
    pub fn get(&self, id: Uuid) -> MemoryResult<Option<Goal>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, kind, title, status, confidence, priority, resumption_context, \
                     parent_id, created_at, last_progress_at FROM goals WHERE id = ?1",
                )
                .map_err(StorageError::Sqlite)?;
            let mut rows = stmt
                .query_map(params![id.to_string()], row_to_goal)
                .map_err(StorageError::Sqlite)?;
            match rows.next() {
                Some(r) => Ok(Some(r.map_err(StorageError::Sqlite)?)),
                None => Ok(None),
            }
        })
    }

    /// All open goals (candidate/active/paused), highest priority first.
    pub fn active_goals(&self, limit: usize) -> MemoryResult<Vec<Goal>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, kind, title, status, confidence, priority, resumption_context, \
                     parent_id, created_at, last_progress_at FROM goals \
                     WHERE status IN ('candidate','active','paused') \
                     ORDER BY priority DESC, confidence DESC, created_at ASC LIMIT ?1",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![limit as i64], row_to_goal)
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(rows)
        })
    }

    /// Whether an open (candidate/active/paused) goal with this exact title
    /// already exists — used to avoid creating duplicate learning goals.
    pub fn has_open_goal_with_title(&self, title: &str) -> MemoryResult<bool> {
        self.db.with_read(|conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM goals WHERE title = ?1 \
                     AND status IN ('candidate','active','paused')",
                    params![title],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            Ok(n > 0)
        })
    }

    /// Direct children of a goal (decomposition graph).
    pub fn children(&self, parent: Uuid) -> MemoryResult<Vec<Goal>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, kind, title, status, confidence, priority, resumption_context, \
                     parent_id, created_at, last_progress_at FROM goals \
                     WHERE parent_id = ?1 ORDER BY priority DESC, created_at ASC",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![parent.to_string()], row_to_goal)
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(rows)
        })
    }

    /// Transition a goal's status, stamping progress time.
    pub fn set_status(&self, id: Uuid, status: GoalStatus) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "UPDATE goals SET status = ?2, last_progress_at = ?3 WHERE id = ?1",
                params![
                    id.to_string(),
                    status.as_str(),
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// Set a goal's planning priority (0..=10).
    pub fn set_priority(&self, id: Uuid, priority: i64) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "UPDATE goals SET priority = ?2 WHERE id = ?1",
                params![id.to_string(), priority.clamp(0, 10)],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// Nudge a goal's confidence by `delta`, clamped to [0,1] (belief update
    /// from outcomes — reflection/dreaming recalibration).
    pub fn adjust_confidence(&self, id: Uuid, delta: f64) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "UPDATE goals SET confidence = MAX(0.0, MIN(1.0, confidence + ?2)) WHERE id = ?1",
                params![id.to_string(), delta],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// Mark progress (updates `last_progress_at`) without changing status.
    pub fn record_progress(&self, id: Uuid) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "UPDATE goals SET last_progress_at = ?2 WHERE id = ?1",
                params![id.to_string(), chrono::Utc::now().to_rfc3339()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// A compact planner/reasoner grounding block of the top open goals, or
    /// `None` when there are none. Injected into reasoning context so plans are
    /// goal-aware (design Priority 1/2).
    pub fn planner_context(&self, limit: usize) -> MemoryResult<Option<String>> {
        let goals = self.active_goals(limit)?;
        if goals.is_empty() {
            return Ok(None);
        }
        let lines: Vec<String> = goals
            .iter()
            .map(|g| {
                format!(
                    "- [{}] {} (priority {}, confidence {:.2})",
                    g.status.as_str(),
                    g.title.trim(),
                    g.priority,
                    g.confidence
                )
            })
            .collect();
        Ok(Some(format!(
            "Active goals (pursue these; highest priority first):\n{}",
            lines.join("\n")
        )))
    }

    /// Completion analytics: counts by status for observability/benchmarking.
    pub fn analytics(&self) -> MemoryResult<GoalAnalytics> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare("SELECT status, COUNT(*) FROM goals GROUP BY status")
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            let mut a = GoalAnalytics::default();
            for (status, count) in rows {
                match GoalStatus::from_str(&status) {
                    GoalStatus::Candidate => a.candidate = count,
                    GoalStatus::Active => a.active = count,
                    GoalStatus::Paused => a.paused = count,
                    GoalStatus::Completed => a.completed = count,
                    GoalStatus::Failed => a.failed = count,
                    GoalStatus::Abandoned => a.abandoned = count,
                }
            }
            Ok(a)
        })
    }
}

/// Aggregate goal-completion analytics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GoalAnalytics {
    pub candidate: i64,
    pub active: i64,
    pub paused: i64,
    pub completed: i64,
    pub failed: i64,
    pub abandoned: i64,
}

impl GoalAnalytics {
    /// Total goals ever tracked.
    pub fn total(&self) -> i64 {
        self.candidate + self.active + self.paused + self.completed + self.failed + self.abandoned
    }

    /// Completion rate over terminal goals (completed / (completed+failed+abandoned)).
    pub fn completion_rate(&self) -> f64 {
        let terminal = self.completed + self.failed + self.abandoned;
        if terminal == 0 {
            0.0
        } else {
            self.completed as f64 / terminal as f64
        }
    }
}

fn row_to_goal(row: &rusqlite::Row<'_>) -> rusqlite::Result<Goal> {
    let id: String = row.get(0)?;
    let status: String = row.get(3)?;
    let parent: Option<String> = row.get(7)?;
    Ok(Goal {
        id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
        kind: row.get(1)?,
        title: row.get(2)?,
        status: GoalStatus::from_str(&status),
        confidence: row.get(4)?,
        priority: row.get(5)?,
        resumption_context: row.get(6)?,
        parent_id: parent.and_then(|p| Uuid::parse_str(&p).ok()),
        created_at: row.get(8)?,
        last_progress_at: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> GoalStore {
        GoalStore::new(Arc::new(Database::open_in_memory().unwrap()))
    }

    #[test]
    fn create_get_and_status_transitions() {
        let gs = store();
        let id = gs.create(NewGoal::user("ship the memory upgrade")).unwrap();
        let g = gs.get(id).unwrap().unwrap();
        assert_eq!(g.title, "ship the memory upgrade");
        assert_eq!(g.status, GoalStatus::Candidate);
        assert!(g.status.is_open());

        gs.set_status(id, GoalStatus::Active).unwrap();
        assert_eq!(gs.get(id).unwrap().unwrap().status, GoalStatus::Active);
        gs.set_status(id, GoalStatus::Completed).unwrap();
        assert!(gs.get(id).unwrap().unwrap().status.is_terminal());
    }

    #[test]
    fn decomposition_builds_goal_graph() {
        let gs = store();
        let parent = gs.create(NewGoal::user("release v1")).unwrap();
        let kids = gs
            .decompose(parent, &["write docs".into(), "run benchmarks".into()])
            .unwrap();
        assert_eq!(kids.len(), 2);
        let children = gs.children(parent).unwrap();
        assert_eq!(children.len(), 2);
        for c in &children {
            assert_eq!(c.parent_id, Some(parent));
            // Sub-goals inherit parent priority minus one.
            assert_eq!(c.priority, gs.get(parent).unwrap().unwrap().priority - 1);
        }
    }

    #[test]
    fn active_goals_ordered_by_priority() {
        let gs = store();
        gs.create(NewGoal::system("improve", "low prio", 2))
            .unwrap();
        let hi = gs
            .create(NewGoal::system("improve", "high prio", 9))
            .unwrap();
        let active = gs.active_goals(10).unwrap();
        assert_eq!(active.len(), 2);
        assert_eq!(active[0].id, hi, "highest priority first");
    }

    #[test]
    fn planner_context_lists_open_goals_only() {
        let gs = store();
        let a = gs.create(NewGoal::user("open goal")).unwrap();
        let done = gs.create(NewGoal::user("finished goal")).unwrap();
        gs.set_status(done, GoalStatus::Completed).unwrap();
        let ctx = gs.planner_context(10).unwrap().unwrap();
        assert!(ctx.contains("open goal"));
        assert!(!ctx.contains("finished goal"));
        gs.set_status(a, GoalStatus::Abandoned).unwrap();
        assert!(gs.planner_context(10).unwrap().is_none());
    }

    #[test]
    fn confidence_clamped_and_analytics() {
        let gs = store();
        let id = gs.create(NewGoal::user("g")).unwrap();
        gs.adjust_confidence(id, 5.0).unwrap();
        assert_eq!(gs.get(id).unwrap().unwrap().confidence, 1.0);
        gs.adjust_confidence(id, -5.0).unwrap();
        assert_eq!(gs.get(id).unwrap().unwrap().confidence, 0.0);

        let done = gs.create(NewGoal::user("d")).unwrap();
        gs.set_status(done, GoalStatus::Completed).unwrap();
        let a = gs.analytics().unwrap();
        assert_eq!(a.completed, 1);
        assert_eq!(a.candidate, 1);
        assert!((a.completion_rate() - 1.0).abs() < f64::EPSILON);
        assert_eq!(a.total(), 2);
    }
}
