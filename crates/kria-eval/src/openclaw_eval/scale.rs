//! Scale validation (tasks.md task 20, design.md "Scale validation").
//!
//! Real-code grounding: design.md's scale requirement (>=1000 skills, >=100
//! publishers, marketplace sync/search, registry lookup, routing under
//! scale) is ALREADY substantially covered by pre-existing, real, passing
//! tests (re-confirmed in this session's full lib runs):
//! - `platform::tests::stress_thousand_skill_repository` — 1000 skills
//!   across EXACTLY 100 publishers, real `RepositoryManager`/`Marketplace`
//!   sync + search.
//! - `semantic_router_tests::test_thousand_skill_benchmark` — real
//!   `SemanticSkillRouter::route` against 1000 registered skills ("Routed
//!   through 1000 skills in 11ms").
//! - `registry_tests::test_registry_stress` — 100 concurrent installs +
//!   concurrent search/state-change against the real `ProductionSkillRegistry`.
//!
//! This module does NOT duplicate that coverage. It adds the ONE genuinely
//! missing piece: a REAL 1000-skill install into `ProductionSkillRegistry`
//! (not just the marketplace-index layer, and past the 100-skill ceiling of
//! `test_registry_stress`), with real per-install and per-search LATENCY
//! measured — the "degradation analysis" dimension design.md asks for that
//! no existing test captures.

use kria_core::openclaw::registry::{
    DiscoverySource, ProductionSkillRegistry, SkillMetadata, SkillQuery, SkillState,
};
use kria_core::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};
use kria_core::safety::RiskLevel;
use std::time::Instant;

fn fixture_metadata(i: usize) -> SkillMetadata {
    SkillMetadata {
        skill_id: format!("oc_scale_{i}"),
        name: format!("Scale Fixture {i}"),
        description: format!("Scale validation fixture skill number {i}."),
        publisher: format!("pub{}", i % 100),
        version: "1.0.0".into(),
        category: "test".into(),
        discovery_source: DiscoverySource::Bundled {
            path: "test".into(),
        },
        discovered_at: chrono::Utc::now(),
        capabilities: SkillCapabilities::default(),
        runtime_requirements: "docker".into(),
        risk_level: RiskLevel::Green,
        resource_class: ResourceClass::Light,
        tags: vec!["scale".into()],
        categories: vec!["test".into()],
        semantic_version: "1.0.0".into(),
        dependencies: vec![],
        compatibility_requirements: vec![],
        trust_tier: TrustTier::Local,
        content_hash: format!("hash_{i}"),
        signature: None,
        granted_capabilities: Vec::new(),
        bundle_path: None,
        manifest_toml: None,
        input_schema: None,
        state: SkillState::Enabled,
        state_changed_at: chrono::Utc::now(),
    }
}

/// Real 1000-skill install into the real `ProductionSkillRegistry`
/// (SQLite-backed), measuring install latency degradation across the run —
/// the piece no existing test measures.
pub struct ScaleReport {
    pub total_installs: usize,
    pub first_100_avg_ms: f64,
    pub last_100_avg_ms: f64,
    pub search_latency_ms: f64,
    pub lookup_latency_ms: f64,
}

pub fn validate_1000_skill_install_and_search() -> Result<ScaleReport, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let db_path = dir.path().join("scale_1000.db");
    let registry = ProductionSkillRegistry::new(&db_path).map_err(|e| e.to_string())?;

    let mut durations_ms = Vec::with_capacity(1000);
    for i in 0..1000 {
        let start = Instant::now();
        registry
            .install_skill(&fixture_metadata(i))
            .map_err(|e| format!("install {i} failed: {e}"))?;
        durations_ms.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    let first_100_avg_ms = durations_ms[..100].iter().sum::<f64>() / 100.0;
    let last_100_avg_ms = durations_ms[900..].iter().sum::<f64>() / 100.0;

    let search_start = Instant::now();
    let query = SkillQuery {
        slug: None,
        publisher: None,
        description_contains: Some("number 5.".into()),
        tags: vec![],
        categories: vec![],
        capabilities: vec![],
        runtime_requirements: None,
        risk_level: None,
        state: None,
        enabled_only: false,
    };
    let results = registry.search_skills(&query).map_err(|e| e.to_string())?;
    let search_latency_ms = search_start.elapsed().as_secs_f64() * 1000.0;
    if results.is_empty() {
        return Err(
            "search over 1000 real installed skills returned no results for a known substring"
                .into(),
        );
    }

    let lookup_start = Instant::now();
    registry
        .get("oc_scale_500")
        .map_err(|e| format!("lookup of a known skill failed: {e}"))?;
    let lookup_latency_ms = lookup_start.elapsed().as_secs_f64() * 1000.0;

    Ok(ScaleReport {
        total_installs: 1000,
        first_100_avg_ms,
        last_100_avg_ms,
        search_latency_ms,
        lookup_latency_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real 1000-skill install + search/lookup latency, with a bounded
    /// degradation budget: the LAST 100 inserts must not be more than 5x
    /// slower on average than the FIRST 100 — a concrete, measured
    /// degradation-analysis assertion (not just "it completed").
    #[test]
    #[ignore = "slow (1000 real SQLite installs) — run explicitly with --ignored"]
    fn scale_1000_skills_no_severe_degradation() {
        let report = validate_1000_skill_install_and_search()
            .expect("1000-skill install+search must succeed");
        eprintln!(
            "[SCALE] total={} first_100_avg={:.3}ms last_100_avg={:.3}ms search={:.3}ms lookup={:.3}ms",
            report.total_installs, report.first_100_avg_ms, report.last_100_avg_ms, report.search_latency_ms, report.lookup_latency_ms
        );

        assert!(
            report.search_latency_ms < 1000.0,
            "search over 1000 skills must complete in under 1s, got {:.3}ms",
            report.search_latency_ms
        );
        assert!(report.lookup_latency_ms < 100.0, "a single-skill lookup must complete in under 100ms even at 1000-skill scale, got {:.3}ms", report.lookup_latency_ms);

        let degradation_factor = report.last_100_avg_ms / report.first_100_avg_ms.max(0.001);
        assert!(
            degradation_factor < 5.0,
            "install latency must not degrade more than 5x from first-100 to last-100 avg, got {degradation_factor:.2}x \
             (first={:.3}ms, last={:.3}ms)",
            report.first_100_avg_ms,
            report.last_100_avg_ms
        );
    }
}
