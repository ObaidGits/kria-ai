//! [`CapabilityPlatform`] — the composition root that ties the CPP pieces
//! together behind one provider-neutral surface.
//!
//! This is what the agent-loop tool entry (the flag-ON branch) will call instead
//! of reaching into any provider: it discovers capabilities across all registered
//! providers via the [`ProviderRegistry`]'s federated index, and executes a chosen
//! capability through the owning provider's adapter. It holds no provider-native
//! type and hardcodes no provider.
//!
//! # Scope (Milestone 3)
//!
//! Discovery + direct execution. Multi-capability planning (composing several
//! descriptors into an `ExecutionGraph`), permission gating, and acquisition are
//! layered on in later milestones — each as a method on this same facade, so the
//! Brain's entry point never changes shape as capability grows.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::descriptor::CapabilityDescriptor;
use super::error::CapError;
use super::events::{CapabilityEvent, Outcome, SharedEventBus, Stage};
use super::index::ScoredDescriptor;
use super::intelligence::marketplace::{
    CatalogCache, CatalogRanker, Quarantine, TrustPolicy, TrustVerdict,
};
use super::provider::{CapabilityOutcome, CapabilityRequest};
use super::registry::{ProviderRegistry, RefreshReport};

/// The single provider-neutral surface the Brain uses for capabilities.
pub struct CapabilityPlatform {
    registry: Arc<ProviderRegistry>,
    events: Option<SharedEventBus>,
    /// Durable learned knowledge (CKB). When present, every execution outcome is
    /// recorded for reuse/ranking/grounding (spec R1). Optional so the platform
    /// works unchanged when the CKB flag is off (flag-off parity).
    knowledge: Option<Arc<dyn super::intelligence::CapabilityKnowledge>>,
    /// Wave 6 marketplace-intelligence ranker (spec R8). Present only when the
    /// `marketplace_v2` flag is on; absent ⇒ legacy index-only recommendation
    /// (flag-off parity).
    marketplace: Option<CatalogRanker>,
    /// Per-provider catalog cache (spec R8.3). Present only with `marketplace_v2`.
    catalog_cache: Option<Mutex<CatalogCache>>,
    /// Durable evolution/health/benchmark store (spec R6/R18). Same concrete CKB
    /// as `knowledge`, exposed via its neutral [`EvolutionStore`] facet so the
    /// oversight surface (proposals/health) works without a second store.
    evolution: Option<Arc<dyn super::intelligence::EvolutionStore>>,
    /// The registered provider id that can SYNTHESIZE capabilities (spec R7,
    /// Wave 9). Injected data (not a hardcoded name in logic) so the Brain's
    /// acquisition can fall through to generation when no catalog candidate
    /// exists and the goal is synthesizable. `None` ⇒ synthesis disabled.
    synthesis_provider: Option<String>,
    /// The Brain's IR proposer for synthesis (Wave 9, W9-R11). Defaults to the
    /// deterministic proposer; the desktop/runtime layer may inject an
    /// LLM-assisted proposer behind the `synthesis_llm` flag. The proposer only
    /// ever *proposes* — `propose_validated` + the provider re-validate, so a bad
    /// model can never produce an unsafe capability.
    ir_proposer: Option<Arc<dyn super::intelligence::IrProposer>>,
    /// The hardened code sandbox for Tier-3 code nodes (BLOCKER 2/3). `None` ⇒
    /// code nodes fail closed (never run). Wired behind the `synthesis_code` flag.
    code_runner: Option<Arc<dyn super::intelligence::CodeRunner>>,
    /// The Brain's trust policy gating activation of acquired capabilities
    /// (spec R8.3). Only enforced on the marketplace_v2 acquisition pipeline.
    trust_policy: TrustPolicy,
    /// Quarantine registry for capabilities that failed the trust/integrity gate
    /// (spec R8.3). Present only with `marketplace_v2`; a quarantined capability
    /// is refused by [`execute`](Self::execute).
    quarantine: Option<Mutex<Quarantine>>,
}

impl CapabilityPlatform {
    /// Build over a provider registry (which owns the federated index).
    pub fn new(registry: Arc<ProviderRegistry>) -> Self {
        Self {
            registry,
            events: None,
            knowledge: None,
            marketplace: None,
            catalog_cache: None,
            evolution: None,
            trust_policy: TrustPolicy::default(),
            quarantine: None,
            synthesis_provider: None,
            ir_proposer: None,
            code_runner: None,
        }
    }

    /// Wire the hardened code sandbox for Tier-3 generated-code nodes (BLOCKER
    /// 2/3), behind the `synthesis_code` flag. Without it, code nodes fail closed.
    pub fn with_code_runner(mut self, runner: Arc<dyn super::intelligence::CodeRunner>) -> Self {
        self.code_runner = Some(runner);
        self
    }

    /// Inject the Brain's IR proposer for synthesis (Wave 9, W9-R11). When unset,
    /// synthesis uses the provider's deterministic goal→IR derivation (parity).
    /// An LLM-assisted proposer is injected here behind the `synthesis_llm` flag.
    pub fn with_ir_proposer(mut self, proposer: Arc<dyn super::intelligence::IrProposer>) -> Self {
        self.ir_proposer = Some(proposer);
        self
    }

    /// Enable synthesis fall-through (spec R7/Wave 9): `provider_id` is the
    /// registered synthesizing provider the Brain invokes when no catalog
    /// candidate exists and the goal is synthesizable. Data-injected — the
    /// acquisition logic never hardcodes a provider name.
    pub fn with_synthesis(mut self, provider_id: impl Into<String>) -> Self {
        self.synthesis_provider = Some(provider_id.into());
        self
    }

    /// Attach an observability event bus (M8): discovery/execution/failure stages
    /// are emitted for the live desktop timeline + tracing.
    pub fn with_events(mut self, events: SharedEventBus) -> Self {
        self.events = Some(events);
        self
    }

    /// The attached observability event bus, if any (used by the continuous
    /// discovery engine to emit `capability:discover` events).
    pub fn events(&self) -> Option<&SharedEventBus> {
        self.events.as_ref()
    }

    /// Attach the durable Capability Knowledge Base (spec R1/P1). With it wired,
    /// execution outcomes (+latency/failure) are recorded to the CKB — the
    /// authoritative learned layer that feeds reuse, ranking, and grounding.
    pub fn with_knowledge(
        mut self,
        knowledge: Arc<dyn super::intelligence::CapabilityKnowledge>,
    ) -> Self {
        self.knowledge = Some(knowledge);
        self
    }

    /// Access the CKB, if wired.
    pub fn knowledge(&self) -> Option<&Arc<dyn super::intelligence::CapabilityKnowledge>> {
        self.knowledge.as_ref()
    }

    /// Attach the durable evolution/health/benchmark store (spec R6/R18). Wire
    /// the SAME concrete CKB as `with_knowledge` (coerced to its EvolutionStore
    /// facet) so proposals/health persist to the one learned layer.
    pub fn with_evolution_store(
        mut self,
        store: Arc<dyn super::intelligence::EvolutionStore>,
    ) -> Self {
        self.evolution = Some(store);
        self
    }

    /// Access the evolution/health store, if wired.
    pub fn evolution_store(&self) -> Option<&Arc<dyn super::intelligence::EvolutionStore>> {
        self.evolution.as_ref()
    }

    /// Enable Wave 6 marketplace intelligence (spec R8): neutral catalog ranking
    /// (trust/quality/cost/adoption + relevance) plus a TTL catalog cache. Wired
    /// behind the `capability.intelligence.marketplace_v2` flag; when absent the
    /// platform uses the legacy index-only recommendation path (flag-off parity).
    pub fn with_marketplace_v2(mut self, ranker: CatalogRanker, cache_ttl: Duration) -> Self {
        self.marketplace = Some(ranker);
        self.catalog_cache = Some(Mutex::new(CatalogCache::new(cache_ttl)));
        self.quarantine = Some(Mutex::new(Quarantine::new()));
        self
    }

    /// Override the Brain's trust policy for acquisition activation (spec R8.3).
    pub fn with_trust_policy(mut self, policy: TrustPolicy) -> Self {
        self.trust_policy = policy;
        self
    }

    /// True when a capability is quarantined (failed the trust/integrity gate).
    pub fn is_quarantined(&self, provider_id: &str, capability_id: &str) -> bool {
        self.quarantine
            .as_ref()
            .and_then(|q| q.lock().ok())
            .map(|q| q.is_quarantined(provider_id, capability_id))
            .unwrap_or(false)
    }

    /// All quarantined capabilities as `(provider_id, capability_id, reason)`
    /// for the marketplace/oversight UI (spec R8.3 visibility). Empty when
    /// marketplace_v2 is off.
    pub fn quarantined(&self) -> Vec<(String, String, String)> {
        self.quarantine
            .as_ref()
            .and_then(|q| q.lock().ok())
            .map(|q| q.list())
            .unwrap_or_default()
    }

    /// Release a capability from quarantine (e.g. after operator review /
    /// re-verification). Returns whether it was quarantined. No-op when
    /// marketplace_v2 is off.
    pub fn release_quarantine(&self, provider_id: &str, capability_id: &str) -> bool {
        self.quarantine
            .as_ref()
            .and_then(|q| q.lock().ok())
            .map(|mut q| q.release(provider_id, capability_id))
            .unwrap_or(false)
    }

    /// The quarantine reason for a capability, if quarantined.
    pub fn quarantine_reason(&self, provider_id: &str, capability_id: &str) -> Option<String> {
        self.quarantine
            .as_ref()
            .and_then(|q| q.lock().ok())
            .and_then(|q| q.reason(provider_id, capability_id).map(|s| s.to_string()))
    }

    /// True when Wave 6 marketplace intelligence is active.
    pub fn marketplace_v2_enabled(&self) -> bool {
        self.marketplace.is_some()
    }

    /// Explicitly invalidate the marketplace catalog cache (spec R8.3). No-op
    /// when `marketplace_v2` is off. `provider_id = None` clears all.
    pub fn invalidate_catalog_cache(&self, provider_id: Option<&str>) {
        if let Some(cache) = &self.catalog_cache {
            if let Ok(mut c) = cache.lock() {
                match provider_id {
                    Some(pid) => {
                        c.invalidate(pid);
                    }
                    None => c.invalidate_all(),
                }
            }
        }
    }

    /// Gather each provider's catalog, using the TTL cache when
    /// `marketplace_v2` is on (else always live).
    async fn gather_catalog(&self) -> Vec<CapabilityDescriptor> {
        let mut catalog = Vec::new();
        for pid in self.registry.provider_ids() {
            // Serve fresh cache hits without touching the provider.
            if let Some(cache) = &self.catalog_cache {
                if let Ok(c) = cache.lock() {
                    if let Some(hit) = c.get(&pid) {
                        catalog.extend_from_slice(hit);
                        continue;
                    }
                }
            }
            if let Some(p) = self.registry.get(&pid) {
                match p.catalog().await {
                    Ok(c) => {
                        if let Some(cache) = &self.catalog_cache {
                            if let Ok(mut cc) = cache.lock() {
                                cc.put(&pid, c.clone());
                            }
                        }
                        catalog.extend(c);
                    }
                    Err(e) => tracing::debug!("provider {pid} catalog unavailable: {e}"),
                }
            }
        }
        catalog
    }

    fn emit(&self, ev: CapabilityEvent) {
        if let Some(bus) = &self.events {
            bus.emit(ev);
        }
    }

    /// Whether a JSON-Schema declares any required/expected arguments (used by
    /// the pre-activation smoke gate to decide if a no-arg liveness run is safe).
    fn schema_expects_args(schema: &serde_json::Value) -> bool {
        let Some(obj) = schema.as_object() else {
            return false;
        };
        if obj
            .get("required")
            .and_then(|r| r.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        obj.get("properties")
            .and_then(|p| p.as_object())
            .map(|p| !p.is_empty())
            .unwrap_or(false)
    }

    /// Pre-activation **smoke gate** (spec R21): before a freshly
    /// acquired/synthesized capability is trusted/activated, run its declared
    /// golden smoke check (`extensions["smoke"].args`) — or a no-arg liveness run
    /// when it takes no args — and require a real `Value`/`Stream` outcome. When
    /// a capability requires args but declares no smoke case we cannot fabricate
    /// inputs, so we honestly pass without running (liveness unknown, not failed).
    /// Any hard failure/decline returns `Err` so the caller quarantines + rolls
    /// back — nothing is trusted on download/generation alone.
    ///
    /// The capability MUST already be discoverable (call after `refresh`).
    async fn smoke_gate(&self, descriptor: &CapabilityDescriptor) -> Result<(), CapError> {
        use super::provider::RequestContext;
        let args = descriptor
            .extensions
            .get("smoke")
            .and_then(|s| s.get("args"))
            .cloned();
        let args = match args {
            Some(a) => a,
            None => {
                if Self::schema_expects_args(&descriptor.input_schema) {
                    return Ok(());
                }
                serde_json::json!({})
            }
        };
        let req = CapabilityRequest {
            provider_id: descriptor.provider_id.clone(),
            capability_id: descriptor.capability_id.clone(),
            args,
            context: RequestContext::new(),
            granted_effects: descriptor.effects.classes.clone(),
        };
        match self.execute(req).await {
            Ok(CapabilityOutcome::Value(_)) | Ok(CapabilityOutcome::Stream(_)) => Ok(()),
            Ok(CapabilityOutcome::Declined { reason }) => {
                Err(CapError::Execute(format!("smoke test declined: {reason}")))
            }
            Err(e) => Err(CapError::Execute(format!("smoke test failed: {e}"))),
        }
    }

    /// Access the underlying provider registry (registration, sessions, health).
    pub fn registry(&self) -> &Arc<ProviderRegistry> {
        &self.registry
    }

    /// Negotiate + describe every provider and (re)build the federated index.
    pub async fn refresh(&self) -> RefreshReport {
        self.registry.refresh().await
    }

    /// Discover the top-k capabilities across all providers for a goal query.
    pub fn discover(&self, query: &str, k: usize) -> Result<Vec<ScoredDescriptor>, CapError> {
        self.registry.search(query, k)
    }

    /// Look up a single indexed descriptor by its `(provider_id, capability_id)`
    /// key. Used by the permission/approval path and the Descriptor Viewer, which
    /// need the capability's full declared effects/guidance without re-running a
    /// goal query. Returns `None` when the capability is not currently federated
    /// (e.g. its provider is offline or the id is unknown).
    pub fn descriptor(
        &self,
        provider_id: &str,
        capability_id: &str,
    ) -> Result<Option<super::descriptor::CapabilityDescriptor>, CapError> {
        // The in-memory index has no direct get-by-key; an empty-query scan
        // returns every indexed descriptor (score 0), which we filter. Cheap at
        // current scale and avoids widening the FederatedIndex trait.
        let all = self.registry.search("", 1_000_000)?;
        Ok(all
            .into_iter()
            .map(|s| s.descriptor)
            .find(|d| d.provider_id == provider_id && d.capability_id == capability_id))
    }

    /// Recommend installable capabilities (marketplace catalog federation) for a
    /// goal: gather each provider's catalog (not-yet-installed entries), rank them
    /// against the query, and return the top-k. Pure read — installs nothing (the
    /// caller drives `acquire` on an approved choice).
    pub async fn recommend(
        &self,
        query: &str,
        k: usize,
    ) -> Result<Vec<ScoredDescriptor>, CapError> {
        // Recommendations are installable (not-yet-installed) capabilities: drop
        // any catalog entry a provider reports as already installed
        // (`extensions["installed"] == true`) so the Brain never recommends or
        // duplicate-installs something it already has (native-first, Phase 8).
        let catalog: Vec<CapabilityDescriptor> = self
            .gather_catalog()
            .await
            .into_iter()
            .filter(|d| d.extensions.get("installed").and_then(|v| v.as_bool()) != Some(true))
            .collect();

        // Flag-off parity: legacy index-only scoring when marketplace_v2 is off.
        let Some(ranker) = &self.marketplace else {
            return self.registry.index().score_descriptors(query, catalog, k);
        };

        // marketplace_v2: fuse the index's semantic/lexical relevance with the
        // neutral trust/quality/cost/adoption catalog signals (spec R8.1/R8.2).
        let relevance: std::collections::HashMap<(String, String), f32> = self
            .registry
            .index()
            .score_descriptors(query, catalog.clone(), catalog.len().max(1))?
            .into_iter()
            .map(|s| {
                (
                    (s.descriptor.provider_id, s.descriptor.capability_id),
                    s.score,
                )
            })
            .collect();

        let ranked = ranker.rank(&catalog, &relevance);
        Ok(ranked
            .into_iter()
            .take(k)
            .map(|e| {
                let rel = relevance
                    .get(&(
                        e.descriptor.provider_id.clone(),
                        e.descriptor.capability_id.clone(),
                    ))
                    .copied()
                    .unwrap_or(0.0);
                ScoredDescriptor {
                    descriptor: e.descriptor,
                    score: e.score,
                    semantic: rel,
                    lexical: e.signals.relevance,
                }
            })
            .collect())
    }

    /// Acquire (install/generate) a capability that satisfies a natural-language
    /// goal, via a provider's lifecycle facet. Provider-neutral: prefers the
    /// provider whose catalog best matches the goal, then falls back to trying
    /// every lifecycle-capable provider. On success the federated index is
    /// refreshed so the newly-installed capability is immediately discoverable
    /// and executable. Returns the installed descriptor, or an honest error.
    ///
    /// This is the ONE marketplace-install entry point the chat/agent path uses
    /// (there is no second installer) — the caller supplies only the user's
    /// free-text request; the owning provider resolves the best marketplace match.
    pub async fn acquire_for_goal(&self, goal: &str) -> Result<CapabilityDescriptor, CapError> {
        let goal = goal.trim();
        if goal.is_empty() {
            return Err(CapError::Acquire("empty acquisition goal".into()));
        }

        // Flag-off parity: with marketplace_v2 OFF, run the byte-identical legacy
        // provider-order acquisition. The Brain-owned pipeline (ranked selection,
        // trust gate, quarantine, decision records, events) is enabled only with
        // the flag (spec Property 1).
        if self.marketplace.is_none() {
            return self.acquire_for_goal_legacy(goal).await;
        }
        self.acquire_for_goal_reasoned(goal).await
    }

    /// Legacy acquisition (flag-off): try providers in catalog-relevance order,
    /// first success wins. Preserved byte-for-byte for parity.
    async fn acquire_for_goal_legacy(&self, goal: &str) -> Result<CapabilityDescriptor, CapError> {
        use crate::capability::provider::{AcquireRequest, RequestContext};

        let mut ordered: Vec<String> = Vec::new();
        if let Ok(hits) = self.recommend(goal, 8).await {
            for h in hits {
                if !ordered.contains(&h.descriptor.provider_id) {
                    ordered.push(h.descriptor.provider_id.clone());
                }
            }
        }
        for pid in self.registry.provider_ids() {
            if !ordered.contains(&pid) {
                ordered.push(pid);
            }
        }

        let req = AcquireRequest {
            capability_tag: goal.to_string(),
            hint: Some(goal.to_string()),
            capability_id: None,
            proposed_graph: None,
            context: RequestContext::new(),
        };

        let mut last_err: Option<CapError> = None;
        for pid in ordered {
            let Some(provider) = self.registry.get(&pid) else {
                continue;
            };
            match provider.acquire(&req).await {
                Ok(descriptor) => {
                    self.refresh().await;
                    return Ok(descriptor);
                }
                Err(CapError::Unsupported(_)) => continue,
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            CapError::Acquire(format!(
                "no provider could install a capability for '{goal}'"
            ))
        }))
    }

    /// Brain-owned acquisition pipeline (spec R8/R9.4/R16, Wave 6):
    /// rank catalog → **Brain selects** the specific capability → record a
    /// Decision Record → check declared dependencies → acquire exactly that
    /// capability from its owning provider → **trust-gate** the result
    /// (quarantine + block activation on failure) → learn (CKB) → activate.
    /// Emits `capability:*` events at each stage. The provider is pure Hands.
    async fn acquire_for_goal_reasoned(
        &self,
        goal: &str,
    ) -> Result<CapabilityDescriptor, CapError> {
        use super::events::{Outcome, Stage};
        use super::intelligence::marketplace::{CapabilityCoordinate, DependencySpec};
        use super::intelligence::{version_satisfies, DecisionRecord, ExecutionPath, GoalClass};
        use crate::capability::provider::{AcquireRequest, RequestContext};

        let ctx = RequestContext::new();
        let corr = ctx.correlation_id.clone();

        // 1) Rank the catalog for the goal — the Brain decides, not the provider.
        let ranked = self.recommend(goal, 8).await?;
        self.emit(CapabilityEvent::new(
            &corr,
            "marketplace",
            None,
            Stage::Rank,
            if ranked.is_empty() {
                Outcome::Declined
            } else {
                Outcome::Ok
            },
            format!("ranked {} catalog candidate(s) for goal", ranked.len()),
        ));
        let Some(top) = ranked.first() else {
            // No catalog candidate — GAP confirmed. Fall through to SYNTHESIS
            // when enabled and the goal is synthesizable (spec R7/Wave 9). This
            // reuses the identical Decision-Record + trust-gate + CKB + events
            // machinery below via a dedicated synthesis path.
            if let Some(syn_id) = &self.synthesis_provider {
                // Propose the IR ONCE via the Brain's proposer (deterministic or
                // LLM-assisted), validator-gated. A goal is synthesizable when
                // either a validated IR graph exists (primitive/composed) OR the
                // deterministic spec maps it (covers multi-input reducers, which
                // execute at the provider boundary and carry no linear graph).
                let (graph, proposer_id) = self.propose_ir(goal).await;
                let synthesizable = graph.is_some()
                    || super::intelligence::CapabilitySpecification::from_goal(goal).is_some();
                if synthesizable {
                    return self
                        .synthesize_for_goal(goal, syn_id, &corr, graph, proposer_id)
                        .await;
                }
            }
            return Err(CapError::Acquire(format!(
                "no marketplace capability matches '{goal}' and it is not synthesizable"
            )));
        };
        let chosen_provider = top.descriptor.provider_id.clone();
        let chosen_cap = top.descriptor.capability_id.clone();
        let chosen_score = top.score;

        // 2) Decision Record (spec R16): why this candidate, why not the rest.
        let candidates: Vec<(String, String, f32)> = ranked
            .iter()
            .map(|h| {
                (
                    h.descriptor.provider_id.clone(),
                    h.descriptor.capability_id.clone(),
                    h.score,
                )
            })
            .collect();
        let rejected: Vec<(String, String, String)> = ranked
            .iter()
            .skip(1)
            .map(|h| {
                (
                    h.descriptor.provider_id.clone(),
                    h.descriptor.capability_id.clone(),
                    format!(
                        "lower fused catalog score {:.3} < {:.3}",
                        h.score, chosen_score
                    ),
                )
            })
            .collect();
        if let Some(ckb) = &self.knowledge {
            let decision = DecisionRecord {
                id: uuid::Uuid::new_v4().to_string(),
                goal: goal.to_string(),
                goal_class: GoalClass::Other("acquisition".into()),
                candidates,
                chosen: Some((chosen_provider.clone(), chosen_cap.clone())),
                rejected,
                path: ExecutionPath::Acquire,
                confidence: chosen_score,
                policy_version: super::intelligence::REASONING_POLICY_VERSION,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            let _ = ckb.record_decision(&decision).await;
        }

        // 3) Declared-dependency check (spec R8.4). Unsatisfied REQUIRED deps are
        //    surfaced honestly (resolution may defer, but we never install blind).
        let deps = DependencySpec::list_from_descriptor(&top.descriptor);
        if !deps.is_empty() {
            let installed = self.discover("", 100_000).unwrap_or_default();
            for dep in &deps {
                if dep.optional {
                    continue;
                }
                let satisfied = installed.iter().any(|s| {
                    let coord = CapabilityCoordinate::from_ids(
                        &s.descriptor.provider_id,
                        &s.descriptor.capability_id,
                    );
                    coord == dep.coordinate
                        && version_satisfies(&s.descriptor.version, &dep.version_req)
                            .unwrap_or(false)
                });
                if !satisfied {
                    self.emit(CapabilityEvent::new(
                        &corr,
                        &chosen_provider,
                        Some(chosen_cap.clone()),
                        Stage::Acquire,
                        Outcome::Failed,
                        format!(
                            "unsatisfied required dependency {} {}",
                            dep.coordinate, dep.version_req
                        ),
                    ));
                    return Err(CapError::Acquire(format!(
                        "capability '{chosen_cap}' requires dependency {} {} which is not installed",
                        dep.coordinate, dep.version_req
                    )));
                }
            }
        }

        // 4) Acquire EXACTLY the Brain-chosen capability from its owning provider.
        let provider = self
            .registry
            .get(&chosen_provider)
            .ok_or_else(|| CapError::Acquire(format!("no such provider '{chosen_provider}'")))?;
        self.emit(CapabilityEvent::new(
            &corr,
            &chosen_provider,
            Some(chosen_cap.clone()),
            Stage::Acquire,
            Outcome::Started,
            format!("installing Brain-selected capability (score {chosen_score:.3})"),
        ));
        let req = AcquireRequest {
            capability_tag: goal.to_string(),
            hint: Some(goal.to_string()),
            capability_id: Some(chosen_cap.clone()),
            proposed_graph: None,
            context: ctx.clone(),
        };
        let descriptor = match provider.acquire(&req).await {
            Ok(d) => d,
            Err(e) => {
                self.emit(CapabilityEvent::new(
                    &corr,
                    &chosen_provider,
                    Some(chosen_cap.clone()),
                    Stage::Acquire,
                    Outcome::Failed,
                    format!("acquire failed: {e}"),
                ));
                if let Some(ckb) = &self.knowledge {
                    let _ = ckb
                        .record_outcome(
                            &chosen_provider,
                            &chosen_cap,
                            false,
                            None,
                            Some(&e.to_string()),
                        )
                        .await;
                }
                return Err(e);
            }
        };
        let (pid, cid) = descriptor.key();

        // 5) Brain trust gate (spec R8.3): quarantine + block activation on fail.
        match self.trust_policy.evaluate(&descriptor.trust) {
            TrustVerdict::Untrusted { reason } => {
                if let Some(q) = &self.quarantine {
                    if let Ok(mut q) = q.lock() {
                        q.quarantine(&pid, &cid, reason.clone());
                    }
                }
                self.emit(CapabilityEvent::new(
                    &corr,
                    &pid,
                    Some(cid.clone()),
                    Stage::Failure,
                    Outcome::Failed,
                    format!("quarantined (trust gate): {reason}"),
                ));
                if let Some(ckb) = &self.knowledge {
                    let _ = ckb
                        .record_outcome(&pid, &cid, false, None, Some(&reason))
                        .await;
                }
                return Err(CapError::Permission(format!(
                    "acquired capability '{cid}' quarantined and not activated: {reason}"
                )));
            }
            TrustVerdict::Trusted => {}
        }

        // 6) Trusted → make discoverable, then SMOKE GATE before activation.
        self.refresh().await;
        self.invalidate_catalog_cache(None);

        // Pre-activation smoke gate (spec R21, W9-R3): an acquired capability must
        // pass its declared liveness check before it is trusted/learned. Failure
        // quarantines + rolls back — nothing trusted on install alone.
        if let Err(e) = self.smoke_gate(&descriptor).await {
            if let Some(q) = &self.quarantine {
                if let Ok(mut q) = q.lock() {
                    q.quarantine(&pid, &cid, format!("failed pre-activation smoke: {e}"));
                }
            }
            if let Some(p) = self.registry.get(&pid) {
                let _ = p.remove(&cid).await;
            }
            self.refresh().await;
            if let Some(ckb) = &self.knowledge {
                let _ = ckb
                    .record_outcome(&pid, &cid, false, None, Some(&e.to_string()))
                    .await;
            }
            self.emit(CapabilityEvent::new(
                &corr,
                &pid,
                Some(cid.clone()),
                Stage::Failure,
                Outcome::Failed,
                format!("acquired capability failed smoke test; rolled back: {e}"),
            ));
            return Err(CapError::Acquire(format!(
                "acquired '{cid}' failed pre-activation smoke test; rolled back: {e}"
            )));
        }

        // 7) Passed ⇒ activate + learn (CKB install + outcome).
        if let Some(ckb) = &self.knowledge {
            let _ = ckb.record_install(&descriptor).await;
            let _ = ckb.record_outcome(&pid, &cid, true, None, None).await;
        }
        self.emit(CapabilityEvent::new(
            &corr,
            &pid,
            Some(cid.clone()),
            Stage::Acquire,
            Outcome::Ok,
            "installed + smoke-gated + trust-gated + activated",
        ));
        self.emit(CapabilityEvent::new(
            &corr,
            &pid,
            Some(cid),
            Stage::Learn,
            Outcome::Ok,
            "recorded install + outcome to CKB",
        ));
        Ok(descriptor)
    }

    /// Synthesis path (spec R7/Wave 9): generate a capability for `goal` via the
    /// registered synthesizing provider, through the SAME neutral machinery as
    /// marketplace acquisition — Decision Record (path=Generate) → provider
    /// acquire(=generate) → trust gate (lowest tier) → CKB install+outcome →
    /// events. No special Brain code beyond choosing the Generate path.
    /// Propose the Capability-Graph IR for a goal (W9-R11), validator-gated. Uses
    /// the injected proposer (deterministic or LLM-assisted) when present, else
    /// the deterministic goal→IR derivation. Returns the validated graph (or
    /// `None` = honest-decline) plus the proposer id for provenance.
    async fn propose_ir(
        &self,
        goal: &str,
    ) -> (Option<super::intelligence::CapabilityGraph>, String) {
        match &self.ir_proposer {
            Some(p) => (
                super::intelligence::propose_validated(p.as_ref(), goal).await,
                p.proposer_id().to_string(),
            ),
            None => (
                super::intelligence::CapabilitySpecification::from_goal(goal)
                    .and_then(|s| s.normalized_graph()),
                "deterministic".to_string(),
            ),
        }
    }

    async fn synthesize_for_goal(
        &self,
        goal: &str,
        synthesis_provider_id: &str,
        corr: &str,
        proposed: Option<super::intelligence::CapabilityGraph>,
        proposer_id: String,
    ) -> Result<CapabilityDescriptor, CapError> {
        use super::events::{Outcome, Stage};
        use super::intelligence::{DecisionRecord, ExecutionPath, GoalClass, TrustVerdict};
        use crate::capability::provider::{AcquireRequest, RequestContext};

        let provider = self.registry.get(synthesis_provider_id).ok_or_else(|| {
            CapError::Acquire(format!(
                "synthesis provider '{synthesis_provider_id}' not registered"
            ))
        })?;

        self.emit(CapabilityEvent::new(
            corr,
            synthesis_provider_id,
            None,
            Stage::Synthesize,
            Outcome::Started,
            format!("no candidate — synthesizing a capability for '{goal}'"),
        ));

        // Preview the proposed IR (granular event, W9-R5): node count, IR hash,
        // pipeline — BEFORE generation. A multi-input reducer capability has no
        // linear graph (executes at the provider boundary) and is reported as such.
        let preview = match &proposed {
            Some(g) => format!(
                "IR proposed by {proposer_id}: {} node(s), pipeline [{}], ir_hash={}",
                g.nodes.len(),
                g.primitive_pipeline()
                    .map(|p| p.join(" → "))
                    .unwrap_or_else(|| "composed".into()),
                &g.hash().chars().take(12).collect::<String>()
            ),
            None => {
                format!("multi-input capability proposed by {proposer_id} (reducer boundary node)")
            }
        };
        self.emit(CapabilityEvent::new(
            corr,
            synthesis_provider_id,
            None,
            Stage::Synthesize,
            Outcome::Ok,
            preview,
        ));

        // The Brain already proposed + validated the IR (W9-R11); pass it to the
        // provider, which RE-VALIDATES (safety, not cognition) + persists it. When
        // `None` (multi-input reducer / no-proposer path) the provider derives the
        // deterministic spec from the goal itself.
        let proposed_graph = proposed.as_ref().and_then(|g| serde_json::to_value(g).ok());

        // Generate (the synthesizing provider's acquire) — honest-declines when
        // not expressible from the audited set.
        let req = AcquireRequest {
            capability_tag: goal.to_string(),
            hint: Some(goal.to_string()),
            capability_id: None,
            proposed_graph,
            context: RequestContext::new(),
        };
        let descriptor = match provider.acquire(&req).await {
            Ok(d) => d,
            Err(e) => {
                self.emit(CapabilityEvent::new(
                    corr,
                    synthesis_provider_id,
                    None,
                    Stage::Failure,
                    Outcome::Failed,
                    format!("synthesis failed: {e}"),
                ));
                return Err(e);
            }
        };
        let (pid, cid) = descriptor.key();

        // Decision Record AFTER generation (W9-R4, spec R16.2): now we can record
        // WHAT was synthesized (`chosen = Some`) + the IR hash provenance, so
        // "why did you synthesize X" is answerable + reproducible — not a stub.
        if let Some(ckb) = &self.knowledge {
            let ir_hash = descriptor
                .extensions
                .get("ir_hash")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let decision = DecisionRecord {
                id: uuid::Uuid::new_v4().to_string(),
                goal: goal.to_string(),
                goal_class: GoalClass::Generation,
                candidates: vec![(pid.clone(), cid.clone(), 0.6)],
                chosen: Some((pid.clone(), cid.clone())),
                rejected: vec![(
                    "marketplace".into(),
                    "*".into(),
                    "no catalog candidate matched the goal".into(),
                )],
                path: ExecutionPath::Generate,
                confidence: 0.6,
                policy_version: super::intelligence::REASONING_POLICY_VERSION,
                created_at: format!(
                    "{} ir_hash={ir_hash} proposer={proposer_id}",
                    chrono::Utc::now().to_rfc3339()
                ),
            };
            let _ = ckb.record_decision(&decision).await;
        }

        // Trust gate — synthesized is the lowest tier; the default policy allows
        // install-for-review, and its elevated effects require permission at
        // execute (never silent activation).
        if let TrustVerdict::Untrusted { reason } = self.trust_policy.evaluate(&descriptor.trust) {
            if let Some(q) = &self.quarantine {
                if let Ok(mut q) = q.lock() {
                    q.quarantine(&pid, &cid, reason.clone());
                }
            }
            self.emit(CapabilityEvent::new(
                corr,
                &pid,
                Some(cid.clone()),
                Stage::Failure,
                Outcome::Failed,
                format!("synthesized capability quarantined: {reason}"),
            ));
            return Err(CapError::Permission(format!(
                "synthesized '{cid}' quarantined: {reason}"
            )));
        }

        self.emit(CapabilityEvent::new(
            corr,
            &pid,
            Some(cid.clone()),
            Stage::Synthesize,
            Outcome::Ok,
            "IR generated + trust-gated; running pre-activation golden smoke",
        ));

        // Make the synthesized capability discoverable so the smoke gate can run
        // it through the real execution path.
        self.refresh().await;
        self.invalidate_catalog_cache(None);

        // Pre-activation SMOKE GATE (spec R21, W9-R3): execute the declared golden
        // case; a failure quarantines + rolls back the artifact — a synthesized
        // capability is NEVER activated on generation alone.
        if let Err(e) = self.smoke_gate(&descriptor).await {
            if let Some(q) = &self.quarantine {
                if let Ok(mut q) = q.lock() {
                    q.quarantine(&pid, &cid, format!("failed pre-activation smoke: {e}"));
                }
            }
            // Roll back the generated artifact so nothing broken lingers.
            if let Some(p) = self.registry.get(&pid) {
                let _ = p.remove(&cid).await;
            }
            self.refresh().await;
            if let Some(ckb) = &self.knowledge {
                let _ = ckb
                    .record_outcome(&pid, &cid, false, None, Some(&e.to_string()))
                    .await;
            }
            self.emit(CapabilityEvent::new(
                corr,
                &pid,
                Some(cid.clone()),
                Stage::Failure,
                Outcome::Failed,
                format!("synthesized capability failed smoke test; rolled back: {e}"),
            ));
            return Err(CapError::Acquire(format!(
                "synthesized '{cid}' failed pre-activation smoke test; rolled back: {e}"
            )));
        }

        // Activate + learn (identical to acquisition).
        if let Some(ckb) = &self.knowledge {
            let _ = ckb.record_install(&descriptor).await;
            let _ = ckb.record_outcome(&pid, &cid, true, None, None).await;
        }
        self.emit(CapabilityEvent::new(
            corr,
            &pid,
            Some(cid.clone()),
            Stage::Acquire,
            Outcome::Ok,
            "synthesized + smoke-gated + trust-gated + activated",
        ));
        self.emit(CapabilityEvent::new(
            corr,
            &pid,
            Some(cid),
            Stage::Learn,
            Outcome::Ok,
            "recorded synthesized install + outcome to CKB",
        ));
        Ok(descriptor)
    }

    /// Execute a **composed synthesized capability** (W9-R8): a Capability-Graph
    /// IR whose nodes may reference other installed capabilities. Pure-primitive
    /// nodes run in-process; capability nodes are routed back through
    /// [`Self::execute`] to their owning provider (text→text). The graph is read
    /// from the descriptor's `extensions["ir_graph"]` (neutral data), so the
    /// platform never reaches into a provider's private store. The whole-graph
    /// effect union is already declared on the descriptor, so permission is
    /// evaluated conservatively (R11.1) before this runs.
    pub async fn execute_synthesized_graph(
        &self,
        provider_id: &str,
        capability_id: &str,
        input: &str,
    ) -> Result<String, CapError> {
        let descriptor = self
            .descriptor(provider_id, capability_id)?
            .ok_or_else(|| CapError::Execute(format!("unknown capability '{capability_id}'")))?;
        let graph_val = descriptor
            .extensions
            .get("ir_graph")
            .ok_or_else(|| CapError::Execute("capability has no IR graph".into()))?;
        let graph: super::intelligence::CapabilityGraph = serde_json::from_value(graph_val.clone())
            .map_err(|e| CapError::Execute(format!("invalid IR graph: {e}")))?;
        graph.validate().map_err(CapError::Execute)?;
        let exec = PlatformNodeExecutor { platform: self };
        graph.execute(input, &exec).await.map_err(CapError::Execute)
    }

    /// Execute a capability with **production hardening** (Wave 11, spec R12.1):
    /// per-attempt wall-clock **timeout**, **bounded jittered retry** on only
    /// *retryable* failure classes (never infinite), and cooperative
    /// **cancellation** via `cancel`. This is the SINGLE reliable execution path
    /// — it wraps [`Self::execute`] (which already does the permission-neutral
    /// provider routing + circuit breaker + CKB learning), adding the reliability
    /// envelope. Emits `capability:retry` / `capability:cancel` events.
    pub async fn execute_reliable(
        &self,
        req: CapabilityRequest,
        policy: &super::intelligence::RetryPolicy,
        cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<CapabilityOutcome, CapError> {
        use super::intelligence::{classify, FailureClass};
        let corr = req.context.correlation_id.clone();
        let pid = req.provider_id.clone();
        let cid = req.capability_id.clone();
        let started = std::time::Instant::now();
        let is_cancelled = || {
            cancel
                .as_ref()
                .map(|c| c.load(std::sync::atomic::Ordering::SeqCst))
                .unwrap_or(false)
        };

        let mut attempt: u32 = 0;
        loop {
            if is_cancelled() {
                self.emit(CapabilityEvent::new(
                    &corr,
                    &pid,
                    Some(cid.clone()),
                    Stage::Cancel,
                    Outcome::Declined,
                    "execution cancelled before attempt",
                ));
                return Err(CapError::Execute("cancelled".into()));
            }
            attempt += 1;

            // One attempt, bounded by the per-attempt timeout AND racing the
            // cancel token so an in-flight attempt is interrupted promptly (the
            // execute future is dropped on cancel — no orphan, spec R12.1).
            let attempt_req = req.clone();
            let attempt_fut =
                tokio::time::timeout(policy.per_attempt_timeout, self.execute(attempt_req));
            tokio::pin!(attempt_fut);
            let (class, result): (Option<FailureClass>, Result<CapabilityOutcome, CapError>) = loop {
                tokio::select! {
                    biased;
                    res = &mut attempt_fut => {
                        break match res {
                            Ok(Ok(outcome)) => (None, Ok(outcome)),
                            Ok(Err(e)) => (Some(classify(&e)), Err(e)),
                            Err(_) => (
                                Some(FailureClass::Timeout),
                                Err(CapError::Execute(format!(
                                    "attempt {attempt} timed out after {:?}",
                                    policy.per_attempt_timeout
                                ))),
                            ),
                        };
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                        if is_cancelled() {
                            // Dropping `attempt_fut` cancels the in-flight execute.
                            self.emit(CapabilityEvent::new(
                                &corr,
                                &pid,
                                Some(cid.clone()),
                                Stage::Cancel,
                                Outcome::Declined,
                                "execution cancelled mid-attempt",
                            ));
                            return Err(CapError::Execute("cancelled".into()));
                        }
                    }
                }
            };

            match class {
                None => return result, // success
                Some(class) => {
                    if is_cancelled() {
                        self.emit(CapabilityEvent::new(
                            &corr,
                            &pid,
                            Some(cid.clone()),
                            Stage::Cancel,
                            Outcome::Declined,
                            "execution cancelled during retry",
                        ));
                        return Err(CapError::Execute("cancelled".into()));
                    }
                    if policy.should_retry(class, attempt, started.elapsed()) {
                        let delay = policy.delay_for(attempt);
                        self.emit(CapabilityEvent::new(
                            &corr,
                            &pid,
                            Some(cid.clone()),
                            Stage::Retry,
                            Outcome::Degraded,
                            format!(
                                "attempt {attempt} failed ({}); retrying after {:?}",
                                class.as_str(),
                                delay
                            ),
                        ));
                        // Cancellable backoff sleep.
                        let mut remaining = delay;
                        let slice = std::time::Duration::from_millis(50);
                        while remaining > std::time::Duration::ZERO {
                            if is_cancelled() {
                                return Err(CapError::Execute("cancelled".into()));
                            }
                            let step = remaining.min(slice);
                            tokio::time::sleep(step).await;
                            remaining = remaining.saturating_sub(step);
                        }
                        continue;
                    }
                    // Not retryable or budget/attempts exhausted → honest failure.
                    self.emit(CapabilityEvent::new(
                        &corr,
                        &pid,
                        Some(cid.clone()),
                        Stage::Failure,
                        Outcome::Failed,
                        format!(
                            "execution failed after {attempt} attempt(s) ({})",
                            class.as_str()
                        ),
                    ));
                    return result;
                }
            }
        }
    }

    /// Execute a specific capability through its owning provider's adapter.
    ///
    /// The Brain resolves *which* capability to run (via [`discover`] + ranking,
    /// and later permission + planning); this routes the validated request to the
    /// right provider without the Brain touching any provider runtime.
    ///
    /// [`discover`]: CapabilityPlatform::discover
    pub async fn execute(&self, req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
        let correlation_id = req.context.correlation_id.clone();
        let provider_id = req.provider_id.clone();
        let capability_id = req.capability_id.clone();

        // Trust/integrity gate (spec R8.3): a quarantined capability must never
        // execute, even if a stale grant/index entry references it.
        if let Some(reason) = self.quarantine_reason(&provider_id, &capability_id) {
            self.emit(CapabilityEvent::new(
                &correlation_id,
                &provider_id,
                Some(capability_id.clone()),
                Stage::Execute,
                Outcome::Failed,
                format!("refused: capability is quarantined ({reason})"),
            ));
            return Err(CapError::Permission(format!(
                "capability '{capability_id}' is quarantined and cannot execute: {reason}"
            )));
        }

        let provider = match self.registry.get(&provider_id) {
            Some(p) => p,
            None => {
                self.emit(CapabilityEvent::new(
                    &correlation_id,
                    &provider_id,
                    Some(capability_id.clone()),
                    Stage::Execute,
                    Outcome::Failed,
                    "no such provider",
                ));
                return Err(CapError::Execute(format!(
                    "no such provider '{provider_id}'"
                )));
            }
        };

        self.emit(CapabilityEvent::new(
            &correlation_id,
            &provider_id,
            Some(capability_id.clone()),
            Stage::Execute,
            Outcome::Started,
            "executing capability",
        ));

        let started = std::time::Instant::now();
        // Keep the input text for a possible composed-graph reroute (below).
        let input_text = req
            .args
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let mut result = provider.execute(req).await;
        // Composed synthesized capability (capability/code nodes): the provider
        // declines with `Unsupported`; transparently reroute through the neutral
        // graph executor (which owns cross-provider routing + the code sandbox).
        if matches!(&result, Err(e) if e.is_unsupported()) {
            if let Some(text) = &input_text {
                match self
                    .execute_synthesized_graph(&provider_id, &capability_id, text)
                    .await
                {
                    Ok(out) => {
                        result = Ok(CapabilityOutcome::Value(
                            serde_json::json!({ "result": out }),
                        ));
                    }
                    Err(e) => result = Err(e),
                }
            }
        }
        let latency_ms = started.elapsed().as_millis() as u64;

        // Feed the circuit breaker + emit the terminal event.
        let (ok, outcome, detail) = match &result {
            Ok(CapabilityOutcome::Declined { reason }) => (true, Outcome::Declined, reason.clone()),
            Ok(_) => (true, Outcome::Ok, "ok".to_string()),
            Err(e) => (false, Outcome::Failed, e.to_string()),
        };
        self.registry
            .record_execution_outcome(&provider_id, &capability_id, ok);

        // Record the outcome to the durable CKB (learning layer, spec R1.4).
        // A declined run is not a hard failure but is not a success either — only
        // a real `Value`/`Stream` counts as ok for learned success.
        if let Some(ckb) = &self.knowledge {
            let learned_ok = matches!(&result, Ok(CapabilityOutcome::Value(_)))
                || matches!(&result, Ok(CapabilityOutcome::Stream(_)));
            let failure = match &result {
                Ok(CapabilityOutcome::Declined { reason }) => Some(reason.as_str()),
                Err(_) => Some(detail.as_str()),
                _ => None,
            };
            if let Err(e) = ckb
                .record_outcome(
                    &provider_id,
                    &capability_id,
                    learned_ok,
                    Some(latency_ms),
                    failure,
                )
                .await
            {
                tracing::debug!("CKB record_outcome failed (non-fatal): {e}");
            }
        }
        self.emit(CapabilityEvent::new(
            &correlation_id,
            &provider_id,
            Some(capability_id),
            Stage::Execute,
            outcome,
            detail,
        ));

        result
    }
}

/// A [`NodeExecutor`] that runs a synthesized graph's **capability nodes** by
/// routing them back through the neutral platform to their owning provider
/// (W9-R8). Capability nodes are text→text at this stage: the input text is
/// passed as `{ "text": input }` and the `result` string is read back. Keeps the
/// pure IR free of any provider runtime — the platform is the single executor.
struct PlatformNodeExecutor<'a> {
    platform: &'a CapabilityPlatform,
}

#[async_trait::async_trait]
impl super::intelligence::NodeExecutor for PlatformNodeExecutor<'_> {
    async fn run_capability(
        &self,
        provider_id: &str,
        capability_id: &str,
        input: &str,
    ) -> Result<String, String> {
        use super::provider::RequestContext;
        let req = CapabilityRequest {
            provider_id: provider_id.to_string(),
            capability_id: capability_id.to_string(),
            args: serde_json::json!({ "text": input }),
            context: RequestContext::new(),
            granted_effects: vec![],
        };
        match self.platform.execute(req).await {
            Ok(CapabilityOutcome::Value(v)) => v
                .get("result")
                .and_then(|r| r.as_str())
                .map(|s| s.to_string())
                .or_else(|| Some(v.to_string()))
                .ok_or_else(|| "capability node returned no result".to_string()),
            Ok(CapabilityOutcome::Declined { reason }) => {
                Err(format!("capability node declined: {reason}"))
            }
            Ok(other) => Err(format!(
                "capability node returned unsupported outcome: {other:?}"
            )),
            Err(e) => Err(format!("capability node failed: {e}")),
        }
    }

    async fn run_code(&self, language: &str, source: &str, input: &str) -> Result<String, String> {
        // Tier-3 code node: run in the hardened sandbox if one is wired
        // (synthesis_code), else FAIL CLOSED — code never runs unsandboxed.
        match &self.platform.code_runner {
            Some(runner) => runner.run(language, source, input).await,
            None => Err("code execution requires the sandbox (enable synthesis_code)".to_string()),
        }
    }
}
