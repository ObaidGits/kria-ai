//! Property-based test for the **single-source-of-truth / idempotent-reindex**
//! invariant of the `capability_profiles` derived view (task 2.5).
//!
//! # Property 1 — Single source of truth (design §Correctness Properties)
//!
//! **Validates: Requirements 5.1**
//!
//! Requirement 5.1: *"rebuilding all derived views from the registry yields
//! identical query results (idempotent reindex)."* The
//! [`ProductionSkillRegistry`] is the sole authoritative store; every
//! `capability_profiles` row is a rebuildable projection of it via
//! [`extract_profile`] + [`ProfileStore`]. This test encodes three facets of
//! that invariant over an arbitrary registry population:
//!
//! 1. **Idempotence** — re-running the full derive-and-persist pass over the
//!    same registry state produces byte-identical query results (`R1 == R2`).
//! 2. **Recovery-by-rebuild** (R5.1/R5.3 spirit) — dropping the entire derived
//!    table and rebuilding from the registry reproduces the same query results
//!    (`R1 == R3`), so corruption/drift is always recoverable by a full reindex,
//!    never manual repair.
//! 3. **Determinism of the projection** — because [`extract_profile`] is a pure,
//!    deterministic function of `SkillMetadata`, the reindex is a well-defined
//!    function of registry state alone (no ordering, time, or randomness leaks).
//!
//! The generator produces 0..=20 skills with **unique** `skill_id`s and varying
//! `category`/`categories`/`dependencies`/`input_schema` shapes — including a
//! deliberately novel, never-enumerated `CapabilityTag` domain — to exercise the
//! projection across the open-vocabulary input space (no hardcoding).
//!
//! [`ProductionSkillRegistry`]: crate::openclaw::registry::ProductionSkillRegistry
//! [`extract_profile`]: crate::openclaw::cil::extract::extract_profile
//! [`ProfileStore`]: crate::openclaw::cil::extract::ProfileStore

use proptest::prelude::*;

use crate::openclaw::cil::extract::ProfileStore;
use crate::openclaw::registry::{
    DiscoverySource, ProductionSkillRegistry, SkillDependency, SkillMetadata, SkillState,
};
use crate::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};
use crate::safety::RiskLevel;

/// A generated skill "shape" — the varying inputs that feed the extractor.
/// The `skill_id` is assigned by index at build time to guarantee uniqueness
/// while these fields still vary freely.
#[derive(Debug, Clone)]
struct SkillSpec {
    id_stub: String,
    category: String,
    categories: Vec<String>,
    dep_ids: Vec<String>,
    schema: Option<serde_json::Value>,
}

/// Open-vocabulary capability strings, mixing common namespaced ids, a
/// deliberately **novel** domain, and freely-generated reverse-DNS strings.
/// This keeps the input space open (no closed enum) per Requirement 1.
fn cap_string() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("media.image".to_string()),
        Just("doc.pdf".to_string()),
        Just("io.file.read".to_string()),
        Just("net.email.send".to_string()),
        // Never-before-seen domain: flows through as an open string, zero code.
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
                "count": { "type": "integer" },
                "opt": { "type": ["string", "null"] }
            }
        }))),
        Just(Some(serde_json::json!({
            "type": "array",
            "items": { "type": ["string", "null"] }
        }))),
        Just(Some(serde_json::json!({
            "type": "object",
            "properties": {
                "nested": {
                    "type": "object",
                    "properties": { "x": { "type": "number" } }
                }
            }
        }))),
    ]
}

fn skill_spec_strategy() -> impl Strategy<Value = SkillSpec> {
    (
        "[a-z]{1,6}",
        cap_string(),
        prop::collection::vec(cap_string(), 0..4),
        prop::collection::vec("[a-z]{1,6}\\.[a-z]{1,6}", 0..3),
        schema_strategy(),
    )
        .prop_map(
            |(id_stub, category, categories, dep_ids, schema)| SkillSpec {
                id_stub,
                category,
                categories,
                dep_ids,
                schema,
            },
        )
}

/// Build enabled `SkillMetadata` from a spec with an index-derived unique id.
fn build_metadata(index: usize, spec: &SkillSpec) -> SkillMetadata {
    // Index guarantees a unique skill_id even when id_stubs collide.
    let skill_id = format!("skill.{}.{index}", spec.id_stub);
    SkillMetadata {
        skill_id: skill_id.clone(),
        name: format!("Skill {skill_id}"),
        description: "pbt skill".to_string(),
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
        tags: vec!["pbt".to_string()],
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
        // Enabled so `get_enabled_skills()` (the source-of-truth read) returns it.
        state: SkillState::Enabled,
        state_changed_at: chrono::Utc::now(),
    }
}

/// A deterministic, comparable snapshot of ALL derived query results, keyed and
/// ordered by `skill_id`. Each entry is the serialized derived profile plus its
/// row-level `profile_epoch` and `embedding`. This is the "query result" `R`
/// the invariant compares (`R1 == R2 == R3`).
fn query_results(
    store: &ProfileStore,
    skill_ids: &[String],
) -> Vec<(String, String, i64, Option<Vec<f32>>)> {
    let mut sorted = skill_ids.to_vec();
    sorted.sort();
    sorted.dedup();
    sorted
        .into_iter()
        .filter_map(|id| {
            let row = store
                .get_profile(&id)
                .expect("get_profile must not error")?;
            let json = serde_json::to_string(&row.profile).expect("profile serializes");
            Some((id, json, row.profile_epoch, row.embedding))
        })
        .collect()
}

/// Derive + persist a profile for every skill currently in the registry
/// (the source of truth), simulating a full reindex pass.
fn reindex_all(registry: &ProductionSkillRegistry, store: &ProfileStore, epoch: i64) {
    let skills = registry
        .get_enabled_skills()
        .expect("registry query is the source of truth");
    for meta in &skills {
        store
            .derive_and_persist(meta, None, epoch)
            .expect("derive + persist");
    }
}

proptest! {
    // Bounded case count keeps the DB-backed test fast and deterministic.
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Property 1: Single source of truth (Validates: Requirements 5.1).
    ///
    /// For an arbitrary registry population, rebuilding all derived profiles from
    /// the registry is idempotent (`R1 == R2`) and fully recoverable by dropping
    /// and rebuilding the derived table (`R1 == R3`).
    #[test]
    fn idempotent_reindex_is_single_source_of_truth(specs in prop::collection::vec(skill_spec_strategy(), 0..20)) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");

        // Fresh registry (the SOLE source of truth) + derived-view store over the
        // SAME skills.db. Frozen migration 3 creates `capability_profiles`.
        let registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let store = ProfileStore::open(&db_path).expect("profile store open");

        // Register the generated skills (unique skill_ids via index).
        let mut skill_ids = Vec::with_capacity(specs.len());
        for (i, spec) in specs.iter().enumerate() {
            let meta = build_metadata(i, spec);
            registry.install_skill(&meta).expect("register skill");
            skill_ids.push(meta.skill_id);
        }

        // Sanity: every registered skill is visible through the source of truth.
        let enabled = registry.get_enabled_skills().expect("enabled query");
        prop_assert_eq!(enabled.len(), skill_ids.len());

        // Pass 1: build all derived profiles from the registry → R1.
        reindex_all(&registry, &store, 0);
        let r1 = query_results(&store, &skill_ids);

        // R1 must cover exactly the registered skills.
        prop_assert_eq!(r1.len(), skill_ids.len());

        // Pass 2: rebuild again from the SAME registry state (idempotent) → R2.
        reindex_all(&registry, &store, 0);
        let r2 = query_results(&store, &skill_ids);
        prop_assert_eq!(&r1, &r2, "idempotent reindex changed query results (R1 != R2)");

        // Recovery-by-rebuild: drop the entire derived table, then rebuild → R3.
        for id in &skill_ids {
            store.delete_profile(id).expect("clear derived row");
        }
        let cleared = query_results(&store, &skill_ids);
        prop_assert!(cleared.is_empty(), "derived table not actually cleared");

        reindex_all(&registry, &store, 0);
        let r3 = query_results(&store, &skill_ids);
        prop_assert_eq!(&r1, &r3, "rebuild-after-clear did not reproduce query results (R1 != R3)");
    }
}
