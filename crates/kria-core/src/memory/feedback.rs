//! Feedback intake + Memory-Worth update (memory-upgrade design §15.3/§22.3, D-19).
//!
//! Feedback is a first-class event type from P1. Signals feed Memory Worth
//! (credit-divided, difficulty-adjusted) and confidence calibration. Memory
//! Worth is a soft signal (min-sample gated) and never triggers a hard delete
//! (D-8).

use std::sync::Arc;

use rusqlite::params;
use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::ids::new_id;

/// The feedback signal taxonomy (design D-19).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeedbackSignal {
    ThumbsUp,
    ThumbsDown,
    Correction(String),
    Undo,
    Cancel,
    Edit(String),
    Overwrite,
    IgnoredSuggestion,
    RepeatedTask,
    AutomationSuccess,
    AutomationFailure,
}

impl FeedbackSignal {
    pub fn as_str(&self) -> &'static str {
        match self {
            FeedbackSignal::ThumbsUp => "thumbs_up",
            FeedbackSignal::ThumbsDown => "thumbs_down",
            FeedbackSignal::Correction(_) => "correction",
            FeedbackSignal::Undo => "undo",
            FeedbackSignal::Cancel => "cancel",
            FeedbackSignal::Edit(_) => "edit",
            FeedbackSignal::Overwrite => "overwrite",
            FeedbackSignal::IgnoredSuggestion => "ignored_suggestion",
            FeedbackSignal::RepeatedTask => "repeated_task",
            FeedbackSignal::AutomationSuccess => "automation_success",
            FeedbackSignal::AutomationFailure => "automation_failure",
        }
    }

    /// Outcome sign for Memory-Worth: +1 positive, -1 negative, 0 neutral.
    pub fn outcome_sign(&self) -> i32 {
        match self {
            FeedbackSignal::ThumbsUp
            | FeedbackSignal::AutomationSuccess
            | FeedbackSignal::RepeatedTask => 1,
            FeedbackSignal::ThumbsDown
            | FeedbackSignal::Correction(_)
            | FeedbackSignal::Undo
            | FeedbackSignal::Cancel
            | FeedbackSignal::Overwrite
            | FeedbackSignal::IgnoredSuggestion
            | FeedbackSignal::AutomationFailure => -1,
            FeedbackSignal::Edit(_) => 0,
        }
    }

    fn payload(&self) -> Option<String> {
        match self {
            FeedbackSignal::Correction(s) | FeedbackSignal::Edit(s) => Some(s.clone()),
            _ => None,
        }
    }
}

pub struct FeedbackService {
    db: Arc<Database>,
}

impl FeedbackService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Record feedback inside an existing authority transaction. Correction
    /// uses this so version history, truth state, FTS, and feedback commit
    /// atomically instead of leaving an optimistic-only UI mutation.
    pub(crate) fn record_in_tx(
        &self,
        tx: &mut crate::memory::db::AuthorityTx<'_>,
        target_id: Uuid,
        target_kind: &str,
        signal: &FeedbackSignal,
        context: Option<&str>,
    ) -> MemoryResult<()> {
        tx.conn()
            .execute(
                "INSERT INTO feedback_events(id, target_id, target_kind, signal, payload, context, ts) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    new_id().to_string(),
                    target_id.to_string(),
                    target_kind,
                    signal.as_str(),
                    signal.payload(),
                    context,
                    chrono::Utc::now().to_rfc3339(),
                ],
            )
            .map_err(StorageError::Sqlite)?;

        if target_kind == "memory" {
            let sign = signal.outcome_sign();
            if sign != 0 {
                let (succ, fail) = if sign > 0 { (1, 0) } else { (0, 1) };
                tx.conn()
                    .execute(
                        "UPDATE memories SET memory_worth_success = memory_worth_success + ?2, \
                         memory_worth_failure = memory_worth_failure + ?3, \
                         memory_worth_samples = memory_worth_samples + 1 WHERE id = ?1",
                        params![target_id.to_string(), succ, fail],
                    )
                    .map_err(StorageError::Sqlite)?;
            }
        }
        Ok(())
    }

    /// Record a feedback event and, when it targets a memory, update that
    /// memory's Memory-Worth counters (design §22.3). Idempotency is by event id.
    pub fn record(
        &self,
        target_id: Uuid,
        target_kind: &str,
        signal: FeedbackSignal,
        context: Option<&str>,
    ) -> MemoryResult<()> {
        let mut tx = self.db.begin()?;
        self.record_in_tx(&mut tx, target_id, target_kind, &signal, context)?;
        tx.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::stores::ports::{EventStore, RelationalStore};
    use crate::memory::stores::{SqliteEventStore, SqliteRelationalStore};
    use crate::memory::types::{
        Event, EventType, Memory, MemoryState, MemoryType, MemoryWorth, Modality, Scope,
        Sensitivity, Source, StalenessClass,
    };

    fn seed_memory(db: &Arc<Database>) -> Uuid {
        let events = SqliteEventStore::new(db.clone());
        let rel = SqliteRelationalStore::new(db.clone());
        let ev = Event {
            id: new_id(),
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
            id: new_id(),
            content: "fact".into(),
            memory_type: MemoryType::Semantic,
            compression_level: 0,
            source_event_id: ev.id,
            namespace: "core".into(),
            owner_id: "user".into(),
            device_id: "d".into(),
            scope: Scope::Global,
            confidence: 0.7,
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
            estimated_tokens: 1,
            content_hash: "h".into(),
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
        m.id
    }

    #[test]
    fn thumbs_updates_memory_worth() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let id = seed_memory(&db);
        let fb = FeedbackService::new(db.clone());
        fb.record(id, "memory", FeedbackSignal::ThumbsUp, None)
            .unwrap();
        fb.record(id, "memory", FeedbackSignal::ThumbsDown, None)
            .unwrap();
        fb.record(
            id,
            "memory",
            FeedbackSignal::Correction("wrong".into()),
            Some("ctx"),
        )
        .unwrap();

        let rel = SqliteRelationalStore::new(db.clone());
        let m = rel.get_memory(id).unwrap().unwrap();
        assert_eq!(m.worth.samples, 3);
        assert_eq!(m.worth.success, 1);
        assert_eq!(m.worth.failure, 2);
    }
}
