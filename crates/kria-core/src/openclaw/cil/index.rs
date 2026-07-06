//! `CapabilityIndex` — the fused semantic + lexical discovery index over
//! **installed** skills (task 3.3, design §8.3 / §7.3, R4.2 / R11.1 / R5.1).
//!
//! Discovery answers "which installed skills could serve this goal?" by fusing
//! two retrieval signals over the SAME source of truth
//! (`ProductionSkillRegistry::get_enabled_skills()`):
//!
//! - **semantic** — dense nearest-neighbor over each skill's `provides`-tag
//!   embeddings ([`DenseIndexHandle`], task 3.2), and
//! - **lexical** — a BM25 inverted-index over each skill's textual metadata
//!   ([`LexicalIndex`] here).
//!
//! Both halves live in lock-free [`arc_swap::ArcSwap`] snapshots and are rebuilt
//! atomically from the registry, mirroring the frozen router's snapshot pattern.
//!
//! # Which frozen lexical component this composes with (IMPORTANT)
//!
//! Design §8.3 names `resolver::SkillIndex` / `resolver::Bm25Index` as the frozen
//! lexical half to reuse. **That module is not compiled** — `openclaw/mod.rs`
//! declares `// pub mod resolver; // A6: REMOVED - replaced by semantic_router`,
//! so `resolver.rs` is dead code. The live routing component is
//! [`SemanticSkillRouter`], which performs lexical matching *inline* via
//! token-overlap over a skill's name/description/categories (its private
//! `calculate_semantic_similarity`) directly from `get_enabled_skills()` — it
//! exposes **no reusable, public lexical index** to compose with.
//!
//! Per the task's explicit constraint we do **not** resurrect the removed
//! `resolver` module. Instead [`LexicalIndex`] is a CIL-owned **derived view**
//! (exactly like [`DenseIndex`] in `dense.rs`) built from the same registry
//! source of truth, keyed by `skill_id`, and rebuilt atomically — introducing no
//! second registry. It uses an **inverted-index BM25** (not a linear scan),
//! satisfying R11.1's "ANN dense + BM25 fusion, not a linear scan". The lexical
//! algorithm mirrors the frozen resolver's BM25 in spirit; the seam is honest and
//! documented so a future re-exposed frozen index can drop in behind it.
//!
//! # GoalIntent seam
//!
//! [`GoalIntent`](design §7.2) lands in task 5.1. To avoid a hard dependency,
//! [`CapabilityIndex::search`] takes the two things it actually needs *now* — a
//! `goal_embedding: &[f32]` (semantic query) and a `query_text: &str` (lexical
//! query) — a signature `GoalIntent { goal_embedding, raw, .. }` satisfies later
//! with no churn. [`CapabilityIndex::search_text`] is an async convenience that
//! embeds the query itself for callers/tests without a `GoalIntent`.
//!
//! # Signals populated here
//!
//! `search` returns [`CapabilityCandidate`]s with only the **semantic** and
//! **lexical** signals populated; `compatibility`/`trust`/`quality`/`popularity`/
//! `success` are left at `0.0` and filled by the `CapabilityRanker` (task 5.2).
//!
//! # Determinism (task 3.5)
//!
//! Both halves sort by descending score with a stable tie-break by `skill_id`,
//! and the fused candidate set is ordered the same way, so discovery is
//! reproducible for a fixed registry state.
//!
//! # No hardcoding (task 3.6)
//!
//! Every skill/capability string flows through one code path — tags are embedded
//! and indexed generically and metadata text is tokenized uniformly. There is no
//! per-skill or per-category branch; a never-before-seen `CapabilityTag` id is
//! embedded, indexed, and searchable identically.
//!
//! # Degraded honesty (task 3.7 structure)
//!
//! If the [`Embedder`] fails at rebuild time, the dense half is left empty and
//! discovery falls back to lexical-only — honestly, never a panic.
//!
//! [`SemanticSkillRouter`]: crate::openclaw::semantic_router::SemanticSkillRouter
//! [`DenseIndex`]: crate::openclaw::cil::dense::DenseIndex

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use arc_swap::ArcSwap;

use super::dense::{aggregate_provides, DenseIndex, DenseIndexHandle};
use super::embed::Embedder;
use super::extract::extract_profile;
use super::profile::{CapabilityProfile, CapabilityTag};
use super::CilError;
use crate::openclaw::registry::SkillMetadata;

/// Where a discovered capability candidate comes from (design §7.3).
///
/// [`CandidateSource::Installed`] is produced by [`CapabilityIndex`]
/// (installed-skill discovery, task 3.3). [`CandidateSource::Marketplace`] is
/// produced by the marketplace-discovery phase (task 6.4): the CIL facade maps
/// each `cil::market::MarketCandidate` from the pre-embedded `market_catalog`
/// cache into a [`CapabilityCandidate`] carrying this variant, so installed and
/// marketplace results live in one ranked candidate set. The `Generatable`
/// variant is filled by acquisition (task 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateSource {
    /// The candidate is an installed, enabled skill in `ProductionSkillRegistry`.
    Installed,
    /// The candidate is a marketplace catalog entry from the offline-embedded
    /// `market_catalog` cache (task 6.4). Carries the federated identity
    /// `(provider_id, slug)` so acquisition (task 8) can resolve the exact entry
    /// without a live per-query fetch (R9.2).
    Marketplace {
        /// Which marketplace this candidate came from (`market_catalog.provider_id`).
        provider_id: String,
        /// The stable marketplace skill identifier (`market_catalog.slug`).
        slug: String,
    },
    /// No installed/marketplace match; the A9 pipeline could synthesize it.
    /// (Produced by the acquisition phase, task 8.)
    Generatable,
}

/// A candidate capability discovered for a goal, with its ranking signals
/// (design §7.3).
///
/// [`CapabilityIndex::search`] populates only [`semantic`](Self::semantic) and
/// [`lexical`](Self::lexical); the remaining signals are left at `0.0` and filled
/// by the `CapabilityRanker` (task 5.2). All signals are conventionally in
/// `0.0..=1.0`.
#[derive(Debug, Clone)]
pub struct CapabilityCandidate {
    /// The capability this candidate provides (a representative `provides` tag,
    /// or a tag derived from the skill id when no profile tag is available).
    pub capability: CapabilityTag,
    /// The skill id, when the candidate corresponds to a concrete skill.
    pub skill_ref: Option<String>,
    /// Where the candidate came from.
    pub source: CandidateSource,
    /// The candidate's capability profile, when known (installed skills).
    pub profile: Option<CapabilityProfile>,

    // ---- Ranking signals (0.0..=1.0), combined by CapabilityRanker (task 5.2).
    /// Dense goal↔capability similarity. Populated here.
    pub semantic: f32,
    /// Lexical (BM25) similarity. Populated here.
    pub lexical: f32,
    /// I/O + runtime + dependency fit. Filled by the ranker (task 5.2).
    pub compatibility: f32,
    /// Publisher/trust-tier score. Filled by the ranker (task 5.2).
    pub trust: f32,
    /// Validator/quality metadata. Filled by the ranker (task 5.2).
    pub quality: f32,
    /// Install/usage counts. Filled by the ranker (task 5.2).
    pub popularity: f32,
    /// Historical success rate. Filled by the ranker (task 5.2).
    pub success: f32,
}

impl CapabilityCandidate {
    /// Build an installed-skill candidate with only semantic/lexical populated
    /// (the remaining signals default to `0.0` for the ranker to fill).
    fn installed(
        skill_id: String,
        profile: Option<CapabilityProfile>,
        semantic: f32,
        lexical: f32,
    ) -> Self {
        // Representative capability tag: first `provides` tag if the profile has
        // one, else a bare tag derived from the skill id (open vocabulary, no
        // per-skill branch).
        let capability = profile
            .as_ref()
            .and_then(|p| p.provides.first().cloned())
            .unwrap_or_else(|| CapabilityTag::new(skill_id.clone()));
        Self {
            capability,
            skill_ref: Some(skill_id),
            source: CandidateSource::Installed,
            profile,
            semantic,
            lexical,
            compatibility: 0.0,
            trust: 0.0,
            quality: 0.0,
            popularity: 0.0,
            success: 0.0,
        }
    }
}

// ─── Lexical (BM25) index ────────────────────────────────────────────────────

/// Default BM25 term-frequency saturation parameter.
const BM25_K1: f32 = 1.2;
/// Default BM25 length-normalization parameter.
const BM25_B: f32 = 0.75;

/// One indexed document: a skill id, its tokenized metadata length, and its
/// per-document term frequencies (sorted by term for deterministic rebuilds).
///
/// The `tf` list is what makes a **bounded incremental** [`LexicalIndex::with_upserted`]
/// possible: the inverted index can be reconstructed from the stored per-doc
/// frequencies without re-tokenizing every other skill — only the single
/// upserted skill is tokenized afresh.
#[derive(Debug, Clone)]
struct LexicalDoc {
    skill_id: String,
    len: usize,
    /// `(term, term_frequency)` for this document, sorted by term.
    tf: Vec<(String, f32)>,
}

/// A CIL-owned derived lexical index over installed skills — an **inverted-index
/// BM25** (not a linear scan) built from the registry source of truth.
///
/// See the module docs for why this is a fresh derived view rather than a reuse
/// of the removed `resolver::Bm25Index`. Immutable once built, so it is cheap to
/// hold behind an [`ArcSwap`] and swap atomically on rebuild.
#[derive(Debug, Clone, Default)]
pub struct LexicalIndex {
    /// term → postings `[(doc_index, term_frequency)]`.
    inverted: HashMap<String, Vec<(usize, f32)>>,
    docs: Vec<LexicalDoc>,
    avg_doc_len: f32,
}

impl LexicalIndex {
    /// An empty index. Used as the initial snapshot.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build a BM25 inverted index from `(skill_id, text)` documents.
    ///
    /// Documents are stored sorted by `skill_id` for deterministic construction
    /// and stable tie-breaks at search time.
    pub fn build(documents: impl IntoIterator<Item = (String, String)>) -> Self {
        // Deterministic doc order (sorted by skill_id).
        let mut sorted: Vec<(String, String)> = documents.into_iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));

        let mut docs: Vec<LexicalDoc> = Vec::with_capacity(sorted.len());
        for (skill_id, text) in sorted {
            docs.push(Self::make_doc(skill_id, &text));
        }
        Self::from_docs(docs)
    }

    /// Tokenize `text` into a [`LexicalDoc`] for `skill_id`. This is the ONLY
    /// place a document is tokenized; incremental upsert reuses it for the single
    /// changed skill, leaving every other skill's stored `tf` untouched.
    fn make_doc(skill_id: String, text: &str) -> LexicalDoc {
        let tokens = tokenize(text);
        let len = tokens.len();
        let mut tf: BTreeMap<String, f32> = BTreeMap::new();
        for tok in tokens {
            *tf.entry(tok).or_insert(0.0) += 1.0;
        }
        LexicalDoc {
            skill_id,
            len,
            tf: tf.into_iter().collect(),
        }
    }

    /// Assemble an index from documents already sorted by `skill_id`, rebuilding
    /// the inverted postings and average document length from the stored per-doc
    /// term frequencies.
    fn from_docs(docs: Vec<LexicalDoc>) -> Self {
        let mut inverted: HashMap<String, Vec<(usize, f32)>> = HashMap::new();
        let mut total_len: usize = 0;
        for (i, doc) in docs.iter().enumerate() {
            total_len += doc.len;
            for (term, freq) in &doc.tf {
                inverted.entry(term.clone()).or_default().push((i, *freq));
            }
        }
        let avg_doc_len = if docs.is_empty() {
            0.0
        } else {
            total_len as f32 / docs.len() as f32
        };
        Self {
            inverted,
            docs,
            avg_doc_len,
        }
    }

    /// Return a new index with `skill_id`'s document added (insert) or replaced
    /// (update) — the **bounded incremental** path used by
    /// [`CapabilityIndex::upsert`] (R11.3, task 3.4).
    ///
    /// Cost: only the single upserted skill is tokenized; the existing docs are
    /// cloned (an `O(existing)` copy) and the inverted postings are rebuilt from
    /// the stored per-doc term frequencies. Crucially it does **not** re-tokenize
    /// the other skills — that is what makes it "bounded" versus a full
    /// [`build`](Self::build)/[`from_skills`](Self::from_skills) reindex.
    /// The result stays sorted by `skill_id`, preserving deterministic tie-breaks.
    pub fn with_upserted(&self, skill_id: &str, text: &str) -> Self {
        let mut docs = self.docs.clone();
        let doc = Self::make_doc(skill_id.to_string(), text);
        match docs.binary_search_by(|d| d.skill_id.as_str().cmp(skill_id)) {
            Ok(i) => docs[i] = doc,        // replace existing skill
            Err(i) => docs.insert(i, doc), // insert new skill, keep sorted
        }
        Self::from_docs(docs)
    }

    /// Build from installed-skill metadata: indexes each skill's textual fields
    /// (name, description, category/categories, tags, skill id) uniformly.
    pub fn from_skills(skills: &[SkillMetadata]) -> Self {
        Self::build(skills.iter().map(|s| (s.skill_id.clone(), lexical_text(s))))
    }

    /// Number of indexed documents.
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// Whether the index holds no documents.
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Return up to `k` skills most relevant to `query` as `(skill_id, bm25)`
    /// pairs, sorted by descending BM25 score with a stable tie-break by
    /// `skill_id`. Scores are **raw** BM25 (not normalized); the fusion layer
    /// normalizes them to the `0.0..=1.0` signal range.
    pub fn search(&self, query: &str, k: usize) -> Vec<(String, f32)> {
        if k == 0 || self.docs.is_empty() {
            return Vec::new();
        }
        let query_terms = tokenize(query);
        if query_terms.is_empty() {
            return Vec::new();
        }

        let n = self.docs.len() as f32;
        let mut scores: HashMap<usize, f32> = HashMap::new();

        // Unique query terms (a repeated query term should not double-count IDF).
        let mut seen_terms: BTreeMap<String, ()> = BTreeMap::new();
        for term in query_terms {
            if seen_terms.insert(term.clone(), ()).is_some() {
                continue;
            }
            let Some(postings) = self.inverted.get(&term) else {
                continue;
            };
            let df = postings.len() as f32;
            // IDF: log(1 + (N - df + 0.5) / (df + 0.5)) — always non-negative.
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            for &(doc_idx, freq) in postings {
                let doc_len = self.docs[doc_idx].len as f32;
                let denom =
                    freq + BM25_K1 * (1.0 - BM25_B + BM25_B * doc_len / self.avg_doc_len.max(1.0));
                let term_score = idf * (freq * (BM25_K1 + 1.0)) / denom.max(f32::MIN_POSITIVE);
                *scores.entry(doc_idx).or_insert(0.0) += term_score;
            }
        }

        let mut results: Vec<(String, f32)> = scores
            .into_iter()
            .map(|(doc_idx, score)| (self.docs[doc_idx].skill_id.clone(), score))
            .collect();
        // Descending score; stable tie-break by skill_id for determinism.
        results.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        results.truncate(k);
        results
    }
}

/// Lock-free, atomically-rebuildable holder for the active [`LexicalIndex`],
/// mirroring [`DenseIndexHandle`] and the frozen router's ArcSwap pattern.
pub struct LexicalIndexHandle {
    snapshot: ArcSwap<LexicalIndex>,
}

impl Default for LexicalIndexHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl LexicalIndexHandle {
    /// A handle wrapping an empty index.
    pub fn new() -> Self {
        Self {
            snapshot: ArcSwap::from_pointee(LexicalIndex::empty()),
        }
    }

    /// Read the current snapshot (zero-contention `Arc` clone).
    pub fn load(&self) -> arc_swap::Guard<Arc<LexicalIndex>> {
        self.snapshot.load()
    }

    /// Atomically replace the active index.
    pub fn store(&self, index: LexicalIndex) {
        self.snapshot.store(Arc::new(index));
    }

    /// Rebuild from installed-skill metadata and swap in atomically.
    pub fn rebuild(&self, skills: &[SkillMetadata]) {
        self.store(LexicalIndex::from_skills(skills));
    }

    /// Bounded incremental upsert: build a fresh snapshot with `skill_id`'s
    /// document added/replaced (tokenizing only that one skill) and swap it in
    /// atomically (zero-downtime; in-flight readers keep the old snapshot).
    pub fn upsert(&self, skill_id: &str, text: &str) {
        let next = self.load().with_upserted(skill_id, text);
        self.store(next);
    }

    /// Lock-free BM25 search against the current snapshot.
    pub fn search(&self, query: &str, k: usize) -> Vec<(String, f32)> {
        self.load().search(query, k)
    }
}

/// Tokenize text for the lexical index: lowercase, split on any non-alphanumeric
/// boundary, drop empties. Uniform for every skill — no per-category rules.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

/// The lexical document text for a skill: its human/metadata fields joined.
/// Mirrors the live `SemanticSkillRouter` lexical surface (name + description +
/// categories) and adds tags + the skill id, all treated uniformly.
fn lexical_text(skill: &SkillMetadata) -> String {
    let mut parts: Vec<&str> = vec![
        &skill.skill_id,
        &skill.name,
        &skill.description,
        &skill.category,
    ];
    for c in &skill.categories {
        parts.push(c);
    }
    for t in &skill.tags {
        parts.push(t);
    }
    parts.join(" ")
}

// ─── CapabilityIndex (dense + lexical fusion) ─────────────────────────────────

/// The fused semantic + lexical discovery index over installed skills
/// (design §8.3).
///
/// Composes the dense index ([`DenseIndexHandle`], task 3.2), the CIL-owned
/// lexical BM25 index ([`LexicalIndexHandle`]), and an `Arc<dyn Embedder>` (task
/// 3.1). Both index halves are rebuilt atomically from the same source of truth
/// (`get_enabled_skills()`); the derived per-skill profiles are held alongside so
/// discovered candidates can carry their profile.
pub struct CapabilityIndex {
    /// Lexical BM25 half (frozen-pattern derived view; see module docs).
    lexical: LexicalIndexHandle,
    /// Dense semantic half (task 3.2).
    dense: DenseIndexHandle,
    /// Pluggable embedder (task 3.1); shared across rebuild/search.
    embedder: Arc<dyn Embedder>,
    /// Derived profiles keyed by `skill_id`, snapshotted for lock-free reads.
    profiles: ArcSwap<HashMap<String, CapabilityProfile>>,
    /// Cached per-skill aggregate `provides`-vectors (pre-normalization), keyed
    /// by `skill_id`. Snapshotted so a bounded incremental [`upsert`] can rebuild
    /// the flat dense index from cached vectors **without re-embedding every
    /// other skill** — only the upserted skill is embedded afresh (R11.3).
    ///
    /// [`upsert`]: CapabilityIndex::upsert
    dense_vectors: ArcSwap<BTreeMap<String, Vec<f32>>>,
    /// The embedder [`model_id`](Embedder::model_id) stamped at the last full
    /// [`rebuild`]. A change between this and the live embedder's model id means
    /// the derived embeddings are stale and a background reindex is warranted
    /// (R5.4). Held behind [`ArcSwap`] so the staleness check is lock-free.
    ///
    /// [`rebuild`]: CapabilityIndex::rebuild
    indexed_model_id: ArcSwap<String>,
    /// Monotonic reindex generation, bumped on every full [`rebuild`]. This is
    /// the in-memory analogue of the `capability_profiles.profile_epoch` column
    /// (design §8.2); it lets callers observe when a model-change reindex has
    /// completed and swapped in a new generation of derived views.
    ///
    /// [`rebuild`]: CapabilityIndex::rebuild
    profile_epoch: AtomicU64,
}

impl CapabilityIndex {
    /// Construct an empty index over the given embedder. Call [`rebuild`] with
    /// `get_enabled_skills()` to populate it.
    ///
    /// [`rebuild`]: CapabilityIndex::rebuild
    pub fn new(embedder: Arc<dyn Embedder>) -> Self {
        Self {
            lexical: LexicalIndexHandle::new(),
            dense: DenseIndexHandle::new(),
            embedder,
            profiles: ArcSwap::from_pointee(HashMap::new()),
            dense_vectors: ArcSwap::from_pointee(BTreeMap::new()),
            // Empty until the first rebuild — a never-indexed view is stale for
            // any real model id, so `needs_reindex_for` reports true initially.
            indexed_model_id: ArcSwap::from_pointee(String::new()),
            profile_epoch: AtomicU64::new(0),
        }
    }

    /// Rebuild both index halves from installed-skill metadata — the SAME source
    /// of truth as the frozen router (`ProductionSkillRegistry::get_enabled_skills()`),
    /// so there is no second registry (R5.1).
    ///
    /// Steps:
    /// 1. Derive a [`CapabilityProfile`] per skill via [`extract_profile`].
    /// 2. Embed the `provides`-tag ids (via the [`Embedder`]) to populate their
    ///    embeddings, so the dense half can aggregate them.
    /// 3. Rebuild the dense index from the embedded profiles and the lexical
    ///    index from the skill metadata, swapping both in atomically.
    ///
    /// Degraded honesty (task 3.7): if the embedder fails, the dense half is left
    /// empty and discovery falls back to lexical-only — never a panic. Async
    /// because embedding the tags requires the async [`Embedder`].
    pub async fn rebuild(&self, skills: &[SkillMetadata]) -> Result<(), CilError> {
        // 1. Derive profiles (deterministic, no I/O).
        let mut profiles: Vec<CapabilityProfile> = skills.iter().map(extract_profile).collect();

        // 2. Embed provides-tag ids. Collect unique ids for one batch call.
        let mut unique_ids: BTreeMap<String, ()> = BTreeMap::new();
        for p in &profiles {
            for tag in &p.provides {
                unique_ids.insert(tag.id.clone(), ());
            }
        }
        let ids: Vec<String> = unique_ids.into_keys().collect();

        if !ids.is_empty() {
            match self.embedder.embed_batch(&ids).await {
                Ok(vectors) => {
                    let map: HashMap<&str, &Vec<f32>> =
                        ids.iter().map(|s| s.as_str()).zip(vectors.iter()).collect();
                    for p in &mut profiles {
                        for tag in &mut p.provides {
                            if let Some(v) = map.get(tag.id.as_str()) {
                                tag.embedding = Some((*v).clone());
                            }
                        }
                    }
                }
                Err(_e) => {
                    // Degraded: embedder unavailable → empty dense half; lexical
                    // still serves discovery honestly (task 3.7). Leave the tag
                    // embeddings unset so `aggregate_provides` yields nothing.
                }
            }
        }

        // 3a. Dense half: cache each skill's aggregate `provides`-vector, then
        // build the flat dense index from the cache. Caching here is what lets a
        // later incremental `upsert` rebuild the dense index cheaply from the
        // stored vectors instead of re-embedding every skill. Skills whose tags
        // carry no embedding (degraded / no provides) contribute no vector.
        let dense_vectors: BTreeMap<String, Vec<f32>> = profiles
            .iter()
            .filter_map(|p| aggregate_provides(p).map(|v| (p.skill_id.clone(), v)))
            .collect();
        self.dense.store(DenseIndex::build(
            dense_vectors.iter().map(|(id, v)| (id.clone(), v.clone())),
        ));

        // 3b. Lexical half from metadata (independent of the embedder).
        self.lexical.rebuild(skills);

        // Snapshot profiles by skill_id for candidate attachment on search.
        let by_id: HashMap<String, CapabilityProfile> = profiles
            .into_iter()
            .map(|p| (p.skill_id.clone(), p))
            .collect();
        self.profiles.store(Arc::new(by_id));
        self.dense_vectors.store(Arc::new(dense_vectors));

        // Stamp the model id this generation was embedded with and bump the
        // reindex epoch (R5.4 versioning): a subsequent model change is now
        // detectable via `needs_reindex_for` / `is_stale`.
        self.indexed_model_id
            .store(Arc::new(self.embedder.model_id().to_string()));
        self.profile_epoch.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }

    /// Bounded incremental **upsert** of a single skill after an acquisition
    /// (design §8.3 / §10, R5.5 / R11.3, task 3.4).
    ///
    /// Adds or replaces exactly one skill in both index halves **without a full
    /// reindex**. The only per-skill-expensive work — profile extraction and
    /// embedding — is performed for the one upserted skill; every other skill's
    /// derived data is reused untouched.
    ///
    /// # Cost bound (R11.3)
    ///
    /// - **Embedding:** one `embed_batch` over just this skill's `provides` tags
    ///   (not all skills). This is the cost that a full [`rebuild`] would pay
    ///   `N` times over — the invariant this method exists to avoid.
    /// - **Profiles:** clone the `skill_id → profile` map and insert one entry —
    ///   an `O(N)` copy plus one map insert, then a single [`ArcSwap`] store.
    /// - **Lexical:** [`LexicalIndex::with_upserted`] tokenizes only this skill
    ///   and rebuilds postings from stored per-doc term frequencies (`O(N)` copy,
    ///   no re-tokenization of others).
    /// - **Dense:** clone the cached-vector map, insert/remove one vector, and
    ///   rebuild the flat index from the cache (`O(N)` normalize — cheap
    ///   arithmetic, **no re-embedding**).
    ///
    /// So upsert is `O(N)` in bookkeeping copies but only `O(1)` in the expensive
    /// embed/derive work, versus `O(N)` embeds for a full reindex. All swaps are
    /// atomic [`ArcSwap`] stores, so discovery stays available throughout
    /// (zero-downtime).
    ///
    /// # Degraded honesty (task 3.7)
    ///
    /// If the embedder fails, the lexical half is still upserted and the dense
    /// vector is skipped (and any stale dense entry for this `skill_id` removed) —
    /// the skill stays discoverable via BM25, never a panic.
    ///
    /// [`rebuild`]: CapabilityIndex::rebuild
    pub async fn upsert(&self, skill: &SkillMetadata) -> Result<(), CilError> {
        // 1. Derive the profile for THIS skill only (deterministic, no I/O).
        let mut profile = extract_profile(skill);

        // 2. Embed only this skill's unique `provides`-tag ids.
        let mut unique_ids: BTreeMap<String, ()> = BTreeMap::new();
        for tag in &profile.provides {
            unique_ids.insert(tag.id.clone(), ());
        }
        let ids: Vec<String> = unique_ids.into_keys().collect();
        let mut aggregate: Option<Vec<f32>> = None;
        if !ids.is_empty() {
            match self.embedder.embed_batch(&ids).await {
                Ok(vectors) => {
                    let map: HashMap<&str, &Vec<f32>> =
                        ids.iter().map(|s| s.as_str()).zip(vectors.iter()).collect();
                    for tag in &mut profile.provides {
                        if let Some(v) = map.get(tag.id.as_str()) {
                            tag.embedding = Some((*v).clone());
                        }
                    }
                    aggregate = aggregate_provides(&profile);
                }
                Err(_e) => {
                    // Degraded: no dense vector for this skill; lexical still
                    // covers it (honest, never a panic).
                }
            }
        }

        // 3. Profiles snapshot: clone-and-insert one, atomic store (bounded).
        let mut profiles = (**self.profiles.load()).clone();
        profiles.insert(skill.skill_id.clone(), profile);
        self.profiles.store(Arc::new(profiles));

        // 4. Lexical half: incremental, tokenizes only this skill.
        self.lexical.upsert(&skill.skill_id, &lexical_text(skill));

        // 5. Dense half: update the cached vector for this skill, then rebuild
        // the flat index from the cache (no re-embedding of other skills).
        let mut vectors = (**self.dense_vectors.load()).clone();
        match aggregate {
            Some(v) => {
                vectors.insert(skill.skill_id.clone(), v);
            }
            None => {
                // No usable vector (degraded / no provides): drop any stale dense
                // entry so discovery never returns a fabricated vector for it.
                vectors.remove(&skill.skill_id);
            }
        }
        self.dense.store(DenseIndex::build(
            vectors.iter().map(|(id, v)| (id.clone(), v.clone())),
        ));
        self.dense_vectors.store(Arc::new(vectors));

        Ok(())
    }

    /// The live embedder's model id (design §8.1). Compare against
    /// [`indexed_model_id`](Self::indexed_model_id) to detect model churn.
    pub fn current_model_id(&self) -> String {
        self.embedder.model_id().to_string()
    }

    /// The embedder model id the currently-indexed derived views were built with,
    /// stamped by the last [`rebuild`](Self::rebuild) (empty before any rebuild).
    pub fn indexed_model_id(&self) -> String {
        (**self.indexed_model_id.load()).clone()
    }

    /// The current reindex generation (bumped on every full [`rebuild`](Self::rebuild)).
    /// The in-memory analogue of `capability_profiles.profile_epoch`.
    pub fn profile_epoch(&self) -> u64 {
        self.profile_epoch.load(Ordering::SeqCst)
    }

    /// Whether the indexed derived views are stale for `model_id` — i.e. a full
    /// reindex is warranted because embeddings were built with a different model
    /// (R5.4). A caller (registry/scheduler) checks this after a model change and
    /// kicks a background [`rebuild`](Self::rebuild); the [`ArcSwap`] swap makes
    /// that reindex zero-downtime.
    pub fn needs_reindex_for(&self, model_id: &str) -> bool {
        self.indexed_model_id().as_str() != model_id
    }

    /// Convenience: whether the live embedder's model differs from the one the
    /// index was built with (`needs_reindex_for(current_model_id())`).
    pub fn is_stale(&self) -> bool {
        self.needs_reindex_for(&self.current_model_id())
    }

    /// Fuse dense + lexical retrieval into a ranked candidate set (design §8.3).
    ///
    /// Runs dense search over `goal_embedding` (semantic signal) and BM25 search
    /// over `query_text` (lexical signal), merges by `skill_id` into a union of
    /// up to `2*k` retrieved skills, populates `semantic`/`lexical` per candidate
    /// (BM25 min-max normalized into `0.0..=1.0`; cosine clamped to `0.0..=1.0`),
    /// and returns the top `k` ordered by the combined preliminary signal with a
    /// **stable tie-break by `skill_id`** (deterministic; the real multi-signal
    /// ranking is applied later by `CapabilityRanker`, task 5.2).
    ///
    /// This is the `GoalIntent` seam: `goal_embedding` + `query_text` is what a
    /// `GoalIntent { goal_embedding, raw, .. }` supplies in task 5.x.
    pub fn search(
        &self,
        goal_embedding: &[f32],
        query_text: &str,
        k: usize,
    ) -> Vec<CapabilityCandidate> {
        if k == 0 {
            return Vec::new();
        }
        // Overfetch each half so the fused union has room before truncation.
        let fetch = k.saturating_mul(2).max(k);

        let dense_hits = self.dense.search(goal_embedding, fetch);
        let lexical_hits = self.lexical.search(query_text, fetch);

        // Normalize BM25 to 0..1 by the max score in this result set (matches the
        // frozen router's min-max normalization; empty/zero-max → 0.0).
        let max_bm25 = lexical_hits.iter().map(|(_, s)| *s).fold(0.0f32, f32::max);

        // Merge by skill_id.
        let mut merged: HashMap<String, (f32, f32)> = HashMap::new();
        for (skill_id, score) in dense_hits {
            // Cosine similarity → clamp to [0,1] for the semantic signal.
            let semantic = score.clamp(0.0, 1.0);
            merged.entry(skill_id).or_insert((0.0, 0.0)).0 = semantic;
        }
        for (skill_id, score) in lexical_hits {
            let lexical = if max_bm25 > 0.0 {
                (score / max_bm25).clamp(0.0, 1.0)
            } else {
                0.0
            };
            merged.entry(skill_id).or_insert((0.0, 0.0)).1 = lexical;
        }

        let profiles = self.profiles.load();
        let mut candidates: Vec<CapabilityCandidate> = merged
            .into_iter()
            .map(|(skill_id, (semantic, lexical))| {
                let profile = profiles.get(&skill_id).cloned();
                CapabilityCandidate::installed(skill_id, profile, semantic, lexical)
            })
            .collect();

        // Deterministic ordering: combined preliminary signal desc, then
        // skill_id asc as a stable tie-break.
        candidates.sort_by(|a, b| {
            let ca = a.semantic + a.lexical;
            let cb = b.semantic + b.lexical;
            cb.total_cmp(&ca).then_with(|| {
                a.skill_ref
                    .as_deref()
                    .unwrap_or("")
                    .cmp(b.skill_ref.as_deref().unwrap_or(""))
            })
        });
        candidates.truncate(k);
        candidates
    }

    /// Async convenience: embed `query_text` with the composed embedder, then
    /// run [`search`](CapabilityIndex::search). Lets callers/tests discover
    /// without constructing a `GoalIntent`. On embedder failure the semantic
    /// half is skipped (empty embedding) → lexical-only discovery (honest).
    pub async fn search_text(&self, query_text: &str, k: usize) -> Vec<CapabilityCandidate> {
        let goal_embedding = self.embedder.embed(query_text).await.unwrap_or_default();
        self.search(&goal_embedding, query_text, k)
    }

    /// Number of installed skills currently indexed (lexical half).
    pub fn len(&self) -> usize {
        self.lexical.load().len()
    }

    /// Whether the index holds no skills.
    pub fn is_empty(&self) -> bool {
        self.lexical.load().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::cil::embed::MemoryEmbedder;
    use crate::openclaw::registry::{DiscoverySource, SkillMetadata, SkillState};
    use crate::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};
    use crate::safety::RiskLevel;

    fn embedder(dim: usize) -> Arc<dyn Embedder> {
        // Hash-fallback backend: deterministic, no model download in CI.
        Arc::new(MemoryEmbedder::load(dim).expect("embedder load"))
    }

    fn skill(skill_id: &str, name: &str, description: &str, categories: &[&str]) -> SkillMetadata {
        SkillMetadata {
            skill_id: skill_id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            publisher: "test".to_string(),
            version: "1.0.0".to_string(),
            category: categories.first().copied().unwrap_or("misc").to_string(),
            discovery_source: DiscoverySource::Bundled {
                path: "test".to_string(),
            },
            discovered_at: chrono::Utc::now(),
            capabilities: SkillCapabilities::default(),
            runtime_requirements: "docker".to_string(),
            risk_level: RiskLevel::Green,
            resource_class: ResourceClass::Light,
            tags: vec![],
            categories: categories.iter().map(|s| s.to_string()).collect(),
            semantic_version: "1.0.0".to_string(),
            dependencies: vec![],
            compatibility_requirements: vec![],
            trust_tier: TrustTier::Local,
            content_hash: format!("hash_{skill_id}"),
            signature: None,
            granted_capabilities: Vec::new(),
            bundle_path: None,
            manifest_toml: None,
            input_schema: None,
            state: SkillState::Enabled,
            state_changed_at: chrono::Utc::now(),
        }
    }

    fn sample_skills() -> Vec<SkillMetadata> {
        vec![
            skill(
                "acme.pdf",
                "PDF Compressor",
                "compress and shrink pdf documents",
                &["doc.pdf.compress"],
            ),
            skill(
                "acme.ocr",
                "Image OCR",
                "extract text from images via ocr",
                &["media.image.ocr"],
            ),
            skill(
                "acme.email",
                "Email Sender",
                "send email messages over smtp",
                &["net.email.send"],
            ),
        ]
    }

    // ─── Lexical BM25 smoke ──────────────────────────────────────────────────

    #[test]
    fn lexical_bm25_finds_keyword_match() {
        let idx = LexicalIndex::from_skills(&sample_skills());
        assert_eq!(idx.len(), 3);
        let hits = idx.search("compress pdf", 10);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].0, "acme.pdf");
    }

    #[test]
    fn lexical_empty_query_and_empty_index_are_empty() {
        let idx = LexicalIndex::from_skills(&sample_skills());
        assert!(idx.search("", 5).is_empty());
        assert!(LexicalIndex::empty().search("anything", 5).is_empty());
        assert!(idx.search("pdf", 0).is_empty());
    }

    #[test]
    fn lexical_ties_break_by_skill_id_deterministically() {
        // Two skills with identical text → identical BM25 → order by skill_id.
        let skills = vec![
            skill("zzz.dup", "Dup", "identical body text", &["c"]),
            skill("aaa.dup", "Dup", "identical body text", &["c"]),
        ];
        let idx = LexicalIndex::from_skills(&skills);
        let hits = idx.search("identical body text", 10);
        assert_eq!(hits[0].0, "aaa.dup");
        assert_eq!(hits[1].0, "zzz.dup");
    }

    // ─── CapabilityIndex fusion ──────────────────────────────────────────────

    #[tokio::test]
    async fn rebuild_then_search_populates_semantic_and_lexical() {
        let idx = CapabilityIndex::new(embedder(64));
        let skills = sample_skills();
        idx.rebuild(&skills).await.expect("rebuild");
        assert_eq!(idx.len(), 3);

        let results = idx.search_text("compress pdf documents", 3).await;
        assert!(!results.is_empty());
        // The pdf skill should surface with a positive lexical signal.
        let pdf = results
            .iter()
            .find(|c| c.skill_ref.as_deref() == Some("acme.pdf"))
            .expect("pdf candidate present");
        assert_eq!(pdf.source, CandidateSource::Installed);
        assert!(pdf.profile.is_some());
        assert!(pdf.lexical > 0.0, "lexical signal must be populated");
        // Other signals are left for the ranker (task 5.2).
        assert_eq!(pdf.compatibility, 0.0);
        assert_eq!(pdf.trust, 0.0);
    }

    #[tokio::test]
    async fn search_respects_k_and_is_deterministic() {
        let idx = CapabilityIndex::new(embedder(48));
        idx.rebuild(&sample_skills()).await.expect("rebuild");

        let a = idx.search_text("send an email", 2).await;
        let b = idx.search_text("send an email", 2).await;
        assert!(a.len() <= 2);
        // Same query, same state → identical ordering (deterministic).
        let ids_a: Vec<_> = a.iter().map(|c| c.skill_ref.clone()).collect();
        let ids_b: Vec<_> = b.iter().map(|c| c.skill_ref.clone()).collect();
        assert_eq!(ids_a, ids_b);
    }

    #[tokio::test]
    async fn rebuild_is_idempotent_for_fixed_skill_set() {
        // Property 1 smoke (task 3.5): rebuilding from the same source of truth
        // yields identical search results.
        let skills = sample_skills();
        let idx1 = CapabilityIndex::new(embedder(64));
        idx1.rebuild(&skills).await.expect("rebuild 1");
        let idx2 = CapabilityIndex::new(embedder(64));
        idx2.rebuild(&skills).await.expect("rebuild 2");

        let emb = embedder(64);
        let q = emb
            .embed("extract text from an image")
            .await
            .expect("embed");
        let r1 = idx1.search(&q, "extract text from an image", 3);
        let r2 = idx2.search(&q, "extract text from an image", 3);
        let ids1: Vec<_> = r1
            .iter()
            .map(|c| (c.skill_ref.clone(), c.semantic, c.lexical))
            .collect();
        let ids2: Vec<_> = r2
            .iter()
            .map(|c| (c.skill_ref.clone(), c.semantic, c.lexical))
            .collect();
        assert_eq!(ids1, ids2);
    }

    #[tokio::test]
    async fn novel_capability_tag_flows_through_unchanged() {
        // Property 2 smoke (task 3.6): a never-before-seen capability id is
        // embedded, indexed, and searchable through the same code path.
        let novel = skill(
            "novel.skill",
            "Quantum Router",
            "route entangled qubits across the mesh",
            &["quantum.entangle.route.v9"],
        );
        let idx = CapabilityIndex::new(embedder(64));
        idx.rebuild(&[novel]).await.expect("rebuild");
        let results = idx.search_text("quantum entangle route", 5).await;
        assert!(results
            .iter()
            .any(|c| c.skill_ref.as_deref() == Some("novel.skill")));
    }

    #[tokio::test]
    async fn empty_registry_yields_empty_discovery() {
        let idx = CapabilityIndex::new(embedder(32));
        idx.rebuild(&[]).await.expect("rebuild empty");
        assert!(idx.is_empty());
        assert!(idx.search_text("anything", 5).await.is_empty());
    }

    // ─── Incremental upsert (task 3.4) ───────────────────────────────────────

    #[test]
    fn lexical_with_upserted_inserts_and_replaces() {
        // Bounded incremental lexical path: insert a new doc, then replace it.
        let idx = LexicalIndex::from_skills(&sample_skills());
        assert_eq!(idx.len(), 3);

        // Insert a brand-new skill; it becomes searchable, existing docs remain.
        let inserted = idx.with_upserted("acme.zip", "zip and archive files into a bundle");
        assert_eq!(inserted.len(), 4);
        assert_eq!(inserted.search("archive bundle", 5)[0].0, "acme.zip");
        assert_eq!(inserted.search("compress pdf", 5)[0].0, "acme.pdf");

        // Replace an existing skill's text; the new terms win, count unchanged.
        let replaced = inserted.with_upserted("acme.zip", "totally different words here");
        assert_eq!(replaced.len(), 4);
        assert!(replaced.search("archive bundle", 5).is_empty());
        assert_eq!(replaced.search("totally different", 5)[0].0, "acme.zip");
    }

    #[tokio::test]
    async fn upsert_adds_searchable_skill_without_full_rebuild() {
        let idx = CapabilityIndex::new(embedder(64));
        idx.rebuild(&sample_skills()).await.expect("rebuild");
        let epoch = idx.profile_epoch();
        assert_eq!(idx.len(), 3);

        // Acquire a new skill incrementally (no rebuild).
        let new_skill = skill(
            "acme.zip",
            "Zip Archiver",
            "compress files into a zip archive",
            &["archive.zip.create"],
        );
        idx.upsert(&new_skill).await.expect("upsert");

        // It is now discoverable through the same fused search path...
        assert_eq!(idx.len(), 4);
        let results = idx.search_text("create a zip archive", 5).await;
        let hit = results
            .iter()
            .find(|c| c.skill_ref.as_deref() == Some("acme.zip"))
            .expect("upserted skill is discoverable");
        assert!(hit.profile.is_some());
        assert!(hit.lexical > 0.0);
        // ...and no full reindex happened: the epoch is unchanged by upsert.
        assert_eq!(
            idx.profile_epoch(),
            epoch,
            "upsert must not trigger a reindex"
        );
        // Pre-existing skills stay discoverable.
        assert!(idx
            .search_text("compress pdf documents", 5)
            .await
            .iter()
            .any(|c| c.skill_ref.as_deref() == Some("acme.pdf")));
    }

    #[tokio::test]
    async fn upsert_replaces_existing_skill() {
        let idx = CapabilityIndex::new(embedder(64));
        idx.rebuild(&sample_skills()).await.expect("rebuild");
        assert_eq!(idx.len(), 3);

        // Replace acme.email's metadata with an unrelated capability.
        let replacement = skill(
            "acme.email",
            "Calendar Scheduler",
            "schedule calendar meetings and reminders",
            &["time.calendar.schedule"],
        );
        idx.upsert(&replacement).await.expect("upsert replace");

        // Count is unchanged (replace, not insert)...
        assert_eq!(idx.len(), 3);
        // ...the new text is searchable...
        assert!(idx
            .search_text("schedule a calendar meeting", 5)
            .await
            .iter()
            .any(|c| c.skill_ref.as_deref() == Some("acme.email")));
        // ...and the profile snapshot reflects the new capability.
        let results = idx.search_text("schedule a calendar meeting", 5).await;
        let hit = results
            .iter()
            .find(|c| c.skill_ref.as_deref() == Some("acme.email"))
            .expect("replaced skill present");
        assert!(hit
            .profile
            .as_ref()
            .unwrap()
            .provides
            .iter()
            .any(|t| t.id == "time.calendar.schedule"));
    }

    #[tokio::test]
    async fn model_id_staleness_detects_reindex_need() {
        let idx = CapabilityIndex::new(embedder(64));
        // Before any rebuild the view is stale for the live model.
        assert!(idx.is_stale());
        assert_eq!(idx.profile_epoch(), 0);

        idx.rebuild(&sample_skills()).await.expect("rebuild");
        // After a rebuild it is fresh for the current model and the epoch bumped.
        assert!(!idx.is_stale());
        assert_eq!(idx.profile_epoch(), 1);
        assert_eq!(idx.indexed_model_id(), idx.current_model_id());

        // A different model id would warrant a background reindex (R5.4).
        assert!(idx.needs_reindex_for("some-other-model-d999"));
        assert!(!idx.needs_reindex_for(&idx.current_model_id()));

        // A second full rebuild bumps the epoch again (new generation).
        idx.rebuild(&sample_skills()).await.expect("rebuild 2");
        assert_eq!(idx.profile_epoch(), 2);
    }

    // ─── Degraded fallback when embedder unavailable (task 3.7, R13.1/R13.2) ──

    /// A test-only [`Embedder`] whose `embed`/`embed_batch` always fail — models
    /// an embedding backend that could not load / is unavailable. `dim`/`model_id`
    /// return sensible constants so the composed index constructs normally; only
    /// the *vector production* path fails, exercising the degraded branch in
    /// [`CapabilityIndex::rebuild`] (R13.1).
    struct FailingEmbedder {
        dim: usize,
        model_id: String,
    }

    impl FailingEmbedder {
        fn new(dim: usize) -> Self {
            Self {
                dim,
                model_id: format!("failing-embedder-d{dim}"),
            }
        }
    }

    #[async_trait::async_trait]
    impl Embedder for FailingEmbedder {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>, CilError> {
            Err(CilError::Embed("embedder unavailable (test)".to_string()))
        }

        async fn embed_batch(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, CilError> {
            Err(CilError::Embed("embedder unavailable (test)".to_string()))
        }

        fn dim(&self) -> usize {
            self.dim
        }

        fn model_id(&self) -> &str {
            &self.model_id
        }
    }

    #[tokio::test]
    async fn rebuild_falls_back_to_frozen_bm25_when_embedder_unavailable() {
        // R13.1: embedder fails to produce vectors → rebuild must NOT error or
        // panic; the dense half is empty but lexical is fully populated, so
        // discovery honestly degrades to frozen-BM25-only.
        let idx = CapabilityIndex::new(Arc::new(FailingEmbedder::new(64)));
        let skills = sample_skills();

        // Does NOT error/panic despite every embed call failing.
        idx.rebuild(&skills)
            .await
            .expect("rebuild degrades, never errors");

        // Lexical half reflects the indexed skills (frozen BM25 is populated).
        assert_eq!(idx.len(), 3);
        assert!(!idx.is_empty());

        // A discovery query still returns candidates via BM25 lexical signal.
        let results = idx.search_text("compress pdf documents", 3).await;
        assert!(
            !results.is_empty(),
            "BM25 fallback must still return candidates"
        );
        let pdf = results
            .iter()
            .find(|c| c.skill_ref.as_deref() == Some("acme.pdf"))
            .expect("pdf candidate present via lexical fallback");
        // Lexical signal populated (frozen BM25 works)...
        assert!(
            pdf.lexical > 0.0,
            "lexical signal must be populated in degraded mode"
        );
        // ...while the semantic signal is exactly 0.0 (dense half is empty —
        // degraded is NOT presented as full-fidelity, R13.2).
        assert_eq!(
            pdf.semantic, 0.0,
            "no semantic signal when embedder is unavailable"
        );
        // Every candidate honestly carries a zero semantic signal.
        assert!(
            results.iter().all(|c| c.semantic == 0.0),
            "no candidate may fabricate a semantic score in degraded mode"
        );
    }

    #[tokio::test]
    async fn dense_query_yields_no_semantic_hits_while_lexical_still_works() {
        // R13.1/R13.2: even when a caller supplies a goal embedding, the empty
        // dense half yields no semantic hits — discovery degrades (lexical only)
        // rather than failing.
        let idx = CapabilityIndex::new(Arc::new(FailingEmbedder::new(64)));
        idx.rebuild(&sample_skills())
            .await
            .expect("rebuild degrades");

        // A non-empty (but arbitrary) goal embedding of the right dimension.
        let goal_embedding = vec![0.1f32; 64];
        let results = idx.search(&goal_embedding, "extract text from images via ocr", 3);

        assert!(!results.is_empty(), "lexical discovery must still work");
        // No semantic hits: the dense index is empty, so semantic stays 0.0.
        assert!(
            results.iter().all(|c| c.semantic == 0.0),
            "dense-only query must yield zero semantic signal when dense half is empty"
        );
        // Lexical still surfaces the OCR skill for its keywords.
        assert!(
            results
                .iter()
                .any(|c| c.skill_ref.as_deref() == Some("acme.ocr") && c.lexical > 0.0),
            "lexical signal must still surface the matching skill"
        );
    }

    #[tokio::test]
    async fn search_with_empty_goal_embedding_still_returns_lexical_candidates() {
        // On embedder failure `search_text` embeds to an empty vector; assert the
        // core `search` handles an empty goal_embedding by still returning lexical
        // candidates (never panics), matching what degraded discovery produces.
        let idx = CapabilityIndex::new(Arc::new(FailingEmbedder::new(64)));
        idx.rebuild(&sample_skills())
            .await
            .expect("rebuild degrades");

        // Empty goal_embedding is exactly what `search_text` yields when the
        // embedder fails (`embed(..).unwrap_or_default()`).
        let results = idx.search(&[], "send email messages over smtp", 3);
        assert!(
            !results.is_empty(),
            "empty embedding must still yield lexical candidates"
        );
        assert!(
            results.iter().all(|c| c.semantic == 0.0),
            "empty goal embedding must produce no semantic signal"
        );
        assert!(
            results
                .iter()
                .any(|c| c.skill_ref.as_deref() == Some("acme.email") && c.lexical > 0.0),
            "lexical fallback must surface the email skill"
        );
    }

    // ─── Scale tests (task 16.1) — #[ignore]d; they take minutes ─────────────
    //
    // Validate the fused dense + frozen-BM25 `CapabilityIndex` at 1k and 10k
    // synthetic skills, each carrying a DISTINCT open-vocabulary `CapabilityTag`
    // (including a never-before-seen `zzz.neverseen.quantum.flux.*` domain). They
    // assert the two runtime-authority invariants that must hold *at scale*:
    //
    //   • Property 2 (No-hardcoding / open extensibility, R1.1) — every novel
    //     synthetic tag is embedded, indexed, and searchable through the SAME
    //     `rebuild` → `search` code path, with no per-capability branch.
    //   • Property 1 (Single source of truth / idempotent reindex, R5.1) —
    //     rebuilding from the same synthetic registry yields byte-identical
    //     `search` results even with 10k skills.
    //
    // They also MEASURE and print (against the ~1000-skill benchmark baseline):
    //   • reindex time            — full `rebuild` duration (R11.1),
    //   • discovery latency       — mean `search_text` time over many queries,
    //   • incremental upsert cost  — one `upsert` vs. a full reindex (R11.3),
    //   • memory                   — an index-size proxy (dense vectors + docs).
    //
    // A hash-fallback `MemoryEmbedder` is used (no model/network), mirroring the
    // other index.rs tests, so these are deterministic and CI-safe — just slow,
    // hence `#[ignore]`. Run explicitly with:
    //   cargo test -p kria-core --lib openclaw::cil::index -- --ignored --nocapture
    // or a single size by exact name, e.g. `scale_1k_synthetic_skills`.

    /// Embedding dimension for the scale runs — small enough to stay fast at 10k
    /// yet exercises the real dense-fusion path (no special-casing by dim).
    const SCALE_DIM: usize = 64;

    /// Build `n` synthetic skills, each with a DISTINCT open-vocabulary
    /// `CapabilityTag`. Every 500th skill uses a wholly never-before-seen domain
    /// (`zzz.neverseen.quantum.flux.*`) so the set includes tags no code has ever
    /// enumerated — the crux of the Property 2 assertions below. Each skill also
    /// gets a unique `token{i}` term in its description for precise lexical pins.
    fn synthetic_skills(n: usize) -> Vec<SkillMetadata> {
        (0..n)
            .map(|i| {
                // A DISTINCT capability tag per skill (open vocabulary, no enum).
                // Sprinkle a never-before-seen domain to prove no-hardcoding.
                let tag = if i % 500 == 0 {
                    format!("zzz.neverseen.quantum.flux.op{i}.v9")
                } else {
                    format!("synth.domain{}.capability.op{i}", i % 37)
                };
                let id = format!("synth.skill.{i:06}");
                let name = format!("Synthetic Skill {i}");
                let desc =
                    format!("performs synthetic operation {i} for capability token{i} {tag}");
                skill(&id, &name, &desc, &[tag.as_str()])
            })
            .collect()
    }

    /// A rough resident-size proxy for the index: dense vectors (`count * dim *
    /// f32`) + lexical doc count. Not an assertion target — just a printed signal
    /// to track memory growth 1k → 10k against the baseline.
    fn approx_index_bytes(n: usize, dim: usize) -> usize {
        // dense: n aggregate vectors of `dim` f32; lexical: ~one posting/token +
        // per-doc bookkeeping. Coarse but monotonic in n — enough to trend.
        n * dim * std::mem::size_of::<f32>() + n * 128
    }

    /// Shared scale scenario: build an index over `n` synthetic skills and assert
    /// Property 2 + Property 1 at that scale while measuring the four metrics.
    async fn run_scale(n: usize) {
        let skills = synthetic_skills(n);
        assert_eq!(skills.len(), n);

        // ---- Reindex time (R11.1): full rebuild from the registry view. -------
        let idx = CapabilityIndex::new(embedder(SCALE_DIM));
        let t_reindex = std::time::Instant::now();
        idx.rebuild(&skills).await.expect("rebuild at scale");
        let reindex = t_reindex.elapsed();
        assert_eq!(idx.len(), n, "every synthetic skill must be indexed");
        assert_eq!(idx.profile_epoch(), 1);

        // ---- Property 2 (R1.1): the never-before-seen tag flows through. ------
        // Skill 0 carries the novel `zzz.neverseen.quantum.flux.op0.v9` domain.
        // It must be (a) searchable through the same fused `search` path, and
        // (b) carry the novel tag verbatim in its derived profile — no branch,
        // no enumeration, no code change for a brand-new capability domain.
        let novel_results = idx.search_text("neverseen quantum flux op0 v9", 10).await;
        let novel = novel_results
            .iter()
            .find(|c| c.skill_ref.as_deref() == Some("synth.skill.000000"))
            .expect("never-before-seen capability tag must be discoverable at scale");
        assert!(
            novel
                .profile
                .as_ref()
                .expect("novel candidate carries its derived profile")
                .provides
                .iter()
                .any(|t| t.id == "zzz.neverseen.quantum.flux.op0.v9"),
            "the novel tag must flow into the profile verbatim (no hardcoding)"
        );

        // A distinct novel domain instance (op500) is ALSO reachable by its own
        // unique token — proving each novel tag is independently indexed, not a
        // special first-element case.
        if n > 500 {
            let other = idx.search_text("token500", 10).await;
            assert!(
                other
                    .iter()
                    .any(|c| c.skill_ref.as_deref() == Some("synth.skill.000500")),
                "each distinct synthetic tag must be independently searchable"
            );
        }

        // ---- Discovery latency: mean `search_text` over many varied queries. --
        // Queries hit distinct skills across the vocabulary so we exercise the
        // dense+lexical fusion broadly rather than a single hot path.
        let query_count = 50usize;
        let t_search = std::time::Instant::now();
        for q in 0..query_count {
            let i = (q * (n / query_count).max(1)) % n;
            let hits = idx.search_text(&format!("token{i}"), 5).await;
            assert!(
                !hits.is_empty(),
                "discovery must return candidates for token{i} at scale"
            );
        }
        let mean_search = t_search.elapsed() / query_count as u32;

        // ---- Incremental upsert cost (R11.3): one upsert must NOT reindex. ----
        // Add a brand-new skill with yet another never-before-seen tag and time
        // just the incremental path. Assert it is discoverable and that no full
        // reindex happened (epoch unchanged) — bounded cost, not O(N) re-embed.
        let epoch_before = idx.profile_epoch();
        let acquired = skill(
            "synth.acquired.late",
            "Late Acquired Skill",
            "freshly acquired capability neverbeforeindexed sentinel",
            &["zzz.neverseen.late.acquire.sentinel.v1"],
        );
        let t_upsert = std::time::Instant::now();
        idx.upsert(&acquired)
            .await
            .expect("incremental upsert at scale");
        let upsert = t_upsert.elapsed();
        assert_eq!(idx.len(), n + 1, "upsert adds exactly one skill");
        assert_eq!(
            idx.profile_epoch(),
            epoch_before,
            "incremental upsert must NOT trigger a full reindex (R11.3)"
        );
        assert!(
            idx.search_text("neverbeforeindexed sentinel", 10)
                .await
                .iter()
                .any(|c| c.skill_ref.as_deref() == Some("synth.acquired.late")),
            "an incrementally-acquired novel skill must be immediately discoverable"
        );

        // ---- Property 1 (R5.1): idempotent reindex at scale. ------------------
        // A second, independent index built from the SAME synthetic registry must
        // yield byte-identical `search` results (skill_ref + signals) for a fixed
        // query — reindexing from the single source of truth is deterministic.
        // NB: `idx` had a late upsert applied above, so compare two *fresh*
        // rebuilds of the original synthetic set (idx_a vs idx2) — the upsert
        // must not perturb the idempotency comparison.
        let idx_a = CapabilityIndex::new(embedder(SCALE_DIM));
        idx_a
            .rebuild(&skills)
            .await
            .expect("fresh rebuild A at scale");
        let idx2 = CapabilityIndex::new(embedder(SCALE_DIM));
        idx2.rebuild(&skills)
            .await
            .expect("fresh rebuild B at scale");
        let probe = embedder(SCALE_DIM)
            .embed("synthetic operation token7")
            .await
            .expect("probe embed");
        let r1 = idx_a.search(&probe, "synthetic operation token7", 8);
        let r2 = idx2.search(&probe, "synthetic operation token7", 8);
        let sig = |c: &CapabilityCandidate| (c.skill_ref.clone(), c.semantic, c.lexical);
        let s1: Vec<_> = r1.iter().map(sig).collect();
        let s2: Vec<_> = r2.iter().map(sig).collect();
        assert_eq!(
            s1, s2,
            "rebuilding from the same source of truth must be idempotent at scale (Property 1)"
        );

        // ---- Report (visible under --nocapture) vs. the baseline. -------------
        println!(
            "[scale n={n}] reindex={reindex:?} mean_search={mean_search:?} \
             upsert={upsert:?} approx_mem={} KiB (dim={SCALE_DIM})",
            approx_index_bytes(n, SCALE_DIM) / 1024
        );
    }

    /// 1,000 synthetic skills — the existing benchmark baseline size. Fast enough
    /// to be the representative run; still `#[ignore]`d to keep the default suite
    /// snappy.
    #[tokio::test]
    #[ignore = "scale test (1k synthetic skills) — run with --ignored"]
    async fn scale_1k_synthetic_skills() {
        run_scale(1_000).await;
    }

    /// 10,000 synthetic skills — the R11 scale target. Takes minutes; run
    /// explicitly with a generous timeout.
    #[tokio::test]
    #[ignore = "scale test (10k synthetic skills) — run with --ignored"]
    async fn scale_10k_synthetic_skills() {
        run_scale(10_000).await;
    }
}
