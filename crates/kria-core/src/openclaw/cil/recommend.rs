//! `Recommender` — pure-read capability recommendations (task 7.1, design §8.7,
//! R8.1 / R8.2 / R8.3 / R8.5).
//!
//! When the goal needs a capability that no acceptable *installed* skill
//! satisfies, the ICP does not silently give up and it does not silently
//! install anything. It surfaces a ranked set of honest [`Recommendation`]s the
//! user (or a policy) can choose to install — the *why* attached to each so the
//! decision is informed.
//!
//! # Pure reads only (R8.2)
//!
//! The [`Recommender`] performs **only reads**: it queries the offline
//! [`MarketIndex`] cache (a pure DB read over pre-embedded `market_catalog`
//! rows — never a live per-query marketplace fetch) and, when one is provided,
//! the derived [`CapabilityGraph`] (for alternatives/successors). It **never**
//! installs, generates, downloads, or mutates any state. Acquisition is a
//! separate, explicitly-approved step (task 8); this layer only *recommends*.
//!
//! # Honesty (R8.5)
//!
//! Recommendations are ranked by the **configured** signals (via
//! [`RankWeights`]) and gated by a relevance threshold from [`CilConfig`]. When
//! nothing clears the threshold the result is an **empty vector** — an honest
//! "nothing worth recommending" — never a fabricated or padded candidate. Every
//! signal in a [`Recommendation`] is copied straight from the `market_catalog`
//! row (or left honestly absent as `None`); none are invented.
//!
//! # No hardcoding (R8.3)
//!
//! Each candidate's [`Recommendation::rationale`] is assembled **from that
//! candidate's real signal values** (semantic match, trust tier, quality,
//! popularity, version/deprecation, offline staleness). It is NEVER a template
//! keyed to a skill's name or category — a never-before-seen skill produces a
//! rationale through the exact same signal-formatting code path, with no
//! per-skill or per-category branch anywhere.
//!
//! # Determinism
//!
//! Ordering is fully deterministic: descending combined score with a stable
//! tie-break by `(provider_id, slug)`. Identical inputs yield identical output.
//!
//! [`MarketIndex`]: crate::openclaw::cil::market::MarketIndex
//! [`CapabilityGraph`]: crate::openclaw::cil::graph::CapabilityGraph
//! [`RankWeights`]: crate::openclaw::cil::config::RankWeights

use std::collections::HashSet;
use std::sync::Arc;

use super::config::{CilConfig, RankWeights};
use super::graph::CapabilityGraph;
use super::market::{MarketCandidate, MarketIndex};
use super::CilError;
use crate::openclaw::types::TrustTier;

/// A ranked, honest recommendation to install a marketplace skill (design §8.7).
///
/// Carries the **real ranking signals** copied from the `market_catalog` row —
/// nothing here is fabricated — plus a human-readable [`rationale`] assembled
/// from those same signals (R8.3) and any interchangeable [`alternatives`] the
/// capability graph knows about. This is the payload of
/// [`Fulfillment::Recommend`](crate::openclaw::cil::Fulfillment::Recommend).
///
/// [`rationale`]: Recommendation::rationale
/// [`alternatives`]: Recommendation::alternatives
#[derive(Debug, Clone, PartialEq)]
pub struct Recommendation {
    /// The marketplace this candidate came from (`market_catalog.provider_id`).
    pub provider_id: String,
    /// Stable skill identifier / slug (`market_catalog.slug`).
    pub slug: String,
    /// Offered semver (`market_catalog.version`).
    pub version: String,
    /// The combined, weighted rank score (`0.0..`) used to order recommendations.
    /// Higher is a better match under the configured [`RankWeights`].
    pub score: f32,
    /// Effective trust tier recorded at sync time, or `None` when the catalog
    /// row carried no trust hint (honestly absent, never guessed).
    pub trust: Option<TrustTier>,
    /// Validator/marketplace quality signal, if the provider supplied one.
    pub quality: Option<f64>,
    /// Install/usage popularity signal, if the provider supplied one.
    pub popularity: Option<f64>,
    /// Whether the marketplace flags this skill deprecated.
    pub deprecated: bool,
    /// Human-readable explanation assembled from the candidate's real signal
    /// values (R8.3). Never a template keyed to the skill name/category.
    pub rationale: String,
    /// Interchangeable alternative skill ids from the [`CapabilityGraph`] (empty
    /// when no graph was provided or none are known). Full alternatives wiring
    /// is task 8.4 / 12.2; a minimal inclusion is provided here.
    pub alternatives: Vec<String>,
}

/// The recommendation seam (design §8.7). Kept a trait so the facade (task 7.2)
/// and the Desktop command layer (task 7.3) depend on the behavior, not the
/// concrete [`DefaultRecommender`], and so an alternative recommender can be
/// swapped in without touching callers.
pub trait Recommender {
    /// Return ranked [`Recommendation`]s for a goal, ordered by the configured
    /// signals and gated by the config relevance threshold.
    ///
    /// - `goal_embedding`: the goal's dense vector (same space as the offline
    ///   `market_catalog` embeddings), produced by the [`Embedder`].
    /// - `installed_skill_ids`: skills already installed; any candidate whose
    ///   slug matches one is **filtered out** (no point recommending what the
    ///   user already has).
    /// - `k`: the maximum number of recommendations to return.
    /// - `config`: supplies the [`RankWeights`] and the relevance threshold.
    ///
    /// # Purity (R8.2)
    ///
    /// This is a **pure read** — it queries the offline market cache (+ optional
    /// capability graph) and installs / mutates nothing.
    ///
    /// # Honesty (R8.5)
    ///
    /// When no candidate clears the relevance threshold the result is an **empty
    /// vector**, never a fabricated candidate.
    ///
    /// [`Embedder`]: crate::openclaw::cil::embed::Embedder
    fn recommend(
        &self,
        goal_embedding: &[f32],
        installed_skill_ids: &[String],
        k: usize,
        config: &CilConfig,
    ) -> Result<Vec<Recommendation>, CilError>;
}

/// The default [`Recommender`]: pure reads over the offline [`MarketIndex`] and
/// an optional [`CapabilityGraph`].
///
/// Holds only shared handles to the read sources; it owns no mutable state and
/// performs no I/O beyond the two read queries. The [`CapabilityGraph`] is
/// optional — when absent, recommendations still rank fully from the market
/// catalog and simply carry no [`alternatives`](Recommendation::alternatives).
pub struct DefaultRecommender {
    market: Arc<MarketIndex>,
    graph: Option<Arc<CapabilityGraph>>,
}

impl DefaultRecommender {
    /// A recommender over the market catalog only (no capability graph, so no
    /// alternatives are attached).
    pub fn new(market: Arc<MarketIndex>) -> Self {
        Self {
            market,
            graph: None,
        }
    }

    /// A recommender that also consults the [`CapabilityGraph`] for
    /// interchangeable alternatives on each recommendation.
    pub fn with_graph(market: Arc<MarketIndex>, graph: Arc<CapabilityGraph>) -> Self {
        Self {
            market,
            graph: Some(graph),
        }
    }

    /// Over-fetch factor: query more raw market candidates than `k` so that the
    /// installed-filter + threshold-gate still leave a full `k` to return. A
    /// small constant multiplier keeps the read bounded (no unbounded query).
    const OVERFETCH: usize = 4;
}

impl Recommender for DefaultRecommender {
    fn recommend(
        &self,
        goal_embedding: &[f32],
        installed_skill_ids: &[String],
        k: usize,
        config: &CilConfig,
    ) -> Result<Vec<Recommendation>, CilError> {
        if k == 0 {
            return Ok(Vec::new());
        }

        // Pure read: offline cosine search over the pre-embedded market_catalog.
        // Over-fetch so the installed-filter + threshold-gate below still leave
        // room for a full k, but keep the fetch bounded.
        let fetch_k = k.saturating_mul(Self::OVERFETCH).max(k);
        let candidates = self.market.search(goal_embedding, fetch_k)?;

        // Filter out already-installed skills (match by slug == skill id).
        let installed: HashSet<&str> = installed_skill_ids.iter().map(|s| s.as_str()).collect();
        let mut candidates: Vec<MarketCandidate> = candidates
            .into_iter()
            .filter(|c| !installed.contains(c.slug.as_str()))
            .collect();

        // Relevance threshold (R8.5): the candidate's semantic match is the
        // market-side relevance signal; a candidate must meet the configured
        // compatibility (match-quality) floor to be worth recommending. This is
        // a config value, not a hardcoded constant.
        let threshold = config.compatibility_threshold;
        candidates.retain(|c| c.score >= threshold);

        // Nothing cleared the bar → honest empty set, never a fabricated pick.
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        // Popularity is a raw provider signal (e.g. install counts) with no fixed
        // scale; max-normalize it across the current set into 0.0..=1.0 so it can
        // join the weighted sum without dominating. Absent popularity contributes
        // 0.0 (honestly absent, not invented).
        let max_pop = candidates
            .iter()
            .filter_map(|c| c.popularity)
            .fold(0.0f64, f64::max);

        let w = &config.weights;
        let mut scored: Vec<Recommendation> = candidates
            .into_iter()
            .map(|c| self.to_recommendation(c, w, max_pop))
            .collect();

        // Deterministic ordering: descending combined score, stable tie-break by
        // (provider_id, slug).
        scored.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.provider_id.cmp(&b.provider_id))
                .then_with(|| a.slug.cmp(&b.slug))
        });
        scored.truncate(k);

        // Enrich with graph alternatives (optional, pure read). Done after the
        // top-k cut so we only query the graph for what we actually return.
        if let Some(graph) = &self.graph {
            for rec in &mut scored {
                rec.alternatives = graph.alternatives(&rec.slug)?;
            }
        }

        Ok(scored)
    }
}

impl DefaultRecommender {
    /// Turn a scored [`MarketCandidate`] into a [`Recommendation`]: compute the
    /// combined weighted score from its real signals and assemble the rationale
    /// from those same values (R8.3). `alternatives` is filled later (post-cut).
    fn to_recommendation(
        &self,
        c: MarketCandidate,
        w: &RankWeights,
        max_pop: f64,
    ) -> Recommendation {
        // Normalize each available signal into 0.0..=1.0. Signals the market
        // layer does not carry (lexical/compatibility/success) contribute 0 —
        // honestly absent, never fabricated.
        let semantic = c.score.clamp(0.0, 1.0);
        let trust = c.trust_hint.map(trust_score).unwrap_or(0.0);
        let quality = c.quality.map(|q| q.clamp(0.0, 1.0) as f32).unwrap_or(0.0);
        let popularity = match c.popularity {
            Some(p) if max_pop > 0.0 => (p / max_pop).clamp(0.0, 1.0) as f32,
            _ => 0.0,
        };

        let score = w.semantic * semantic
            + w.trust * trust
            + w.quality * quality
            + w.popularity * popularity;

        let rationale = build_rationale(&c);

        Recommendation {
            provider_id: c.provider_id,
            slug: c.slug,
            version: c.version,
            score,
            trust: c.trust_hint,
            quality: c.quality,
            popularity: c.popularity,
            deprecated: c.deprecated,
            rationale,
            alternatives: Vec::new(),
        }
    }
}

/// Map a [`TrustTier`] onto a `0.0..=1.0` trust signal (Verified highest,
/// Untrusted lowest). Deny-by-default: an absent tier is treated as `0.0` by the
/// caller. This is a generic tier→scalar mapping — not a per-skill rule.
fn trust_score(tier: TrustTier) -> f32 {
    match tier {
        TrustTier::Verified => 1.0,
        TrustTier::Community => 0.75,
        TrustTier::Local => 0.5,
        TrustTier::Untrusted => 0.0,
    }
}

/// Assemble a recommendation rationale **from the candidate's real signal
/// values** (R8.3).
///
/// Every clause is derived from a concrete `market_catalog` field of *this*
/// candidate; a signal that is honestly absent (`None`) simply contributes no
/// clause. There is deliberately **no** reference to the skill's name or
/// category and **no** per-skill/per-category branch — a novel skill flows
/// through the exact same formatting, so the rationale can never be a canned
/// per-name/category template.
fn build_rationale(c: &MarketCandidate) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Semantic match is always present (it is how the candidate was found).
    parts.push(format!("semantic match {:.2}", c.score.clamp(0.0, 1.0)));

    if let Some(tier) = c.trust_hint {
        parts.push(format!("trust: {}", tier.as_str()));
    }
    if let Some(q) = c.quality {
        parts.push(format!("quality {:.2}", q));
    }
    if let Some(p) = c.popularity {
        // Present popularity as-is from the signal; large values read as counts.
        if p >= 1.0 {
            parts.push(format!("{:.0} installs", p));
        } else {
            parts.push(format!("popularity {:.2}", p));
        }
    }
    parts.push(format!("v{}", c.version));
    if c.deprecated {
        parts.push("deprecated".to_string());
    }
    if c.offline {
        parts.push("offline (served from stale cache)".to_string());
    }

    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::cil::embed::Embedder;
    use crate::openclaw::cil::market::{MarketEntry, MarketplaceProvider};
    use crate::openclaw::registry::ProductionSkillRegistry;
    use async_trait::async_trait;
    use tempfile::tempdir;

    /// Deterministic bag-of-tokens mock embedder (no model/network): shared
    /// vocabulary between two texts drives up cosine similarity. Mirrors the
    /// stand-in used in the market index tests.
    struct MockEmbedder {
        dim: usize,
    }

    #[async_trait]
    impl Embedder for MockEmbedder {
        async fn embed(&self, text: &str) -> Result<Vec<f32>, CilError> {
            let mut v = vec![0.0f32; self.dim];
            for tok in text
                .to_lowercase()
                .split(|ch: char| !ch.is_alphanumeric())
                .filter(|t| !t.is_empty())
            {
                let mut h: u64 = 1469598103934665603;
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

    /// Minimal reachable mock provider serving a fixed catalog (no network).
    struct MockProvider {
        id: String,
        entries: Vec<MarketEntry>,
    }

    #[async_trait]
    impl MarketplaceProvider for MockProvider {
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
            entry.declared_trust.parse().unwrap_or(TrustTier::Community)
        }
    }

    fn entry(slug: &str, desc: &str, quality: f64, popularity: f64) -> MarketEntry {
        MarketEntry {
            provider_id: "mock".into(),
            slug: slug.into(),
            name: slug.into(),
            description: desc.into(),
            category: "developer".into(),
            version: "1.2.0".into(),
            manifest_url: format!("https://example.com/{slug}/SKILL.md"),
            declared_trust: "community".into(),
            capabilities_summary: vec!["subprocess".into()],
            quality: Some(quality),
            popularity: Some(popularity),
            deprecated: false,
        }
    }

    /// Build a `MarketIndex` over a real migrated skills.db (migration 4 creates
    /// `market_catalog`), synced once so the cache is populated. Returns the
    /// shared embedder so tests can build matching goal embeddings offline.
    async fn synced_index(
        entries: Vec<MarketEntry>,
    ) -> (Arc<MarketIndex>, Arc<MockEmbedder>, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("skills.db");
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry migrations");
        let embedder = Arc::new(MockEmbedder { dim: 16 });
        let provider = Arc::new(MockProvider {
            id: "mock".into(),
            entries,
        });
        let index = MarketIndex::open(
            &db_path,
            embedder.clone() as Arc<dyn Embedder>,
            vec![provider as Arc<dyn MarketplaceProvider>],
        )
        .expect("market index open");
        index.sync().await.expect("sync ok");
        (Arc::new(index), embedder, dir)
    }

    fn low_threshold_config() -> CilConfig {
        let mut cfg = CilConfig::default();
        // Accept anything with a non-trivial match for the ranking tests.
        cfg.compatibility_threshold = 0.01;
        cfg
    }

    #[tokio::test]
    async fn recommends_ranked_from_market_cache() {
        let (index, embedder, _dir) = synced_index(vec![
            entry("oc_archive", "compress and archive zip files", 0.9, 1200.0),
            entry("oc_email", "send an email message over smtp", 0.4, 5.0),
        ])
        .await;
        let goal = embedder
            .embed("compress and archive zip files")
            .await
            .unwrap();

        let rec = DefaultRecommender::new(index);
        let out = rec
            .recommend(&goal, &[], 5, &low_threshold_config())
            .expect("recommend ok");

        assert_eq!(out.len(), 2, "both catalog entries recommended");
        // The archive entry (identical text) is the strongest semantic match.
        assert_eq!(out[0].slug, "oc_archive");
        assert!(out[0].score >= out[1].score, "descending combined score");
        // Signals are copied straight from the catalog row (not fabricated).
        assert_eq!(out[0].trust, Some(TrustTier::Community));
        assert_eq!(out[0].quality, Some(0.9));
        assert_eq!(out[0].popularity, Some(1200.0));
        assert_eq!(out[0].version, "1.2.0");
    }

    #[tokio::test]
    async fn rationale_is_non_empty_and_signal_derived() {
        let (index, embedder, _dir) = synced_index(vec![entry(
            "oc_archive",
            "compress and archive zip files",
            0.9,
            1200.0,
        )])
        .await;
        let goal = embedder
            .embed("compress and archive zip files")
            .await
            .unwrap();

        let rec = DefaultRecommender::new(index);
        let out = rec
            .recommend(&goal, &[], 5, &low_threshold_config())
            .expect("recommend ok");

        assert_eq!(out.len(), 1);
        let r = &out[0];
        assert!(!r.rationale.is_empty(), "rationale must not be empty");
        // Derived from real signals: mentions semantic match, trust, quality,
        // popularity, and version — and NEVER the skill name/category.
        assert!(
            r.rationale.contains("semantic match"),
            "rationale: {}",
            r.rationale
        );
        assert!(
            r.rationale.contains("trust: community"),
            "rationale: {}",
            r.rationale
        );
        assert!(
            r.rationale.contains("quality 0.90"),
            "rationale: {}",
            r.rationale
        );
        assert!(
            r.rationale.contains("1200 installs"),
            "rationale: {}",
            r.rationale
        );
        assert!(r.rationale.contains("v1.2.0"), "rationale: {}", r.rationale);
        assert!(
            !r.rationale.contains("oc_archive")
                && !r.rationale.to_lowercase().contains("developer"),
            "rationale must not be templated on name/category: {}",
            r.rationale
        );
    }

    #[tokio::test]
    async fn nothing_above_threshold_returns_empty() {
        let (index, embedder, _dir) =
            synced_index(vec![entry("oc_email", "send an email message", 0.4, 5.0)]).await;
        // A goal with disjoint vocabulary → near-zero cosine similarity.
        let goal = embedder
            .embed("quantum entanglement route optimizer")
            .await
            .unwrap();

        let mut cfg = CilConfig::default();
        cfg.compatibility_threshold = 0.99; // Nothing can clear this.

        let rec = DefaultRecommender::new(index);
        let out = rec.recommend(&goal, &[], 5, &cfg).expect("recommend ok");
        assert!(
            out.is_empty(),
            "no candidate above threshold → honest empty set"
        );
    }

    #[tokio::test]
    async fn installed_skills_are_filtered_out() {
        let (index, embedder, _dir) = synced_index(vec![
            entry("oc_archive", "compress and archive zip files", 0.9, 1200.0),
            entry(
                "oc_archive_alt",
                "compress and archive zip files",
                0.9,
                10.0,
            ),
        ])
        .await;
        let goal = embedder
            .embed("compress and archive zip files")
            .await
            .unwrap();

        let rec = DefaultRecommender::new(index);
        let installed = vec!["oc_archive".to_string()];
        let out = rec
            .recommend(&goal, &installed, 5, &low_threshold_config())
            .expect("recommend ok");

        assert!(
            out.iter().all(|r| r.slug != "oc_archive"),
            "installed skill must be filtered out"
        );
        assert!(
            out.iter().any(|r| r.slug == "oc_archive_alt"),
            "non-installed alternative still recommended"
        );
    }

    #[tokio::test]
    async fn k_zero_returns_empty() {
        let (index, embedder, _dir) =
            synced_index(vec![entry("oc_archive", "compress files", 0.9, 10.0)]).await;
        let goal = embedder.embed("compress files").await.unwrap();
        let rec = DefaultRecommender::new(index);
        let out = rec
            .recommend(&goal, &[], 0, &low_threshold_config())
            .expect("recommend ok");
        assert!(out.is_empty(), "k=0 yields empty");
    }

    // ---------------------------------------------------------------------
    // Property 10: Honesty — no fabricated candidates, no fabricated signals,
    // rationale derived only from a candidate's real signal values.
    // **Validates: Requirements 7.1, 8.5**
    // ---------------------------------------------------------------------

    /// A `MarketEntry` whose optional ranking signals are honestly absent
    /// (`quality = None`, `popularity = None`) so we can assert nothing invents
    /// a value where the catalog carried none.
    fn entry_no_signals(slug: &str, desc: &str) -> MarketEntry {
        MarketEntry {
            provider_id: "mock".into(),
            slug: slug.into(),
            name: slug.into(),
            description: desc.into(),
            category: "developer".into(),
            version: "2.0.0".into(),
            manifest_url: format!("https://example.com/{slug}/SKILL.md"),
            declared_trust: "community".into(),
            capabilities_summary: vec!["subprocess".into()],
            quality: None,
            popularity: None,
            deprecated: false,
        }
    }

    /// (a) Honesty — an empty market cache (nothing ever synced worth returning)
    /// yields an empty recommendation set, never a fabricated placeholder pick.
    #[tokio::test]
    async fn honesty_empty_catalog_never_fabricates() {
        let (index, embedder, _dir) = synced_index(vec![]).await;
        let goal = embedder
            .embed("compress and archive zip files")
            .await
            .unwrap();

        let rec = DefaultRecommender::new(index);
        let out = rec
            .recommend(&goal, &[], 5, &low_threshold_config())
            .expect("recommend ok");

        assert!(
            out.is_empty(),
            "empty catalog must yield empty recommendations, not a fabricated candidate"
        );
    }

    /// (a) Honesty — with a mix of above- and below-threshold candidates, only
    /// the ones that actually clear the bar are returned. The result is NEVER
    /// padded up to `k` with sub-threshold (fabricated-relevance) picks.
    #[tokio::test]
    async fn honesty_below_threshold_dropped_never_padded_to_k() {
        let (index, embedder, _dir) = synced_index(vec![
            entry("oc_archive", "compress and archive zip files", 0.9, 1200.0),
            entry("oc_email", "send an email message over smtp", 0.4, 5.0),
        ])
        .await;
        // Goal matches the archive entry's vocabulary exactly; the email entry
        // shares almost nothing, so its cosine score falls well below the floor.
        let goal = embedder
            .embed("compress and archive zip files")
            .await
            .unwrap();

        let mut cfg = CilConfig::default();
        // A mid floor: the exact-match archive entry clears it, the disjoint
        // email entry does not.
        cfg.compatibility_threshold = 0.5;

        let rec = DefaultRecommender::new(index);
        // Ask for more than can clear the bar to prove there is no padding.
        let out = rec.recommend(&goal, &[], 5, &cfg).expect("recommend ok");

        assert_eq!(
            out.len(),
            1,
            "only the above-threshold candidate is returned; never padded to k"
        );
        assert_eq!(out[0].slug, "oc_archive");
        assert!(
            out.iter().all(|r| r.slug != "oc_email"),
            "the sub-threshold candidate must be dropped, not fabricated in"
        );
    }

    /// (c) No fabricated signals — every signal on a `Recommendation` is copied
    /// verbatim from its source `market_catalog` row. Signals the row does not
    /// carry stay `None`; they are never invented as `0.0`/`Some(_)`.
    #[tokio::test]
    async fn honesty_absent_signals_stay_none() {
        let (index, embedder, _dir) = synced_index(vec![entry_no_signals(
            "oc_novel",
            "transcode a rare media container",
        )])
        .await;
        let goal = embedder
            .embed("transcode a rare media container")
            .await
            .unwrap();

        let rec = DefaultRecommender::new(index);
        let out = rec
            .recommend(&goal, &[], 5, &low_threshold_config())
            .expect("recommend ok");

        assert_eq!(out.len(), 1, "the single catalog entry is recommended");
        let r = &out[0];
        // The catalog row carried no quality/popularity → they remain honestly
        // absent on the recommendation (not fabricated).
        assert_eq!(r.quality, None, "absent quality must not be fabricated");
        assert_eq!(
            r.popularity, None,
            "absent popularity must not be fabricated"
        );
        // Signals that ARE present are the real row values, unaltered.
        assert_eq!(r.trust, Some(TrustTier::Community));
        assert_eq!(r.version, "2.0.0");
        assert!(!r.deprecated);
        // The rationale must not invent quality/popularity clauses for absent
        // signals (no "quality 0.00", no "installs", no "popularity").
        assert!(
            !r.rationale.contains("quality"),
            "rationale invented a quality clause: {}",
            r.rationale
        );
        assert!(
            !r.rationale.contains("installs") && !r.rationale.contains("popularity"),
            "rationale invented a popularity clause: {}",
            r.rationale
        );
        // Present signals still appear (derived from real values).
        assert!(
            r.rationale.contains("semantic match"),
            "rationale: {}",
            r.rationale
        );
        assert!(
            r.rationale.contains("trust: community"),
            "rationale: {}",
            r.rationale
        );
        assert!(r.rationale.contains("v2.0.0"), "rationale: {}", r.rationale);
    }

    /// (c) No fabricated signals — the recommendation's carried signals equal
    /// exactly the source entry's fields (trust/quality/popularity/version/
    /// deprecated), proving nothing is transformed into a different value on the
    /// way out.
    #[tokio::test]
    async fn honesty_signals_equal_source_row() {
        let mut deprecated_entry = entry("oc_legacy", "extract text from a scanned pdf", 0.7, 42.0);
        deprecated_entry.deprecated = true;
        let (index, embedder, _dir) = synced_index(vec![deprecated_entry]).await;
        let goal = embedder
            .embed("extract text from a scanned pdf")
            .await
            .unwrap();

        let rec = DefaultRecommender::new(index);
        let out = rec
            .recommend(&goal, &[], 5, &low_threshold_config())
            .expect("recommend ok");

        assert_eq!(out.len(), 1);
        let r = &out[0];
        assert_eq!(r.provider_id, "mock");
        assert_eq!(r.slug, "oc_legacy");
        assert_eq!(r.version, "1.2.0");
        assert_eq!(r.trust, Some(TrustTier::Community));
        assert_eq!(r.quality, Some(0.7));
        assert_eq!(r.popularity, Some(42.0));
        assert!(
            r.deprecated,
            "deprecation flag copied straight from the row"
        );
        // The deprecated flag surfaces truthfully in the rationale.
        assert!(
            r.rationale.contains("deprecated"),
            "deprecated signal must be surfaced honestly: {}",
            r.rationale
        );
    }

    /// (b) Rationale derived from real signals — changing a candidate's actual
    /// quality value changes the rationale text accordingly, and the rationale
    /// never contains the skill's name or category. This proves the rationale is
    /// signal-driven, not a template keyed to identity (R8.3 / Property 10).
    #[tokio::test]
    async fn honesty_rationale_tracks_real_values_not_identity() {
        // Two catalog rows with identical descriptions but different quality.
        let (index, embedder, _dir) = synced_index(vec![
            entry("oc_hi_q", "resize and crop an image file", 0.95, 100.0),
            entry("oc_lo_q", "resize and crop an image file", 0.20, 100.0),
        ])
        .await;
        let goal = embedder
            .embed("resize and crop an image file")
            .await
            .unwrap();

        let rec = DefaultRecommender::new(index);
        let out = rec
            .recommend(&goal, &[], 5, &low_threshold_config())
            .expect("recommend ok");

        assert_eq!(out.len(), 2);
        let hi = out
            .iter()
            .find(|r| r.slug == "oc_hi_q")
            .expect("hi present");
        let lo = out
            .iter()
            .find(|r| r.slug == "oc_lo_q")
            .expect("lo present");

        // Rationale text reflects each candidate's own quality value — proof it
        // is assembled from real signals, not a shared name/category template.
        assert!(
            hi.rationale.contains("quality 0.95"),
            "hi rationale: {}",
            hi.rationale
        );
        assert!(
            lo.rationale.contains("quality 0.20"),
            "lo rationale: {}",
            lo.rationale
        );
        assert_ne!(
            hi.rationale, lo.rationale,
            "rationales differ by real value"
        );

        // Never templated on skill name or category.
        for r in &out {
            assert!(
                !r.rationale.contains(&r.slug),
                "rationale must not embed the skill name: {}",
                r.rationale
            );
            assert!(
                !r.rationale.to_lowercase().contains("developer"),
                "rationale must not embed the category: {}",
                r.rationale
            );
        }
    }
}
