//! Self-Improvement engine (memory-upgrade Phase 2, Priority 7).
//!
//! KRIA turns its own observed weaknesses into first-class **improvement
//! goals**. It reuses existing signals — chronically failing plans
//! ([`PlanStore::weak_plans`]) — rather than inventing a parallel task queue:
//! improvement tasks ARE goals in [`GoalStore`], so they flow through the same
//! goal-aware planning/reasoning path. Reflection/dreaming feed the same door by
//! recording weak plans; this promoter is the reflection→goal edge.

use std::sync::Arc;

use uuid::Uuid;

use crate::db::Database;
use crate::error::MemoryResult;
use crate::goals::{GoalStore, NewGoal};
use crate::planning::PlanStore;

/// Improvement-goal title prefix (stable, used for dedup).
const IMPROVE_PREFIX: &str = "Improve approach for: ";
/// Evidence floor before a weak plan is worth escalating.
const MIN_SAMPLES: u32 = 3;
/// Worth below this (with enough samples) is "chronically failing".
const WORTH_CEILING: f64 = 0.4;

/// Self-Improvement engine bridging weak plans → optimization goals.
pub struct SelfImprovement {
    plans: PlanStore,
    goals: GoalStore,
}

impl SelfImprovement {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            plans: PlanStore::new(db.clone()),
            goals: GoalStore::new(db),
        }
    }

    /// Promote chronically failing plans into improvement goals (up to `max_new`
    /// per pass, worst first). Deduped by title. Returns new goal ids. Priority
    /// scales inversely with worth (worse plans → higher-priority fixes).
    pub fn promote_weak_plans(&self, max_new: usize) -> MemoryResult<Vec<Uuid>> {
        let weak = self.plans.weak_plans(MIN_SAMPLES, WORTH_CEILING)?;
        let mut created = Vec::new();
        for plan in weak {
            if created.len() >= max_new {
                break;
            }
            let title = format!("{IMPROVE_PREFIX}{}", plan.task_label.trim());
            if self.goals.has_open_goal_with_title(&title)? {
                continue;
            }
            // worth in [0, WORTH_CEILING) → priority 6..=9 (worse ⇒ higher).
            let severity = ((WORTH_CEILING - plan.worth()) / WORTH_CEILING).clamp(0.0, 1.0);
            let priority = 6 + (severity * 3.0).round() as i64;
            let mut spec = NewGoal::system("improvement", title, priority);
            spec.resumption_context = Some(format!(
                "failing steps: {} (worth {:.0}%, {} samples)",
                plan.steps.join(" → "),
                plan.worth() * 100.0,
                plan.samples
            ));
            created.push(self.goals.create(spec)?);
        }
        Ok(created)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    #[test]
    fn chronically_failing_plans_become_improvement_goals() {
        let db = db();
        let plans = PlanStore::new(db.clone());
        // A chronically failing approach: 1 success / 5 failures (worth ~0.29).
        plans
            .record_outcome("deploy the app", &["bad_tool".into()], true)
            .unwrap();
        for _ in 0..5 {
            plans
                .record_outcome("deploy the app", &["bad_tool".into()], false)
                .unwrap();
        }
        // A healthy approach that must NOT be escalated.
        for _ in 0..4 {
            plans
                .record_outcome("healthy task", &["good_tool".into()], true)
                .unwrap();
        }

        let si = SelfImprovement::new(db.clone());
        let created = si.promote_weak_plans(10).unwrap();
        assert_eq!(created.len(), 1, "only the failing plan is escalated");

        let goals = GoalStore::new(db.clone());
        let g = goals.get(created[0]).unwrap().unwrap();
        assert!(g.title.starts_with("Improve approach for: deploy the app"));
        assert_eq!(g.kind, "improvement");
        assert!(g.priority >= 6);

        // Idempotent — a second pass makes no duplicates.
        assert!(si.promote_weak_plans(10).unwrap().is_empty());
    }
}
