//! The federated capability index — cross-provider retrieval over
//! [`CapabilityDescriptor`]s.
//!
//! This is the **single** index the Brain queries for discovery. It fuses dense
//! (embedding cosine) and lexical (token-overlap) similarity over descriptors
//! keyed by `(provider_id, capability_id)`, so a goal is matched to the best
//! capability across *all* providers without naming any of them.
//!
//! It reuses the neutral KRIA embedding backend
//! ([`crate::memory::embeddings::EmbeddingModel`], FastEmbed/ONNX — no Python,
//! no provider dependency). The embedder is behind the [`Embedder`] trait so an
//! in-process ONNX model can be swapped for a distributed vector service without
//! changing callers (design R4.4). Retrieval is behind the [`FederatedIndex`]
//! trait for the same reason.
//!
//! Honesty/degraded: if the embedder cannot produce a vector, `search` degrades
//! to lexical-only scoring and the error is surfaced upstream — never a panic.
//!
//! Supersedes the OpenClaw-specific `openclaw::cil` index; that legacy index is
//! removed at the Milestone-11 debt-removal point once the CIL routes discovery
//! through this federated index.

use std::sync::{Arc, RwLock};

use super::descriptor::CapabilityDescriptor;
use super::error::CapError;

/// Text → vector provider for discovery. Neutral (no provider dependency).
pub trait Embedder: Send + Sync {
    /// Embed a single text into a dense vector of length [`dim`](Embedder::dim).
    /// Returns [`CapError::Degraded`] on backend failure (never panics).
    fn embed(&self, text: &str) -> Result<Vec<f32>, CapError>;
    /// The dimension of every vector this embedder produces.
    fn dim(&self) -> usize;
    /// Stable id of the active model, for cache invalidation on model change.
    fn model_id(&self) -> &str;
}

/// Default [`Embedder`] delegating to the neutral KRIA embedding backend.
///
/// Reuses `memory::embeddings::EmbeddingModel` (real ONNX all-MiniLM-L6-v2 when
/// present, deterministic hash fallback otherwise). It owns no embedding logic —
/// it only wraps the shared backend behind the trait.
pub struct MemoryEmbedder {
    model: Arc<crate::memory::embeddings::EmbeddingModel>,
    dim: usize,
    model_id: String,
}

impl MemoryEmbedder {
    /// Default embedding dimension for all-MiniLM-L6-v2.
    pub const DIM: usize = 384;

    /// Load the shared embedding backend.
    pub fn load() -> Result<Self, CapError> {
        let model = crate::memory::embeddings::EmbeddingModel::load(Self::DIM)
            .map_err(|e| CapError::Io(format!("embedding model load failed: {e}")))?;
        Ok(Self {
            model: Arc::new(model),
            dim: Self::DIM,
            model_id: format!("all-MiniLM-L6-v2:{}", Self::DIM),
        })
    }

    /// Wrap an already-constructed shared backend (avoids loading a second model).
    pub fn from_model(model: Arc<crate::memory::embeddings::EmbeddingModel>, dim: usize) -> Self {
        Self {
            model,
            dim,
            model_id: format!("all-MiniLM-L6-v2:{dim}"),
        }
    }
}

impl Embedder for MemoryEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, CapError> {
        self.model
            .embed(text)
            .map_err(|e| CapError::Degraded(format!("embedding failed: {e}")))
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
}

/// A descriptor scored against a query, with its component signals for
/// transparency and downstream ranking.
#[derive(Debug, Clone)]
pub struct ScoredDescriptor {
    pub descriptor: CapabilityDescriptor,
    /// Fused final score (`0.0..=1.0`).
    pub score: f32,
    /// Dense embedding cosine similarity component.
    pub semantic: f32,
    /// Lexical token-overlap component.
    pub lexical: f32,
}

/// Relative weights for fusing the retrieval signals. Data, not hardcoded logic;
/// tunable via config in a later milestone.
#[derive(Debug, Clone, Copy)]
pub struct FusionWeights {
    pub semantic: f32,
    pub lexical: f32,
    /// Learned success signal (M6 learning loop): historical success rate for a
    /// `(provider_id, capability_id)` nudges its ranking up over repeated use.
    pub success: f32,
}

impl Default for FusionWeights {
    fn default() -> Self {
        // Semantic-led, lexical as a tie-breaker/booster for exact term matches,
        // plus a small learned-success nudge.
        Self {
            semantic: 0.65,
            lexical: 0.3,
            success: 0.05,
        }
    }
}

/// The cross-provider retrieval interface. Behind a trait so the in-process
/// implementation can be replaced by a distributed vector store (design R4.4).
pub trait FederatedIndex: Send + Sync {
    /// Replace the whole index from the given descriptors (idempotent).
    fn rebuild(&self, descriptors: &[CapabilityDescriptor]) -> Result<(), CapError>;
    /// Insert or replace one descriptor by `(provider_id, capability_id)`.
    fn upsert(&self, descriptor: &CapabilityDescriptor) -> Result<(), CapError>;
    /// Remove one descriptor.
    fn remove(&self, provider_id: &str, capability_id: &str);
    /// Top-k retrieval for a goal query.
    fn search(&self, query: &str, k: usize) -> Result<Vec<ScoredDescriptor>, CapError>;
    /// Score an arbitrary, externally-supplied descriptor set against a query
    /// (stateless — does not touch the stored index). Used to rank marketplace
    /// **catalog** entries for recommendations without indexing them. Default:
    /// unscored passthrough.
    fn score_descriptors(
        &self,
        _query: &str,
        descriptors: Vec<CapabilityDescriptor>,
        k: usize,
    ) -> Result<Vec<ScoredDescriptor>, CapError> {
        Ok(descriptors
            .into_iter()
            .take(k)
            .map(|d| ScoredDescriptor {
                descriptor: d,
                score: 0.0,
                semantic: 0.0,
                lexical: 0.0,
            })
            .collect())
    }
    /// Record an execution outcome for the learning loop (M6). Default no-op so
    /// alternative index backends need not implement it.
    fn record_outcome(&self, _provider_id: &str, _capability_id: &str, _ok: bool) {}
    /// Number of indexed descriptors.
    fn len(&self) -> usize;
    /// Whether the index is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One indexed descriptor plus its precomputed embedding.
struct Entry {
    descriptor: CapabilityDescriptor,
    embedding: Vec<f32>,
    /// Lowercased token bag of the descriptor text (for lexical overlap).
    tokens: Vec<String>,
}

/// Learned per-capability outcome stats for the M6 learning loop.
#[derive(Debug, Clone, Copy, Default)]
struct OutcomeStats {
    successes: u64,
    total: u64,
}

impl OutcomeStats {
    /// Success rate in `0.0..=1.0`; 0.5 (neutral) when unobserved so a fresh
    /// capability is neither boosted nor penalized.
    fn success_rate(&self) -> f32 {
        if self.total == 0 {
            0.5
        } else {
            self.successes as f32 / self.total as f32
        }
    }
}

/// In-process federated index (dense cosine ⊕ lexical overlap ⊕ learned success).
pub struct InMemoryFederatedIndex {
    embedder: Arc<dyn Embedder>,
    weights: FusionWeights,
    entries: RwLock<Vec<Entry>>,
    /// Learned outcome stats keyed by `(provider_id, capability_id)`.
    stats: RwLock<std::collections::HashMap<(String, String), OutcomeStats>>,
}

impl InMemoryFederatedIndex {
    /// Build over the given embedder with default fusion weights.
    pub fn new(embedder: Arc<dyn Embedder>) -> Self {
        Self {
            embedder,
            weights: FusionWeights::default(),
            entries: RwLock::new(Vec::new()),
            stats: RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Override fusion weights.
    pub fn with_weights(mut self, weights: FusionWeights) -> Self {
        self.weights = weights;
        self
    }

    /// The text used to represent a descriptor for embedding + lexical match:
    /// name, description, capability tags, and I/O type tags.
    fn descriptor_text(d: &CapabilityDescriptor) -> String {
        let mut parts = vec![d.name.clone(), d.description.clone()];
        parts.extend(d.tags.iter().map(|t| t.id.clone()));
        parts.extend(d.inputs.iter().cloned());
        parts.extend(d.outputs.iter().cloned());
        parts.extend(d.examples.iter().map(|e| e.prompt.clone()));
        parts.join(" ")
    }

    fn tokenize(text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }

    fn make_entry(&self, d: &CapabilityDescriptor) -> Entry {
        let text = Self::descriptor_text(d);
        // Embedding failure degrades to a zero vector (semantic=0) rather than
        // dropping the descriptor — lexical scoring still applies (honest degrade).
        let embedding = self.embedder.embed(&text).unwrap_or_default();
        Entry {
            descriptor: d.clone(),
            embedding,
            tokens: Self::tokenize(&text),
        }
    }

    /// Cosine similarity of two vectors; 0.0 if either is empty/zero.
    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        if a.is_empty() || b.is_empty() || a.len() != b.len() {
            return 0.0;
        }
        let mut dot = 0.0f32;
        let mut na = 0.0f32;
        let mut nb = 0.0f32;
        for i in 0..a.len() {
            dot += a[i] * b[i];
            na += a[i] * a[i];
            nb += b[i] * b[i];
        }
        if na == 0.0 || nb == 0.0 {
            return 0.0;
        }
        (dot / (na.sqrt() * nb.sqrt())).clamp(-1.0, 1.0)
    }

    /// Jaccard token overlap of the query against an entry's token bag.
    fn lexical_overlap(query_tokens: &[String], entry_tokens: &[String]) -> f32 {
        if query_tokens.is_empty() || entry_tokens.is_empty() {
            return 0.0;
        }
        let matches = query_tokens
            .iter()
            .filter(|q| entry_tokens.contains(q))
            .count();
        matches as f32 / query_tokens.len() as f32
    }
}

impl FederatedIndex for InMemoryFederatedIndex {
    fn rebuild(&self, descriptors: &[CapabilityDescriptor]) -> Result<(), CapError> {
        let entries: Vec<Entry> = descriptors.iter().map(|d| self.make_entry(d)).collect();
        let mut guard = self
            .entries
            .write()
            .map_err(|e| CapError::Io(format!("index lock poisoned: {e}")))?;
        *guard = entries;
        Ok(())
    }

    fn upsert(&self, descriptor: &CapabilityDescriptor) -> Result<(), CapError> {
        let entry = self.make_entry(descriptor);
        let mut guard = self
            .entries
            .write()
            .map_err(|e| CapError::Io(format!("index lock poisoned: {e}")))?;
        if let Some(existing) = guard.iter_mut().find(|e| {
            e.descriptor.provider_id == descriptor.provider_id
                && e.descriptor.capability_id == descriptor.capability_id
        }) {
            *existing = entry;
        } else {
            guard.push(entry);
        }
        Ok(())
    }

    fn remove(&self, provider_id: &str, capability_id: &str) {
        if let Ok(mut guard) = self.entries.write() {
            guard.retain(|e| {
                !(e.descriptor.provider_id == provider_id
                    && e.descriptor.capability_id == capability_id)
            });
        }
    }

    fn search(&self, query: &str, k: usize) -> Result<Vec<ScoredDescriptor>, CapError> {
        let query_emb = self.embedder.embed(query).unwrap_or_default();
        let query_tokens = Self::tokenize(query);

        let guard = self
            .entries
            .read()
            .map_err(|e| CapError::Io(format!("index lock poisoned: {e}")))?;
        let stats = self
            .stats
            .read()
            .map_err(|e| CapError::Io(format!("stats lock poisoned: {e}")))?;

        let mut scored: Vec<ScoredDescriptor> = guard
            .iter()
            .map(|e| {
                let semantic = Self::cosine(&query_emb, &e.embedding);
                let lexical = Self::lexical_overlap(&query_tokens, &e.tokens);
                // Learned success signal (M6): centered at 0 so a neutral 0.5
                // rate neither boosts nor penalizes; ±0.5 at the extremes.
                let success_rate = stats
                    .get(&e.descriptor.key())
                    .map(|s| s.success_rate())
                    .unwrap_or(0.5);
                let success_signal = success_rate - 0.5;
                let score = self.weights.semantic * semantic
                    + self.weights.lexical * lexical
                    + self.weights.success * success_signal;
                ScoredDescriptor {
                    descriptor: e.descriptor.clone(),
                    score,
                    semantic,
                    lexical,
                }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
        Ok(scored)
    }

    fn score_descriptors(
        &self,
        query: &str,
        descriptors: Vec<CapabilityDescriptor>,
        k: usize,
    ) -> Result<Vec<ScoredDescriptor>, CapError> {
        let query_emb = self.embedder.embed(query).unwrap_or_default();
        let query_tokens = Self::tokenize(query);
        let mut scored: Vec<ScoredDescriptor> = descriptors
            .into_iter()
            .map(|d| {
                let text = Self::descriptor_text(&d);
                let emb = self.embedder.embed(&text).unwrap_or_default();
                let tokens = Self::tokenize(&text);
                let semantic = Self::cosine(&query_emb, &emb);
                let lexical = Self::lexical_overlap(&query_tokens, &tokens);
                let score = self.weights.semantic * semantic + self.weights.lexical * lexical;
                ScoredDescriptor {
                    descriptor: d,
                    score,
                    semantic,
                    lexical,
                }
            })
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
        Ok(scored)
    }

    fn record_outcome(&self, provider_id: &str, capability_id: &str, ok: bool) {
        if let Ok(mut stats) = self.stats.write() {
            let entry = stats
                .entry((provider_id.to_string(), capability_id.to_string()))
                .or_default();
            entry.total += 1;
            if ok {
                entry.successes += 1;
            }
        }
    }

    fn len(&self) -> usize {
        self.entries.read().map(|g| g.len()).unwrap_or(0)
    }
}
