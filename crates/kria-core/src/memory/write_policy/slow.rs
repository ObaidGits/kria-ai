//! Write Policy slow path — best-effort enrichment (memory-upgrade design §18.2).
//!
//! Consumes freshly-durable event ids from the fast path and derives the durable
//! `Memory`: quality filter → embed (degrade if unavailable, L8) → dedup →
//! classify → importance → commit derived memory + FTS in one authority txn →
//! upsert the vector (idempotent). The raw event is already durable, so any
//! failure here is retried/dead-lettered without data loss (design §18.2).

use std::sync::Arc;

use std::time::Duration;

use rusqlite::params;
use tokio::sync::mpsc::Receiver;
use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::governance;
use crate::memory::ids::{new_id, normalized_content_hash};
use crate::memory::stores::ports::{Embedder, EventStore, RelationalStore, VectorStore};
use crate::memory::stores::sqlite_search::index_fts_in_tx;
use crate::memory::types::{
    Availability, EmphasisSignals, Event, EventType, Memory, MemoryState, MemoryType, MemoryWorth,
    Modality, Scope, Sensitivity, VectorPayload, VerifyPredicate,
};

/// Durable consumer-cursor name for the enrichment slow path (R2 gauge key).
pub(crate) const CONSUMER: &str = "slow_path";

/// The async enrichment worker.
pub struct SlowPath {
    db: Arc<Database>,
    events: Arc<dyn EventStore>,
    relational: Arc<dyn RelationalStore>,
    vectors: Arc<dyn VectorStore>,
    embedder: Arc<dyn Embedder>,
    device_id: String,
}

/// Fields pulled out of a write-candidate event payload.
struct Candidate {
    content: String,
    namespace: String,
    scope: Scope,
    sensitivity: Sensitivity,
    proposed_type: Option<MemoryType>,
    emphasis: EmphasisSignals,
    verify_against: Option<VerifyPredicate>,
    redacted: bool,
}

impl SlowPath {
    pub fn new(
        db: Arc<Database>,
        events: Arc<dyn EventStore>,
        relational: Arc<dyn RelationalStore>,
        vectors: Arc<dyn VectorStore>,
        embedder: Arc<dyn Embedder>,
        device_id: impl Into<String>,
    ) -> Self {
        Self {
            db,
            events,
            relational,
            vectors,
            embedder,
            device_id: device_id.into(),
        }
    }

    /// Run the consume loop until the wake channel closes.
    ///
    /// Durability + backpressure (R1/R2):
    /// - On start, sweep the durable event log for anything left un-enriched by
    ///   a previous crash (crash-recovery — the wake channel is in-memory, but
    ///   the events + consumer cursor are durable).
    /// - Consume low-latency wakes from the bounded channel.
    /// - On a timer, sweep again to pick up events whose wake was dropped under
    ///   backpressure (a full bounded channel).
    ///
    /// `enrich` is idempotent (content-hash dedup + cursor advance), so replays
    /// never duplicate or lose a memory. Per-event errors are dead-lettered; the
    /// loop never dies on a single bad event.
    pub async fn run(
        &self,
        mut rx: Receiver<Uuid>,
        catchup_interval: Duration,
        enabled: Arc<std::sync::atomic::AtomicBool>,
    ) {
        // Crash-recovery sweep before serving live wakes, unless startup has
        // memory disabled. Durable events remain available for later catch-up.
        if enabled.load(std::sync::atomic::Ordering::Acquire) {
            if let Err(e) = self.enrich_pending(64).await {
                tracing::warn!(error = %e, "slow-path startup catch-up failed (will retry on timer)");
            }
        }
        let mut ticker = tokio::time::interval(catchup_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // First tick fires immediately; skip it (we just swept above).
        ticker.tick().await;
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(event_id) => {
                        if !enabled.load(std::sync::atomic::Ordering::Acquire) {
                            continue;
                        }
                        if let Err(e) = self.enrich(event_id).await {
                            tracing::warn!(%event_id, error = %e, "slow-path enrichment failed; dead-lettering");
                            self.dead_letter(event_id, &e.to_string());
                        }
                    }
                    None => break, // channel closed → shut down
                },
                _ = ticker.tick() => {
                    if !enabled.load(std::sync::atomic::Ordering::Acquire) {
                        continue;
                    }
                    // Recover wakes dropped under backpressure (R1). Best-effort.
                    if let Err(e) = self.enrich_pending(64).await {
                        tracing::debug!(error = %e, "slow-path catch-up sweep incomplete");
                    }
                }
            }
        }
    }

    /// Drain and enrich all events past the consumer cursor (used for graceful
    /// shutdown flush and deterministic tests). Returns the count processed.
    /// Idempotent: `enrich` advances the cursor per event.
    pub async fn enrich_pending(&self, batch: usize) -> MemoryResult<usize> {
        let mut total = 0usize;
        loop {
            let cursor = self.events.cursor(CONSUMER)?;
            let events = self.events.read_range(&cursor, batch)?;
            if events.is_empty() {
                break;
            }
            for ev in &events {
                self.enrich(ev.id).await?;
            }
            total += events.len();
            if events.len() < batch {
                break;
            }
        }
        Ok(total)
    }

    /// Enrich a single event into a derived memory (idempotent by content hash).
    pub async fn enrich(&self, event_id: Uuid) -> MemoryResult<()> {
        let Some(event) = self.events.get(event_id)? else {
            return Ok(());
        };
        // Only write-candidate events produce memories.
        if !matches!(
            event.event_type,
            EventType::UserMessage | EventType::Observation
        ) {
            self.advance_cursor(&event)?;
            return Ok(());
        }
        let Some(cand) = parse_candidate(&event) else {
            self.advance_cursor(&event)?;
            return Ok(());
        };
        // Quality filter (R4) — drop noise, but still advance the cursor.
        if governance::is_noise(&cand.content) {
            self.advance_cursor(&event)?;
            return Ok(());
        }

        let memory_type =
            governance::classify_type(&cand.content, cand.proposed_type.as_ref(), &event.source);
        let content_hash = normalized_content_hash(&cand.content);

        // Dedup: an existing active memory with the same content → reconsolidate.
        if let Some(existing) = self.relational.find_by_content_hash(
            &cand.namespace,
            memory_type.as_str(),
            &content_hash,
        )? {
            self.reconsolidate(existing.id, &event)?;
            return Ok(());
        }

        // Embed (skip for secret content — never embedded, §29/N8). Degrade if
        // the embedder is unavailable (store raw; FTS still works, L8).
        let embedding = if cand.sensitivity == Sensitivity::Secret {
            None
        } else {
            match self
                .embedder
                .embed(std::slice::from_ref(&cand.content))
                .await
            {
                Ok(mut v) if !v.is_empty() => Some(v.remove(0)),
                _ => None,
            }
        };

        let novelty = 1.0; // exact dup already excluded; vector-sim novelty is task 21
        let importance =
            governance::score_importance(novelty, &event.source, &cand.emphasis, false);
        let confidence = governance::score_confidence(&event.source, false);
        let staleness = governance::default_staleness(&memory_type, cand.verify_against.is_some());
        let model_version = self.embedder.model_version();
        let now = chrono::Utc::now();
        // Vector hits are resolved through `RelationalStore::get_memory`, so
        // vector ids MUST equal memory ids. A separate embedding-row id creates
        // dangling candidates that retrieval discards.
        let memory_id = new_id();

        let memory = Memory {
            id: memory_id,
            content: cand.content.clone(),
            memory_type: memory_type.clone(),
            compression_level: 0,
            source_event_id: event.id,
            namespace: cand.namespace.clone(),
            owner_id: "user".to_string(),
            device_id: self.device_id.clone(),
            scope: cand.scope.clone(),
            confidence,
            importance,
            access_count: 0,
            decay_score: 1.0,
            staleness_class: staleness,
            sensitivity: cand.sensitivity.clone(),
            state: MemoryState::Active,
            created_at: now,
            last_accessed: None,
            valid_from: now,
            valid_until: None,
            embedding_id: embedding.as_ref().map(|_| memory_id),
            embedding_model_version: embedding.as_ref().map(|_| model_version.clone()),
            estimated_tokens: governance::estimate_tokens(&cand.content),
            content_hash: content_hash.clone(),
            shred_key_id: None,
            verify_against: cand.verify_against.clone(),
            superseded_by: None,
            episode_id: None,
            goal_context_id: None,
            worth: MemoryWorth::default(),
            modality: Modality::Text,
            preference_pair_id: None,
            training_eligible: false,
        };

        // Commit derived memory + FTS + cursor in one authority transaction.
        {
            let mut tx = self.db.begin()?;
            self.relational.upsert_memory(&mut tx, &memory)?;
            if !cand.redacted {
                index_fts_in_tx(&mut tx, memory.id, &memory.content, &memory.namespace)?;
            }
            self.events.advance_cursor(&mut tx, CONSUMER, &event.hlc)?;
            tx.commit()?;
        }

        // Upsert the vector directly (idempotent by memory id). Reconciliation
        // repairs any gap; retrieval degrades to keyword floor if skipped (L8).
        if let Some(vec) = embedding {
            let payload = VectorPayload {
                namespace: memory.namespace.clone(),
                scope: memory.scope.clone(),
                sensitivity: memory.sensitivity.clone(),
                memory_type: memory.memory_type.clone(),
                content_hash: memory.content_hash.clone(),
                created_at: memory.created_at,
            };
            self.vectors
                .upsert(&model_version, memory.id, &vec, &payload)
                .await?;
        }
        Ok(())
    }

    /// Reconsolidate a duplicate: bump access + advance the cursor.
    fn reconsolidate(&self, memory_id: Uuid, event: &Event) -> MemoryResult<()> {
        let mut tx = self.db.begin()?;
        tx.conn()
            .execute(
                "UPDATE memories SET access_count = access_count + 1, last_accessed = ?2 \
                 WHERE id = ?1",
                params![memory_id.to_string(), chrono::Utc::now().to_rfc3339()],
            )
            .map_err(StorageError::Sqlite)?;
        self.events.advance_cursor(&mut tx, CONSUMER, &event.hlc)?;
        tx.commit()
    }

    fn advance_cursor(&self, event: &Event) -> MemoryResult<()> {
        let mut tx = self.db.begin()?;
        self.events.advance_cursor(&mut tx, CONSUMER, &event.hlc)?;
        tx.commit()
    }

    fn dead_letter(&self, event_id: Uuid, err: &str) {
        if let Ok(tx) = self.db.begin() {
            let _ = tx.conn().execute(
                "INSERT INTO enrichment_deadletter(event_id, stage, error, attempts, ts) \
                 VALUES(?1,'enrich',?2,1,?3) ON CONFLICT(event_id) DO UPDATE SET \
                 attempts = attempts + 1, error = excluded.error, ts = excluded.ts",
                params![event_id.to_string(), err, chrono::Utc::now().to_rfc3339()],
            );
            let _ = tx.commit();
        }
    }

    /// Embedder availability for the degradation ladder.
    pub async fn embedder_health(&self) -> Availability {
        self.embedder.health().await
    }
}

fn parse_candidate(event: &Event) -> Option<Candidate> {
    let p = &event.payload;
    let content = p.get("content")?.as_str()?.to_string();
    let namespace = p
        .get("namespace")
        .and_then(|v| v.as_str())
        .unwrap_or("core")
        .to_string();
    let scope = p
        .get("scope")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(Scope::Global);
    let sensitivity = p
        .get("sensitivity")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(Sensitivity::Private);
    let proposed_type = p
        .get("proposed_type")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok());
    let emphasis = p
        .get("emphasis")
        .and_then(|v| serde_json::from_value::<EmphasisSignals>(v.clone()).ok())
        .unwrap_or_default();
    let verify_against = p
        .get("verify_against")
        .and_then(|v| serde_json::from_value::<Option<VerifyPredicate>>(v.clone()).ok())
        .flatten();
    let redacted = p.get("redacted").and_then(|v| v.as_bool()).unwrap_or(false);
    // Source guardrail (SI-1): untrusted content must never become a rule/procedure.
    let _ = &event.source;
    Some(Candidate {
        content,
        namespace,
        scope,
        sensitivity,
        proposed_type,
        emphasis,
        verify_against,
        redacted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::stores::{SqliteEventStore, SqliteRelationalStore, SqliteVectorStore};
    use crate::memory::types::{ModelVersion, Source};
    use async_trait::async_trait;

    /// A deterministic fake embedder so the vector path is testable without an
    /// ONNX model on disk.
    struct FakeEmbedder {
        dim: usize,
    }
    #[async_trait]
    impl Embedder for FakeEmbedder {
        fn model_version(&self) -> ModelVersion {
            ModelVersion("fake_v1".into())
        }
        fn dim(&self) -> usize {
            self.dim
        }
        async fn embed(&self, texts: &[String]) -> MemoryResult<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0f32; self.dim];
                    for (i, b) in t.bytes().enumerate() {
                        v[i % self.dim] += b as f32 / 255.0;
                    }
                    v
                })
                .collect())
        }
        async fn health(&self) -> Availability {
            Availability::Up
        }
    }

    fn build(db: Arc<Database>) -> (SlowPath, Arc<SqliteEventStore>) {
        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let relational = Arc::new(SqliteRelationalStore::new(db.clone()));
        let vectors = Arc::new(SqliteVectorStore::new(db.clone()));
        let embedder = Arc::new(FakeEmbedder { dim: 8 });
        let sp = SlowPath::new(
            db,
            events.clone(),
            relational,
            vectors,
            embedder,
            "test-dev",
        );
        (sp, events)
    }

    fn candidate_event(session: Uuid, content: &str) -> Event {
        Event {
            id: new_id(),
            hlc: crate::memory::ids::HlcGenerator::new().now(),
            ts_utc: chrono::Utc::now(),
            tz_offset_min: 0,
            event_type: EventType::UserMessage,
            source: Source::User,
            session_id: Some(session),
            parent_event_id: None,
            shred_key_id: None,
            payload: serde_json::json!({
                "content": content, "namespace": "core", "scope": "global",
                "sensitivity": "private", "redacted": false, "emphasis": EmphasisSignals::default(),
                "derived_from": [], "proposed_type": null, "verify_against": null
            }),
            encrypted: false,
            checksum: "c".into(),
        }
    }

    #[tokio::test]
    async fn enrich_creates_memory_fts_and_vector() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let (sp, events) = build(db.clone());
        let ev = candidate_event(Uuid::now_v7(), "the user prefers dark mode");
        {
            let mut tx = db.begin().unwrap();
            events.append(&mut tx, &ev).unwrap();
            tx.commit().unwrap();
        }
        sp.enrich(ev.id).await.unwrap();

        let mem_count: i64 = db
            .with_read(|c| {
                Ok(
                    c.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
                        .map_err(StorageError::Sqlite)?,
                )
            })
            .unwrap();
        assert_eq!(mem_count, 1);
        let vec_count: i64 = db
            .with_read(|c| {
                Ok(
                    c.query_row("SELECT COUNT(*) FROM mem_vectors", [], |r| r.get(0))
                        .map_err(StorageError::Sqlite)?,
                )
            })
            .unwrap();
        assert_eq!(vec_count, 1, "vector should be upserted");
        let fts_count: i64 = db
            .with_read(|c| {
                Ok(
                    c.query_row("SELECT COUNT(*) FROM memories_fts", [], |r| r.get(0))
                        .map_err(StorageError::Sqlite)?,
                )
            })
            .unwrap();
        assert_eq!(fts_count, 1, "FTS should be indexed");
    }

    #[tokio::test]
    async fn duplicate_content_reconsolidates_not_duplicates() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let (sp, events) = build(db.clone());
        let s = Uuid::now_v7();
        let e1 = candidate_event(s, "kria runs locally");
        let e2 = candidate_event(s, "kria runs locally");
        {
            let mut tx = db.begin().unwrap();
            events.append(&mut tx, &e1).unwrap();
            events.append(&mut tx, &e2).unwrap();
            tx.commit().unwrap();
        }
        sp.enrich(e1.id).await.unwrap();
        sp.enrich(e2.id).await.unwrap();

        let mem_count: i64 = db
            .with_read(|c| {
                Ok(
                    c.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
                        .map_err(StorageError::Sqlite)?,
                )
            })
            .unwrap();
        assert_eq!(
            mem_count, 1,
            "duplicate must reconsolidate, not create a 2nd row"
        );
        let access: i64 = db
            .with_read(|c| {
                Ok(
                    c.query_row("SELECT access_count FROM memories", [], |r| r.get(0))
                        .map_err(StorageError::Sqlite)?,
                )
            })
            .unwrap();
        assert_eq!(access, 1, "reconsolidation bumps access_count");
    }
}
