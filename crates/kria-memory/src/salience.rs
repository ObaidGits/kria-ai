//! Proactive / salience retrieval (memory-upgrade design §19, Issue 7/N7, task 29).
//!
//! Before the user asks, a salience loop compares the current context (file
//! open, app focus, new message) against memory and surfaces strong matches into
//! the next turn. Event-driven + **debounced** (≥60s) + **coalesced** + disabled
//! on battery (power-aware), and it re-embeds only when the context text changes
//! (N7). This module owns the debounce/gating; the actual match reuses the
//! `Retriever`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use tokio::sync::Mutex;

use crate::error::MemoryResult;
use crate::retriever::{RetrievalCtx, RetrievalHit, Retriever};

/// Minimum interval between salience passes (N7).
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_secs(60);
/// Minimum fused score for a memory to be proactively surfaced.
pub const SURFACE_THRESHOLD: f32 = 0.02;

/// Power state gate for the salience loop.
pub trait PowerState: Send + Sync {
    fn on_battery(&self) -> bool;
}

/// Always-AC power state (dev default; real detection wired via config).
pub struct AcPower;
impl PowerState for AcPower {
    fn on_battery(&self) -> bool {
        false
    }
}

/// A fixed power state for tests.
pub struct StaticPower(pub bool);
impl PowerState for StaticPower {
    fn on_battery(&self) -> bool {
        self.0
    }
}

/// The salience engine.
pub struct Salience {
    retriever: Arc<Retriever>,
    power: Arc<dyn PowerState>,
    debounce: Duration,
    last_run: Mutex<Option<Instant>>,
    /// Last context text we acted on — re-embed/re-run only when it changes (N7).
    last_context: ArcSwapOption<String>,
}

impl Salience {
    pub fn new(retriever: Arc<Retriever>, power: Arc<dyn PowerState>) -> Self {
        Self {
            retriever,
            power,
            debounce: DEFAULT_DEBOUNCE,
            last_run: Mutex::new(None),
            last_context: ArcSwapOption::from(None),
        }
    }

    pub fn with_debounce(mut self, d: Duration) -> Self {
        self.debounce = d;
        self
    }

    /// Consider surfacing memories for the current `context`. Returns `None`
    /// when suppressed (battery / debounced / unchanged context); otherwise the
    /// above-threshold hits for the next turn.
    pub async fn maybe_surface(
        &self,
        context: &str,
        ctx: &RetrievalCtx,
    ) -> MemoryResult<Option<Vec<RetrievalHit>>> {
        // Power-aware: never run proactive work on battery (§25/N7).
        if self.power.on_battery() {
            return Ok(None);
        }
        // Coalesce: skip if the context text is unchanged since last run (N7).
        if let Some(prev) = self.last_context.load_full() {
            if prev.as_str() == context {
                return Ok(None);
            }
        }
        // Debounce.
        {
            let mut last = self.last_run.lock().await;
            let now = Instant::now();
            if let Some(t) = *last {
                if now.duration_since(t) < self.debounce {
                    return Ok(None);
                }
            }
            *last = Some(now);
        }
        self.last_context.store(Some(Arc::new(context.to_string())));

        let result = self.retriever.search(context, ctx).await?;
        let surfaced: Vec<RetrievalHit> = result
            .hits
            .into_iter()
            .filter(|h| h.score >= SURFACE_THRESHOLD)
            .collect();
        Ok(Some(surfaced))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::stores::ports::Embedder;
    use crate::stores::{SqliteRelationalStore, SqliteSearchStore, SqliteVectorStore};
    use crate::types::{Availability, ModelVersion};
    use async_trait::async_trait;

    struct FakeEmbedder;
    #[async_trait]
    impl Embedder for FakeEmbedder {
        fn model_version(&self) -> ModelVersion {
            ModelVersion("fake_v1".into())
        }
        fn dim(&self) -> usize {
            8
        }
        async fn embed(&self, texts: &[String]) -> MemoryResult<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|_| vec![0.1f32; 8]).collect())
        }
        async fn health(&self) -> Availability {
            Availability::Up
        }
    }

    fn retriever(db: &Arc<Database>) -> Arc<Retriever> {
        Arc::new(Retriever::new(
            Arc::new(SqliteRelationalStore::new(db.clone())),
            Arc::new(SqliteVectorStore::new(db.clone())),
            Arc::new(SqliteSearchStore::new(db.clone())),
            Arc::new(FakeEmbedder),
        ))
    }

    #[tokio::test]
    async fn battery_suppresses_salience() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let sal = Salience::new(retriever(&db), Arc::new(StaticPower(true)));
        assert!(sal
            .maybe_surface("some context", &RetrievalCtx::default())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn debounce_and_coalesce() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let sal = Salience::new(retriever(&db), Arc::new(AcPower))
            .with_debounce(Duration::from_secs(3600));
        // First run acts (returns Some, even if empty hits).
        assert!(sal
            .maybe_surface("context A", &RetrievalCtx::default())
            .await
            .unwrap()
            .is_some());
        // Same context → coalesced (None).
        assert!(sal
            .maybe_surface("context A", &RetrievalCtx::default())
            .await
            .unwrap()
            .is_none());
        // Different context but within debounce → suppressed (None).
        assert!(sal
            .maybe_surface("context B", &RetrievalCtx::default())
            .await
            .unwrap()
            .is_none());
    }
}
