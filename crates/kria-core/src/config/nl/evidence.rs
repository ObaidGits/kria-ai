//! Evidence seams for the settings intent classifier (settings-nl-intelligence
//! Wave 2). The classifier reasons from multiple INDEPENDENT evidence sources
//! fused into a confidence, instead of treating keyword/marker lists as the
//! authority. These seams are all OPTIONAL and degrade gracefully: when an
//! embedder or memory source is absent the classifier falls back to the always-
//! available lexical tier (offline / cold-start), preserving behaviour exactly.
//!
//! - [`TextEmbedder`]: produces a semantic vector for a piece of text (tier-B).
//!   A real FastEmbed/ONNX embedder implements this; the pipeline only calls it
//!   when a lexical decision is ambiguous, keeping ordinary chat fast.
//! - [`MemoryEvidenceSource`]: reports how strongly recalled memory / an ongoing
//!   topic suggests the message is about a NON-configuration subject (negative
//!   evidence for a settings decision).

use std::sync::Arc;

/// Semantic text embedder seam (tier-B). Returns `None` when unavailable so the
/// caller uses the lexical fallback (graceful degradation, design F3).
pub trait TextEmbedder: Send + Sync {
    fn embed(&self, text: &str) -> Option<Vec<f32>>;
}

/// Optional memory/topic evidence. `topic_affinity` ∈ [0,1] = strength that the
/// message concerns an ongoing NON-config topic (biases toward conversation).
pub trait MemoryEvidenceSource: Send + Sync {
    fn topic_affinity(&self, text: &str) -> Option<f32>;
}

/// Cosine similarity of two vectors, clamped to `[0,1]` (negative → 0).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
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
    (dot / (na.sqrt() * nb.sqrt())).clamp(0.0, 1.0)
}

/// Documented weighted-fusion parameters (design Wave 5 F8, now real). Defaults
/// are calibrated so that with NO embedder/memory the fused result is identical
/// to the prior lexical behaviour (golden-preserving): the conversation penalty
/// factor of 0.40 reproduces the previous `entity_conf *= 0.6` bias.
#[derive(Clone, Copy, Debug)]
pub struct EvidenceWeights {
    /// How much an active conversation topic (semantic or lexical) suppresses a
    /// weak, value-less, neutral-subject settings guess.
    pub conversation_penalty: f32,
    /// How much memory topic-affinity suppresses the same.
    pub memory_penalty: f32,
    /// Additive confidence when the subject is explicitly KRIA-directed.
    pub subject_kria_boost: f32,
    /// Semantic-topic threshold above which conversation evidence is considered
    /// "active" (used when an embedder is present).
    pub topic_active: f32,
}

impl Default for EvidenceWeights {
    fn default() -> Self {
        Self {
            conversation_penalty: 0.40,
            memory_penalty: 0.40,
            subject_kria_boost: 0.0,
            topic_active: 0.62,
        }
    }
}

/// Bundle of optional evidence dependencies injected into the pipeline.
#[derive(Clone, Default)]
pub struct EvidenceDeps {
    pub embedder: Option<Arc<dyn TextEmbedder>>,
    pub memory: Option<Arc<dyn MemoryEvidenceSource>>,
    pub weights: EvidenceWeights,
}

impl EvidenceDeps {
    pub fn with_embedder(mut self, e: Arc<dyn TextEmbedder>) -> Self {
        self.embedder = Some(e);
        self
    }
    pub fn with_memory(mut self, m: Arc<dyn MemoryEvidenceSource>) -> Self {
        self.memory = Some(m);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_basic() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        // Opposite → clamped to 0.
        assert_eq!(cosine(&[1.0, 0.0], &[-1.0, 0.0]), 0.0);
        // Mismatched len → 0.
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
    }
}
