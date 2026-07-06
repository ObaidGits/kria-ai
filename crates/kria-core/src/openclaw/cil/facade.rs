//! `CapabilityIntelligence` — the CIL facade and single entry point the handler
//! calls when `openclaw_icp_enabled` is ON (design §8.8, task 5.3).
//!
//! This is the phase-5 wiring of the discover → rank → plan stages (design §9,
//! stages 1–4) for the **installed** single-skill case. It composes the frozen
//! building blocks introduced by earlier phases — the [`Embedder`], the fused
//! [`CapabilityIndex`], the multi-signal [`CapabilityRanker`], the frozen
//! [`ModelRouter`] (for one structured goal-intent call), and the frozen
//! [`AuditLedger`] — and produces a [`Fulfillment`]:
//!
//! - [`Fulfillment::Plan`] — a **1-node** frozen [`ExecutionGraph`] referencing
//!   the selected installed skill, when the top-ranked candidate clears the
//!   config compatibility threshold. Permission decisions are filled by task 11;
//!   an empty `Vec` is returned here.
//! - [`Fulfillment::Decline`] — honest decline when no acceptable installed
//!   candidate exists.
//!
//! The multi-capability planner and the frozen-engine execution are later phases
//! (§10/§11 and tasks 10.x); this facade never touches containers — it hands a
//! frozen `ExecutionGraph` back to the handler, preserving KRIA orchestration
//! authority.
//!
//! # Honesty invariant (R7.1)
//!
//! Every decision stage — goal-intent derivation, discovery, ranking, and the
//! terminal plan/decline — emits an [`AuditLedger`] entry so the decision trail
//! is honest telemetry, never a fabricated success. A degraded backend (no LLM,
//! embedder failure) is reported truthfully via [`CilError`] and never panics.
//!
//! # No hardcoding (R4.4)
//!
//! Selection is driven entirely by the [`GoalIntent`], the ranked candidate
//! signals, and [`CilConfig`] thresholds/weights. There is **no** per-skill or
//! per-category branch anywhere: a never-before-seen capability id flows through
//! discovery, ranking, and planning identically to a built-in.

use std::sync::Arc;

use super::config::CilConfig;
use super::embed::Embedder;
use super::index::{CandidateSource, CapabilityCandidate, CapabilityIndex};
use super::intent::{derive_goal_intent_llm, GoalIntent};
use super::market::{MarketCandidate, MarketIndex};
use super::profile::CapabilityTag;
use super::rank::CapabilityRanker;
use super::recommend::Recommender;
use super::{CilError, Fulfillment, PermissionDecision, RequestCtx};
use crate::execution::{ExecutionGraph, GraphNode, NodeKind};
use crate::llm::ModelRouter;
use crate::openclaw::audit::{AuditEntry, AuditLedger};
use crate::openclaw::types::{AuditEventType, TrustTier};
use crate::safety::RiskLevel;

/// Audit stage identifiers, recorded in the entry's `tool_name` column so the
/// decision trail for one `fulfill` call is queryable by stage.
mod stage {
    pub const GOAL_INTENT: &str = "cil.goal_intent";
    pub const DISCOVERY: &str = "cil.discovery";
    pub const RANKING: &str = "cil.ranking";
    pub const PLAN: &str = "cil.plan";
    pub const RECOMMEND: &str = "cil.recommend";
    pub const DECLINE: &str = "cil.decline";
}

/// Map a marketplace [`TrustTier`] hint to a `0.0..=1.0` trust signal for the
/// ranker, mirroring the frozen [`SemanticSkillRouter`]'s trust-tier scale
/// (Verified 1.0 / Community 0.8 / Local 0.6 / Untrusted 0.0) so an installed
/// and a marketplace candidate score trust on the same axis. Data-derived, no
/// per-skill branch.
///
/// [`SemanticSkillRouter`]: crate::openclaw::semantic_router::SemanticSkillRouter
fn trust_tier_score(tier: TrustTier) -> f32 {
    match tier {
        TrustTier::Verified => 1.0,
        TrustTier::Community => 0.8,
        TrustTier::Local => 0.6,
        TrustTier::Untrusted => 0.0,
    }
}

/// The CIL facade (design §8.8).
///
/// Holds the composed frozen building blocks needed for the installed
/// single-skill path. Later phases extend this with the marketplace index,
/// capability graph, acquisition orchestrator, planner, permission engine,
/// recommender, and learner (design §8.8); those fields are additive and not
/// required for task 5.3.
pub struct CapabilityIntelligence {
    /// Fused semantic + lexical discovery index over installed skills (task 3.3).
    index: Arc<CapabilityIndex>,
    /// Multi-signal ranker (task 5.2). `dyn` so the backend stays pluggable.
    ranker: Arc<dyn CapabilityRanker>,
    /// Embedder for goal-intent derivation (task 3.1 / 5.1).
    embedder: Arc<dyn Embedder>,
    /// Frozen model router for the single structured goal-intent call (task 5.1).
    /// `None` → honest degraded: `fulfill` cannot derive intent and reports it.
    llm: Option<Arc<ModelRouter>>,
    /// Data-only thresholds + weights (design §8.4/§8.8, no hardcoded constants).
    config: CilConfig,
    /// Frozen append-only audit ledger for per-stage honesty telemetry.
    audit: Arc<AuditLedger>,
    /// The maximum risk a derived goal may reach, threaded from request policy so
    /// risk authority stays with the frozen safety layer.
    max_risk: RiskLevel,
    /// Optional offline-embedded federated marketplace catalog (task 6.2).
    ///
    /// `None` → installed-only discovery, exactly the phase-5 behavior (task 5.3
    /// tests + the handler wiring construct the facade this way). `Some` → task
    /// 6.4 marketplace discovery runs **in parallel** with installed discovery and
    /// its candidates are merged into the ranked set. Wire it via
    /// [`CapabilityIntelligence::with_market`]. `MarketIndex::search` is a pure
    /// offline cache read (R9.2 — never a live per-query marketplace fetch).
    market: Option<Arc<MarketIndex>>,
    /// Optional pure-read recommender (task 7.1/7.2, design §8.7).
    ///
    /// `None` → the terminal single-skill decision behaves exactly as task
    /// 5.3/6.4: no acceptable installed candidate → honest
    /// [`Fulfillment::Decline`]. `Some` → when the goal needs a capability with
    /// no acceptable **installed** candidate, the facade instead asks the
    /// recommender for ranked marketplace candidates (a pure offline read, R8.2)
    /// and, when it returns a NON-EMPTY set, emits [`Fulfillment::Recommend`]
    /// including any alternatives/successors the recommender's capability graph
    /// knows about (R8.4). An empty result keeps the honest decline (R8.5 —
    /// never a fabricated recommendation). Wire it via
    /// [`CapabilityIntelligence::with_recommender`].
    recommender: Option<Arc<dyn Recommender + Send + Sync>>,
}

impl CapabilityIntelligence {
    /// Construct the phase-5 facade. This is the constructor the handler's
    /// `with_cil` wiring needs to flip the ICP path live.
    ///
    /// `llm` is optional: with no model router the facade cannot derive a
    /// `GoalIntent` and every `fulfill` honestly reports a degraded backend
    /// rather than fabricating one.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        index: Arc<CapabilityIndex>,
        ranker: Arc<dyn CapabilityRanker>,
        embedder: Arc<dyn Embedder>,
        llm: Option<Arc<ModelRouter>>,
        config: CilConfig,
        audit: Arc<AuditLedger>,
        max_risk: RiskLevel,
    ) -> Self {
        Self {
            index,
            ranker,
            embedder,
            llm,
            config,
            audit,
            max_risk,
            // Installed-only by default; opt into marketplace discovery with
            // `with_market` so task 5.3's constructor/tests stay unchanged.
            market: None,
            // No recommender by default; opt in with `with_recommender` so the
            // no-acceptable-installed-candidate case declines honestly exactly as
            // task 5.3/6.4 unless a recommender is explicitly wired.
            recommender: None,
        }
    }

    /// Enable marketplace discovery (task 6.4) by attaching an offline-embedded
    /// [`MarketIndex`]. Builder-style so the existing [`new`](Self::new) signature
    /// (and every caller/test wired against it) is untouched.
    ///
    /// With a market attached, [`fulfill`](Self::fulfill) discovers marketplace
    /// candidates from the pre-embedded `market_catalog` cache **in parallel** with
    /// installed discovery (R4.2) and merges them into the ranked candidate set.
    /// The read is pure offline (R9.2): no live per-query marketplace fetch.
    ///
    /// Note: the terminal single-skill decision still requires an **installed**
    /// top candidate (see [`is_acceptable`](Self::is_acceptable)); surfacing a
    /// marketplace-only match as a `Recommend`/acquisition is tasks 7/8. This
    /// method only makes marketplace candidates flow into the ranked set.
    #[must_use]
    pub fn with_market(mut self, market: Arc<MarketIndex>) -> Self {
        self.market = Some(market);
        self
    }

    /// Enable intelligent recommendations (task 7.2, design §8.7) by attaching a
    /// pure-read [`Recommender`]. Builder-style so the existing
    /// [`new`](Self::new)/[`with_market`](Self::with_market) signatures (and every
    /// caller/test wired against them) are untouched — additive only.
    ///
    /// With a recommender attached, the terminal single-skill decision changes in
    /// exactly one place: when there is **no acceptable installed candidate** (the
    /// case that previously always [`Declined`](Fulfillment::Decline)), the facade
    /// first asks the recommender for ranked marketplace candidates the user could
    /// install. If it returns a NON-EMPTY set the facade emits
    /// [`Fulfillment::Recommend`] (including alternatives/successors from the
    /// recommender's capability graph, R8.4); if it returns an EMPTY set the honest
    /// [`Fulfillment::Decline`] is preserved (R8.5 — no fabrication). The
    /// recommendation is a **pure read** (R8.2): nothing is installed here —
    /// acquisition is task 8.
    #[must_use]
    pub fn with_recommender(mut self, recommender: Arc<dyn Recommender + Send + Sync>) -> Self {
        self.recommender = Some(recommender);
        self
    }

    /// The single method the handler calls when the ICP flag is ON (design §8.8).
    ///
    /// Runs stages 1–4 of design §9 for the installed single-skill case:
    /// 1. Derive [`GoalIntent`] via one embed + one structured LLM call.
    /// 2. Discover installed candidates via [`CapabilityIndex::search`].
    /// 3. Rank them via [`CapabilityRanker::rank`] with config weights.
    /// 4. Return a 1-node [`Fulfillment::Plan`] for an acceptable top candidate,
    ///    else an honest [`Fulfillment::Decline`].
    ///
    /// An [`AuditLedger`] entry is emitted at each stage (honesty invariant,
    /// R7.1). Backend failures surface as [`CilError`], never a panic.
    pub async fn fulfill(&self, query: &str, ctx: &RequestCtx) -> Result<Fulfillment, CilError> {
        let invocation_id = uuid::Uuid::new_v4().to_string();

        // ── Stage 1: derive the goal intent (embed + one structured LLM call).
        let Some(llm) = self.llm.as_ref() else {
            // Honest degraded: no LLM backend wired → cannot derive intent.
            self.emit_stage(
                &invocation_id,
                ctx,
                stage::GOAL_INTENT,
                AuditEventType::InvocationFailed,
                "",
                false,
                "no LLM backend wired for goal-intent derivation (degraded)",
            );
            return Err(CilError::Degraded(
                "no LLM backend available for goal-intent derivation".to_string(),
            ));
        };

        let intent = match derive_goal_intent_llm(
            query,
            self.embedder.as_ref(),
            llm.as_ref(),
            self.max_risk,
        )
        .await
        {
            Ok(intent) => intent,
            Err(e) => {
                self.emit_stage(
                    &invocation_id,
                    ctx,
                    stage::GOAL_INTENT,
                    AuditEventType::InvocationFailed,
                    "",
                    false,
                    format!("goal-intent derivation failed: {e}"),
                );
                return Err(e);
            }
        };
        self.emit_stage(
            &invocation_id,
            ctx,
            stage::GOAL_INTENT,
            AuditEventType::InvocationStarted,
            "",
            true,
            format!(
                "required={} composite={} max_risk={:?}",
                intent.required.len(),
                intent.composite,
                intent.max_risk
            ),
        );

        // Run the discover → rank → decide pipeline (also used by tests that
        // supply a pre-derived intent without a live LLM).
        Ok(self.plan_for_intent(&invocation_id, ctx, &intent).await)
    }

    /// Stages 2–3: discover installed + marketplace candidates **in parallel**,
    /// merge them into ONE set, and rank it with the config weights. Emits the
    /// discovery and ranking audit entries and returns the ranked candidate set.
    ///
    /// # Parallel discovery (R4.2, task 6.4)
    ///
    /// Installed discovery ([`CapabilityIndex::search`]) and marketplace discovery
    /// ([`MarketIndex::search`] over the offline `market_catalog` cache) both run
    /// **concurrently** via [`tokio::join!`], each on a [`spawn_blocking`] worker
    /// (both are synchronous CPU/DB reads). When no [`market`](Self::market) is
    /// wired the marketplace side resolves to an empty set immediately, so the
    /// installed-only path is behaviorally unchanged from task 5.3. Marketplace
    /// discovery is a **pure offline cache read** — never a live per-query fetch
    /// (R9.2). Both candidate sets are then merged into ONE `Vec` and handed to
    /// the ranker so installed and marketplace candidates are ranked together.
    ///
    /// [`spawn_blocking`]: tokio::task::spawn_blocking
    async fn ranked_candidates(
        &self,
        invocation_id: &str,
        ctx: &RequestCtx,
        intent: &GoalIntent,
    ) -> Vec<CapabilityCandidate> {
        // ── Stage 2: discover installed + marketplace candidates IN PARALLEL.
        // Discovery breadth is a config value (no hardcoded constant).
        let k = self.config.planner_max_breadth.max(1);

        // Installed discovery over the frozen fused CapabilityIndex.
        let installed_index = Arc::clone(&self.index);
        let installed_goal = intent.goal_embedding.clone();
        let installed_raw = intent.raw.clone();
        let installed_fut = async move {
            tokio::task::spawn_blocking(move || {
                installed_index.search(&installed_goal, &installed_raw, k)
            })
            .await
            // A join (panic/cancel) failure degrades honestly to no installed
            // candidates rather than aborting the whole decision.
            .unwrap_or_default()
        };

        // Marketplace discovery over the offline-embedded market_catalog cache
        // (R9.2 — pure cache read, no live fetch). Absent market → empty set.
        let market = self.market.clone();
        let market_goal = intent.goal_embedding.clone();
        let market_fut = async move {
            match market {
                Some(m) => tokio::task::spawn_blocking(move || m.search(&market_goal, k))
                    .await
                    .unwrap_or_else(|_| Ok(Vec::new())),
                None => Ok(Vec::new()),
            }
        };

        // Both discovery reads run concurrently; join collects both results.
        let (mut candidates, market_result) = tokio::join!(installed_fut, market_fut);
        let installed_count = candidates.len();

        // Merge marketplace candidates into the single ranked candidate set. A
        // market-side error degrades honestly to installed-only discovery.
        let market_candidates = match market_result {
            Ok(m) => m,
            Err(e) => {
                self.emit_stage(
                    invocation_id,
                    ctx,
                    stage::DISCOVERY,
                    AuditEventType::InvocationFailed,
                    "",
                    false,
                    format!("marketplace discovery failed (installed-only): {e}"),
                );
                Vec::new()
            }
        };
        let market_count = market_candidates.len();
        for mc in market_candidates {
            candidates.push(market_candidate_to_capability(mc));
        }

        self.emit_stage(
            invocation_id,
            ctx,
            stage::DISCOVERY,
            AuditEventType::InvocationStarted,
            "",
            true,
            format!(
                "candidates={} installed={} market={} k={}",
                candidates.len(),
                installed_count,
                market_count,
                k
            ),
        );

        // ── Stage 3: rank with config weights (fills compatibility, sorts).
        self.ranker
            .rank(intent, &mut candidates, &self.config.weights);
        self.emit_stage(
            invocation_id,
            ctx,
            stage::RANKING,
            AuditEventType::InvocationStarted,
            candidates
                .first()
                .and_then(|c| c.skill_ref.as_deref())
                .unwrap_or(""),
            true,
            format!("ranked={}", candidates.len()),
        );

        candidates
    }

    /// Discover → rank → decide over an already-derived [`GoalIntent`] (stages
    /// 2–4). Split out from [`fulfill`] so the deterministic core is testable
    /// without a live LLM. Emits the discovery, ranking, and terminal
    /// (plan/decline) audit entries. Discovery runs installed and marketplace
    /// sources in parallel via [`ranked_candidates`](Self::ranked_candidates).
    async fn plan_for_intent(
        &self,
        invocation_id: &str,
        ctx: &RequestCtx,
        intent: &GoalIntent,
    ) -> Fulfillment {
        // ── Stages 2–3: parallel discovery (installed ∥ marketplace) + merge +
        // rank. Returns the single ranked candidate set.
        let candidates = self.ranked_candidates(invocation_id, ctx, intent).await;

        // ── Stage 4: single-skill decision.
        match candidates.first() {
            Some(top) if self.is_acceptable(top) => {
                let skill_id = top
                    .skill_ref
                    .clone()
                    .expect("acceptable candidate has a skill_ref");
                let graph = single_skill_graph(invocation_id, &skill_id);
                self.emit_stage(
                    invocation_id,
                    ctx,
                    stage::PLAN,
                    AuditEventType::InvocationCompleted,
                    &skill_id,
                    true,
                    format!(
                        "plan=single-skill compatibility={:.3} trust={:.3}",
                        top.compatibility, top.trust
                    ),
                );
                // Permission decisions are filled in task 11; empty for now.
                let decisions: Vec<PermissionDecision> = Vec::new();
                Fulfillment::Plan(graph, decisions)
            }
            other => {
                // No acceptable INSTALLED candidate. Before declining, if a
                // recommender is wired, ask it for ranked marketplace candidates
                // the user could install to satisfy the goal (task 7.2, R8.1).
                // A NON-EMPTY result → Recommend; EMPTY → honest Decline (R8.5).
                if let Some(recs) = self.recommend_for(invocation_id, ctx, intent, &candidates) {
                    return recs;
                }
                let reason = decline_reason(other, self.config.compatibility_threshold);
                self.emit_stage(
                    invocation_id,
                    ctx,
                    stage::DECLINE,
                    AuditEventType::InvocationCompleted,
                    other.and_then(|c| c.skill_ref.as_deref()).unwrap_or(""),
                    true,
                    reason.clone(),
                );
                Fulfillment::Decline { reason }
            }
        }
    }

    /// When the single-skill decision has no acceptable **installed** candidate
    /// and a [`Recommender`] is wired, produce a [`Fulfillment::Recommend`] of the
    /// ranked marketplace candidates the user could install (task 7.2, design
    /// §8.7). Returns `None` when no recommender is wired **or** the recommender
    /// honestly finds nothing above threshold — in both cases the caller keeps the
    /// honest [`Fulfillment::Decline`] (R8.5, no fabrication).
    ///
    /// # Pure read (R8.2)
    ///
    /// [`Recommender::recommend`] queries only the offline market cache (+ optional
    /// capability graph for alternatives/successors, R8.4). Nothing is installed or
    /// mutated here — acquisition is task 8.
    ///
    /// `installed_skill_ids` is derived from the ranked set's **installed**
    /// candidates so the recommender filters out skills the user already has.
    fn recommend_for(
        &self,
        invocation_id: &str,
        ctx: &RequestCtx,
        intent: &GoalIntent,
        candidates: &[CapabilityCandidate],
    ) -> Option<Fulfillment> {
        let recommender = self.recommender.as_ref()?;

        // Already-installed skills from the ranked set → filtered out by the
        // recommender (no point recommending what the user already has).
        let installed_skill_ids: Vec<String> = candidates
            .iter()
            .filter(|c| matches!(c.source, CandidateSource::Installed))
            .filter_map(|c| c.skill_ref.clone())
            .collect();

        // Recommendation breadth mirrors discovery breadth (config value, no
        // hardcoded constant).
        let k = self.config.planner_max_breadth.max(1);

        match recommender.recommend(
            &intent.goal_embedding,
            &installed_skill_ids,
            k,
            &self.config,
        ) {
            // NON-EMPTY → surface honest, ranked options to install (R8.1/R8.4).
            Ok(recs) if !recs.is_empty() => {
                self.emit_stage(
                    invocation_id,
                    ctx,
                    stage::RECOMMEND,
                    AuditEventType::InvocationCompleted,
                    recs.first().map(|r| r.slug.as_str()).unwrap_or(""),
                    true,
                    format!(
                        "recommend={} installed_filtered={}",
                        recs.len(),
                        installed_skill_ids.len()
                    ),
                );
                Some(Fulfillment::Recommend(recs))
            }
            // EMPTY → nothing above threshold; keep the honest decline (R8.5).
            Ok(_) => None,
            // A recommender read failure degrades honestly to a decline; the
            // failure is audited so the trail stays truthful (R7.1).
            Err(e) => {
                self.emit_stage(
                    invocation_id,
                    ctx,
                    stage::RECOMMEND,
                    AuditEventType::InvocationFailed,
                    "",
                    false,
                    format!("recommendation read failed (declining): {e}"),
                );
                None
            }
        }
    }

    /// Whether the top-ranked candidate is an acceptable single-skill match.
    ///
    /// Gate (design §9 stage 4 / §10 trust gate):
    /// - the candidate must correspond to a concrete skill (`skill_ref`), and
    /// - be an **installed** skill (this facade plans only over installed skills;
    ///   marketplace/generatable acquisition is a later phase), and
    /// - clear the config **compatibility** threshold.
    ///
    /// The **trust** threshold governs *acquiring new* (marketplace/generatable)
    /// candidates (design §10, "best candidate ≥ trust&compat threshold"). An
    /// already-installed skill was trust-gated at install time, so trust is
    /// treated as satisfied for it here; a non-installed candidate must clear the
    /// configured trust threshold. This keeps the check honest and generic — no
    /// per-skill or per-category branch.
    fn is_acceptable(&self, c: &CapabilityCandidate) -> bool {
        if c.skill_ref.is_none() {
            return false;
        }
        let trust_ok = match c.source {
            CandidateSource::Installed => true,
            _ => c.trust >= self.config.trust_threshold,
        };
        let compat_ok = c.compatibility >= self.config.compatibility_threshold;
        matches!(c.source, CandidateSource::Installed) && trust_ok && compat_ok
    }

    /// Build and sign a per-stage audit entry, appending it to the frozen ledger.
    ///
    /// A ledger append failure is logged and swallowed (the decision itself must
    /// still complete honestly) — it never panics and never aborts `fulfill`.
    #[allow(clippy::too_many_arguments)]
    fn emit_stage(
        &self,
        invocation_id: &str,
        ctx: &RequestCtx,
        stage: &str,
        event_type: AuditEventType,
        skill_id: &str,
        success: bool,
        detail: impl Into<String>,
    ) {
        let mut entry = AuditEntry {
            timestamp: chrono::Utc::now(),
            event_type,
            skill_id: skill_id.to_string(),
            invocation_id: invocation_id.to_string(),
            session_id: ctx.session_id.clone().unwrap_or_default(),
            turn_id: String::new(),
            tool_name: stage.to_string(),
            risk_level: format!("{:?}", self.max_risk),
            input_hash: String::new(),
            output_hash: String::new(),
            duration_ms: 0,
            success,
            error_summary: Some(detail.into()),
            resource_class: "cil".to_string(),
            container_id: String::new(),
            signature: String::new(),
        };
        entry.signature = self.audit.sign_entry(&entry);
        if let Err(e) = self.audit.append(&entry) {
            tracing::warn!(
                error = %e,
                stage = %stage,
                "CIL audit append failed; continuing (decision still honest)"
            );
        }
    }
}

/// Construct the 1-node frozen [`ExecutionGraph`] for a selected installed skill.
///
/// The node is a `NodeKind::Skill` dispatched to the frozen OpenClaw executor
/// with `action_id = skill_id`. Concrete arguments are generated by the frozen
/// arg-gen / permission phases at execution time (tasks 10.x/11); an empty params
/// object is the stable placeholder here. This facade never executes the graph —
/// it hands it to the frozen engine via the handler (KRIA orchestration authority).
fn single_skill_graph(invocation_id: &str, skill_id: &str) -> ExecutionGraph {
    let mut graph = ExecutionGraph::new(
        format!("cil-plan-{invocation_id}"),
        format!("cil-goal-{invocation_id}"),
    );
    let node = GraphNode::new(
        format!("skill-{skill_id}"),
        NodeKind::Skill {
            provider_id: crate::capability::acl::openclaw::OPENCLAW_PROVIDER_ID.to_string(),
            action_id: skill_id.to_string(),
            params: serde_json::json!({}),
        },
    )
    .with_label(skill_id.to_string());
    graph.add_node(node);
    graph
}

/// Map a marketplace [`MarketCandidate`] into the shared [`CapabilityCandidate`]
/// shape so it can be ranked alongside installed candidates (task 6.4).
///
/// - `source` carries the federated identity via
///   [`CandidateSource::Marketplace { provider_id, slug }`] so acquisition
///   (task 8) can resolve the exact catalog entry with no live fetch (R9.2).
/// - `skill_ref` is the marketplace `slug` (a marketplace candidate is not yet an
///   installed skill id; it becomes one only after acquisition/install).
/// - `semantic` is the offline cosine `score` computed against the pre-embedded
///   `market_catalog.embedding`, so it lands on the same semantic axis as
///   installed discovery.
/// - `trust`/`quality`/`popularity` are copied from the recorded R9.4 signals:
///   `trust` from the `trust_hint` tier mapped to `0.0..=1.0`
///   ([`trust_tier_score`]), and `quality`/`popularity` as-is where the provider
///   supplied them (absent → `0.0`, honestly not fabricated).
/// - `profile` is `None`: a marketplace candidate has no installed capability
///   profile yet (one is derived at install time).
///
/// `lexical`/`compatibility`/`success` default to `0.0` for the ranker to fill,
/// exactly as for a freshly discovered installed candidate. The `capability` tag
/// is derived from the `slug` (open vocabulary, no per-skill branch).
fn market_candidate_to_capability(mc: MarketCandidate) -> CapabilityCandidate {
    let trust = mc.trust_hint.map(trust_tier_score).unwrap_or(0.0);
    CapabilityCandidate {
        capability: CapabilityTag::new(mc.slug.clone()),
        skill_ref: Some(mc.slug.clone()),
        source: CandidateSource::Marketplace {
            provider_id: mc.provider_id,
            slug: mc.slug,
        },
        profile: None,
        semantic: mc.score,
        lexical: 0.0,
        compatibility: 0.0,
        trust,
        quality: mc.quality.unwrap_or(0.0) as f32,
        popularity: mc.popularity.unwrap_or(0.0) as f32,
        success: 0.0,
    }
}

/// The honest, user-facing reason for a decline (no acceptable installed match).
fn decline_reason(top: Option<&CapabilityCandidate>, compat_threshold: f32) -> String {
    match top {
        None => "no installed skill matched the goal".to_string(),
        Some(c) => format!(
            "no acceptable installed skill (best '{}' compatibility {:.3} < threshold {:.3})",
            c.skill_ref.as_deref().unwrap_or("<unknown>"),
            c.compatibility,
            compat_threshold
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::cil::embed::MemoryEmbedder;
    use crate::openclaw::cil::intent::derive_goal_intent;
    use crate::openclaw::cil::rank::DefaultCapabilityRanker;
    use crate::openclaw::registry::{DiscoverySource, SkillMetadata, SkillState};
    use crate::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};
    use chrono::Utc;
    use tempfile::TempDir;

    const TEST_HMAC_KEY: &[u8] = b"cil-facade-test-hmac-key-000000000000";

    fn sample_skill(
        skill_id: &str,
        name: &str,
        description: &str,
        category: &str,
    ) -> SkillMetadata {
        SkillMetadata {
            skill_id: skill_id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            publisher: "test".to_string(),
            version: "1.0.0".to_string(),
            category: category.to_string(),
            discovery_source: DiscoverySource::Bundled {
                path: "test".to_string(),
            },
            discovered_at: Utc::now(),
            capabilities: SkillCapabilities::default(),
            runtime_requirements: "docker".to_string(),
            risk_level: RiskLevel::Green,
            resource_class: ResourceClass::Light,
            tags: vec![category.to_string()],
            categories: vec![category.to_string()],
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

    /// Build a facade over a temp audit ledger with `skills` indexed. Returns the
    /// facade, the shared embedder, and the `TempDir` (kept alive for the db) plus
    /// the audit db path so tests can inspect the emitted entries.
    async fn setup(
        skills: &[SkillMetadata],
    ) -> (
        CapabilityIntelligence,
        Arc<dyn Embedder>,
        TempDir,
        std::path::PathBuf,
    ) {
        let dir = TempDir::new().expect("tempdir");
        let audit_path = dir.path().join("audit.db");
        let audit = Arc::new(
            AuditLedger::open(&audit_path, TEST_HMAC_KEY.to_vec()).expect("open audit ledger"),
        );

        let embedder: Arc<dyn Embedder> =
            Arc::new(MemoryEmbedder::load(64).expect("load embedder (hash fallback in CI)"));
        let index = Arc::new(CapabilityIndex::new(embedder.clone()));
        index.rebuild(skills).await.expect("rebuild index");

        let ranker: Arc<dyn CapabilityRanker> = Arc::new(DefaultCapabilityRanker::new());

        let facade = CapabilityIntelligence::new(
            index,
            ranker,
            embedder.clone(),
            None, // no LLM: plan_for_intent path is exercised directly
            CilConfig::default(),
            audit,
            RiskLevel::Green,
        );
        (facade, embedder, dir, audit_path)
    }

    /// Count emitted CIL audit entries by stage (`tool_name`) in the ledger db.
    fn audit_stages(db_path: &std::path::Path) -> Vec<String> {
        let conn = rusqlite::Connection::open(db_path).expect("open audit db");
        let mut stmt = conn
            .prepare("SELECT tool_name FROM audit_log WHERE tool_name LIKE 'cil.%' ORDER BY id")
            .expect("prepare");
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .expect("query")
            .map(|r| r.unwrap())
            .collect();
        rows
    }

    #[tokio::test]
    async fn single_skill_goal_yields_one_node_plan() {
        let skills = vec![sample_skill(
            "oc_pdf_compress",
            "PDF Compressor",
            "Compress and shrink PDF documents to reduce file size",
            "documents",
        )];
        let (facade, embedder, _dir, audit_path) = setup(&skills).await;

        // Intent whose text lexically matches the installed skill.
        let intent = derive_goal_intent(
            "compress a pdf document",
            embedder.as_ref(),
            vec![],
            false,
            RiskLevel::Green,
        )
        .await
        .expect("derive intent");

        let outcome = facade
            .plan_for_intent("inv-plan", &RequestCtx::default(), &intent)
            .await;

        match outcome {
            Fulfillment::Plan(graph, decisions) => {
                assert_eq!(graph.node_count(), 1, "single-skill plan is a 1-node graph");
                assert!(
                    decisions.is_empty(),
                    "permission decisions filled in task 11"
                );
                let node = graph.nodes().next().expect("one node");
                match &node.kind {
                    NodeKind::Skill {
                        provider_id,
                        action_id,
                        ..
                    } => {
                        assert_eq!(
                            provider_id,
                            crate::capability::acl::openclaw::OPENCLAW_PROVIDER_ID
                        );
                        assert_eq!(
                            action_id, "oc_pdf_compress",
                            "node references the selected skill"
                        );
                    }
                    other => panic!("expected a Skill node, got {other:?}"),
                }
            }
            other => panic!("expected Plan, got {other:?}"),
        }

        // Honesty: discovery, ranking, and the terminal plan stage are audited.
        let stages = audit_stages(&audit_path);
        assert_eq!(stages, vec!["cil.discovery", "cil.ranking", "cil.plan"]);
    }

    #[tokio::test]
    async fn no_candidate_declines_honestly() {
        // Empty index → discovery finds nothing → honest decline.
        let (facade, embedder, _dir, audit_path) = setup(&[]).await;

        let intent = derive_goal_intent(
            "do something no installed skill can",
            embedder.as_ref(),
            vec![],
            false,
            RiskLevel::Green,
        )
        .await
        .expect("derive intent");

        let outcome = facade
            .plan_for_intent("inv-decline", &RequestCtx::default(), &intent)
            .await;

        match outcome {
            Fulfillment::Decline { reason } => {
                assert!(!reason.is_empty(), "decline carries an honest reason");
            }
            other => panic!("expected Decline, got {other:?}"),
        }

        let stages = audit_stages(&audit_path);
        assert_eq!(stages, vec!["cil.discovery", "cil.ranking", "cil.decline"]);
    }

    #[tokio::test]
    async fn below_compatibility_threshold_declines() {
        let skills = vec![sample_skill(
            "oc_pdf_compress",
            "PDF Compressor",
            "Compress and shrink PDF documents",
            "documents",
        )];
        let (_f, embedder, dir, _audit_path) = setup(&skills).await;

        // Rebuild a facade with an impossibly high compatibility threshold so even
        // a discovered candidate is declined (threshold is data, not code).
        let audit = Arc::new(
            AuditLedger::open(&dir.path().join("audit2.db"), TEST_HMAC_KEY.to_vec()).unwrap(),
        );
        let index = Arc::new(CapabilityIndex::new(embedder.clone()));
        index.rebuild(&skills).await.unwrap();
        let mut config = CilConfig::default();
        config.compatibility_threshold = 2.0; // unreachable (signals are <= 1.0)
        let facade = CapabilityIntelligence::new(
            index,
            Arc::new(DefaultCapabilityRanker::new()),
            embedder.clone(),
            None,
            config,
            audit,
            RiskLevel::Green,
        );

        let intent = derive_goal_intent(
            "compress a pdf document",
            embedder.as_ref(),
            vec![],
            false,
            RiskLevel::Green,
        )
        .await
        .unwrap();

        let outcome = facade
            .plan_for_intent("inv-thresh", &RequestCtx::default(), &intent)
            .await;
        assert!(
            matches!(outcome, Fulfillment::Decline { .. }),
            "candidate below the compatibility threshold is declined"
        );
    }

    #[tokio::test]
    async fn fulfill_without_llm_is_honest_degraded() {
        // With no LLM wired, `fulfill` cannot derive intent → honest degraded
        // error and a failed goal-intent audit entry (never a panic/fake success).
        let (facade, _embedder, _dir, audit_path) = setup(&[]).await;

        let err = facade
            .fulfill("anything", &RequestCtx::default())
            .await
            .expect_err("no LLM → degraded error");
        assert!(matches!(err, CilError::Degraded(_)), "got {err:?}");

        let stages = audit_stages(&audit_path);
        assert_eq!(
            stages,
            vec!["cil.goal_intent"],
            "the degraded stage is audited"
        );
    }

    // ── Task 6.4: marketplace discovery in parallel with installed ───────────

    use crate::openclaw::cil::market::{MarketEntry, MarketIndex, MarketplaceProvider};
    use crate::openclaw::registry::ProductionSkillRegistry;
    use async_trait::async_trait;

    /// A no-network mock marketplace provider returning a fixed catalog. Trust is
    /// clamped to `Community` (never elevate a remote entry to `Verified`).
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
        fn trust_hint(&self, _entry: &MarketEntry) -> TrustTier {
            TrustTier::Community
        }
    }

    fn market_entry(provider: &str, slug: &str, name: &str, desc: &str) -> MarketEntry {
        MarketEntry {
            provider_id: provider.to_string(),
            slug: slug.to_string(),
            name: name.to_string(),
            description: desc.to_string(),
            category: "documents".to_string(),
            version: "1.0.0".to_string(),
            manifest_url: format!("https://example.com/{slug}/SKILL.md"),
            declared_trust: "community".to_string(),
            capabilities_summary: vec!["subprocess".to_string()],
            quality: Some(0.7),
            popularity: Some(0.5),
            deprecated: false,
        }
    }

    /// With a `MarketIndex` wired, marketplace candidates from the offline
    /// `market_catalog` cache appear in the ranked candidate set ALONGSIDE
    /// installed candidates (R4.2 parallel discovery + merge). The market read is
    /// a pure offline cache read (R9.2 — the provider's `fetch_manifest` errors,
    /// proving discovery never fetches live).
    #[tokio::test]
    async fn market_candidates_join_the_ranked_set() {
        let dir = TempDir::new().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        // Frozen registry migrations create the market_catalog table (migration 4).
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry migrations");

        // One shared embedder so installed + market candidates share a vector space.
        let embedder: Arc<dyn Embedder> =
            Arc::new(MemoryEmbedder::load(64).expect("load embedder"));

        // Installed side: one PDF-compress skill indexed.
        let installed = vec![sample_skill(
            "oc_pdf_compress",
            "PDF Compressor",
            "Compress and shrink PDF documents to reduce file size",
            "documents",
        )];
        let index = Arc::new(CapabilityIndex::new(embedder.clone()));
        index.rebuild(&installed).await.expect("rebuild index");

        // Market side: one marketplace PDF-compress entry, synced + embedded offline.
        let provider: Arc<dyn MarketplaceProvider> = Arc::new(MockProvider {
            id: "mock".to_string(),
            entries: vec![market_entry(
                "mock",
                "market_pdf_compress",
                "Market PDF Compressor",
                "Compress and shrink PDF documents to reduce file size",
            )],
        });
        let market = MarketIndex::open(&db_path, embedder.clone(), vec![provider])
            .expect("open market index");
        let report = market.sync().await.expect("sync market catalog");
        assert_eq!(
            report.upserted, 1,
            "one market entry embedded offline into the cache"
        );

        let audit = Arc::new(
            AuditLedger::open(&dir.path().join("audit.db"), TEST_HMAC_KEY.to_vec())
                .expect("open audit ledger"),
        );
        let facade = CapabilityIntelligence::new(
            index,
            Arc::new(DefaultCapabilityRanker::new()),
            embedder.clone(),
            None,
            CilConfig::default(),
            audit,
            RiskLevel::Green,
        )
        .with_market(Arc::new(market));

        let intent = derive_goal_intent(
            "compress a pdf document",
            embedder.as_ref(),
            vec![],
            false,
            RiskLevel::Green,
        )
        .await
        .expect("derive intent");

        let ranked = facade
            .ranked_candidates("inv-market", &RequestCtx::default(), &intent)
            .await;

        // Both an installed and a marketplace candidate are present in ONE set.
        let has_installed = ranked
            .iter()
            .any(|c| matches!(c.source, CandidateSource::Installed));
        let market_hit = ranked.iter().find(|c| {
            matches!(
                &c.source,
                CandidateSource::Marketplace { provider_id, slug }
                    if provider_id == "mock" && slug == "market_pdf_compress"
            )
        });
        assert!(
            has_installed,
            "installed candidate present in the ranked set"
        );
        let market_hit = market_hit.expect("marketplace candidate merged into the ranked set");
        assert_eq!(
            market_hit.skill_ref.as_deref(),
            Some("market_pdf_compress"),
            "market candidate's skill_ref is its slug"
        );
        assert!(
            market_hit.trust > 0.0,
            "trust_hint mapped onto the trust signal (Community → 0.8)"
        );
        assert!(
            (market_hit.quality - 0.7).abs() < 1e-6,
            "quality copied from the catalog signal"
        );
    }

    /// The installed-only path (no market wired) is behaviorally unchanged: the
    /// ranked set contains only installed candidates and the same audit stages
    /// as task 5.3.
    #[tokio::test]
    async fn market_none_keeps_installed_only_path() {
        let skills = vec![sample_skill(
            "oc_pdf_compress",
            "PDF Compressor",
            "Compress and shrink PDF documents",
            "documents",
        )];
        let (facade, embedder, _dir, audit_path) = setup(&skills).await;

        let intent = derive_goal_intent(
            "compress a pdf document",
            embedder.as_ref(),
            vec![],
            false,
            RiskLevel::Green,
        )
        .await
        .expect("derive intent");

        let ranked = facade
            .ranked_candidates("inv-nomarket", &RequestCtx::default(), &intent)
            .await;
        assert!(
            ranked
                .iter()
                .all(|c| matches!(c.source, CandidateSource::Installed)),
            "no market wired → only installed candidates"
        );

        // Discovery + ranking audited, no marketplace-failure entry.
        let stages = audit_stages(&audit_path);
        assert_eq!(stages, vec!["cil.discovery", "cil.ranking"]);
    }

    // ── Task 7.2: Recommend on capability-missing ────────────────────────────

    use crate::openclaw::cil::recommend::DefaultRecommender;

    /// Deterministic bag-of-tokens embedder (no model/network): shared vocabulary
    /// between two texts drives up cosine similarity, so a goal and a matching
    /// catalog entry land close in vector space (mirrors the recommender tests).
    struct BagEmbedder {
        dim: usize,
    }

    #[async_trait]
    impl Embedder for BagEmbedder {
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
            "bag-embedder-v1"
        }
    }

    /// Build a synced offline `MarketIndex` over a fresh migrated skills.db with
    /// the given entries, sharing `embedder` so goal + catalog embeddings live in
    /// one space. The returned `TempDir` keeps the db alive.
    async fn synced_market(
        embedder: Arc<dyn Embedder>,
        entries: Vec<MarketEntry>,
    ) -> (Arc<MarketIndex>, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry migrations");
        let provider: Arc<dyn MarketplaceProvider> = Arc::new(MockProvider {
            id: "mock".to_string(),
            entries,
        });
        let market = MarketIndex::open(&db_path, embedder, vec![provider]).expect("open market");
        market.sync().await.expect("sync market catalog");
        (Arc::new(market), dir)
    }

    /// With a recommender wired and NO acceptable installed candidate (empty
    /// installed index), the facade returns [`Fulfillment::Recommend`] carrying the
    /// ranked marketplace candidates, and audits a `cil.recommend` stage instead of
    /// `cil.decline` (task 7.2, R8.1).
    #[tokio::test]
    async fn no_installed_candidate_recommends_from_market() {
        // Bag-of-tokens embedder so a goal and a matching catalog entry share a
        // real (non-orthogonal) vector-space overlap.
        let embedder: Arc<dyn Embedder> = Arc::new(BagEmbedder { dim: 64 });

        // Offline market carries a matching skill; its text overlaps the goal so
        // the offline cosine clears the relevance threshold.
        let desc = "compress and shrink pdf documents to reduce file size";
        let (market, _mdir) = synced_market(
            embedder.clone(),
            vec![market_entry(
                "mock",
                "market_pdf_compress",
                "Market PDF Compressor",
                desc,
            )],
        )
        .await;

        // Fresh facade over an EMPTY installed index (→ no acceptable installed
        // candidate) plus a recommender.
        let tdir = TempDir::new().expect("tempdir");
        let audit = Arc::new(
            AuditLedger::open(&tdir.path().join("audit.db"), TEST_HMAC_KEY.to_vec())
                .expect("open audit ledger"),
        );
        let index = Arc::new(CapabilityIndex::new(embedder.clone()));
        index.rebuild(&[]).await.expect("rebuild empty index");
        let recommender: Arc<dyn Recommender + Send + Sync> =
            Arc::new(DefaultRecommender::new(market));
        // Relevance gate is a config value; use a low (non-zero) threshold so the
        // overlapping candidate is accepted deterministically in CI.
        let mut config = CilConfig::default();
        config.compatibility_threshold = 0.1;
        let facade = CapabilityIntelligence::new(
            index,
            Arc::new(DefaultCapabilityRanker::new()),
            embedder.clone(),
            None,
            config,
            audit,
            RiskLevel::Green,
        )
        .with_recommender(recommender);

        let intent = derive_goal_intent(
            "compress and shrink pdf documents",
            embedder.as_ref(),
            vec![],
            false,
            RiskLevel::Green,
        )
        .await
        .expect("derive intent");

        let audit_path = tdir.path().join("audit.db");
        let outcome = facade
            .plan_for_intent("inv-recommend", &RequestCtx::default(), &intent)
            .await;

        match outcome {
            Fulfillment::Recommend(recs) => {
                assert!(!recs.is_empty(), "recommend carries ranked candidates");
                assert!(
                    recs.iter().any(|r| r.slug == "market_pdf_compress"),
                    "the matching marketplace skill is recommended"
                );
            }
            other => panic!("expected Recommend, got {other:?}"),
        }

        // The terminal stage is an honest `cil.recommend`, not a decline.
        let stages = audit_stages(&audit_path);
        assert_eq!(
            stages,
            vec!["cil.discovery", "cil.ranking", "cil.recommend"]
        );
    }

    /// With a recommender wired but an EMPTY market (nothing above threshold), the
    /// facade preserves the honest [`Fulfillment::Decline`] — never a fabricated
    /// recommendation (R8.5).
    #[tokio::test]
    async fn empty_market_recommender_still_declines_honestly() {
        let (_f, embedder, _dir, _audit_path) = setup(&[]).await;

        // Empty offline market → recommender honestly finds nothing.
        let (market, _mdir) = synced_market(embedder.clone(), vec![]).await;

        let tdir = TempDir::new().expect("tempdir");
        let audit = Arc::new(
            AuditLedger::open(&tdir.path().join("audit.db"), TEST_HMAC_KEY.to_vec())
                .expect("open audit ledger"),
        );
        let index = Arc::new(CapabilityIndex::new(embedder.clone()));
        index.rebuild(&[]).await.expect("rebuild empty index");
        let recommender: Arc<dyn Recommender + Send + Sync> =
            Arc::new(DefaultRecommender::new(market));
        let facade = CapabilityIntelligence::new(
            index,
            Arc::new(DefaultCapabilityRanker::new()),
            embedder.clone(),
            None,
            CilConfig::default(),
            audit,
            RiskLevel::Green,
        )
        .with_recommender(recommender);

        let intent = derive_goal_intent(
            "do something no installed or market skill can",
            embedder.as_ref(),
            vec![],
            false,
            RiskLevel::Green,
        )
        .await
        .expect("derive intent");

        let audit_path = tdir.path().join("audit.db");
        let outcome = facade
            .plan_for_intent("inv-empty-market", &RequestCtx::default(), &intent)
            .await;

        assert!(
            matches!(outcome, Fulfillment::Decline { .. }),
            "empty market → honest decline, never a fabricated recommendation"
        );
        // Terminal stage is a decline (no recommend fabricated).
        let stages = audit_stages(&audit_path);
        assert_eq!(stages, vec!["cil.discovery", "cil.ranking", "cil.decline"]);
    }
}
