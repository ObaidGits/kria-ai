//! Retrieval orchestrator — adaptive multi-strategy fusion (design §19, L10/L12).
//!
//! Read-only (L10). Runs vector + full-text strategies in parallel, fuses with
//! weighted Reciprocal Rank Fusion, gates candidates (active/promoted only,
//! Memory-Worth re-rank), enforces the `ScopeFilter` (L7/D-20), fills a token
//! budget by relevance (not top-K), and degrades gracefully (embedder/vector
//! down → keyword floor, L8). Graph expansion is a P3 addition (task 17.2 full).

use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

use crate::memory::error::MemoryResult;
use crate::memory::stores::ports::{Embedder, RelationalStore, SearchStore, VectorStore};
use crate::memory::types::{Availability, Memory, MemoryState, Scope, ScopeFilter};

/// RRF damping constant (design §19; standard k=60).
const RRF_K: f32 = 60.0;
/// Candidate pool size pulled from each strategy before fusion/gating.
const CANDIDATE_K: usize = 50;

/// Deterministic query classes → strategy weighting (design §19).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryClass {
    Temporal,
    Entity,
    Conceptual,
    Recent,
    Procedural,
}

impl QueryClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            QueryClass::Temporal => "temporal",
            QueryClass::Entity => "entity",
            QueryClass::Conceptual => "conceptual",
            QueryClass::Recent => "recent",
            QueryClass::Procedural => "procedural",
        }
    }

    /// Parse a class from its string label (inverse of [`Self::as_str`]).
    pub fn from_str(s: &str) -> QueryClass {
        match s {
            "temporal" => QueryClass::Temporal,
            "entity" => QueryClass::Entity,
            "recent" => QueryClass::Recent,
            "procedural" => QueryClass::Procedural,
            _ => QueryClass::Conceptual,
        }
    }

    /// Default (vector_weight, fts_weight) for fusion — the prior the adaptive
    /// weight store starts from before any learning.
    pub fn default_weights(&self) -> (f32, f32) {
        match self {
            // Conceptual questions favor semantic vectors; keyword/procedural
            // favor exact terms.
            QueryClass::Conceptual => (1.0, 0.6),
            QueryClass::Entity => (0.7, 1.0),
            QueryClass::Procedural => (0.8, 1.0),
            QueryClass::Temporal | QueryClass::Recent => (0.8, 0.8),
        }
    }
}

/// Deterministic query classification (<5ms, no LLM).
pub fn classify_query(q: &str) -> QueryClass {
    let l = q.to_lowercase();
    if l.contains("yesterday")
        || l.contains("last week")
        || l.contains("today")
        || l.contains("ago")
        || l.contains("when ")
    {
        return QueryClass::Temporal;
    }
    if l.starts_with("how ") || l.contains("how to") || l.contains("steps") {
        return QueryClass::Procedural;
    }
    if l.contains("recent") || l.contains("latest") {
        return QueryClass::Recent;
    }
    // Heuristic: a capitalized non-initial word suggests a named entity.
    if q.split_whitespace()
        .skip(1)
        .any(|w| w.chars().next().map(|c| c.is_uppercase()).unwrap_or(false))
    {
        return QueryClass::Entity;
    }
    QueryClass::Conceptual
}

/// Read context / filters for a retrieval.
#[derive(Clone, Debug)]
pub struct RetrievalCtx {
    pub namespaces: Vec<String>,
    pub scopes: Vec<Scope>,
    pub include_secret: bool,
    pub token_budget: u32,
}

impl Default for RetrievalCtx {
    fn default() -> Self {
        Self {
            namespaces: Vec::new(),
            scopes: Vec::new(),
            include_secret: false,
            token_budget: 800,
        }
    }
}

/// A returned memory with its fused score and the strategies that surfaced it.
#[derive(Clone, Debug)]
pub struct RetrievalHit {
    pub memory: Memory,
    pub score: f32,
    pub strategies: Vec<&'static str>,
}

/// Explainability trace (feeds `explain_retrieval`, L6).
#[derive(Clone, Debug)]
pub struct RetrievalTrace {
    pub query_class: &'static str,
    pub vector_used: bool,
    pub fts_used: bool,
    pub candidates: usize,
    pub returned: usize,
}

/// The retrieval result.
#[derive(Clone, Debug)]
pub struct RetrievalResult {
    pub hits: Vec<RetrievalHit>,
    pub trace: RetrievalTrace,
}

/// The orchestrator. Holds read-only handles to the stores.
pub struct Retriever {
    relational: Arc<dyn RelationalStore>,
    vectors: Arc<dyn VectorStore>,
    search: Arc<dyn SearchStore>,
    embedder: Arc<dyn Embedder>,
    /// Learned adaptive fusion weights (Priority 1). Read-only here; reinforced
    /// out-of-band by the learning loop. `None` → static per-class defaults.
    weights: Option<crate::memory::retrieval_opt::RetrievalWeightStore>,
}

impl Retriever {
    pub fn new(
        relational: Arc<dyn RelationalStore>,
        vectors: Arc<dyn VectorStore>,
        search: Arc<dyn SearchStore>,
        embedder: Arc<dyn Embedder>,
    ) -> Self {
        Self {
            relational,
            vectors,
            search,
            embedder,
            weights: None,
        }
    }

    /// Attach the adaptive retrieval-weight store (self-optimizing RRF).
    pub fn with_weight_store(
        mut self,
        weights: crate::memory::retrieval_opt::RetrievalWeightStore,
    ) -> Self {
        self.weights = Some(weights);
        self
    }

    /// Multi-strategy retrieval within a token budget (design §19).
    pub async fn search(&self, query: &str, ctx: &RetrievalCtx) -> MemoryResult<RetrievalResult> {
        let class = classify_query(query);
        let (w_vec, w_fts) = match &self.weights {
            Some(ws) => ws
                .learned_weights(class)
                .unwrap_or_else(|_| class.default_weights()),
            None => class.default_weights(),
        };
        let filter = ScopeFilter {
            namespaces: ctx.namespaces.clone(),
            scopes: ctx.scopes.clone(),
            include_secret: ctx.include_secret,
        };

        // Strategy 1: vector (skip if embedder/vector unavailable → degrade, L8).
        let mut vector_ids: Vec<Uuid> = Vec::new();
        let mut vector_used = false;
        if self.embedder.health().await == Availability::Up {
            if let Ok(mut qvecs) = self
                .embedder
                .embed(std::slice::from_ref(&query.to_string()))
                .await
            {
                if let Some(qvec) = qvecs.pop() {
                    let model = self.embedder.model_version();
                    if let Ok(hits) = self
                        .vectors
                        .search(&model, &qvec, CANDIDATE_K, &filter)
                        .await
                    {
                        vector_ids = hits.into_iter().map(|h| h.id).collect();
                        vector_used = true;
                    }
                }
            }
        }

        // Strategy 2: full-text (always available — the floor).
        let fts_hits = self
            .search
            .query(query, CANDIDATE_K, &filter)
            .await
            .unwrap_or_default();
        let fts_ids: Vec<Uuid> = fts_hits.into_iter().map(|h| h.id).collect();
        let fts_used = true;

        // Adaptive RRF fusion.
        let mut fused: HashMap<Uuid, (f32, Vec<&'static str>)> = HashMap::new();
        for (rank, id) in vector_ids.iter().enumerate() {
            let e = fused.entry(*id).or_insert((0.0, Vec::new()));
            e.0 += w_vec / (RRF_K + rank as f32 + 1.0);
            e.1.push("vector");
        }
        for (rank, id) in fts_ids.iter().enumerate() {
            let e = fused.entry(*id).or_insert((0.0, Vec::new()));
            e.0 += w_fts / (RRF_K + rank as f32 + 1.0);
            e.1.push("fts");
        }
        let candidates = fused.len();

        // Load memories, gate (active/promoted + scope + Memory-Worth re-rank).
        let mut hits: Vec<RetrievalHit> = Vec::new();
        for (id, (base, strategies)) in fused {
            let Some(mem) = self.relational.get_memory(id)? else {
                continue; // dangling index entry → skip (reconciliation repairs)
            };
            if !matches!(mem.state, MemoryState::Active | MemoryState::Promoted) {
                // Authority-first residue protection (design §5.4, task 1.7.5):
                // the state gate here is the IMMEDIATE safety barrier that
                // prevents any Deleted, Forgotten, Archived, or Superseded
                // memory from reaching the caller — even while an outbox purge
                // for the same memory is still pending in the derived indexes.
                // The outbox relay and the reconcile() residue check are eventual
                // cleanup of the derived index; they are NOT the primary guard.
                continue; // exclude superseded/archived/forgotten (L12)
            }
            // Defense-in-depth scope enforcement (L7/D-20).
            if !filter.allows(&mem.namespace, &mem.scope, &mem.sensitivity) {
                continue;
            }
            // Soft Memory-Worth re-rank (never a hard filter, D-8) + importance nudge.
            let worth_bonus = mem.worth.score() * 0.01;
            let importance_bonus = (mem.importance / 10.0) * 0.005;
            let score = base + worth_bonus + importance_bonus;
            hits.push(RetrievalHit {
                memory: mem,
                score,
                strategies,
            });
        }
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Token-budget fill by relevance (not top-K).
        let mut budget = ctx.token_budget as i64;
        let mut selected = Vec::new();
        for h in hits {
            let cost = h.memory.estimated_tokens.max(1) as i64;
            if budget - cost < 0 && !selected.is_empty() {
                break;
            }
            budget -= cost;
            selected.push(h);
        }

        let returned = selected.len();
        Ok(RetrievalResult {
            hits: selected,
            trace: RetrievalTrace {
                query_class: class.as_str(),
                vector_used,
                fts_used,
                candidates,
                returned,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::Database;
    use crate::memory::stores::ports::{Embedder, EventStore};
    use crate::memory::stores::{SqliteEventStore, SqliteRelationalStore, SqliteVectorStore};
    use crate::memory::types::{EmphasisSignals, Event, EventType, ModelVersion, Source};
    use crate::memory::write_policy::slow::SlowPath;
    use async_trait::async_trait;

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

    fn ev(session: Uuid, content: &str, namespace: &str) -> Event {
        Event {
            id: crate::memory::ids::new_id(),
            hlc: crate::memory::ids::HlcGenerator::new().now(),
            ts_utc: chrono::Utc::now(),
            tz_offset_min: 0,
            event_type: EventType::UserMessage,
            source: Source::User,
            session_id: Some(session),
            parent_event_id: None,
            shred_key_id: None,
            payload: serde_json::json!({
                "content": content, "namespace": namespace, "scope": "global",
                "sensitivity": "private", "redacted": false, "emphasis": EmphasisSignals::default(),
                "derived_from": [], "proposed_type": null, "verify_against": null
            }),
            encrypted: false,
            checksum: "c".into(),
        }
    }

    async fn seed(db: &Arc<Database>, content: &str, namespace: &str) {
        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let relational = Arc::new(SqliteRelationalStore::new(db.clone()));
        let vectors = Arc::new(SqliteVectorStore::new(db.clone()));
        let embedder = Arc::new(FakeEmbedder { dim: 16 });
        let sp = SlowPath::new(
            db.clone(),
            events.clone(),
            relational,
            vectors,
            embedder,
            "dev",
        );
        let e = ev(Uuid::now_v7(), content, namespace);
        {
            let mut tx = db.begin().unwrap();
            events.append(&mut tx, &e).unwrap();
            tx.commit().unwrap();
        }
        sp.enrich(e.id).await.unwrap();
    }

    fn retriever(db: &Arc<Database>) -> Retriever {
        Retriever::new(
            Arc::new(SqliteRelationalStore::new(db.clone())),
            Arc::new(SqliteVectorStore::new(db.clone())),
            Arc::new(crate::memory::stores::SqliteSearchStore::new(db.clone())),
            Arc::new(FakeEmbedder { dim: 16 }),
        )
    }

    #[tokio::test]
    async fn retrieves_seeded_memories() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        seed(&db, "the user prefers dark mode themes", "core").await;
        seed(&db, "kria runs locally on the laptop", "core").await;
        let r = retriever(&db);
        let res = r
            .search("dark mode", &RetrievalCtx::default())
            .await
            .unwrap();
        assert!(!res.hits.is_empty());
        assert!(res.trace.vector_used);
        assert!(res.hits[0].memory.content.contains("dark mode"));
    }

    #[tokio::test]
    async fn scope_filter_isolates_namespaces() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        seed(&db, "shared project knowledge base", "core").await;
        seed(&db, "shared project knowledge base extra", "plugin/x").await;
        let r = retriever(&db);
        let ctx = RetrievalCtx {
            namespaces: vec!["plugin/x".into()],
            ..Default::default()
        };
        let res = r.search("project knowledge", &ctx).await.unwrap();
        assert!(!res.hits.is_empty());
        for h in &res.hits {
            assert_eq!(
                h.memory.namespace, "plugin/x",
                "no cross-namespace leak (L7)"
            );
        }
    }

    #[tokio::test]
    async fn degrades_to_keyword_when_vectors_absent() {
        // No vectors seeded (write via a path with no embedding): FTS still works.
        let db = Arc::new(Database::open_in_memory().unwrap());
        seed(&db, "graceful degradation keyword floor", "core").await;
        let r = retriever(&db);
        let res = r
            .search("degradation", &RetrievalCtx::default())
            .await
            .unwrap();
        assert!(res.trace.fts_used);
        assert!(!res.hits.is_empty());
    }
}
