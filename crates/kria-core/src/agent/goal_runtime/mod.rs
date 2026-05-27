//! Batch 3 — Persistent Goal Runtime.
//!
//! # Core Mission
//!
//! Operational goals that survive KRIA restarts, workflow interruptions, and
//! session boundaries. A goal is a named, bounded, user-auditable objective
//! that KRIA tracks across multiple turns.
//!
//! # Design
//!
//! - Goals are NOT autonomous execution targets. KRIA cannot execute a goal
//!   without an explicit user turn.
//! - Goals are observable: `list_goals()` returns all active goals.
//! - Goals expire after `MAX_GOAL_AGE_DAYS` (7 days) to prevent unbounded growth.
//! - Goals are persisted to disk as atomic JSON writes (same pattern as
//!   `SessionManager`).
//! - Bounded to `MAX_ACTIVE_GOALS` (20) active goals at any time.
//!
//! # Goal Lifecycle
//!
//! ```text
//! Pending → Active → [Completed | Failed | Cancelled | Expired]
//! ```
//!
//! # Safety
//!
//! Goals only carry intent descriptions and metadata. They contain NO
//! executable payloads. They are human-auditable and deletable at any time.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Maximum number of active goals at any time.
pub const MAX_ACTIVE_GOALS: usize = 20;

/// Goals older than this are auto-expired.
pub const MAX_GOAL_AGE_DAYS: u64 = 7;

const MAX_GOAL_AGE_SECS: u64 = MAX_GOAL_AGE_DAYS * 86_400;

// ─── Goal Status ──────────────────────────────────────────────────────────────

/// Lifecycle status of an operational goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalStatus {
    /// Goal created, not yet started.
    Pending,
    /// Goal is actively being worked on.
    Active,
    /// Goal was completed successfully.
    Completed { at: u64 },
    /// Goal failed — may be retried.
    Failed { reason: String, at: u64 },
    /// Goal was cancelled by the user.
    Cancelled { at: u64 },
    /// Goal expired (too old) — automatically archived.
    Expired { at: u64 },
}

impl GoalStatus {
    /// Whether this goal is still actionable.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. }
                | Self::Failed { .. }
                | Self::Cancelled { .. }
                | Self::Expired { .. }
        )
    }
}

// ─── Operational Goal ─────────────────────────────────────────────────────────

/// An operational goal that survives KRIA restarts.
///
/// Goals are purely descriptive — they contain no executable payloads.
/// KRIA cannot autonomously execute a goal; a user turn is always required.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalGoal {
    /// Stable unique ID.
    pub goal_id: String,
    /// Human-readable description of the objective.
    pub description: String,
    /// Optional session ID of the last workflow that worked toward this goal.
    pub associated_session_id: Option<String>,
    /// Current lifecycle status.
    pub status: GoalStatus,
    /// When the goal was created (Unix seconds).
    pub created_at: u64,
    /// When the goal was last updated (Unix seconds).
    pub updated_at: u64,
    /// Optional continuation hint (what to do next to advance this goal).
    pub continuation_hint: Option<String>,
    /// Number of times this goal has been attempted.
    pub attempt_count: u32,
}

impl OperationalGoal {
    /// Whether this goal has exceeded the maximum age.
    pub fn is_expired(&self) -> bool {
        let now = now_epoch();
        now.saturating_sub(self.created_at) > MAX_GOAL_AGE_SECS
    }

    /// Whether this goal can still be worked on.
    pub fn is_actionable(&self) -> bool {
        !self.status.is_terminal() && !self.is_expired()
    }
}

// ─── Persistent Goal Runtime ──────────────────────────────────────────────────

/// Manages operational goals with disk persistence.
///
///
/// Goals survive process restarts. On construction, existing goals are loaded
/// from the data directory. Expired and completed goals are auto-archived on
/// each `maintenance()` call.
pub struct PersistentGoalRuntime {
    /// Goals directory (e.g., `~/.kria/goals/`).
    data_dir: Option<PathBuf>,
    /// In-memory goal index: goal_id → OperationalGoal.
    pub goals: Mutex<HashMap<String, OperationalGoal>>,
}

impl PersistentGoalRuntime {
    /// Create a new runtime. If `data_dir` is Some, goals are persisted there.
    pub fn new(data_dir: Option<PathBuf>) -> Self {
        let mut runtime = Self {
            data_dir,
            goals: Mutex::new(HashMap::new()),
        };
        runtime.load_from_disk();
        runtime
    }

    /// Create an in-memory-only runtime (useful in tests).
    pub fn ephemeral() -> Self {
        Self::new(None)
    }

    /// Create a new goal.
    ///
    /// Returns `None` if `MAX_ACTIVE_GOALS` is already reached.
    pub fn create_goal(
        &self,
        description: impl Into<String>,
        continuation_hint: Option<String>,
    ) -> Option<OperationalGoal> {
        let mut goals = self.goals.lock().unwrap();
        let active_count = goals.values().filter(|g| g.is_actionable()).count();
        if active_count >= MAX_ACTIVE_GOALS {
            warn!(
                target: "goal_runtime",
                active = active_count,
                "MAX_ACTIVE_GOALS reached — cannot create new goal"
            );
            return None;
        }

        let goal_id = format!("goal-{}", uuid_short());
        let now = now_epoch();
        let goal = OperationalGoal {
            goal_id: goal_id.clone(),
            description: description.into(),
            associated_session_id: None,
            status: GoalStatus::Pending,
            created_at: now,
            updated_at: now,
            continuation_hint,
            attempt_count: 0,
        };

        goals.insert(goal_id, goal.clone());
        drop(goals);

        info!(target: "goal_runtime", goal_id = %goal.goal_id, "Goal created");
        self.persist_goal(&goal);
        Some(goal)
    }

    /// Activate a pending goal.
    pub fn activate_goal(&self, goal_id: &str, session_id: Option<String>) -> bool {
        self.update_goal(goal_id, |g| {
            g.status = GoalStatus::Active;
            g.associated_session_id = session_id;
            g.attempt_count += 1;
        })
    }

    /// Mark a goal as completed.
    pub fn complete_goal(&self, goal_id: &str) -> bool {
        let at = now_epoch();
        self.update_goal(goal_id, |g| {
            g.status = GoalStatus::Completed { at };
        })
    }

    /// Mark a goal as failed.
    pub fn fail_goal(&self, goal_id: &str, reason: impl Into<String>) -> bool {
        let at = now_epoch();
        let reason = reason.into();
        self.update_goal(goal_id, move |g| {
            g.status = GoalStatus::Failed {
                reason: reason.clone(),
                at,
            };
        })
    }

    /// Cancel a goal (user-initiated).
    pub fn cancel_goal(&self, goal_id: &str) -> bool {
        let at = now_epoch();
        self.update_goal(goal_id, |g| {
            g.status = GoalStatus::Cancelled { at };
        })
    }

    /// Update the continuation hint for a goal.
    pub fn update_hint(&self, goal_id: &str, hint: impl Into<String>) -> bool {
        let hint = hint.into();
        self.update_goal(goal_id, move |g| {
            g.continuation_hint = Some(hint.clone());
        })
    }

    /// List all active (non-terminal, non-expired) goals.
    pub fn list_active_goals(&self) -> Vec<OperationalGoal> {
        self.goals
            .lock()
            .unwrap()
            .values()
            .filter(|g| g.is_actionable())
            .cloned()
            .collect()
    }

    /// List all stalled goals — active goals with no recent progress
    /// (attempt_count > 0 and status is still Active with no update in 1h).
    pub fn list_stalled(&self) -> Vec<OperationalGoal> {
        let one_hour = 3600;
        let threshold = now_epoch().saturating_sub(one_hour);
        self.goals
            .lock()
            .unwrap()
            .values()
            .filter(|g| {
                matches!(g.status, GoalStatus::Active)
                    && g.attempt_count > 0
                    && g.updated_at < threshold
            })
            .cloned()
            .collect()
    }

    /// Look up a goal by ID.
    pub fn get_goal(&self, goal_id: &str) -> Option<OperationalGoal> {
        self.goals.lock().unwrap().get(goal_id).cloned()
    }

    /// Run maintenance: expire old goals, purge terminal goals older than 24h.
    pub fn maintenance(&self) {
        let mut goals = self.goals.lock().unwrap();
        let now = now_epoch();
        let before = goals.len();
        let one_day = 86_400;

        goals.retain(|_, g| {
            // Expire age-exceeded goals
            if !g.status.is_terminal() && g.is_expired() {
                g.status = GoalStatus::Expired { at: now };
                return true; // keep the entry with Expired status for auditing
            }
            // Purge terminal goals older than 24h
            if g.status.is_terminal() && now.saturating_sub(g.updated_at) > one_day {
                return false;
            }
            true
        });

        let pruned = before - goals.len();
        if pruned > 0 {
            debug!(target: "goal_runtime", pruned, "Goals pruned in maintenance");
        }
    }

    /// Total goal count (all statuses).
    pub fn total_count(&self) -> usize {
        self.goals.lock().unwrap().len()
    }

    // ── Internal ─────────────────────────────────────────────────────────────

    fn update_goal<F: FnOnce(&mut OperationalGoal)>(&self, goal_id: &str, f: F) -> bool {
        let mut goals = self.goals.lock().unwrap();
        if let Some(goal) = goals.get_mut(goal_id) {
            f(goal);
            goal.updated_at = now_epoch();
            let goal_clone = goal.clone();
            drop(goals);
            self.persist_goal(&goal_clone);
            true
        } else {
            false
        }
    }

    fn persist_goal(&self, goal: &OperationalGoal) {
        let data_dir = match &self.data_dir {
            Some(d) => d.clone(),
            None => return,
        };
        if let Err(e) = std::fs::create_dir_all(&data_dir) {
            debug!(target: "goal_runtime", error = %e, "Failed to create goals dir");
            return;
        }
        let path = data_dir.join(format!("{}.json", goal.goal_id));
        let json = match serde_json::to_string_pretty(goal) {
            Ok(j) => j,
            Err(e) => {
                debug!(target: "goal_runtime", error = %e, "Failed to serialize goal");
                return;
            }
        };
        let tmp_path = path.with_extension("json.tmp");
        if std::fs::write(&tmp_path, &json).is_ok() {
            let _ = std::fs::rename(&tmp_path, &path);
        }
    }

    fn load_from_disk(&mut self) {
        let data_dir = match &self.data_dir {
            Some(d) => d.clone(),
            None => return,
        };
        let entries = match std::fs::read_dir(&data_dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        let mut goals = self.goals.lock().unwrap();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(json) = std::fs::read_to_string(&path) {
                if let Ok(goal) = serde_json::from_str::<OperationalGoal>(&json) {
                    goals.insert(goal.goal_id.clone(), goal);
                }
            }
        }
    }
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn uuid_short() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(12345);
    format!("{:08x}", nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt() -> PersistentGoalRuntime {
        PersistentGoalRuntime::ephemeral()
    }

    #[test]
    fn create_and_retrieve_goal() {
        let r = rt();
        let g = r.create_goal("finish the report", None).unwrap();
        let found = r.get_goal(&g.goal_id).unwrap();
        assert_eq!(found.description, "finish the report");
        assert!(matches!(found.status, GoalStatus::Pending));
    }

    #[test]
    fn activate_transitions_to_active() {
        let r = rt();
        let g = r.create_goal("code review", None).unwrap();
        assert!(r.activate_goal(&g.goal_id, None));
        assert!(matches!(
            r.get_goal(&g.goal_id).unwrap().status,
            GoalStatus::Active
        ));
    }

    #[test]
    fn complete_goal_marks_completed() {
        let r = rt();
        let g = r.create_goal("deploy staging", None).unwrap();
        r.activate_goal(&g.goal_id, None);
        r.complete_goal(&g.goal_id);
        assert!(matches!(
            r.get_goal(&g.goal_id).unwrap().status,
            GoalStatus::Completed { .. }
        ));
    }

    #[test]
    fn cancel_goal_marks_cancelled() {
        let r = rt();
        let g = r.create_goal("refactor module", None).unwrap();
        r.cancel_goal(&g.goal_id);
        assert!(matches!(
            r.get_goal(&g.goal_id).unwrap().status,
            GoalStatus::Cancelled { .. }
        ));
    }

    #[test]
    fn fail_goal_marks_failed() {
        let r = rt();
        let g = r.create_goal("run migration", None).unwrap();
        r.fail_goal(&g.goal_id, "connection timeout");
        let status = r.get_goal(&g.goal_id).unwrap().status;
        assert!(matches!(status, GoalStatus::Failed { .. }));
    }

    #[test]
    fn active_goals_excludes_terminal() {
        let r = rt();
        let g1 = r.create_goal("active goal", None).unwrap();
        let g2 = r.create_goal("done goal", None).unwrap();
        r.complete_goal(&g2.goal_id);
        let active = r.list_active_goals();
        assert!(active.iter().any(|g| g.goal_id == g1.goal_id));
        assert!(!active.iter().any(|g| g.goal_id == g2.goal_id));
    }

    #[test]
    fn max_active_goals_cap_enforced() {
        let r = rt();
        for i in 0..=MAX_ACTIVE_GOALS {
            r.create_goal(format!("goal {}", i), None);
        }
        let active = r.list_active_goals();
        assert!(
            active.len() <= MAX_ACTIVE_GOALS,
            "cap exceeded: {}",
            active.len()
        );
    }

    #[test]
    fn update_hint_persists() {
        let r = rt();
        let g = r.create_goal("write docs", None).unwrap();
        r.update_hint(&g.goal_id, "Start with the API section");
        let hint = r.get_goal(&g.goal_id).unwrap().continuation_hint;
        assert_eq!(hint.as_deref(), Some("Start with the API section"));
    }

    #[test]
    fn maintenance_expires_old_goals() {
        let r = rt();
        // Insert a goal with an old created_at
        {
            let mut goals = r.goals.lock().unwrap();
            goals.insert(
                "old-goal".to_string(),
                OperationalGoal {
                    goal_id: "old-goal".to_string(),
                    description: "old".to_string(),
                    associated_session_id: None,
                    status: GoalStatus::Pending,
                    created_at: 0, // epoch 0 — extremely old
                    updated_at: 0,
                    continuation_hint: None,
                    attempt_count: 0,
                },
            );
        }
        r.maintenance();
        let g = r.get_goal("old-goal").unwrap();
        assert!(matches!(g.status, GoalStatus::Expired { .. }));
    }
}
