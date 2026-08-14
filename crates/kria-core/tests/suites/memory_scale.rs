//! Retrieval quality gate at scale (memory-upgrade task 36, R12/CP-17).
//!
//! Seeds a reproducible synthetic corpus with *planted* query→relevant labels
//! and asserts recall stays above the baseline as the bank grows. This is the
//! mechanism the CI release gate scales to 500K; the test runs a fast, bounded N
//! so it can gate every build. Deterministic (fixed content) → reproducible.

use std::sync::Arc;

use async_trait::async_trait;
use kria_core::memory::api::{MemoryConfig, MemorySystem};
use kria_core::memory::error::MemoryResult;
use kria_core::memory::stores::ports::Embedder;
use kria_core::memory::types::{Availability, ModelVersion, WriteCandidate};
use uuid::Uuid;

/// Content-derived embedder: similar text → similar vectors (bag-of-bytes into a
/// fixed dimension), so vector search is meaningful and reproducible.
struct BagEmbedder {
    dim: usize,
}
#[async_trait]
impl Embedder for BagEmbedder {
    fn model_version(&self) -> ModelVersion {
        ModelVersion("bag_v1".into())
    }
    fn dim(&self) -> usize {
        self.dim
    }
    async fn embed(&self, texts: &[String]) -> MemoryResult<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; self.dim];
                for w in t.split_whitespace() {
                    let h = w
                        .bytes()
                        .fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize));
                    v[h % self.dim] += 1.0;
                }
                // L2 normalize.
                let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for x in &mut v {
                        *x /= norm;
                    }
                }
                v
            })
            .collect())
    }
    async fn health(&self) -> Availability {
        Availability::Up
    }
}

#[tokio::test]
async fn retrieval_recall_at_scale() {
    const N: usize = 500;
    let sys =
        MemorySystem::open_for_test(MemoryConfig::default(), Arc::new(BagEmbedder { dim: 64 }))
            .unwrap();
    let session = Uuid::now_v7();

    // Seed a reproducible corpus: each memory has a distinctive rare token
    // (`marker{i}`) plus shared topic filler so retrieval must discriminate.
    for i in 0..N {
        let topic = i % 25;
        let content = format!(
            "note marker{i} about topic{topic} : the system records observation number {i} \
             regarding subsystem{topic} behavior and configuration"
        );
        sys.remember(WriteCandidate::user(session, content))
            .unwrap();
    }
    let processed = sys.flush().await.unwrap();
    assert_eq!(processed, N, "all seeded events enriched");
    assert_eq!(sys.health().await.unwrap().memory_count as usize, N);

    // Query each planted marker; the matching memory must appear in the top-K.
    let sample: Vec<usize> = (0..N).step_by(17).collect(); // ~30 probes
    let mut hits_at_k = 0usize;
    for &i in &sample {
        let q = format!("marker{i}");
        let res = sys.search(&q, None).await.unwrap();
        let found = res
            .hits
            .iter()
            .any(|h| h.memory.content.contains(&format!("marker{i} ")));
        if found {
            hits_at_k += 1;
        }
    }
    let recall = hits_at_k as f64 / sample.len() as f64;
    assert!(
        recall >= 0.9,
        "recall@budget regressed below baseline: {recall:.3} ({hits_at_k}/{})",
        sample.len()
    );
}
