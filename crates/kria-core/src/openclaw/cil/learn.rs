//! Learning loop (task 15.1) — the [`FeedbackLearner`].
//!
//! Closes the **discover → execute → learn** loop (R4.3). When a CIL execution
//! node completes, the learner funnels the outcome into the **frozen** learning
//! primitive [`SemanticSkillRouter::record_feedback`], which updates the
//! existing [`SkillStatistics`] rows in the [`ProductionSkillRegistry`]
//! (`success_rate`, `usage_count`, `average_latency_ms`, `average_resource_usage`).
//!
//! # Extend, never fork
//!
//! There is exactly **one** stats path in OpenClaw:
//! `SemanticSkillRouter::record_feedback` → `ProductionSkillRegistry::record_execution`
//! → the `skill_statistics` table. The `FeedbackLearner` is a *thin extension*
//! that routes CIL execution outcomes into that frozen primitive. It:
//!
//! - creates **no** second stats store and **no** new schema — it writes the
//!   existing stats tables only, via the frozen path;
//! - reimplements **no** rolling-average math — the registry owns that;
//! - keeps the registry as the sole source of truth for skill statistics.
//!
//! The updated `SkillStatistics` feed the `CapabilityRanker`'s
//! `popularity`/`success` signals on subsequent goals (task 15.2 wires the
//! ranker read; task 15.3 tests the influence). Task 15.1 is the recording
//! extension only.

use std::sync::Arc;

use super::CilError;
use crate::openclaw::semantic_router::SemanticSkillRouter;

/// The outcome of a single executed capability node, as observed by the CIL
/// facade after the frozen `ExecutionEngine` finishes a node.
///
/// This is a plain data carrier so the handler can hand per-node results to the
/// learner without coupling the learner to the execution graph internals. Every
/// field maps 1:1 onto the frozen [`SemanticSkillRouter::record_feedback`]
/// parameters.
#[derive(Debug, Clone)]
pub struct NodeOutcome {
    /// The skill that executed (the registry key).
    pub skill_id: String,
    /// Whether the node completed successfully.
    pub success: bool,
    /// Wall-clock latency of the node in milliseconds.
    pub latency_ms: u64,
    /// Normalized resource usage observed for the node (frozen semantics).
    pub resource_usage: f64,
    /// Routing/plan confidence associated with the node (frozen semantics).
    pub confidence: f32,
}

/// Funnels CIL execution outcomes into the frozen learning primitive.
///
/// `FeedbackLearner` holds an [`Arc`] to the frozen [`SemanticSkillRouter`] and
/// delegates every recording call to [`SemanticSkillRouter::record_feedback`].
/// It adds no state of its own and owns no statistics — the registry behind the
/// router remains the single source of truth.
#[derive(Clone)]
pub struct FeedbackLearner {
    /// The frozen router whose `record_feedback` is THE learning primitive.
    router: Arc<SemanticSkillRouter>,
}

impl FeedbackLearner {
    /// Create a learner that extends the given frozen router's `record_feedback`.
    pub fn new(router: Arc<SemanticSkillRouter>) -> Self {
        Self { router }
    }

    /// Record a single node/skill completion by delegating to the frozen
    /// [`SemanticSkillRouter::record_feedback`].
    ///
    /// The parameter list mirrors the frozen signature exactly. The router
    /// updates the existing [`SkillStatistics`](crate::openclaw::registry::SkillStatistics)
    /// row (success rate, usage count, latency, resource usage) through
    /// `ProductionSkillRegistry::record_execution`; no new tables are written.
    ///
    /// A frozen `RouterError` is surfaced honestly as [`CilError::Io`] (a stats
    /// persistence failure) rather than silently swallowed.
    pub async fn record(
        &self,
        skill_id: &str,
        success: bool,
        latency_ms: u64,
        resource_usage: f64,
        confidence: f32,
    ) -> Result<(), CilError> {
        self.router
            .record_feedback(skill_id, success, latency_ms, resource_usage, confidence)
            .await
            .map_err(|e| CilError::Io(format!("record feedback for {skill_id}: {e}")))
    }

    /// Convenience wrapper that records a single [`NodeOutcome`]. Thin sugar over
    /// [`record`](Self::record); performs no stats math itself.
    pub async fn record_node(&self, outcome: &NodeOutcome) -> Result<(), CilError> {
        self.record(
            &outcome.skill_id,
            outcome.success,
            outcome.latency_ms,
            outcome.resource_usage,
            outcome.confidence,
        )
        .await
    }

    /// Record every executed node of a multi-capability plan, in order. A thin
    /// loop over the frozen primitive — the first failure to persist is returned
    /// honestly and stops the batch.
    pub async fn record_all(&self, outcomes: &[NodeOutcome]) -> Result<(), CilError> {
        for outcome in outcomes {
            self.record_node(outcome).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::registry::{
        DiscoverySource, ProductionSkillRegistry, SkillMetadata, SkillState,
    };
    use crate::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};
    use crate::safety::RiskLevel;
    use chrono::Utc;
    use tempfile::TempDir;

    fn sample_skill(skill_id: &str) -> SkillMetadata {
        SkillMetadata {
            skill_id: skill_id.to_string(),
            name: skill_id.to_string(),
            description: "test skill".to_string(),
            publisher: "test".to_string(),
            version: "1.0.0".to_string(),
            category: "test".to_string(),
            discovery_source: DiscoverySource::Bundled {
                path: "test".to_string(),
            },
            discovered_at: Utc::now(),
            capabilities: SkillCapabilities::default(),
            runtime_requirements: "docker".to_string(),
            risk_level: RiskLevel::Green,
            resource_class: ResourceClass::Light,
            tags: vec!["test".to_string()],
            categories: vec!["test".to_string()],
            semantic_version: "1.0.0".to_string(),
            dependencies: vec![],
            compatibility_requirements: vec![],
            trust_tier: TrustTier::Community,
            content_hash: format!("hash_{skill_id}"),
            signature: None,
            granted_capabilities: Vec::new(),
            bundle_path: None,
            manifest_toml: None,
            input_schema: None,
            state: SkillState::Enabled,
            state_changed_at: Utc::now(),
        }
    }

    fn setup() -> (TempDir, Arc<ProductionSkillRegistry>, FeedbackLearner) {
        let dir = TempDir::new().expect("temp dir");
        let db_path = dir.path().join("learn_test.db");
        let registry = Arc::new(ProductionSkillRegistry::new(&db_path).expect("create registry"));
        let router = Arc::new(SemanticSkillRouter::new(Arc::clone(&registry), None));
        let learner = FeedbackLearner::new(router);
        (dir, registry, learner)
    }

    // ---- Task 15.3: feedback → stats → ranking influence (R4.3) -----------
    //
    // These tests exercise the FULL discover→execute→learn loop from the WRITE
    // side: a run recorded through the [`FeedbackLearner`] shifts the frozen
    // `SkillStatistics`, and the ranker's `popularity`/`success` signals reflect
    // that change on the next goal. They compose the real learning primitive
    // (`FeedbackLearner`) with the real read seam (`RegistryStatistics`) and the
    // real ranker (`DefaultCapabilityRanker::with_statistics`) over ONE frozen
    // registry — no mocks, no second stats store. This complements rank.rs's
    // `registry_statistics_closes_the_learn_loop` (which drives the loop via the
    // frozen `record_execution` directly): here the loop is driven through the
    // CIL `FeedbackLearner` extension, covering the write side.

    use crate::openclaw::cil::config::RankWeights;
    use crate::openclaw::cil::index::{CandidateSource, CapabilityCandidate};
    use crate::openclaw::cil::intent::GoalIntent;
    use crate::openclaw::cil::profile::{CapabilityProfile, CapabilityTag};
    use crate::openclaw::cil::rank::{
        CapabilityRanker, DefaultCapabilityRanker, RegistryStatistics,
        DEFAULT_POPULARITY_HALF_SATURATION,
    };

    /// A one-tag profile for a skill (structurally identical across skills so
    /// only the learned signals can separate them in ranking).
    fn stats_profile(skill_id: &str) -> CapabilityProfile {
        CapabilityProfile {
            skill_id: skill_id.to_string(),
            provides: vec![CapabilityTag::new("x")],
            consumes: vec![],
            permissions: vec![],
            inputs: vec![],
            outputs: vec![],
        }
    }

    /// A candidate carrying zeroed learned signals — the ranker fills
    /// `popularity`/`success` from the statistics source it reads.
    fn stats_candidate(skill_id: &str) -> CapabilityCandidate {
        let profile = stats_profile(skill_id);
        CapabilityCandidate {
            capability: profile.provides[0].clone(),
            skill_ref: Some(skill_id.to_string()),
            source: CandidateSource::Installed,
            profile: Some(profile),
            // Identical non-learned signals for every candidate so ordering is
            // driven purely by the learned popularity/success.
            semantic: 0.5,
            lexical: 0.5,
            compatibility: 0.0,
            trust: 0.0,
            quality: 0.0,
            popularity: 0.0,
            success: 0.0,
        }
    }

    /// A goal with no required capabilities: ranking then leans on the learned
    /// signals we weight below.
    fn empty_intent() -> GoalIntent {
        GoalIntent {
            raw: "goal".into(),
            goal_embedding: vec![],
            required: vec![],
            composite: false,
            max_risk: RiskLevel::Green,
        }
    }

    /// Weights that isolate the learned signals (popularity + success only).
    fn learned_only_weights() -> RankWeights {
        RankWeights {
            semantic: 0.0,
            lexical: 0.0,
            compatibility: 0.0,
            trust: 0.0,
            quality: 0.0,
            popularity: 1.0,
            success: 1.0,
        }
    }

    /// A SUCCESSFUL run recorded through the `FeedbackLearner` shifts the frozen
    /// `SkillStatistics`, and on the next goal the ranker (reading via
    /// `RegistryStatistics`) surfaces the raised `popularity`/`success` signals,
    /// lifting the executed skill above a never-run peer. (R4.3, write side.)
    #[tokio::test]
    async fn successful_run_shifts_stats_and_ranker_signals() {
        let (_dir, registry, learner) = setup();
        registry
            .install_skill(&sample_skill("oc_used"))
            .expect("install used");
        registry
            .install_skill(&sample_skill("oc_fresh"))
            .expect("install fresh");

        // Baseline: no usage, so RegistryStatistics leaves signals untouched.
        assert_eq!(
            registry
                .get_skill_statistics("oc_used")
                .expect("stats")
                .usage_count,
            0
        );

        // Drive the loop through the FeedbackLearner (write side): enough
        // successful runs to reach the half-saturation popularity point.
        for _ in 0..DEFAULT_POPULARITY_HALF_SATURATION as u64 {
            learner
                .record("oc_used", true, 100, 0.1, 0.9)
                .await
                .expect("record success");
        }

        // The frozen stats table shifted via the frozen `record_feedback` path.
        let shifted = registry.get_skill_statistics("oc_used").expect("stats");
        assert_eq!(
            shifted.usage_count,
            DEFAULT_POPULARITY_HALF_SATURATION as u64
        );
        assert_eq!(shifted.success_rate, 1.0, "all successes => rate 1.0");

        // Next goal: the ranker reads those updated stats through RegistryStatistics.
        let ranker = DefaultCapabilityRanker::with_statistics(RegistryStatistics::new(Arc::clone(
            &registry,
        )));
        let mut cands = vec![stats_candidate("oc_used"), stats_candidate("oc_fresh")];
        ranker.rank(&empty_intent(), &mut cands, &learned_only_weights());

        // The executed skill ranks first; its learned signals came from the
        // shifted stats, while the never-run skill's stayed untouched.
        assert_eq!(cands[0].skill_ref.as_deref(), Some("oc_used"));
        let used = cands
            .iter()
            .find(|c| c.skill_ref.as_deref() == Some("oc_used"))
            .unwrap();
        let fresh = cands
            .iter()
            .find(|c| c.skill_ref.as_deref() == Some("oc_fresh"))
            .unwrap();
        assert!(
            (used.success - 1.0).abs() < 1e-6,
            "success signal from stats"
        );
        assert!(
            (used.popularity - 0.5).abs() < 1e-6,
            "usage==half_saturation => popularity 0.5, got {}",
            used.popularity
        );
        assert_eq!(
            fresh.popularity, 0.0,
            "never-run => popularity signal untouched"
        );
        assert_eq!(fresh.success, 0.0, "never-run => success signal untouched");
    }

    /// A FAILED run recorded through the `FeedbackLearner` also shifts the frozen
    /// `SkillStatistics` (lowering `success_rate`), and the ranker's `success`
    /// signal reflects the drop on the next goal — ranking the mostly-failing
    /// skill below an all-success peer under a success-weighted goal. (R4.3.)
    #[tokio::test]
    async fn failed_run_shifts_stats_and_lowers_ranker_success_signal() {
        let (_dir, registry, learner) = setup();
        registry
            .install_skill(&sample_skill("oc_reliable"))
            .expect("install reliable");
        registry
            .install_skill(&sample_skill("oc_flaky"))
            .expect("install flaky");

        // Equal usage counts so `popularity` matches; only `success` differs.
        // Reliable: 2 successes. Flaky: 1 success + 1 failure (rate 0.5).
        learner
            .record("oc_reliable", true, 100, 0.1, 0.9)
            .await
            .expect("ok");
        learner
            .record("oc_reliable", true, 100, 0.1, 0.9)
            .await
            .expect("ok");
        learner
            .record("oc_flaky", true, 100, 0.1, 0.9)
            .await
            .expect("ok");
        learner
            .record("oc_flaky", false, 200, 0.2, 0.9)
            .await
            .expect("ok");

        // The failure shifted the frozen stats: flaky's success_rate dropped.
        let reliable_stats = registry.get_skill_statistics("oc_reliable").expect("stats");
        let flaky_stats = registry.get_skill_statistics("oc_flaky").expect("stats");
        assert_eq!(reliable_stats.usage_count, 2);
        assert_eq!(flaky_stats.usage_count, 2);
        assert_eq!(reliable_stats.success_rate, 1.0);
        assert!(
            (flaky_stats.success_rate - 0.5).abs() < 1e-9,
            "one failure of two => 0.5, got {}",
            flaky_stats.success_rate
        );

        // Weight success only so ordering is driven by the learned success signal
        // (popularity is equal, so it cannot separate them).
        let w = RankWeights {
            semantic: 0.0,
            lexical: 0.0,
            compatibility: 0.0,
            trust: 0.0,
            quality: 0.0,
            popularity: 0.0,
            success: 1.0,
        };
        let ranker = DefaultCapabilityRanker::with_statistics(RegistryStatistics::new(Arc::clone(
            &registry,
        )));
        let mut cands = vec![stats_candidate("oc_flaky"), stats_candidate("oc_reliable")];
        ranker.rank(&empty_intent(), &mut cands, &w);

        // The failure pushed the flaky skill below the reliable one, and the
        // ranker's success signals mirror the shifted stats exactly.
        assert_eq!(cands[0].skill_ref.as_deref(), Some("oc_reliable"));
        let reliable = cands
            .iter()
            .find(|c| c.skill_ref.as_deref() == Some("oc_reliable"))
            .unwrap();
        let flaky = cands
            .iter()
            .find(|c| c.skill_ref.as_deref() == Some("oc_flaky"))
            .unwrap();
        assert!((reliable.success - 1.0).abs() < 1e-6);
        assert!(
            (flaky.success - 0.5).abs() < 1e-6,
            "failed run lowered ranker success signal, got {}",
            flaky.success
        );
        assert!(
            flaky.success < reliable.success,
            "failed run must lower the flaky skill's success signal below the reliable one"
        );
    }

    /// Recording a success via the frozen `record_feedback` path bumps
    /// `usage_count` and drives `success_rate` to 1.0 in the existing stats
    /// table — read back through the frozen registry.
    #[tokio::test]
    async fn record_success_updates_frozen_statistics() {
        let (_dir, registry, learner) = setup();
        registry
            .install_skill(&sample_skill("oc_alpha"))
            .expect("install");

        let before = registry.get_skill_statistics("oc_alpha").expect("stats");
        assert_eq!(before.usage_count, 0);

        learner
            .record("oc_alpha", true, 120, 0.25, 0.9)
            .await
            .expect("record success");

        let after = registry.get_skill_statistics("oc_alpha").expect("stats");
        assert_eq!(after.usage_count, 1, "usage_count must increment");
        assert_eq!(after.success_rate, 1.0, "single success => rate 1.0");
        assert_eq!(after.average_latency_ms, 120.0);
        assert!(after.last_execution.is_some());
    }

    /// A failure recording lowers `success_rate` below 1.0 while still counting
    /// the usage — proving the frozen rolling-average path (not a second store)
    /// is what changes.
    #[tokio::test]
    async fn record_failure_lowers_success_rate() {
        let (_dir, registry, learner) = setup();
        registry
            .install_skill(&sample_skill("oc_beta"))
            .expect("install");

        learner
            .record("oc_beta", true, 100, 0.1, 0.8)
            .await
            .expect("ok");
        learner
            .record("oc_beta", false, 200, 0.2, 0.8)
            .await
            .expect("ok");

        let stats = registry.get_skill_statistics("oc_beta").expect("stats");
        assert_eq!(stats.usage_count, 2);
        assert!(
            (stats.success_rate - 0.5).abs() < 1e-9,
            "one success of two => 0.5, got {}",
            stats.success_rate
        );
        assert!(stats.failure_rate > 0.0);
    }

    /// `record_all` funnels each `NodeOutcome` through the frozen primitive.
    #[tokio::test]
    async fn record_all_records_each_node() {
        let (_dir, registry, learner) = setup();
        registry
            .install_skill(&sample_skill("oc_gamma"))
            .expect("install");

        let outcomes = vec![
            NodeOutcome {
                skill_id: "oc_gamma".to_string(),
                success: true,
                latency_ms: 50,
                resource_usage: 0.1,
                confidence: 0.7,
            },
            NodeOutcome {
                skill_id: "oc_gamma".to_string(),
                success: true,
                latency_ms: 70,
                resource_usage: 0.2,
                confidence: 0.7,
            },
        ];
        learner.record_all(&outcomes).await.expect("record all");

        let stats = registry.get_skill_statistics("oc_gamma").expect("stats");
        assert_eq!(stats.usage_count, 2);
        assert_eq!(stats.success_rate, 1.0);
        assert_eq!(stats.average_latency_ms, 60.0);
    }
}
