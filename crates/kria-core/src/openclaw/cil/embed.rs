//! Embedding provider for the Capability Intelligence Layer (design §8.1).
//!
//! CIL turns skills, goals, and open-vocabulary [`CapabilityTag`]s into vectors
//! for dense discovery. Rather than owning an embedding backend, CIL defines the
//! [`Embedder`] **trait** so the backend is pluggable (in-process ONNX today, a
//! distributed service later) and scale-testable with mocks — no caller changes.
//!
//! The default impl, [`MemoryEmbedder`], **delegates to the frozen KRIA
//! embedding backend** (`crate::memory::embeddings::EmbeddingModel`, FastEmbed/
//! ONNX — no Python). It introduces **no new embedding backend**: it reuses the
//! frozen component per the design's "extend, never fork" invariant.
//!
//! # Degraded honesty (design §13.1)
//!
//! If the backend fails to embed, the default impl surfaces a
//! [`CilError::Embed`] rather than panicking, so callers (the `CapabilityIndex`)
//! can fall back to the frozen BM25 index and report the degraded state honestly.
//!
//! # Model-id / cache invalidation (design §8.1, Iteration-4)
//!
//! [`Embedder::model_id`] returns a stable identifier for the active model. When
//! the model changes (e.g. ONNX weights swapped for the hash fallback, or the
//! dimension changes), the id changes too, letting derived indexes detect churn
//! and trigger a background reindex (the `profile_epoch` versioning of task 3.4).

use std::sync::Arc;

use async_trait::async_trait;

use super::CilError;
use crate::memory::embeddings::EmbeddingModel;

/// Text → vector provider for CIL discovery (design §8.1).
///
/// All CIL boundaries are traits so backends stay pluggable and scale-testable;
/// the default impl ([`MemoryEmbedder`]) delegates to the frozen KRIA embedding
/// backend. Implementations must be `Send + Sync` so the facade can share one
/// behind an `Arc<dyn Embedder>` across concurrent discovery stages.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Embed a single text into a dense vector of length [`dim`](Embedder::dim).
    ///
    /// Returns [`CilError::Embed`] on backend failure — never panics — so the
    /// caller can fall back to lexical-only discovery (honest degraded mode).
    async fn embed(&self, text: &str) -> Result<Vec<f32>, CilError>;

    /// Embed a batch of texts, one vector per input, order-preserving.
    ///
    /// Delegates to the backend's batch API when available, else maps over
    /// [`embed`](Embedder::embed).
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, CilError>;

    /// The dimension of every vector this embedder produces.
    fn dim(&self) -> usize;

    /// A stable identifier for the active model, used for cache invalidation on
    /// model change (design §8.1 — `profile_epoch` versioning).
    fn model_id(&self) -> &str;
}

/// Default [`Embedder`] delegating to the frozen KRIA embedding backend
/// (`memory::embeddings::EmbeddingModel`, FastEmbed/ONNX — no Python).
///
/// This is the "extend, never fork" default: it owns no embedding logic of its
/// own, only wrapping the shared frozen model behind the trait boundary. The
/// backend's `embed` is synchronous and CPU-bound (ONNX inference under a
/// `Mutex`), so the async methods offload to `tokio::task::spawn_blocking` to
/// avoid stalling the executor — matching the convention used elsewhere in
/// kria-core (e.g. `install_sink`, telemetry, STT/TTS).
pub struct MemoryEmbedder {
    /// The frozen embedding backend, shared for cheap `Arc` clones into blocking
    /// tasks.
    model: Arc<EmbeddingModel>,
    /// Cached vector dimension (backend value, avoids a lock on the hot path).
    dim: usize,
    /// Stable model identifier for cache invalidation. Derived from the backend
    /// (real ONNX vs hash fallback + dimension); the backend does not expose a
    /// model name directly, so it is computed and stored at construction.
    model_id: String,
}

impl MemoryEmbedder {
    /// Wrap a frozen [`EmbeddingModel`] as the default [`Embedder`].
    ///
    /// The `model_id` is derived from the backend so it changes whenever the
    /// active model changes: the real `all-MiniLM-L6-v2` ONNX model and the
    /// deterministic hash fallback report distinct ids, and the dimension is
    /// folded in so a dimension change also invalidates derived caches.
    pub fn new(model: Arc<EmbeddingModel>) -> Self {
        let dim = model.dimension();
        let backend = if model.is_onnx_loaded() {
            "all-MiniLM-L6-v2-onnx"
        } else {
            "hash-fallback-v1"
        };
        let model_id = format!("{backend}-d{dim}");
        Self {
            model,
            dim,
            model_id,
        }
    }

    /// Construct the default embedder by loading the frozen backend at `dim`.
    ///
    /// Surfaces [`CilError::Embed`] (never panics) if the backend cannot be
    /// initialized, so the caller can enter honest degraded mode and fall back
    /// to the frozen BM25 index (design §13.1).
    pub fn load(dim: usize) -> Result<Self, CilError> {
        let model = EmbeddingModel::load(dim)
            .map_err(|e| CilError::Embed(format!("failed to load embedding model: {e}")))?;
        Ok(Self::new(Arc::new(model)))
    }
}

#[async_trait]
impl Embedder for MemoryEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, CilError> {
        let model = Arc::clone(&self.model);
        let text = text.to_owned();
        tokio::task::spawn_blocking(move || model.embed(&text))
            .await
            .map_err(|e| CilError::Embed(format!("embedding task panicked: {e}")))?
            .map_err(|e| CilError::Embed(e.to_string()))
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, CilError> {
        // The frozen backend has no batch API, so map over the single-text embed
        // inside one blocking task (avoids per-item task overhead while keeping
        // the executor unblocked). Order-preserving.
        let model = Arc::clone(&self.model);
        let texts = texts.to_vec();
        tokio::task::spawn_blocking(move || {
            texts
                .iter()
                .map(|t| model.embed(t))
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .await
        .map_err(|e| CilError::Embed(format!("embedding batch task panicked: {e}")))?
        .map_err(|e| CilError::Embed(e.to_string()))
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the default embedder over the frozen backend. The backend's `load`
    /// falls back to a deterministic hash embedding when no ONNX model file is
    /// present, so this constructs successfully in CI without model downloads.
    fn test_embedder(dim: usize) -> MemoryEmbedder {
        MemoryEmbedder::load(dim).expect("frozen backend load (hash fallback in CI)")
    }

    #[tokio::test]
    async fn embed_returns_vector_of_declared_dim() {
        let emb = test_embedder(64);
        assert_eq!(emb.dim(), 64);
        let v = emb.embed("compress a pdf file").await.expect("embed ok");
        assert_eq!(v.len(), 64, "vector length must equal dim()");
    }

    #[tokio::test]
    async fn embed_batch_is_order_preserving_and_matches_single() {
        let emb = test_embedder(48);
        let inputs = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        let batch = emb.embed_batch(&inputs).await.expect("batch ok");
        assert_eq!(batch.len(), inputs.len());
        for (i, text) in inputs.iter().enumerate() {
            assert_eq!(batch[i].len(), emb.dim());
            let single = emb.embed(text).await.expect("single ok");
            assert_eq!(batch[i], single, "batch[{i}] must match single embed");
        }
    }

    #[tokio::test]
    async fn empty_batch_yields_empty_result() {
        let emb = test_embedder(32);
        let out = emb.embed_batch(&[]).await.expect("empty batch ok");
        assert!(out.is_empty());
    }

    #[test]
    fn model_id_is_stable_and_encodes_dim() {
        let emb = test_embedder(128);
        let id = emb.model_id().to_string();
        assert!(!id.is_empty(), "model_id must be non-empty");
        // Stable across calls (used as a cache key).
        assert_eq!(id, emb.model_id());
        // Dimension is folded in so a dim change invalidates derived caches.
        assert!(id.contains("d128"), "model_id should encode dim, got: {id}");
    }

    #[test]
    fn model_id_changes_with_dim() {
        let a = test_embedder(64);
        let b = test_embedder(96);
        assert_ne!(
            a.model_id(),
            b.model_id(),
            "different dimensions must yield different model ids"
        );
    }
}
