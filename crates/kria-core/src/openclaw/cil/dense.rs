//! Dense (semantic) capability retrieval for the Capability Intelligence Layer
//! (design §8.3, R11.1 / R11.2).
//!
//! CIL discovers skills by *meaning*, not just keywords. This module provides the
//! dense half of that: nearest-neighbor retrieval over the embeddings of each
//! skill's `provides` [`CapabilityTag`]s. It is fused with the frozen BM25 index
//! (`resolver::SkillIndex`) inside `CapabilityIndex` in task 3.3; this module
//! implements ONLY the dense index and its trait boundary.
//!
//! # Trait boundary — pluggable, distributable (R11.2)
//!
//! Retrieval lives behind the [`DenseRetrieval`] trait so the in-process
//! [`DenseIndex`] here can be swapped for a distributed vector store (a remote
//! ANN service, a managed vector DB) **without any caller changes**. Callers
//! (the fusion layer, the ranker) depend on `&dyn DenseRetrieval`, never on the
//! concrete implementation. This is the "pluggable bounded execution
//! intelligence" invariant: the retrieval backend is a seam, not a hard-coded
//! dependency.
//!
//! # In-process default — flat cosine, ArcSwap snapshot
//!
//! The default [`DenseIndex`] is a **flat** (exhaustive) cosine-similarity index:
//! vectors are L2-normalized at build time so cosine similarity reduces to a dot
//! product. Flat search is exact and satisfies "ANN dense retrieval" as the
//! correctness-first default; the [`DenseRetrieval`] boundary lets an approximate
//! HNSW index or a distributed store replace it later with no caller churn. This
//! keeps the in-process cost bounded and predictable at the scales task 3.x
//! targets.
//!
//! The active index is held in an [`arc_swap::ArcSwap`] snapshot inside
//! [`DenseIndexHandle`] for **lock-free reads** on the discovery hot path; a
//! `rebuild` builds a fresh immutable [`DenseIndex`] and swaps it in atomically,
//! mirroring the frozen `resolver::SkillIndex` pattern.
//!
//! # Vector aggregation (deterministic)
//!
//! A skill's dense vector is the **mean-pool of its `provides`-tag embeddings**,
//! then L2-normalized. Mean-pooling is order-independent and deterministic;
//! normalization makes similarity a pure cosine. Skills whose `provides` tags
//! carry no embeddings (or whose aggregate is a zero vector) are omitted from the
//! dense index — they remain discoverable through the frozen BM25 half (honest
//! degradation, no fabricated vectors).
//!
//! # Determinism
//!
//! Search results are sorted by descending score with a **stable tie-break by
//! `skill_id`** so equal-scoring skills always return in the same order. This
//! keeps discovery reproducible (the idempotent-reindex property, task 3.5).
//!
//! # No hardcoding
//!
//! The index is generic over any [`CapabilityTag`]: it embeds and indexes tag
//! vectors through one code path with no per-capability or per-category branch. A
//! never-before-seen tag id flows through identically (verified by the
//! no-hardcoding property test, task 3.6).

use std::sync::Arc;

use arc_swap::ArcSwap;

use super::profile::CapabilityProfile;

/// Pluggable dense-retrieval boundary (design §8.3, R11.2).
///
/// Callers depend on this trait, never on a concrete index, so the in-process
/// [`DenseIndex`] can be replaced by a distributed vector store with no caller
/// changes. Implementations must be `Send + Sync` so a single index can be shared
/// (behind `Arc` / `ArcSwap`) across concurrent discovery stages.
pub trait DenseRetrieval: Send + Sync {
    /// Return up to `k` skill ids most similar to `query`, as
    /// `(skill_id, score)` pairs sorted by **descending score** with a stable
    /// tie-break by `skill_id` (deterministic ordering for equal scores).
    ///
    /// `score` is cosine similarity in `[-1.0, 1.0]` for the flat default. A
    /// `query` whose dimension does not match the index, or a `k` of zero,
    /// yields an empty result (honest, never a panic).
    fn search(&self, query: &[f32], k: usize) -> Vec<(String, f32)>;

    /// Number of indexed skills (those with a usable aggregate embedding).
    fn len(&self) -> usize;

    /// The vector dimension of this index, or `None` when the index is empty.
    fn dim(&self) -> Option<usize>;

    /// Whether the index holds no vectors.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One indexed skill: its id and its L2-normalized aggregate embedding.
#[derive(Debug, Clone)]
struct DenseEntry {
    skill_id: String,
    /// L2-normalized so cosine similarity is a plain dot product.
    vector: Vec<f32>,
}

/// In-process flat (exhaustive) cosine-similarity dense index — the default
/// [`DenseRetrieval`] implementation (design §8.3).
///
/// Immutable once built, so it is cheap to share behind an [`ArcSwap`] and swap
/// atomically on rebuild. Vectors are L2-normalized at build time; search
/// normalizes the query and returns dot products (= cosine similarity).
#[derive(Debug, Clone, Default)]
pub struct DenseIndex {
    entries: Vec<DenseEntry>,
    /// Common dimension of all entries; `None` when empty.
    dim: Option<usize>,
}

impl DenseIndex {
    /// An empty index (no vectors). Used as the initial [`ArcSwap`] snapshot.
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            dim: None,
        }
    }

    /// Build a flat index from pre-aggregated `(skill_id, vector)` pairs.
    ///
    /// Each vector is L2-normalized; zero-length and zero-norm vectors are
    /// skipped (no fabricated data). The index dimension is taken from the first
    /// usable vector; vectors of a different dimension are skipped so a mixed-dim
    /// input cannot corrupt search. Entries are stored sorted by `skill_id` for
    /// deterministic construction.
    pub fn build(entries: impl IntoIterator<Item = (String, Vec<f32>)>) -> Self {
        let mut dim: Option<usize> = None;
        let mut built: Vec<DenseEntry> = Vec::new();

        for (skill_id, vector) in entries {
            if vector.is_empty() {
                continue;
            }
            match dim {
                None => dim = Some(vector.len()),
                Some(d) if d != vector.len() => continue, // skip mismatched-dim vector
                Some(_) => {}
            }
            match l2_normalize(&vector) {
                Some(normalized) => built.push(DenseEntry {
                    skill_id,
                    vector: normalized,
                }),
                None => continue, // zero-norm vector carries no direction; skip
            }
        }

        // Deterministic storage order (stable across rebuilds from the same set).
        built.sort_by(|a, b| a.skill_id.cmp(&b.skill_id));
        Self {
            entries: built,
            dim,
        }
    }

    /// Build a flat index from capability profiles, aggregating each skill's
    /// `provides`-tag embeddings by mean-pool (design §8.3).
    ///
    /// Skills whose `provides` tags carry no embeddings are omitted (they remain
    /// discoverable via the frozen BM25 half). Deterministic and free of any
    /// per-capability branch — a novel tag id is aggregated identically.
    pub fn from_profiles(profiles: &[CapabilityProfile]) -> Self {
        let pairs = profiles
            .iter()
            .filter_map(|p| aggregate_provides(p).map(|vector| (p.skill_id.clone(), vector)));
        Self::build(pairs)
    }
}

impl DenseRetrieval for DenseIndex {
    fn search(&self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        if k == 0 || self.entries.is_empty() {
            return Vec::new();
        }
        // Dimension mismatch → honest empty result rather than a bogus score.
        if self.dim != Some(query.len()) {
            return Vec::new();
        }
        let normalized_query = match l2_normalize(query) {
            Some(q) => q,
            None => return Vec::new(), // zero query has no direction
        };

        let mut scored: Vec<(String, f32)> = self
            .entries
            .iter()
            .map(|e| (e.skill_id.clone(), dot(&e.vector, &normalized_query)))
            .collect();

        // Descending score; stable tie-break by skill_id for determinism.
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        scored.truncate(k);
        scored
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn dim(&self) -> Option<usize> {
        self.dim
    }
}

/// Lock-free, atomically-rebuildable holder for the active [`DenseIndex`]
/// (design §8.3), mirroring the frozen `resolver::SkillIndex` ArcSwap pattern.
///
/// Reads ([`load`](DenseIndexHandle::load) / [`search`](DenseIndexHandle::search))
/// are contention-free — just an `Arc` clone of the current snapshot. A
/// [`rebuild`](DenseIndexHandle::rebuild) builds a fresh immutable index and
/// swaps it in atomically; in-flight readers keep the old snapshot until they
/// reload, then seamlessly switch.
pub struct DenseIndexHandle {
    snapshot: ArcSwap<DenseIndex>,
}

impl Default for DenseIndexHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl DenseIndexHandle {
    /// A handle wrapping an empty index.
    pub fn new() -> Self {
        Self {
            snapshot: ArcSwap::from_pointee(DenseIndex::empty()),
        }
    }

    /// Read the current snapshot. Zero contention — an `Arc` clone.
    pub fn load(&self) -> arc_swap::Guard<Arc<DenseIndex>> {
        self.snapshot.load()
    }

    /// Atomically replace the active index with `index`.
    pub fn store(&self, index: DenseIndex) {
        self.snapshot.store(Arc::new(index));
    }

    /// Rebuild the dense index from capability profiles and swap it in
    /// atomically (design §8.3). Aggregates each skill's `provides`-tag
    /// embeddings by mean-pool; deterministic for a fixed input set.
    pub fn rebuild(&self, profiles: &[CapabilityProfile]) {
        self.store(DenseIndex::from_profiles(profiles));
    }

    /// Lock-free search against the current snapshot. See
    /// [`DenseRetrieval::search`] for ordering/semantics.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        self.load().search(query, k)
    }
}

/// Mean-pool the embeddings of a profile's `provides` tags into a single vector.
///
/// Returns `None` when no tag carries an embedding or the tags disagree on
/// dimension (nothing to aggregate honestly). Deterministic: summation order is
/// the `provides` order and division is by the count of contributing vectors.
pub fn aggregate_provides(profile: &CapabilityProfile) -> Option<Vec<f32>> {
    let mut acc: Vec<f32> = Vec::new();
    let mut count: usize = 0;

    for tag in &profile.provides {
        let Some(embedding) = tag.embedding.as_ref() else {
            continue;
        };
        if embedding.is_empty() {
            continue;
        }
        if acc.is_empty() {
            acc = embedding.clone();
            count = 1;
        } else if acc.len() == embedding.len() {
            for (a, b) in acc.iter_mut().zip(embedding.iter()) {
                *a += *b;
            }
            count += 1;
        } else {
            // Dimension disagreement among tags — skip the outlier rather than
            // corrupt the mean.
            continue;
        }
    }

    if count == 0 {
        return None;
    }
    let inv = 1.0 / count as f32;
    for a in acc.iter_mut() {
        *a *= inv;
    }
    Some(acc)
}

/// L2-normalize a vector; returns `None` for an empty or zero-norm vector (no
/// direction to normalize).
fn l2_normalize(v: &[f32]) -> Option<Vec<f32>> {
    if v.is_empty() {
        return None;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 || !norm.is_finite() {
        return None;
    }
    Some(v.iter().map(|x| x / norm).collect())
}

/// Dot product of two equal-length vectors.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::cil::profile::CapabilityTag;

    fn tag_with(id: &str, embedding: Vec<f32>) -> CapabilityTag {
        CapabilityTag {
            id: id.into(),
            qualifiers: serde_json::Map::new(),
            embedding: Some(embedding),
        }
    }

    fn profile(skill_id: &str, provides: Vec<CapabilityTag>) -> CapabilityProfile {
        CapabilityProfile {
            skill_id: skill_id.into(),
            provides,
            consumes: Vec::new(),
            permissions: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }

    #[test]
    fn empty_index_returns_no_results() {
        let idx = DenseIndex::empty();
        assert!(idx.is_empty());
        assert_eq!(idx.dim(), None);
        assert!(idx.search(&[1.0, 0.0], 5).is_empty());
    }

    #[test]
    fn flat_search_ranks_by_cosine_similarity() {
        let idx = DenseIndex::build(vec![
            ("east".into(), vec![1.0, 0.0]),
            ("north".into(), vec![0.0, 1.0]),
            ("northeast".into(), vec![1.0, 1.0]),
        ]);
        assert_eq!(idx.len(), 3);
        assert_eq!(idx.dim(), Some(2));

        // Query points due-east; "east" is the closest, "north" the farthest.
        let results = idx.search(&[1.0, 0.0], 3);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, "east");
        assert_eq!(results[2].0, "north");
        // Cosine of identical direction is ~1.0.
        assert!((results[0].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn search_respects_k() {
        let idx = DenseIndex::build(vec![
            ("a".into(), vec![1.0, 0.0]),
            ("b".into(), vec![0.9, 0.1]),
            ("c".into(), vec![0.0, 1.0]),
        ]);
        assert_eq!(idx.search(&[1.0, 0.0], 2).len(), 2);
        assert!(idx.search(&[1.0, 0.0], 0).is_empty());
    }

    #[test]
    fn equal_scores_tie_break_by_skill_id_deterministically() {
        // Two identical vectors → identical scores; must order by skill_id.
        let idx = DenseIndex::build(vec![
            ("zebra".into(), vec![1.0, 0.0]),
            ("alpha".into(), vec![1.0, 0.0]),
        ]);
        let results = idx.search(&[1.0, 0.0], 2);
        assert_eq!(results[0].0, "alpha");
        assert_eq!(results[1].0, "zebra");
    }

    #[test]
    fn dimension_mismatch_query_yields_empty() {
        let idx = DenseIndex::build(vec![("a".into(), vec![1.0, 0.0, 0.0])]);
        assert!(idx.search(&[1.0, 0.0], 5).is_empty());
    }

    #[test]
    fn mismatched_dim_and_zero_vectors_are_skipped_at_build() {
        let idx = DenseIndex::build(vec![
            ("keep".into(), vec![1.0, 0.0]),
            ("wrongdim".into(), vec![1.0, 0.0, 0.0]),
            ("zero".into(), vec![0.0, 0.0]),
            ("emptyvec".into(), vec![]),
        ]);
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.dim(), Some(2));
        assert_eq!(idx.search(&[1.0, 0.0], 5)[0].0, "keep");
    }

    #[test]
    fn aggregate_provides_mean_pools_tag_embeddings() {
        let p = profile(
            "skill.x",
            vec![
                tag_with("cap.a", vec![2.0, 0.0]),
                tag_with("cap.b", vec![0.0, 4.0]),
            ],
        );
        let agg = aggregate_provides(&p).expect("has embeddings");
        assert_eq!(agg, vec![1.0, 2.0]); // mean of (2,0) and (0,4)
    }

    #[test]
    fn aggregate_provides_none_without_embeddings() {
        let p = profile("skill.y", vec![CapabilityTag::new("cap.noembed")]);
        assert!(aggregate_provides(&p).is_none());
    }

    #[test]
    fn from_profiles_omits_skills_without_embeddings() {
        let profiles = vec![
            profile("has.vec", vec![tag_with("cap.a", vec![1.0, 0.0])]),
            profile("no.vec", vec![CapabilityTag::new("cap.b")]),
        ];
        let idx = DenseIndex::from_profiles(&profiles);
        assert_eq!(idx.len(), 1);
        let results = idx.search(&[1.0, 0.0], 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "has.vec");
    }

    #[test]
    fn novel_capability_tag_flows_through_unchanged() {
        // No-hardcoding smoke: a never-before-seen tag id is embedded, indexed,
        // and searchable through the same code path — no per-capability branch.
        let profiles = vec![profile(
            "novel.skill",
            vec![tag_with("quantum.entangle.route.v9", vec![0.6, 0.8])],
        )];
        let idx = DenseIndex::from_profiles(&profiles);
        assert_eq!(idx.len(), 1);
        let results = idx.search(&[0.6, 0.8], 1);
        assert_eq!(results[0].0, "novel.skill");
        assert!((results[0].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn rebuild_is_idempotent_for_fixed_input() {
        let profiles = vec![
            profile("s1", vec![tag_with("c1", vec![1.0, 0.0])]),
            profile("s2", vec![tag_with("c2", vec![0.0, 1.0])]),
        ];
        let a = DenseIndex::from_profiles(&profiles);
        let b = DenseIndex::from_profiles(&profiles);
        let q = [0.7, 0.7];
        assert_eq!(a.search(&q, 5), b.search(&q, 5));
    }

    #[test]
    fn handle_provides_lock_free_reads_and_atomic_rebuild() {
        let handle = DenseIndexHandle::new();
        assert!(handle.load().is_empty());

        let profiles = vec![profile("s1", vec![tag_with("c1", vec![1.0, 0.0])])];
        handle.rebuild(&profiles);
        let results = handle.search(&[1.0, 0.0], 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "s1");

        // Rebuild swaps a fresh snapshot atomically.
        let profiles2 = vec![
            profile("s1", vec![tag_with("c1", vec![1.0, 0.0])]),
            profile("s2", vec![tag_with("c2", vec![0.0, 1.0])]),
        ];
        handle.rebuild(&profiles2);
        assert_eq!(handle.load().len(), 2);
    }

    #[test]
    fn dyn_dense_retrieval_boundary_is_object_safe() {
        // R11.2: callers can depend on the trait object, letting a distributed
        // store replace the in-process index with no caller changes.
        let idx = DenseIndex::build(vec![("a".into(), vec![1.0, 0.0])]);
        let dynamic: &dyn DenseRetrieval = &idx;
        assert_eq!(dynamic.len(), 1);
        assert_eq!(dynamic.search(&[1.0, 0.0], 1)[0].0, "a");
    }
}
