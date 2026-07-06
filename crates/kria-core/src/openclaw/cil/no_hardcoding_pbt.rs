//! Property-based test for the **no-hardcoding / open-extensibility** invariant
//! of the fused [`CapabilityIndex`] (dense ANN + BM25) — task 3.6.
//!
//! # Property 2 — No hardcoding / open extensibility (design §Correctness Properties)
//!
//! **Validates: Requirements 1.1**
//!
//! Requirement 1.1: *"WHERE a `CapabilityTag` has never been encountered before,
//! THE Capability_Intelligence_Layer SHALL perform discovery … for that tag
//! without any code change and without any branch that enumerates specific
//! capabilities."* This test demonstrates that empirically for the **discovery**
//! stage: an arbitrary, freely-generated, never-before-seen reverse-DNS
//! [`CapabilityTag`] id is embedded, indexed (dense + BM25), and searchable
//! through the **same** [`CapabilityIndex`] code path used by any built-in
//! capability — with zero code change.
//!
//! # How novelty is guaranteed
//!
//! The novel capability id is generated from the open-vocabulary strategy
//! `"[a-z]{3,10}(\\.[a-z]{3,10}){1,3}"` (a reverse-DNS-ish string of 2..=4
//! segments). It is then constrained by [`prop_filter`] so that:
//!
//! 1. it is **not equal** to any id in the fixed [`KNOWN_CAPS`] set (the
//!    "common-looking" capabilities the code might plausibly have seen), and
//! 2. its tokens are **disjoint** from both the [`KNOWN_CAPS`] tokens and a small
//!    set of metadata [`RESERVED_TOKENS`] boilerplate words.
//!
//! Because CIL contains **no closed enumeration** of capabilities anywhere, every
//! generated id is by construction one the code has never special-cased — the
//! filter's only job is to keep the novel id's tokens *disjoint* from the common
//! skill's tokens so the two queries cleanly resolve to their own skill (no
//! accidental cross-match), making the assertions exact rather than approximate.
//!
//! # What is asserted (the essence of open extensibility)
//!
//! For a skill set containing one **common** skill (a `KNOWN_CAPS` id), one
//! **novel** skill (the generated id), and 0..=3 common distractors — all built
//! and indexed through the identical [`CapabilityIndex::rebuild`] path:
//!
//! 1. **Extraction flows through with no per-category branch** — the novel
//!    skill's derived [`CapabilityProfile`] (via [`extract_profile`]) lists the
//!    novel id in `provides`, exactly like any known category.
//! 2. **The novel capability is embedded, indexed, and searchable** — querying
//!    for the novel id returns the novel skill as a candidate through the same
//!    `search` path, and it ranks **first** for its own query (its unique tokens
//!    give it the dominant lexical signal).
//! 3. **Identical treatment vs a known-looking capability** — in the same index,
//!    querying for the common id returns the common skill first. Novel and known
//!    capabilities are discovered by one code path with no special-casing; a
//!    never-seen tag is not privileged nor penalized.
//!
//! If a generated novel tag ever failed to be indexed/searchable, that would be a
//! real no-hardcoding violation and the counterexample is reported (the property
//! is never weakened to tolerate it).
//!
//! [`CapabilityIndex`]: crate::openclaw::cil::index::CapabilityIndex
//! [`extract_profile`]: crate::openclaw::cil::extract::extract_profile
//! [`CapabilityProfile`]: crate::openclaw::cil::profile::CapabilityProfile

use std::collections::BTreeSet;
use std::sync::Arc;

use proptest::prelude::*;

use crate::openclaw::cil::embed::{Embedder, MemoryEmbedder};
use crate::openclaw::cil::extract::extract_profile;
use crate::openclaw::cil::index::CapabilityIndex;
use crate::openclaw::registry::{DiscoverySource, SkillMetadata, SkillState};
use crate::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};
use crate::safety::RiskLevel;

/// Small embedding dimension keeps the embed-heavy test fast while still
/// exercising the real aggregate/normalize dense path.
const EMBED_DIM: usize = 32;

/// "Common-looking" capability ids the code might plausibly have encountered.
/// The novel generated id is guaranteed distinct from — and token-disjoint with —
/// every id here, so the novel and common queries resolve to their own skills.
const KNOWN_CAPS: &[&str] = &[
    "io.file.read",
    "net.email.send",
    "doc.pdf.render",
    "media.image.ocr",
];

/// Boilerplate tokens that appear in every generated skill's metadata (name,
/// description, runtime, etc.). The novel id must not reuse any of these, so its
/// query tokens stay unique to the novel skill.
const RESERVED_TOKENS: &[&str] = &[
    "pbt",
    "skill",
    "providing",
    "provides",
    "docker",
    "test",
    "light",
    "green",
    "misc",
    "bundled",
    "name",
    "hash",
    "tag",
    "tags",
    "novel",
    "common",
];

/// Lowercase alphanumeric tokenization mirroring the index's own tokenizer, used
/// only to enforce token-disjointness in the generator's filter.
fn tokens_of(s: &str) -> BTreeSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

/// The union of all `KNOWN_CAPS` tokens plus `RESERVED_TOKENS` — the set the
/// novel id's tokens must avoid.
fn forbidden_tokens() -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for cap in KNOWN_CAPS {
        set.extend(tokens_of(cap));
    }
    for t in RESERVED_TOKENS {
        set.insert((*t).to_string());
    }
    set
}

/// Strategy over a freely-generated, never-before-seen reverse-DNS capability id.
///
/// Generates 2..=4 lowercase segments, then filters to guarantee the id is
/// genuinely novel: not equal to any `KNOWN_CAPS` id and token-disjoint from the
/// forbidden token set (see module docs — novelty is inherent since CIL has no
/// capability enum; the filter only preserves query cleanliness).
fn novel_cap_id() -> impl Strategy<Value = String> {
    "[a-z]{3,10}(\\.[a-z]{3,10}){1,3}".prop_filter(
        "novel id must be distinct from and token-disjoint with known/reserved tokens",
        |id| {
            if KNOWN_CAPS.contains(&id.as_str()) {
                return false;
            }
            let forbidden = forbidden_tokens();
            tokens_of(id).is_disjoint(&forbidden)
        },
    )
}

/// Build an enabled `SkillMetadata` whose `category`/`categories` carry `cap_id`
/// as the (open-vocabulary) capability it provides.
fn build_skill(skill_id: &str, cap_id: &str) -> SkillMetadata {
    SkillMetadata {
        skill_id: skill_id.to_string(),
        // Name/description use only boilerplate tokens + the capability id, so the
        // only unique tokens a skill contributes are those of its capability id.
        name: format!("pbt skill {skill_id}"),
        description: format!("pbt skill providing {cap_id}"),
        publisher: "test".to_string(),
        version: "1.0.0".to_string(),
        category: cap_id.to_string(),
        discovery_source: DiscoverySource::Bundled {
            path: "test".to_string(),
        },
        discovered_at: chrono::Utc::now(),
        capabilities: SkillCapabilities::default(),
        runtime_requirements: "docker".to_string(),
        risk_level: RiskLevel::Green,
        resource_class: ResourceClass::Light,
        tags: vec![],
        categories: vec![cap_id.to_string()],
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

/// Whether `search(query)` returns `skill_id` among its top-`k` candidates.
fn returns_skill(
    index: &CapabilityIndex,
    goal_embedding: &[f32],
    query_text: &str,
    k: usize,
    skill_id: &str,
) -> bool {
    index
        .search(goal_embedding, query_text, k)
        .iter()
        .any(|c| c.skill_ref.as_deref() == Some(skill_id))
}

proptest! {
    // Bounded case count keeps this embed-heavy test fast and deterministic.
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// Property 2: No hardcoding / open extensibility (Validates: Requirements 1.1).
    ///
    /// A never-before-seen `CapabilityTag` id flows through extraction, embedding,
    /// indexing, and search on the same code path as a known capability — with no
    /// branch enumerating capabilities.
    #[test]
    fn novel_capability_is_indexed_and_searchable(
        novel_id in novel_cap_id(),
        // Pick the "common-looking" capability for the known skill.
        common_idx in 0usize..KNOWN_CAPS.len(),
        // 0..=3 additional common distractors so discovery is non-trivial.
        distractor_idxs in prop::collection::vec(0usize..KNOWN_CAPS.len(), 0..4),
    ) {
        let common_id = KNOWN_CAPS[common_idx];

        // ---- Part 1: extraction flows through with no per-category branch. ----
        // The novel skill's derived profile lists the novel id in `provides`,
        // exactly like any known category would be — proving the extractor does
        // not enumerate capabilities.
        let novel_skill = build_skill("skill.novel.0", &novel_id);
        let novel_profile = extract_profile(&novel_skill);
        prop_assert!(
            novel_profile.provides.iter().any(|t| t.id == novel_id),
            "novel capability {:?} must appear in the derived profile's `provides` \
             (extraction must not branch on capability id)",
            novel_id
        );

        // ---- Build the skill set: one common, one novel, some distractors. ----
        let common_skill = build_skill("skill.common.0", common_id);
        let mut skills = vec![common_skill, novel_skill];
        for (i, idx) in distractor_idxs.iter().enumerate() {
            // Distractors carry other known ids; unique skill_ids keep them apart.
            skills.push(build_skill(&format!("skill.distractor.{i}"), KNOWN_CAPS[*idx]));
        }
        let skill_count = skills.len();

        // One deterministic runtime drives the async rebuild/embed calls.
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("current-thread runtime");

        rt.block_on(async {
            // Deterministic embedder (hash fallback in CI, no downloads).
            let embedder: Arc<dyn Embedder> = Arc::new(
                MemoryEmbedder::load(EMBED_DIM).expect("frozen embedder (hash fallback in CI)"),
            );

            // Index every skill through the SAME rebuild path — no capability is
            // special-cased on the way in.
            let index = CapabilityIndex::new(Arc::clone(&embedder));
            index.rebuild(&skills).await.expect("rebuild");

            // Embed the two queries (the capability ids themselves) with the same
            // embedder used to index — this is the GoalIntent seam.
            let novel_emb = embedder.embed(&novel_id).await.expect("embed novel query");
            let common_emb = embedder.embed(common_id).await.expect("embed common query");

            // ---- Part 2: the novel capability is embedded, indexed, searchable. ----
            // (a) With k covering all skills, the novel skill is returned — it was
            //     embedded and indexed through the same path as everyone else.
            prop_assert!(
                returns_skill(&index, &novel_emb, &novel_id, skill_count, "skill.novel.0"),
                "novel skill must be discoverable for its own query {:?} \
                 (indexed via the same code path)",
                novel_id
            );

            // (b) Searchability is real, not just "return everything": querying for
            //     the novel id ranks the novel skill FIRST (its unique tokens give
            //     it the dominant lexical signal). Uses a tight k.
            let tight_k = 3usize;
            let novel_hits = index.search(&novel_emb, &novel_id, tight_k);
            prop_assert!(
                novel_hits
                    .iter()
                    .any(|c| c.skill_ref.as_deref() == Some("skill.novel.0")),
                "novel skill must appear in the top-{} for its own query {:?}",
                tight_k,
                novel_id
            );
            prop_assert_eq!(
                novel_hits[0].skill_ref.as_deref(),
                Some("skill.novel.0"),
                "novel skill must rank first for its own query {:?} \
                 (searchable through the same fused dense+BM25 path)",
                &novel_id
            );

            // ---- Part 3: identical treatment vs a known-looking capability. ----
            // In the SAME index, the common capability's query returns the common
            // skill first — no special-casing distinguishes novel from known.
            let common_hits = index.search(&common_emb, common_id, tight_k);
            prop_assert!(
                common_hits
                    .iter()
                    .any(|c| c.skill_ref.as_deref() == Some("skill.common.0")),
                "common skill must appear in the top-{} for its own query {:?}",
                tight_k,
                common_id
            );
            prop_assert_eq!(
                common_hits[0].skill_ref.as_deref(),
                Some("skill.common.0"),
                "common skill must rank first for its own query {:?} \
                 (same code path as the novel capability)",
                common_id
            );

            Ok(())
        })?;
    }
}
