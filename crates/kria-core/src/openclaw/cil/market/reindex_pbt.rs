//! Property-based test for the **single-source-of-truth / idempotent-reindex**
//! invariant of the federated [`MarketIndex`] over the `market_catalog` derived
//! table — task 6.5.
//!
//! # Property 1 — Single source of truth (design §Correctness Properties)
//!
//! **Validates: Requirements 5.1**
//!
//! Requirement 5.1: *"rebuilding all derived views from the registry yields
//! identical query results (idempotent reindex)."* Where [`index_reindex_pbt`]
//! covers the in-memory installed-discovery index and [`profile_reindex_pbt`]
//! the persisted `capability_profiles` projection, this test covers the
//! **federated marketplace read model**: the pre-embedded `market_catalog`
//! cache that [`MarketIndex::sync`] rebuilds and [`MarketIndex::search`] ranks.
//!
//! `market_catalog` is a **rebuildable derived view** keyed by its PK
//! `(provider_id, slug)`: every row is re-derivable by re-syncing the providers,
//! and each write is an `INSERT OR REPLACE`, so a re-sync is idempotent by
//! construction. Because the mock [`MockEmbedder`] (FNV bag-of-tokens) and the
//! mock provider's catalog are pure deterministic functions of the generated
//! entry set, a re-sync + search is a well-defined function of that set alone.
//! This test encodes two facets:
//!
//! 1. **Cross-instance determinism** — two independently built indexes (`A` and
//!    `B`), each over a FRESH `skills.db` synced from the SAME entry set, return
//!    byte-identical `search` results for every goal embedding. Two catalogs →
//!    one source of truth → one answer.
//! 2. **Idempotence** — re-syncing the SAME index a second time from the same
//!    entry set produces search results identical to the first sync. A reindex
//!    is a no-op on query results when the source of truth is unchanged.
//!
//! Each candidate is compared as
//! `(provider_id, slug, version, score_bits, deprecated, quality_bits,
//! popularity_bits)` with the `f32`/`f64` signals taken as raw IEEE-754 bits
//! ([`f32::to_bits`] / [`f64::to_bits`]), and the candidate list is sorted by
//! `(provider_id, slug)` before comparison — so the assertion is exact and
//! order-insensitive to the ranker's tie-break, catching any non-determinism
//! (ordering, hashing, accumulation drift) rather than tolerating it.
//!
//! The generator produces 0..=12 entries with **unique** `(provider_id, slug)`
//! and varying `name`/`description`/`category`/`version`/`quality`/`popularity`/
//! `deprecated`, so the reindex is exercised across the open-vocabulary input
//! space (no hardcoding).
//!
//! [`index_reindex_pbt`]: crate::openclaw::cil::index_reindex_pbt
//! [`profile_reindex_pbt`]: crate::openclaw::cil::profile_reindex_pbt
//! [`MarketIndex`]: crate::openclaw::cil::market::index::MarketIndex

use std::sync::Arc;

use async_trait::async_trait;
use proptest::prelude::*;
use tempfile::tempdir;

use super::super::embed::Embedder;
use super::super::CilError;
use super::index::MarketIndex;
use super::provider::{MarketEntry, MarketplaceProvider};
use crate::openclaw::registry::ProductionSkillRegistry;
use crate::openclaw::types::TrustTier;

/// Small embedding dimension keeps the DB-backed, embed-heavy test fast while
/// still exercising the real offline-embed + cosine-rank path.
const EMBED_DIM: usize = 16;

/// Deterministic mock embedder: FNV bag-of-tokens hashing into a fixed-dim
/// vector, so tests are reproducible without model downloads or network. A
/// shared vocabulary between two texts drives up cosine similarity — a faithful
/// (if crude) stand-in for a real embedder. Copied from `index.rs`'s test module
/// (which is private) so this file owns its own mock (task 6.5 file ownership).
struct MockEmbedder {
    dim: usize,
}

#[async_trait]
impl Embedder for MockEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, CilError> {
        let mut v = vec![0.0f32; self.dim];
        for tok in text
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
        {
            let mut h: u64 = 1469598103934665603; // FNV-1a offset basis
            for b in tok.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(1099511628211);
            }
            v[(h as usize) % self.dim] += 1.0;
        }
        Ok(v)
    }
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, CilError> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(self.embed(t).await?);
        }
        Ok(out)
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn model_id(&self) -> &str {
        "mock-embedder-v1"
    }
}

/// A minimal deterministic provider returning a fixed catalog (no network, no
/// interior mutability — each sync yields the same entries).
struct FixedProvider {
    id: String,
    entries: Vec<MarketEntry>,
}

#[async_trait]
impl MarketplaceProvider for FixedProvider {
    fn provider_id(&self) -> &str {
        &self.id
    }
    async fn sync_index(&self) -> Result<Vec<MarketEntry>, CilError> {
        Ok(self.entries.clone())
    }
    async fn fetch_manifest(&self, slug: &str) -> Result<String, CilError> {
        Err(CilError::Market(format!(
            "mock has no manifest for '{slug}'"
        )))
    }
    fn trust_hint(&self, entry: &MarketEntry) -> TrustTier {
        // Clamp declared "verified" down to Community (never elevate remote),
        // mirroring the real ClawHub adapter's trust ceiling.
        parse_trust_tier(&entry.declared_trust).max(TrustTier::Community)
    }
}

/// Parse a declared trust-tier string (case-insensitive) into a [`TrustTier`],
/// deny-by-default on unknown values (mirrors the provider/index parsers).
fn parse_trust_tier(s: &str) -> TrustTier {
    match s.trim().to_ascii_lowercase().as_str() {
        "verified" => TrustTier::Verified,
        "community" => TrustTier::Community,
        "local" => TrustTier::Local,
        _ => TrustTier::Untrusted,
    }
}

/// A generated market-entry "shape" — the varying inputs the generator produces.
/// The `slug` is assigned by index at build time to guarantee uniqueness of the
/// `(provider_id, slug)` PK while these fields still vary freely.
#[derive(Debug, Clone)]
struct EntrySpec {
    slug_stub: String,
    name_word: String,
    description: String,
    category: String,
    version: String,
    declared_trust: String,
    quality: Option<f64>,
    popularity: Option<f64>,
    deprecated: bool,
}

/// Open-vocabulary description fragments, mixing common capability phrasing with
/// freely-generated tokens so the offline embedding surface stays open.
fn description_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("compress and archive zip files".to_string()),
        Just("send an email message over smtp".to_string()),
        Just("read a pdf document and extract text".to_string()),
        Just("com.example.novel.capability handler".to_string()),
        "[a-z]{2,8}( [a-z]{2,8}){0,3}",
    ]
}

fn version_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("1.0.0".to_string()),
        Just("2.1.3".to_string()),
        Just("0.9.7".to_string()),
        "[1-9]\\.[0-9]\\.[0-9]",
    ]
}

fn trust_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("verified".to_string()),
        Just("community".to_string()),
        Just("local".to_string()),
        Just("untrusted".to_string()),
    ]
}

/// Optional bounded ranking signal in `0.0..=1.0` (or honestly absent).
fn signal_strategy() -> impl Strategy<Value = Option<f64>> {
    prop_oneof![Just(None), (0.0f64..=1.0).prop_map(Some)]
}

fn entry_spec_strategy() -> impl Strategy<Value = EntrySpec> {
    (
        "[a-z]{1,6}",
        "[a-z]{2,8}",
        description_strategy(),
        prop_oneof![
            Just("developer".to_string()),
            Just("productivity".to_string()),
            Just("web".to_string()),
            "[a-z]{3,8}",
        ],
        version_strategy(),
        trust_strategy(),
        signal_strategy(),
        signal_strategy(),
        any::<bool>(),
    )
        .prop_map(
            |(
                slug_stub,
                name_word,
                description,
                category,
                version,
                declared_trust,
                quality,
                popularity,
                deprecated,
            )| EntrySpec {
                slug_stub,
                name_word,
                description,
                category,
                version,
                declared_trust,
                quality,
                popularity,
                deprecated,
            },
        )
}

/// Query strings for the goal-embedding set: common capability words plus a
/// novel reverse-DNS token and freely-generated queries.
fn query_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("compress and archive zip files".to_string()),
        Just("send an email".to_string()),
        Just("read a pdf document".to_string()),
        Just("com.example.novel.capability".to_string()),
        "[a-z]{2,8}( [a-z]{2,8}){0,3}",
    ]
}

/// Build a `MarketEntry` from a spec with an index-derived unique slug (so the
/// `(provider_id, slug)` PK is unique even when slug_stubs collide).
fn build_entry(provider_id: &str, index: usize, spec: &EntrySpec) -> MarketEntry {
    let slug = format!("oc_{}_{index}", spec.slug_stub);
    MarketEntry {
        provider_id: provider_id.to_string(),
        slug: slug.clone(),
        name: format!("{} {slug}", spec.name_word),
        description: spec.description.clone(),
        category: spec.category.clone(),
        version: spec.version.clone(),
        manifest_url: format!("https://example.com/{slug}/SKILL.md"),
        declared_trust: spec.declared_trust.clone(),
        capabilities_summary: vec!["subprocess".to_string(), spec.name_word.clone()],
        quality: spec.quality,
        popularity: spec.popularity,
        deprecated: spec.deprecated,
    }
}

/// A deterministic, exactly-comparable snapshot of a `search` result, sorted by
/// `(provider_id, slug)` so the comparison is insensitive to the ranker's score
/// tie-break yet exact on every persisted signal. `f32`/`f64` signals are
/// captured as raw IEEE-754 bits so equality is bit-exact (catches any
/// non-determinism rather than tolerating it with an epsilon).
type CandidateSig = (String, String, String, u32, bool, Option<u64>, Option<u64>);

fn search_signature(index: &MarketIndex, goal_embedding: &[f32], k: usize) -> Vec<CandidateSig> {
    let mut sig: Vec<CandidateSig> = index
        .search(goal_embedding, k)
        .expect("search ok")
        .into_iter()
        .map(|c| {
            (
                c.provider_id,
                c.slug,
                c.version,
                c.score.to_bits(),
                c.deprecated,
                c.quality.map(f64::to_bits),
                c.popularity.map(f64::to_bits),
            )
        })
        .collect();
    sig.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    sig
}

/// Open a fresh migrated `skills.db` (frozen migration 4 creates
/// `market_catalog`) and build a `MarketIndex` over it with one `FixedProvider`
/// serving the given entries. Mirrors `index.rs`'s `test_index` setup.
fn build_index(entries: Vec<MarketEntry>) -> (MarketIndex, tempfile::TempDir) {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("skills.db");
    // Frozen registry migrations create the market_catalog table (migration 4).
    let _registry = ProductionSkillRegistry::new(&db_path).expect("registry migrations");
    let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim: EMBED_DIM });
    let provider: Arc<dyn MarketplaceProvider> = Arc::new(FixedProvider {
        id: "mock".to_string(),
        entries,
    });
    let index = MarketIndex::open(&db_path, embedder, vec![provider]).expect("market index open");
    (index, dir)
}

proptest! {
    // Bounded case count keeps this DB-backed, embed-heavy test fast.
    #![proptest_config(ProptestConfig::with_cases(28))]

    /// Property 1: Single source of truth (Validates: Requirements 5.1).
    ///
    /// For an arbitrary market-entry set (the source of truth) and a set of goal
    /// embeddings:
    /// - two independently synced indexes (`A`, `B`) over fresh databases agree
    ///   on every query, and
    /// - re-syncing the same index a second time is idempotent on query results.
    #[test]
    fn idempotent_reindex_yields_identical_search(
        specs in prop::collection::vec(entry_spec_strategy(), 0..12),
        queries in prop::collection::vec(query_strategy(), 1..6),
    ) {
        // One deterministic current-thread runtime drives the async sync/embed
        // calls (mirrors cil/index_reindex_pbt.rs).
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("current-thread runtime");

        rt.block_on(async {
            // Build the entry set with unique (provider_id, slug) PKs.
            let entries: Vec<MarketEntry> = specs
                .iter()
                .enumerate()
                .map(|(i, spec)| build_entry("mock", i, spec))
                .collect();

            // Pre-embed the goal queries once with the mock embedder (same
            // embedder → same vectors for A/B).
            let embedder = MockEmbedder { dim: EMBED_DIM };
            let mut goal_embeddings: Vec<Vec<f32>> = Vec::with_capacity(queries.len());
            for q in &queries {
                goal_embeddings.push(embedder.embed(q).await.expect("query embed"));
            }

            // Index A: fresh db + sync from the entry set.
            let (index_a, _dir_a) = build_index(entries.clone());
            index_a.sync().await.expect("sync A");

            // Index B: a SECOND, independent index over a FRESH db from the SAME set.
            let (index_b, _dir_b) = build_index(entries.clone());
            index_b.sync().await.expect("sync B");

            // A few k values, including k > entry count (returns all).
            let ks = [1usize, 5, 20];

            for (qi, goal) in goal_embeddings.iter().enumerate() {
                for &k in &ks {
                    let sig_a = search_signature(&index_a, goal, k);
                    let sig_b = search_signature(&index_b, goal, k);
                    // Cross-instance: two rebuilds of the same source of truth
                    // must return byte-identical results.
                    prop_assert_eq!(
                        &sig_a,
                        &sig_b,
                        "independent reindex disagreed (A != B) for query #{} k={}",
                        qi,
                        k
                    );
                }
            }

            // Idempotence: re-sync index A a SECOND time from the same set and
            // assert every query's result is identical to the first sync.
            let mut first: Vec<Vec<CandidateSig>> = Vec::with_capacity(goal_embeddings.len());
            for goal in &goal_embeddings {
                first.push(search_signature(&index_a, goal, 20));
            }

            index_a.sync().await.expect("re-sync A");

            for (qi, (goal, before)) in goal_embeddings.iter().zip(first.iter()).enumerate() {
                let after = search_signature(&index_a, goal, 20);
                prop_assert_eq!(
                    before,
                    &after,
                    "re-sync of same index changed results for query #{}",
                    qi
                );
            }

            Ok(())
        })?;
    }
}
