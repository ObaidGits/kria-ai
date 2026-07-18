//! Active Learning (memory-upgrade Phase 2, Priority 3).
//!
//! KRIA discovers what it does not know — recurring [`KnowledgeGap`]s that
//! retrieval keeps missing — and promotes the persistent ones into first-class
//! **learning goals** in [`GoalStore`]. Those goals then flow through the same
//! goal-aware planning/reasoning path as any other goal, so KRIA proactively
//! works to close its own gaps. No parallel task queue: learning tasks ARE
//! goals (one substrate).

use std::sync::Arc;

use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::MemoryResult;
use crate::memory::goals::{GoalStore, NewGoal};
use crate::memory::knowledge_gap::KnowledgeGapEngine;

/// The learning-goal title prefix (stable, used for dedup + resolution).
const LEARNING_PREFIX: &str = "Learn: ";

/// Active-Learning engine bridging knowledge gaps → learning goals.
pub struct ActiveLearning {
    gaps: KnowledgeGapEngine,
    goals: GoalStore,
}

impl ActiveLearning {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            gaps: KnowledgeGapEngine::new(db.clone()),
            goals: GoalStore::new(db),
        }
    }

    /// Promote unresolved knowledge gaps missed at least `min_misses` times into
    /// learning goals (up to `max_new` per pass, highest-missed first). Skips
    /// gaps that already have an open learning goal (dedup). Returns the ids of
    /// newly created goals. Priority scales with miss count (more misses → more
    /// pressing to learn).
    pub fn promote_gaps(&self, min_misses: u32, max_new: usize) -> MemoryResult<Vec<Uuid>> {
        let candidates = self.gaps.top_gaps(max_new.saturating_mul(4).max(max_new))?;
        let mut created = Vec::new();
        for gap in candidates {
            if created.len() >= max_new {
                break;
            }
            if gap.times_missed < min_misses {
                continue;
            }
            let title = format!("{LEARNING_PREFIX}{}", gap.query.trim());
            if self.goals.has_open_goal_with_title(&title)? {
                continue; // already tracked
            }
            // Priority 4..=9 scaling with persistence of the gap.
            let priority = (4 + gap.times_missed.min(5)) as i64;
            let mut spec = NewGoal::system("learning", title, priority);
            spec.resumption_context = gap.domain.clone();
            created.push(self.goals.create(spec)?);
        }
        Ok(created)
    }

    /// Mark the knowledge gap behind a learning goal resolved (called when the
    /// goal completes). Idempotent.
    pub fn resolve_for_goal_title(&self, goal_title: &str) -> MemoryResult<()> {
        if let Some(query) = goal_title.strip_prefix(LEARNING_PREFIX) {
            self.gaps.resolve(query)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::goals::GoalStatus;

    fn db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    #[test]
    fn recurring_gaps_become_learning_goals() {
        let db = db();
        let gaps = KnowledgeGapEngine::new(db.clone());
        // A gap missed 3× (persistent) and one missed once (transient).
        for _ in 0..3 {
            gaps.record_miss("how to configure the GPU lease arbiter", Some("infra"))
                .unwrap();
        }
        gaps.record_miss("one-off question", None).unwrap();

        let al = ActiveLearning::new(db.clone());
        let created = al.promote_gaps(2, 10).unwrap();
        assert_eq!(created.len(), 1, "only the persistent gap is promoted");

        let goals = GoalStore::new(db.clone());
        let g = goals.get(created[0]).unwrap().unwrap();
        assert!(g.title.starts_with("Learn: "));
        assert_eq!(g.kind, "learning");
        assert!(g.status.is_open());

        // Idempotent: a second pass creates no duplicates.
        assert!(al.promote_gaps(2, 10).unwrap().is_empty());
    }

    #[test]
    fn completing_learning_goal_resolves_gap() {
        let db = db();
        let gaps = KnowledgeGapEngine::new(db.clone());
        for _ in 0..2 {
            gaps.record_miss("what port does the sidecar use", None)
                .unwrap();
        }
        let al = ActiveLearning::new(db.clone());
        let created = al.promote_gaps(2, 5).unwrap();
        assert_eq!(created.len(), 1);

        let goals = GoalStore::new(db.clone());
        let title = goals.get(created[0]).unwrap().unwrap().title;
        goals.set_status(created[0], GoalStatus::Completed).unwrap();
        al.resolve_for_goal_title(&title).unwrap();

        // Gap resolved → no longer surfaced, no longer re-promoted.
        assert!(gaps.top_gaps(10).unwrap().is_empty());
        assert!(al.promote_gaps(2, 5).unwrap().is_empty());
    }
}
