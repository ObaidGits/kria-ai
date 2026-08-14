//! ONNX text embedder (memory-upgrade design §45.2, tasks 8/12).
//!
//! Wraps the in-tree ONNX loader as the `Embedder` port. **MiniLM-L6-v2
//! (384-dim, Apache-2.0) is the default provisioned tier** (architecture §38.6);
//! EmbeddingGemma is an opt-in upgrade (future). Critical guard: when only the
//! deterministic hash fallback is available, the embedder reports itself
//! **unavailable** rather than emitting meaningless hash vectors into the ANN
//! index (design §45.2 bug-fix / IA-4) — retrieval then degrades to the keyword
//! floor (L8).

use std::sync::Arc;

use async_trait::async_trait;

use crate::embeddings::EmbeddingModel;
use crate::error::{EmbeddingError, MemoryResult};
use crate::stores::ports::Embedder;
use crate::types::{Availability, ModelVersion};

/// MiniLM tier constants.
const MINILM_VERSION: &str = "minilm_v1";
const MINILM_DIM: usize = 384;

/// ONNX-backed embedder over the shared [`EmbeddingModel`].
pub struct OnnxEmbedder {
    model: Arc<EmbeddingModel>,
    version: ModelVersion,
    dim: usize,
}

impl OnnxEmbedder {
    /// Load the default MiniLM tier. Succeeds even if the ONNX model is absent
    /// (the loader falls back internally), but in that case [`Self::health`]
    /// reports `Down` and [`Self::embed`] returns `Unavailable` (never hash
    /// vectors — §45.2).
    pub fn new_minilm() -> MemoryResult<Self> {
        let model = EmbeddingModel::load(MINILM_DIM)
            .map_err(|e| EmbeddingError::Inference(e.to_string()))?;
        Ok(Self {
            model: Arc::new(model),
            version: ModelVersion(MINILM_VERSION.to_string()),
            dim: MINILM_DIM,
        })
    }

    /// Wrap an already-loaded [`EmbeddingModel`] (MiniLM tier) as the `Embedder`
    /// port. Lets the runtime share one model instance across the legacy RAG
    /// path and the cognitive memory system instead of loading ONNX twice.
    pub fn from_model(model: Arc<EmbeddingModel>) -> Self {
        Self {
            model,
            version: ModelVersion(MINILM_VERSION.to_string()),
            dim: MINILM_DIM,
        }
    }

    /// Whether a real ONNX model (not the hash fallback) is loaded.
    pub fn is_ready(&self) -> bool {
        self.model.is_onnx_loaded()
    }
}

#[async_trait]
impl Embedder for OnnxEmbedder {
    fn model_version(&self) -> ModelVersion {
        self.version.clone()
    }

    fn dim(&self) -> usize {
        self.dim
    }

    async fn embed(&self, texts: &[String]) -> MemoryResult<Vec<Vec<f32>>> {
        // Never index hash-fallback vectors: treat "no real model" as unavailable.
        if !self.model.is_onnx_loaded() {
            return Err(EmbeddingError::Unavailable.into());
        }
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let model = Arc::clone(&self.model);
        let owned: Vec<String> = texts.to_vec();
        // ONNX inference is CPU-bound → run off the async executor.
        let result = tokio::task::spawn_blocking(move || {
            let mut out = Vec::with_capacity(owned.len());
            for t in &owned {
                out.push(model.embed(t));
            }
            out
        })
        .await
        .map_err(|e| EmbeddingError::Inference(format!("join error: {e}")))?;

        let mut vectors = Vec::with_capacity(result.len());
        for r in result {
            let v = r.map_err(|e| EmbeddingError::Inference(e.to_string()))?;
            if v.len() != self.dim {
                return Err(EmbeddingError::DimMismatch {
                    expected: self.dim,
                    got: v.len(),
                }
                .into());
            }
            vectors.push(v);
        }
        Ok(vectors)
    }

    async fn health(&self) -> Availability {
        if self.model.is_onnx_loaded() {
            Availability::Up
        } else {
            // Hash fallback only → embeddings are effectively unavailable.
            Availability::Down
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reports_unavailable_without_onnx_model() {
        // In CI/test there is no ONNX model on disk → loader uses hash fallback.
        let emb = OnnxEmbedder::new_minilm().unwrap();
        assert_eq!(emb.dim(), MINILM_DIM);
        assert_eq!(emb.model_version(), ModelVersion(MINILM_VERSION.into()));
        if !emb.is_ready() {
            // The key invariant: no hash vectors leak out.
            assert_eq!(emb.health().await, Availability::Down);
            let err = emb.embed(&["hello".to_string()]).await.unwrap_err();
            assert!(err.is_degradation(), "must be a degradation signal (L8)");
        }
    }
}
