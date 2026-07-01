//! Embedding wrapper around fastembed-rs.
//!
//! Provides:
//! - A bounded **pool** of `TextEmbedding` instances (sharded) so embedding is not globally
//!   serialized behind a single mutex (HRA Task 16 / R4.3). Pool size defaults to 1 (identical
//!   memory + behavior to the old single-model path); set `KRIA_EMBED_POOL=N` to allow up to N
//!   concurrent embeds (each shard is a full model copy → N× memory, opt-in).
//! - `embed_one` / `embed_batch`: produce L2-normalised f32 vectors.
//! - `cosine_sim`: dot product of two pre-normalised vectors.

use anyhow::Result;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use once_cell::sync::OnceCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// A sharded pool of embedding models. Concurrent callers round-robin across shards, so up to
/// `shards.len()` embeds run in parallel instead of all serializing on one mutex.
struct EmbedPool {
    shards: Vec<Mutex<TextEmbedding>>,
    rr: AtomicUsize,
}

impl EmbedPool {
    /// Pick the next shard round-robin and run `f` while holding only that shard's lock.
    fn with_shard<R>(&self, f: impl FnOnce(&mut TextEmbedding) -> R) -> Result<R> {
        let idx = self.rr.fetch_add(1, Ordering::Relaxed) % self.shards.len();
        let mut model = self.shards[idx]
            .lock()
            .map_err(|_| anyhow::anyhow!("embedding model mutex poisoned"))?;
        Ok(f(&mut model))
    }
}

static EMBED_POOL: OnceCell<EmbedPool> = OnceCell::new();

/// Desired pool size. `KRIA_EMBED_POOL` (1..=8), default 1. Each shard is a full model copy.
fn desired_pool_size() -> usize {
    std::env::var("KRIA_EMBED_POOL")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 8)
}

/// Initialise (or no-op if already done).
/// `cache_dir` is the directory where fastembed downloads/stores the model.
/// Model: multilingual-e5-small for Hinglish support.
pub fn init_embedding_model(cache_dir: PathBuf) -> Result<()> {
    if EMBED_POOL.get().is_some() {
        return Ok(());
    }
    let n = desired_pool_size();
    let mut shards = Vec::with_capacity(n);
    for i in 0..n {
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::MultilingualE5Small)
                .with_cache_dir(cache_dir.clone())
                .with_show_download_progress(false),
        )
        .map_err(|e| anyhow::anyhow!("failed to load embedding model shard {i}: {e}"))?;
        shards.push(Mutex::new(model));
    }
    tracing::info!(pool_size = n, "embedding model pool initialised");
    // Ignore error if another thread already set it (race on first boot).
    let _ = EMBED_POOL.set(EmbedPool {
        shards,
        rr: AtomicUsize::new(0),
    });
    Ok(())
}

/// Check whether the embedding model has been initialised.
pub fn is_ready() -> bool {
    EMBED_POOL.get().is_some()
}

/// Embed a batch of texts. Returns L2-normalised vectors.
pub fn embed_batch(texts: &[&str]) -> Result<Vec<Vec<f32>>> {
    let pool = EMBED_POOL.get().ok_or_else(|| {
        anyhow::anyhow!("embedding model not initialised; call init_embedding_model first")
    })?;
    let raw = pool.with_shard(|model| model.embed(texts, None))??;
    Ok(raw.into_iter().map(l2_normalise).collect())
}

/// Embed a single text. Returns an L2-normalised vector.
pub fn embed_one(text: &str) -> Result<Vec<f32>> {
    let mut batch = embed_batch(&[text])?;
    batch
        .pop()
        .ok_or_else(|| anyhow::anyhow!("empty embedding result"))
}

/// Cosine similarity of two pre-normalised vectors (= dot product).
#[inline]
pub fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// In-place L2 normalise.
fn l2_normalise(mut v: Vec<f32>) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-9 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}
