//! Background cognition jobs (memory-upgrade design §20/§25, ADR-008).
//!
//! These adapters bridge the [`Cognition`] engine to the [`CognitiveScheduler`]
//! so consolidation / reflection / dreaming actually run as priority-classed,
//! resource-gated background work. Each variant is the same engine driven by a
//! different [`CognitionTrigger`]; all run at [`Priority::P3Cognition`] so they
//! are suspended on battery or under memory pressure (§25). All produced
//! insights re-enter through the Write Policy as untrusted `self_reflection`
//! (L11) — these jobs never write memory directly.

use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::cognition::{Cognition, CognitionTrigger};
use crate::error::MemoryResult;
use crate::scheduler::{BackgroundJob, JobProfile, Priority};

/// How many recently-active sessions one background pass will consolidate.
/// Bounded so a single run stays cheap and yields to the user quickly (§25).
const DEFAULT_MAX_SESSIONS: usize = 8;

/// A scheduler job that runs a cognition pass under one trigger.
///
/// Construct via the trigger-specific constructors ([`ConsolidationJob::session_end`],
/// [`ConsolidationJob::daily_reflection`], [`ConsolidationJob::weekly_dreaming`],
/// [`ConsolidationJob::idle_micro`]) so single-flight keys stay distinct and a
/// slow weekly pass never blocks a session-end pass (N3).
pub struct ConsolidationJob {
    cognition: Arc<Cognition>,
    trigger: CognitionTrigger,
    max_sessions: usize,
    name: &'static str,
    single_flight_key: &'static str,
}

impl ConsolidationJob {
    fn new(
        cognition: Arc<Cognition>,
        trigger: CognitionTrigger,
        name: &'static str,
        single_flight_key: &'static str,
    ) -> Self {
        Self {
            cognition,
            trigger,
            max_sessions: DEFAULT_MAX_SESSIONS,
            name,
            single_flight_key,
        }
    }

    /// Session-end consolidation: compress the session just finished.
    pub fn session_end(cognition: Arc<Cognition>) -> Self {
        Self::new(
            cognition,
            CognitionTrigger::SessionEnd,
            "cognition.consolidate.session_end",
            "cognition.session_end",
        )
    }

    /// Idle micro-consolidation: tiny opportunistic pass while the user is away.
    pub fn idle_micro(cognition: Arc<Cognition>) -> Self {
        Self {
            max_sessions: 2,
            ..Self::new(
                cognition,
                CognitionTrigger::IdleMicro,
                "cognition.consolidate.idle_micro",
                "cognition.idle_micro",
            )
        }
    }

    /// Daily reflection: derive lessons/patterns across recent sessions.
    pub fn daily_reflection(cognition: Arc<Cognition>) -> Self {
        Self::new(
            cognition,
            CognitionTrigger::Daily,
            "cognition.reflect.daily",
            "cognition.daily",
        )
    }

    /// Weekly dreaming: broader generalization/abstraction pass.
    pub fn weekly_dreaming(cognition: Arc<Cognition>) -> Self {
        Self {
            max_sessions: 32,
            ..Self::new(
                cognition,
                CognitionTrigger::Weekly,
                "cognition.dream.weekly",
                "cognition.weekly",
            )
        }
    }

    /// Override the per-run session budget (default [`DEFAULT_MAX_SESSIONS`]).
    pub fn with_max_sessions(mut self, max_sessions: usize) -> Self {
        self.max_sessions = max_sessions.max(1);
        self
    }
}

#[async_trait]
impl BackgroundJob for ConsolidationJob {
    fn profile(&self) -> JobProfile {
        JobProfile {
            name: self.name,
            priority: Priority::P3Cognition,
            single_flight_key: self.single_flight_key,
        }
    }

    async fn run(&self, cancel: CancellationToken) -> MemoryResult<()> {
        let (sessions, accepted) = self
            .cognition
            .consolidate_recent(self.trigger, self.max_sessions, &cancel)
            .await?;
        if sessions > 0 {
            tracing::info!(
                job = self.name,
                sessions,
                accepted,
                "background cognition pass complete"
            );
        }
        Ok(())
    }
}

/// Background Active-Learning job: promote recurring knowledge gaps into
/// learning goals (Priority 3). Runs at `P4Maintenance` — lowest priority, so
/// it never competes with integrity/enrichment/cognition and is suspended on
/// battery / memory pressure (§25).
pub struct ActiveLearningJob {
    active_learning: Arc<crate::active_learning::ActiveLearning>,
    min_misses: u32,
    max_new: usize,
}

impl ActiveLearningJob {
    pub fn new(db: Arc<crate::db::Database>) -> Self {
        Self {
            active_learning: Arc::new(crate::active_learning::ActiveLearning::new(db)),
            min_misses: 3,
            max_new: 5,
        }
    }
}

#[async_trait]
impl BackgroundJob for ActiveLearningJob {
    fn profile(&self) -> JobProfile {
        JobProfile {
            name: "active_learning.promote_gaps",
            priority: Priority::P4Maintenance,
            single_flight_key: "active_learning",
        }
    }

    async fn run(&self, _cancel: CancellationToken) -> MemoryResult<()> {
        let created = self
            .active_learning
            .promote_gaps(self.min_misses, self.max_new)?;
        if !created.is_empty() {
            tracing::info!(
                count = created.len(),
                "active learning promoted knowledge gaps to goals"
            );
        }
        Ok(())
    }
}

/// Background Self-Improvement job: escalate chronically failing plans into
/// improvement goals (Priority 7). `P4Maintenance`, single-flight.
pub struct SelfImprovementJob {
    engine: Arc<crate::self_improvement::SelfImprovement>,
    max_new: usize,
}

impl SelfImprovementJob {
    pub fn new(db: Arc<crate::db::Database>) -> Self {
        Self {
            engine: Arc::new(crate::self_improvement::SelfImprovement::new(db)),
            max_new: 5,
        }
    }
}

#[async_trait]
impl BackgroundJob for SelfImprovementJob {
    fn profile(&self) -> JobProfile {
        JobProfile {
            name: "self_improvement.promote_weak_plans",
            priority: Priority::P4Maintenance,
            single_flight_key: "self_improvement",
        }
    }

    async fn run(&self, _cancel: CancellationToken) -> MemoryResult<()> {
        let created = self.engine.promote_weak_plans(self.max_new)?;
        if !created.is_empty() {
            tracing::info!(
                count = created.len(),
                "self-improvement escalated weak plans to goals"
            );
        }
        Ok(())
    }
}

/// Background entity-extraction job: populate the knowledge graph from memories
/// lacking mentions (Priority P2 enrichment). Runs frequently + cheaply so the
/// graph stays current with observations.
pub struct EntityExtractionJob {
    pipeline: Arc<crate::extraction::EntityExtractionPipeline>,
    batch: usize,
}

impl EntityExtractionJob {
    pub fn new(db: Arc<crate::db::Database>) -> Self {
        Self {
            pipeline: Arc::new(crate::extraction::EntityExtractionPipeline::new(db)),
            batch: 64,
        }
    }
}

#[async_trait]
impl BackgroundJob for EntityExtractionJob {
    fn profile(&self) -> JobProfile {
        JobProfile {
            name: "graph.entity_extraction",
            priority: Priority::P2Enrichment,
            single_flight_key: "entity_extraction",
        }
    }

    async fn run(&self, _cancel: CancellationToken) -> MemoryResult<()> {
        let (processed, linked) = self.pipeline.process_pending(self.batch)?;
        if processed > 0 {
            tracing::info!(
                memories = processed,
                entities = linked,
                "entity extraction populated graph"
            );
        }
        Ok(())
    }
}

/// Background Dream Intelligence job: synthesize procedures from repeated
/// successes + optimize the goal graph (Priority A). `P3Cognition`, single-
/// flight — suspended on battery / memory pressure.
pub struct DreamJob {
    engine: Arc<crate::dreaming::DreamEngine>,
    max_procedures: usize,
}

impl DreamJob {
    pub fn new(
        db: Arc<crate::db::Database>,
        write_policy: Arc<crate::write_policy::WritePolicy>,
    ) -> Self {
        Self {
            engine: Arc::new(crate::dreaming::DreamEngine::new(db, write_policy)),
            max_procedures: 8,
        }
    }
}

#[async_trait]
impl BackgroundJob for DreamJob {
    fn profile(&self) -> JobProfile {
        JobProfile {
            name: "dream.synthesis",
            priority: Priority::P3Cognition,
            single_flight_key: "dream",
        }
    }

    async fn run(&self, _cancel: CancellationToken) -> MemoryResult<()> {
        let (procs, merged, recal) = self.engine.run_all(self.max_procedures)?;
        if procs > 0 || merged > 0 || recal > 0 {
            tracing::info!(
                procedures = procs,
                goals_merged = merged,
                worth_recalibrated = recal,
                "dream synthesis pass complete"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cognition::CognitionTrigger;
    use crate::db::Database;
    use crate::modes::ModeManager;
    use crate::scheduler::{CognitiveScheduler, StaticResourceMonitor};
    use crate::stores::ports::{EventStore, RelationalStore};
    use crate::stores::{SqliteEventStore, SqliteRelationalStore};
    use crate::types::{
        Event, EventType, Memory, MemoryMode, MemoryType, MemoryWorth, Modality, Scope,
        Sensitivity, Source, StalenessClass,
    };
    use crate::write_policy::admission::Admission;
    use crate::write_policy::WritePolicy;
    use std::time::Duration;
    use uuid::Uuid;

    fn write_policy(db: &Arc<Database>) -> Arc<WritePolicy> {
        Arc::new(WritePolicy::new(
            db.clone(),
            Arc::new(SqliteEventStore::new(db.clone())),
            Arc::new(SqliteRelationalStore::new(db.clone())),
            Arc::new(ModeManager::new(MemoryMode::Permanent)),
            Arc::new(Admission::new(Duration::from_secs(0))),
            "dev",
            None,
        ))
    }

    fn seed(db: &Arc<Database>, session: Uuid, content: &str, hash: &str) {
        let events = SqliteEventStore::new(db.clone());
        let rel = SqliteRelationalStore::new(db.clone());
        let ev = Event {
            id: crate::ids::new_id(),
            hlc: crate::ids::HlcGenerator::new().now(),
            ts_utc: chrono::Utc::now(),
            tz_offset_min: 0,
            event_type: EventType::UserMessage,
            source: Source::User,
            session_id: Some(session),
            parent_event_id: None,
            shred_key_id: None,
            payload: serde_json::json!({}),
            encrypted: false,
            checksum: "c".into(),
        };
        let now = chrono::Utc::now();
        let m = Memory {
            id: crate::ids::new_id(),
            content: content.into(),
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
            state: crate::types::MemoryState::Active,
            created_at: now,
            last_accessed: None,
            valid_from: now,
            valid_until: None,
            embedding_id: None,
            embedding_model_version: None,
            estimated_tokens: 3,
            content_hash: hash.into(),
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
        let mut tx = db.begin().unwrap();
        events.append(&mut tx, &ev).unwrap();
        rel.upsert_memory(&mut tx, &m).unwrap();
        tx.commit().unwrap();
    }

    fn reflection_event_count(db: &Arc<Database>) -> i64 {
        db.with_read(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM events WHERE source='self_reflection'",
                [],
                |r| r.get(0),
            )
            .map_err(crate::error::StorageError::Sqlite)?)
        })
        .unwrap()
    }

    #[tokio::test]
    async fn scheduler_runs_cognition_job_and_produces_reflection() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let session = Uuid::now_v7();
        seed(&db, session, "worked on the memory scheduler", "h1");
        seed(&db, session, "wired cognition into background jobs", "h2");
        seed(&db, session, "added tests for the job adapter", "h3");

        let cognition = Arc::new(Cognition::new(db.clone(), write_policy(&db), None));

        let mut sched = CognitiveScheduler::new(Arc::new(StaticResourceMonitor {
            on_battery: false,
            memory_pressure: false,
            thermal_pressure: false,
            model_pressure: false,
        }));
        sched.register(Arc::new(ConsolidationJob::session_end(cognition)));

        assert_eq!(sched.run_ready().await, 1, "the P3 cognition job ran");
        assert_eq!(
            reflection_event_count(&db),
            1,
            "one untrusted reflection persisted via the write policy"
        );
    }

    #[tokio::test]
    async fn battery_suspends_background_cognition() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let session = Uuid::now_v7();
        seed(&db, session, "a", "h1");
        seed(&db, session, "b", "h2");
        seed(&db, session, "c", "h3");
        let cognition = Arc::new(Cognition::new(db.clone(), write_policy(&db), None));

        let mut sched = CognitiveScheduler::new(Arc::new(StaticResourceMonitor {
            on_battery: true,
            memory_pressure: false,
            thermal_pressure: false,
            model_pressure: false,
        }));
        sched.register(Arc::new(ConsolidationJob::daily_reflection(cognition)));

        // P3 is above the on-battery ceiling (P2) → job does not run.
        assert_eq!(sched.run_ready().await, 0);
        assert_eq!(reflection_event_count(&db), 0);
    }

    #[tokio::test]
    async fn distinct_triggers_have_distinct_single_flight_keys() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let cognition = Arc::new(Cognition::new(db.clone(), write_policy(&db), None));
        let jobs = [
            ConsolidationJob::session_end(cognition.clone()).profile(),
            ConsolidationJob::idle_micro(cognition.clone()).profile(),
            ConsolidationJob::daily_reflection(cognition.clone()).profile(),
            ConsolidationJob::weekly_dreaming(cognition).profile(),
        ];
        let mut keys: Vec<&str> = jobs.iter().map(|p| p.single_flight_key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(
            keys.len(),
            4,
            "each cognition trigger is single-flight-distinct"
        );
        assert!(jobs.iter().all(|p| p.priority == Priority::P3Cognition));
        let _ = CognitionTrigger::SessionEnd; // trigger enum in scope
    }
}
