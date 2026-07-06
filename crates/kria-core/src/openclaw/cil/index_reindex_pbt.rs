//! Property-based test for the **single-source-of-truth / idempotent-reindex**
//! invariant of the fused [`CapabilityIndex`] (dense ANN + BM25) — task 3.5.
//!
//! # Property 1 — Single source of truth (design §Correctness Properties)
//!
//! **Validates: Requirements 5.1**
//!
//! Requirement 5.1: *"rebuilding all derived views from the registry yields
//! identical query results (idempotent reindex)."* Where [`profile_reindex_pbt`]
//! covers the persisted `capability_profiles` projection, this test covers the
//! **in-memory discovery index** built on top of it: the dense semantic half and
//! the frozen-pattern BM25 lexical half, fused by [`CapabilityIndex::search`].
//!
//! The [`CapabilityIndex`] is a rebuildable derived view over the same skill
//! source of truth (`ProductionSkillRegistry::get_enabled_skills()`, modeled here
//! by the generated `Vec<SkillMetadata>` handed to [`CapabilityIndex::rebuild`]).
//! Because [`extract_profile`] and the [`MemoryEmbedder`] (hash fallback in CI)
//! are pure deterministic functions of the skill set, a reindex is a
//! well-defined function of that set alone. This test encodes two facets:
//!
//! 1. **Cross-instance determinism** — two independently constructed
//!    [`CapabilityIndex`]es (`A` and `B`), each `new` + `rebuild(&skills)` from
//!    the SAME skill set, return byte-identical `search` results for every query.
//!    Two indexes → one source of truth → one answer.
//! 2. **Idempotence** — rebuilding the SAME index a second time from the same
//!    skill set produces search results identical to the first rebuild. A
//!    reindex is a no-op on query results when the source of truth is unchanged.
//!
//! `(skill_ref, semantic, lexical)` is compared with the two `f32` signals taken
//! as raw IEEE-754 bits ([`f32::to_bits`]), so the assertion is exact — any
//! non-determinism (ordering, hashing, accumulation drift) in either index half
//! is caught, not tolerated by an epsilon.
//!
//! The generator produces 0..=15 skills with **unique** `skill_id`s and varying
//! `category`/`categories`/`dependencies`/`input_schema` shapes — including a
//! deliberately novel, never-enumerated reverse-DNS `CapabilityTag` domain — so
//! the reindex is exercised across the open-vocabulary input space (no
//! hardcoding).
//!
//! [`profile_reindex_pbt`]: crate::openclaw::cil::profile_reindex_pbt
//! [`CapabilityIndex`]: crate::openclaw::cil::index::CapabilityIndex
//! [`extract_profile`]: crate::openclaw::cil::extract::extract_profile
//! [`MemoryEmbedder`]: crate::openclaw::cil::embed::MemoryEmbedder

use std::sync::Arc;

use proptest::prelude::*;

use crate::openclaw::cil::embed::{Embedder, MemoryEmbedder};
use crate::openclaw::cil::index::CapabilityIndex;
use crate::openclaw::registry::{DiscoverySource, SkillDependency, SkillMetadata, SkillState};
use crate::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};
use crate::safety::RiskLevel;

/// Small embedding dimension keeps the DB-free, embed-heavy test fast while
/// still exercising the real aggregate/normalize dense path.
const EMBED_DIM: usize = 32;

/// A generated skill "shape" — the varying inputs that feed the extractor and
/// both index halves. The `skill_id` is assigned by index at build time to
/// guarantee uniqueness while these fields still vary freely.
#[derive(Debug, Clone)]
struct SkillSpec {
    id_stub: String,
    name_word: String,
    category: String,
    categories: Vec<String>,
    dep_ids: Vec<String>,
    schema: Option<serde_json::Value>,
}

/// Open-vocabulary capability strings, mixing common namespaced ids, a
/// deliberately **novel** reverse-DNS domain, and freely-generated strings.
/// This keeps the input space open (no closed enum) per Requirement 1.
fn cap_string() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("media.image".to_string()),
        Just("doc.pdf".to_string()),
        Just("io.file.read".to_string()),
        Just("net.email.send".to_string()),
        // Never-before-seen reverse-DNS domain: flows through as an open string,
        // embedded/indexed/searched by the same code path (no branch).
        Just("com.example.novel.capability".to_string()),
        Just("quantum.entangle.route".to_string()),
        "[a-z]{1,8}(\\.[a-z]{1,8}){0,2}",
    ]
}

/// A few structurally distinct MCP `input_schema` shapes (plus `None`), so the
/// generic `type`/`format` walk in the extractor is exercised broadly.
fn schema_strategy() -> impl Strategy<Value = Option<serde_json::Value>> {
    prop_oneof![
        Just(None),
        Just(Some(serde_json::json!({ "type": "object" }))),
        Just(Some(serde_json::json!({
            "type": "object",
            "properties": {
                "file": { "type": "string", "format": "binary" },
                "count": { "type": "integer" }
            }
        }))),
        Just(Some(serde_json::json!({
            "type": "array",
            "items": { "type": ["string", "null"] }
        }))),
    ]
}

fn skill_spec_strategy() -> impl Strategy<Value = SkillSpec> {
    (
        "[a-z]{1,6}",
        "[a-z]{2,8}",
        cap_string(),
        prop::collection::vec(cap_string(), 0..4),
        prop::collection::vec("[a-z]{1,6}\\.[a-z]{1,6}", 0..3),
        schema_strategy(),
    )
        .prop_map(
            |(id_stub, name_word, category, categories, dep_ids, schema)| SkillSpec {
                id_stub,
                name_word,
                category,
                categories,
                dep_ids,
                schema,
            },
        )
}

/// Query strings for the search comparison: a mix of common capability words and
/// a deliberately novel reverse-DNS token, plus freely-generated queries.
fn query_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("compress a pdf document".to_string()),
        Just("send an email".to_string()),
        Just("read a file from disk".to_string()),
        Just("com.example.novel.capability".to_string()),
        Just("quantum entangle route".to_string()),
        "[a-z]{2,8}( [a-z]{2,8}){0,3}",
    ]
}

/// Build enabled `SkillMetadata` from a spec with an index-derived unique id.
fn build_metadata(index: usize, spec: &SkillSpec) -> SkillMetadata {
    // Index guarantees a unique skill_id even when id_stubs collide.
    let skill_id = format!("skill.{}.{index}", spec.id_stub);
    SkillMetadata {
        skill_id: skill_id.clone(),
        name: format!("{} {skill_id}", spec.name_word),
        description: format!("pbt skill providing {}", spec.category),
        publisher: "test".to_string(),
        version: "1.0.0".to_string(),
        category: spec.category.clone(),
        discovery_source: DiscoverySource::Bundled {
            path: "test".to_string(),
        },
        discovered_at: chrono::Utc::now(),
        capabilities: SkillCapabilities::default(),
        runtime_requirements: "docker".to_string(),
        risk_level: RiskLevel::Green,
        resource_class: ResourceClass::Light,
        tags: vec!["pbt".to_string(), spec.name_word.clone()],
        categories: spec.categories.clone(),
        semantic_version: "1.0.0".to_string(),
        dependencies: spec
            .dep_ids
            .iter()
            .map(|d| SkillDependency {
                skill_id: d.clone(),
                version_requirement: "*".to_string(),
                optional: false,
            })
            .collect(),
        compatibility_requirements: vec![],
        trust_tier: TrustTier::Local,
        content_hash: format!("hash_{skill_id}"),
        signature: None,
        granted_capabilities: Vec::new(),
        bundle_path: None,
        manifest_toml: None,
        input_schema: spec.schema.clone(),
        state: SkillState::Enabled,
        state_changed_at: chrono::Utc::now(),
    }
}

/// A deterministic, exactly-comparable snapshot of a single `search` result:
/// `(skill_ref, semantic_bits, lexical_bits)` for each returned candidate, in
/// the index's returned order. The two `f32` signals are captured as raw
/// IEEE-754 bits so equality is bit-exact (catches any non-determinism rather
/// than tolerating it with an epsilon).
fn search_signature(
    index: &CapabilityIndex,
    goal_embedding: &[f32],
    query_text: &str,
    k: usize,
) -> Vec<(Option<String>, u32, u32)> {
    index
        .search(goal_embedding, query_text, k)
        .into_iter()
        .map(|c| (c.skill_ref, c.semantic.to_bits(), c.lexical.to_bits()))
        .collect()
}

proptest! {
    // Bounded case count keeps this embed-heavy test fast and deterministic.
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// Property 1: Single source of truth (Validates: Requirements 5.1).
    ///
    /// For an arbitrary skill set (the source of truth) and query set:
    /// - two independently rebuilt indexes (`A`, `B`) agree on every query, and
    /// - rebuilding the same index a second time is idempotent on query results.
    #[test]
    fn idempotent_reindex_yields_identical_search(
        specs in prop::collection::vec(skill_spec_strategy(), 0..15),
        queries in prop::collection::vec(query_strategy(), 1..6),
    ) {
        // One deterministic runtime drives the async rebuild/embed calls.
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("current-thread runtime");

        rt.block_on(async {
            // Build the skill set with unique skill_ids (the source of truth).
            let skills: Vec<SkillMetadata> = specs
                .iter()
                .enumerate()
                .map(|(i, spec)| build_metadata(i, spec))
                .collect();

            // Shared deterministic embedder (hash fallback in CI, no downloads).
            let embedder: Arc<dyn Embedder> = Arc::new(
                MemoryEmbedder::load(EMBED_DIM).expect("frozen embedder (hash fallback in CI)"),
            );

            // Pre-embed the queries once (same embedder → same vectors for A/B).
            let mut query_embeddings: Vec<Vec<f32>> = Vec::with_capacity(queries.len());
            for q in &queries {
                let v = embedder.embed(q).await.expect("query embed");
                query_embeddings.push(v);
            }

            // Index A: new + rebuild from the skill set.
            let index_a = CapabilityIndex::new(Arc::clone(&embedder));
            index_a.rebuild(&skills).await.expect("rebuild A");

            // Index B: a SECOND, independent index rebuilt from the SAME set.
            let index_b = CapabilityIndex::new(Arc::clone(&embedder));
            index_b.rebuild(&skills).await.expect("rebuild B");

            // A few k values, including k > skill count (returns all).
            let ks = [1usize, 3, 10];

            for (q_text, q_emb) in queries.iter().zip(query_embeddings.iter()) {
                for &k in &ks {
                    let sig_a = search_signature(&index_a, q_emb, q_text, k);
                    let sig_b = search_signature(&index_b, q_emb, q_text, k);
                    // Cross-instance: two rebuilds of the same source of truth
                    // must return byte-identical results (skill_ref + signals).
                    prop_assert_eq!(
                        &sig_a,
                        &sig_b,
                        "independent reindex disagreed (A != B) for query {:?} k={}",
                        q_text,
                        k
                    );
                }
            }

            // Idempotence: rebuild index A a SECOND time from the same set and
            // assert every query's result is identical to the first rebuild.
            let mut first: Vec<Vec<(Option<String>, u32, u32)>> = Vec::new();
            for (q_text, q_emb) in queries.iter().zip(query_embeddings.iter()) {
                first.push(search_signature(&index_a, q_emb, q_text, 10));
                let _ = q_text;
            }

            index_a.rebuild(&skills).await.expect("rebuild A again");

            for ((q_text, q_emb), before) in
                queries.iter().zip(query_embeddings.iter()).zip(first.iter())
            {
                let after = search_signature(&index_a, q_emb, q_text, 10);
                prop_assert_eq!(
                    before,
                    &after,
                    "second rebuild of same index changed results for query {:?}",
                    q_text
                );
            }

            Ok(())
        })?;
    }
}
