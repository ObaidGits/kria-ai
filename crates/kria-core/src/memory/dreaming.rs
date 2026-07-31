//! Dream Intelligence (memory-upgrade Phase 2, Priority A).
//!
//! Dedicated autonomous synthesis passes that turn accumulated experience into
//! reusable, higher-order memory. Unlike [`cognition`](crate::memory::cognition)
//! (which consolidates recent session content into reflections), Dream
//! Intelligence generalizes across the whole store:
//!
//! * **Procedure synthesis** — repeated *successful* plans ([`PlanStore`]) are
//!   generalized into `Procedural` memories through the Write Policy (L11: they
//!   re-enter as untrusted `self_reflection`, so contradiction/dedup/security
//!   gating still applies).
//! * **Goal optimization** — duplicate open goals ([`GoalStore`]) are merged
//!   (highest priority kept, the rest abandoned), keeping the goal graph clean.
//!
//! Everything is idempotent (content-hash dedup for procedures; title dedup for
//! goals) and runs as bounded background jobs.

use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::MemoryResult;
use crate::memory::goals::{GoalStatus, GoalStore};
use crate::memory::planning::PlanStore;
use crate::memory::types::{MemoryType, Source, WriteCandidate, WriteDecision};
use crate::memory::write_policy::WritePolicy;

/// Evidence floor before a repeated plan is generalized into a procedure.
const PROC_MIN_SAMPLES: u32 = 3;
/// Worth a plan must exceed to be considered a repeated success.
const PROC_WORTH_FLOOR: f64 = 0.7;

/// Dream Intelligence engine. Reuses the plan/goal stores + Write Policy.
pub struct DreamEngine {
    db: Arc<Database>,
    write_policy: Arc<WritePolicy>,
}

impl DreamEngine {
    pub fn new(db: Arc<Database>, write_policy: Arc<WritePolicy>) -> Self {
        Self { db, write_policy }
    }

    /// Stable pseudo-session for dream-synthesized writes.
    fn dream_session() -> Uuid {
        Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0DEA_0001)
    }

    /// Procedure synthesis: generalize repeated successful plans into reusable
    /// `Procedural` memories through the Write Policy. Returns the number of
    /// procedures accepted by the gate (dedup collapses repeats).
    pub fn synthesize_procedures(&self, max_new: usize) -> MemoryResult<usize> {
        let plans = PlanStore::new(self.db.clone());
        let strong = plans.strong_plans(PROC_MIN_SAMPLES, PROC_WORTH_FLOOR)?;
        let session = Self::dream_session();
        let mut accepted = 0usize;
        for plan in strong.into_iter().take(max_new) {
            let content = format!(
                "Procedure — to accomplish \"{}\": {} (worked {:.0}% over {} attempts)",
                plan.task_label.trim(),
                plan.steps.join(" → "),
                plan.worth() * 100.0,
                plan.samples,
            );
            let candidate = WriteCandidate {
                source: Source::SelfReflection,
                proposed_type: Some(MemoryType::Procedural),
                ..WriteCandidate::user(session, content)
            };
            match self.write_policy.submit(candidate) {
                Ok(WriteDecision::Queued { .. }) | Ok(WriteDecision::Stored { .. }) => {
                    accepted += 1
                }
                _ => {}
            }
        }
        Ok(accepted)
    }

    /// Goal optimization: merge duplicate open goals sharing a title. The
    /// highest-priority instance survives; the rest are abandoned (reversible
    /// status change, provenance preserved). Returns the number abandoned.
    pub fn optimize_goals(&self) -> MemoryResult<usize> {
        let goals = GoalStore::new(self.db.clone());
        let open = goals.active_goals(500)?;
        let mut by_title: HashMap<String, Vec<_>> = HashMap::new();
        for g in open {
            by_title.entry(g.title.clone()).or_default().push(g);
        }
        let mut abandoned = 0usize;
        for (_title, mut group) in by_title {
            if group.len() < 2 {
                continue;
            }
            // Keep the highest priority (then highest confidence); abandon rest.
            group.sort_by(|a, b| {
                b.priority.cmp(&a.priority).then(
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
            });
            for dup in group.into_iter().skip(1) {
                goals.set_status(dup.id, GoalStatus::Abandoned)?;
                abandoned += 1;
            }
        }
        Ok(abandoned)
    }

    /// Rule synthesis: turn recurring causal *failures* into reusable IF→THEN
    /// avoidance rules, written as `Reflection` memories through the Write
    /// Policy. Reuses [`CausalMemory`](crate::memory::causal::CausalMemory). A
    /// cause is "recurring failure" when observed ≥3× with <40% success for a
    /// given effect. Returns rules accepted by the gate.
    pub fn synthesize_rules(&self, effect: &str, max_new: usize) -> MemoryResult<usize> {
        let causal = crate::memory::causal::CausalMemory::new(self.db.clone());
        let session = Self::dream_session();
        let mut accepted = 0usize;
        for link in causal.failure_causes(effect)?.into_iter().take(max_new) {
            if link.observations < 3 {
                continue;
            }
            let content = format!(
                "Rule — IF pursuing \"{}\" THEN avoid \"{}\" (failed {:.0}% over {} observations)",
                link.effect,
                link.cause,
                (1.0 - link.confidence()) * 100.0,
                link.observations,
            );
            let candidate = WriteCandidate {
                source: Source::SelfReflection,
                proposed_type: Some(MemoryType::Reflection),
                ..WriteCandidate::user(session, content)
            };
            match self.write_policy.submit(candidate) {
                Ok(WriteDecision::Queued { .. }) | Ok(WriteDecision::Stored { .. }) => {
                    accepted += 1
                }
                _ => {}
            }
        }
        Ok(accepted)
    }

    /// Worth/decay recalibration: apply gentle time-decay to the `decay_score`
    /// of active memories not accessed since `stale_cutoff_rfc3339`, so
    /// long-unused memories gradually lose retrieval priority (never deleted —
    /// D-8). Returns the number of memories recalibrated.
    ///
    /// F1.5.5: this is a lifecycle-maintenance authority mutation (it rewrites
    /// `memories.decay_score`), so it records a `memory_audit` row in the same
    /// transaction (MGR-033 AC3), matching [`Cognition::record_run`]'s pattern
    /// for the other background-maintenance direct-SQL write. It is not routed
    /// through [`AuthorityCommandBus`] because there is no `Correct` semantic
    /// builder for a bulk decay sweep yet (F2 scope); the UPDATE predicate
    /// itself is naturally idempotent (re-running with the same cutoff only
    /// re-applies decay to rows still stale, converging toward the 0.1 floor
    /// rather than duplicating any row or effect).
    ///
    /// [`Cognition::record_run`]: crate::memory::cognition::Cognition
    /// [`AuthorityCommandBus`]: crate::memory::authority::AuthorityCommandBus
    pub fn recalibrate_worth(&self, stale_cutoff_rfc3339: &str) -> MemoryResult<usize> {
        let tx = self.db.begin()?;
        let n = tx
            .conn()
            .execute(
                "UPDATE memories SET decay_score = MAX(0.1, decay_score * 0.95) \
                 WHERE state = 'active' \
                 AND (last_accessed IS NULL OR last_accessed < ?1)",
                rusqlite::params![stale_cutoff_rfc3339],
            )
            .map_err(crate::memory::error::StorageError::Sqlite)?;
        if n > 0 {
            tx.conn()
                .execute(
                    "INSERT INTO memory_audit(id, ts, decision, reason, namespace) \
                     VALUES(?1,?2,'stored',?3,'core')",
                    rusqlite::params![
                        crate::memory::ids::new_id().to_string(),
                        chrono::Utc::now().to_rfc3339(),
                        format!("dream.recalibrate_worth:recalibrated={n}"),
                    ],
                )
                .map_err(crate::memory::error::StorageError::Sqlite)?;
        }
        tx.commit()?;
        Ok(n)
    }

    /// Run all dream passes once. Returns
    /// `(procedures_synthesized, goals_merged, worth_recalibrated)`.
    pub fn run_all(&self, max_procedures: usize) -> MemoryResult<(usize, usize, usize)> {
        let procs = self.synthesize_procedures(max_procedures)?;
        let merged = self.optimize_goals()?;
        // Recalibrate worth for memories untouched for 30+ days.
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        let recal = self.recalibrate_worth(&cutoff)?;
        Ok((procs, merged, recal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::goals::NewGoal;
    use crate::memory::modes::ModeManager;
    use crate::memory::stores::{SqliteEventStore, SqliteRelationalStore};
    use crate::memory::types::MemoryMode;
    use crate::memory::write_policy::admission::Admission;
    use std::time::Duration;

    fn engine(db: &Arc<Database>) -> DreamEngine {
        let wp = Arc::new(WritePolicy::new(
            db.clone(),
            Arc::new(SqliteEventStore::new(db.clone())),
            Arc::new(SqliteRelationalStore::new(db.clone())),
            Arc::new(ModeManager::new(MemoryMode::Permanent)),
            Arc::new(Admission::new(Duration::from_secs(0))),
            "dev",
            None,
        ));
        DreamEngine::new(db.clone(), wp)
    }

    #[test]
    fn synthesizes_procedures_from_repeated_successes() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let plans = PlanStore::new(db.clone());
        // A repeatedly successful plan (5/5) → eligible for procedure synthesis.
        for _ in 0..5 {
            plans
                .record_outcome(
                    "back up the database",
                    &["snapshot".into(), "upload".into()],
                    true,
                )
                .unwrap();
        }
        // A weak plan that must NOT be generalized.
        for _ in 0..4 {
            plans
                .record_outcome("flaky task", &["bad".into()], false)
                .unwrap();
        }

        let de = engine(&db);
        let accepted = de.synthesize_procedures(10).unwrap();
        assert_eq!(accepted, 1, "only the repeated success becomes a procedure");

        // The procedure persisted as a self_reflection event through the gate.
        let cnt: i64 = db
            .with_read(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM events WHERE source='self_reflection'",
                    [],
                    |r| r.get(0),
                )
                .map_err(crate::memory::error::StorageError::Sqlite)?)
            })
            .unwrap();
        assert_eq!(cnt, 1);
    }

    #[test]
    fn synthesizes_avoidance_rules_from_causal_failures() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let causal = crate::memory::causal::CausalMemory::new(db.clone());
        // A recurring failure cause for "deploy": 0/4 success.
        for _ in 0..4 {
            causal.observe("skip tests", "deploy", false).unwrap();
        }
        let de = engine(&db);
        let rules = de.synthesize_rules("deploy", 10).unwrap();
        assert_eq!(rules, 1, "one avoidance rule synthesized");
        let cnt: i64 = db
            .with_read(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM events WHERE source='self_reflection'",
                    [],
                    |r| r.get(0),
                )
                .map_err(crate::memory::error::StorageError::Sqlite)?)
            })
            .unwrap();
        assert_eq!(cnt, 1);
    }

    #[test]
    fn worth_recalibration_decays_stale_memories() {
        use crate::memory::stores::ports::{EventStore, RelationalStore};
        use crate::memory::types::{
            Event, EventType, Memory, MemoryState, MemoryType, MemoryWorth, Modality, Scope,
            Sensitivity, Source, StalenessClass,
        };
        let db = Arc::new(Database::open_in_memory().unwrap());
        // Seed one active memory (decay_score 1.0, last_accessed NULL → stale).
        let events = SqliteEventStore::new(db.clone());
        let rel = SqliteRelationalStore::new(db.clone());
        let ev = Event {
            id: crate::memory::ids::new_id(),
            hlc: crate::memory::ids::HlcGenerator::new().now(),
            ts_utc: chrono::Utc::now(),
            tz_offset_min: 0,
            event_type: EventType::UserMessage,
            source: Source::User,
            session_id: None,
            parent_event_id: None,
            shred_key_id: None,
            payload: serde_json::json!({}),
            encrypted: false,
            checksum: "c".into(),
        };
        let now = chrono::Utc::now();
        let m = Memory {
            id: crate::memory::ids::new_id(),
            content: "stale fact".into(),
            memory_type: MemoryType::Semantic,
            compression_level: 0,
            source_event_id: ev.id,
            namespace: "core".into(),
            owner_id: "user".into(),
            device_id: "d".into(),
            scope: Scope::Global,
            confidence: 0.8,
            importance: 5.0,
            access_count: 0,
            decay_score: 1.0,
            staleness_class: StalenessClass::Slow,
            sensitivity: Sensitivity::Private,
            state: MemoryState::Active,
            created_at: now,
            last_accessed: None,
            valid_from: now,
            valid_until: None,
            embedding_id: None,
            embedding_model_version: None,
            estimated_tokens: 3,
            content_hash: "h-stale".into(),
            shred_key_id: None,
            verify_against: None,
            superseded_by: None,
            episode_id: None,
            goal_context_id: None,
            worth: MemoryWorth::default(),
            modality: Modality::Text,
            preference_pair_id: None,
            training_eligible: false,
        };
        {
            let mut tx = db.begin().unwrap();
            events.append(&mut tx, &ev).unwrap();
            rel.upsert_memory(&mut tx, &m).unwrap();
            tx.commit().unwrap();
        }
        let de = engine(&db);
        let future = (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339();
        let n = de.recalibrate_worth(&future).unwrap();
        assert_eq!(n, 1, "the stale active memory is recalibrated");
        assert!(rel.get_memory(m.id).unwrap().unwrap().decay_score < 1.0);

        // F1.5.5: the authority mutation must leave an audit trail.
        let (audit_count, reason): (i64, String) = db
            .with_read(|c| {
                Ok((
                    c.query_row("SELECT COUNT(*) FROM memory_audit", [], |r| r.get(0))
                        .map_err(crate::memory::error::StorageError::Sqlite)?,
                    c.query_row("SELECT reason FROM memory_audit LIMIT 1", [], |r| r.get(0))
                        .map_err(crate::memory::error::StorageError::Sqlite)?,
                ))
            })
            .unwrap();
        assert_eq!(audit_count, 1, "decay recalibration must be audited");
        assert!(
            reason.contains("recalibrated=1"),
            "audit reason must record the count, got {reason:?}"
        );

        // A further call once the memory is no longer stale (touched since a
        // past cutoff) repairs/audits nothing — no spurious audit growth.
        db.write()
            .execute(
                "UPDATE memories SET last_accessed = ?2 WHERE id = ?1",
                rusqlite::params![m.id.to_string(), chrono::Utc::now().to_rfc3339()],
            )
            .unwrap();
        let past = (chrono::Utc::now() - chrono::Duration::days(365)).to_rfc3339();
        let n2 = de.recalibrate_worth(&past).unwrap();
        assert_eq!(n2, 0, "no memory is stale relative to a past cutoff");
        let audit_count_after: i64 = db
            .with_read(|c| {
                Ok(
                    c.query_row("SELECT COUNT(*) FROM memory_audit", [], |r| r.get(0))
                        .map_err(crate::memory::error::StorageError::Sqlite)?,
                )
            })
            .unwrap();
        assert_eq!(
            audit_count_after, 1,
            "a no-op recalibration must not add a spurious audit row"
        );
    }

    #[test]
    fn optimize_goals_merges_duplicates() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let goals = GoalStore::new(db.clone());
        let a = goals
            .create(NewGoal::system("user", "ship release", 5))
            .unwrap();
        let b = goals
            .create(NewGoal::system("user", "ship release", 8))
            .unwrap();
        let solo = goals.create(NewGoal::user("unrelated goal")).unwrap();

        let de = engine(&db);
        let merged = de.optimize_goals().unwrap();
        assert_eq!(merged, 1, "one duplicate abandoned");

        // Highest-priority duplicate (b) survives; a is abandoned; solo untouched.
        assert!(goals.get(b).unwrap().unwrap().status.is_open());
        assert_eq!(goals.get(a).unwrap().unwrap().status, GoalStatus::Abandoned);
        assert!(goals.get(solo).unwrap().unwrap().status.is_open());
    }
}
