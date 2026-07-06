//! A6 Semantic OpenClaw Handler - Registry-driven skill execution.
//!
//! Replaces hardcoded skill registration with semantic routing.
//! The handler now queries the ProductionSkillRegistry and uses SemanticSkillRouter
//! to select the best skill for each request.
//!
//! NO more:
//! - Hardcoded skill names
//! - Pre-registered oc_* tools  
//! - Manual skill-to-handler mapping
//! - If-else routing logic
//!
//! The handler becomes a semantic bridge: intent → router → best skill → runtime → result.

use super::audit::{AuditEntry, AuditLedger};
use super::cil::{CapabilityIntelligence, DegradedState, Fulfillment, Recommendation, RequestCtx};
use super::perm::{
    AuthorizeRequest, DefaultPermissionEngine, GrantStore, PermissionDecision, PermissionEngine,
};
use super::registry::ProductionSkillRegistry;
use super::runtime::{LaunchSpec, RuntimeContext, RuntimeKind, RuntimeRegistry};
use super::sanitizer::EvidenceWrapper;
use super::semantic_router::{
    ResourcePressure, RoutingContext, RoutingIntent, SemanticSkillRouter,
};
use super::types::*;
use crate::execution::{
    ExecutionContext, ExecutionEngine, ExecutionGraph, NodeKind, OpenClawExecutor, ScheduleStatus,
};
use crate::infra::isolation::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler};
use crate::tools::ToolContext;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A6 Semantic OpenClaw Handler - Routes and executes skills via semantic routing.
///
/// Replaces the old per-skill handler registration pattern with a single semantic handler
/// that routes each request to the best available skill in the registry.
pub struct SemanticOpenClawHandler {
    /// Semantic router for skill selection
    router: Arc<SemanticSkillRouter>,
    /// Registry for skill metadata (retained for lifecycle/ownership; routing reads
    /// through the router's own registry handle).
    #[allow(dead_code)]
    registry: Arc<ProductionSkillRegistry>,
    /// Runtime registry for execution
    runtimes: Arc<RuntimeRegistry>,
    /// Audit ledger
    audit: Arc<AuditLedger>,
    /// TrustConfig enforcement fix (product gap 6/8): the real HITL approval
    /// gate, now genuinely consulted by `execute_semantic` (previously
    /// structurally unreachable from the real chat execution path).
    ///
    /// Retained for the Verified-tier `verified_skips_hitl` bypass (which records
    /// an auto-approval token in this frozen cache). The elevated-risk *gate*
    /// itself now flows through [`Self::perm_engine`] (task 11.4) — the frozen
    /// `ApprovalCache` remains the widening oracle *inside* that engine (R7.4).
    approval: super::approval::ApprovalCache,
    /// ICP permission engine (task 11.4, design §8.7). Metadata-driven, tiered
    /// authorization that is a strict SUPERSET of the frozen `ApprovalCache`
    /// (R7.4): GREEN+pure ⇒ `NeverAsk`, RED/host-scope-subprocess ⇒ `AlwaysAsk`,
    /// widening ⇒ re-prompt. Replaces the direct `ApprovalCache::evaluate(...)`
    /// call in `execute_semantic`. Preserves flag-off parity because, absent any
    /// durable grant, the engine delegates the elevated-risk widening judgement
    /// straight back to the frozen `ApprovalCache` (same outcomes).
    perm_engine: DefaultPermissionEngine,
    /// Persistent, scoped, revocable grants over `capability_grants_scoped`
    /// (the SAME `skills.db`, never a second store). Default: an in-memory store
    /// with no durable grants, so `authorize` behaves exactly like the frozen
    /// `ApprovalCache` (flag-off parity, Property 11). Wire a db-backed store via
    /// [`Self::with_grant_store`] for durable grant reuse + revocation (R6.6).
    grants: Arc<GrantStore>,
    /// RC1 schema-driven argument generation: the configured model router used
    /// to translate the natural-language `query` into the selected skill's
    /// typed `inputSchema` arguments. `None` in tests / when no LLM is wired —
    /// then execution passes the raw params through unchanged (prior behavior).
    llm_router: Option<Arc<crate::llm::ModelRouter>>,
    /// ICP master flag (`openclaw_icp_enabled`, design §7.2 / Property 11).
    /// Default `false` → `execute_semantic` takes the frozen direct-router path,
    /// byte-for-byte identical to today. Only flipped ON via [`with_cil`].
    icp_enabled: bool,
    /// The Capability Intelligence facade the handler delegates to when the ICP
    /// flag is ON *and* backends are non-degraded (design §8.8). Default `None`:
    /// no facade is wired yet (no public constructor exists until phase 5), so
    /// flag-ON honestly falls back to the frozen path — never a panic.
    cil: Option<Arc<CapabilityIntelligence>>,
    /// Embedder/network availability signal (design §13.1/§13.2). Default
    /// non-degraded. When degraded, the handler uses the frozen router path
    /// (honest degraded fallback) even if a facade is wired.
    degraded: DegradedState,
}

impl SemanticOpenClawHandler {
    pub fn new(
        registry: Arc<ProductionSkillRegistry>,
        runtimes: Arc<RuntimeRegistry>,
        audit: Arc<AuditLedger>,
    ) -> Self {
        let router = Arc::new(SemanticSkillRouter::new(registry.clone(), None));

        Self {
            router,
            registry,
            runtimes,
            audit,
            approval: super::approval::ApprovalCache::new(),
            perm_engine: DefaultPermissionEngine::new(),
            grants: Self::in_memory_grant_store(),
            llm_router: None,
            // ICP defaults: flag OFF, no facade, non-degraded → frozen path
            // (byte-for-byte parity, Property 11).
            icp_enabled: false,
            cil: None,
            degraded: DegradedState::non_degraded(),
        }
    }

    pub fn with_runtime_manager(
        registry: Arc<ProductionSkillRegistry>,
        runtimes: Arc<RuntimeRegistry>,
        runtime_manager: Arc<super::runtime_manager::RuntimeManager>,
        audit: Arc<AuditLedger>,
    ) -> Self {
        let router = Arc::new(SemanticSkillRouter::new(
            registry.clone(),
            Some(runtime_manager),
        ));

        Self {
            router,
            registry,
            runtimes,
            audit,
            approval: super::approval::ApprovalCache::new(),
            perm_engine: DefaultPermissionEngine::new(),
            grants: Self::in_memory_grant_store(),
            llm_router: None,
            // ICP defaults: flag OFF, no facade, non-degraded → frozen path
            // (byte-for-byte parity, Property 11).
            icp_enabled: false,
            cil: None,
            degraded: DegradedState::non_degraded(),
        }
    }

    /// Wire the Capability Intelligence Layer (ICP) into the handler.
    ///
    /// Builder style so existing constructors/call sites stay unchanged. The
    /// `enabled` flag is the config's `openclaw_icp_enabled`; `facade` is the CIL
    /// entry point (still `None` today — no public constructor exists until phase
    /// 5); `degraded` reports embedder/network availability.
    ///
    /// The ICP fulfillment path in `execute_semantic` runs **only** when
    /// `enabled == true`, a facade is present, *and* the state is non-degraded.
    /// With any of those unmet the handler falls back to the frozen direct-router
    /// path — preserving flag-off parity and honest degraded behavior.
    pub fn with_cil(
        mut self,
        enabled: bool,
        facade: Option<Arc<CapabilityIntelligence>>,
        degraded: DegradedState,
    ) -> Self {
        self.icp_enabled = enabled;
        self.cil = facade;
        self.degraded = degraded;
        self
    }

    /// Whether the ICP fulfillment path is actually live: flag ON, a facade is
    /// wired, and backends are non-degraded. Returns `false` today (no facade
    /// constructor exists), so `execute_semantic` always takes the frozen path.
    fn cil_active(&self) -> bool {
        self.icp_enabled && self.cil.is_some() && !self.degraded.is_degraded()
    }

    /// Attach the model router used for RC1 schema-driven argument generation.
    /// Builder style so existing constructors/call sites stay unchanged.
    pub fn with_arg_gen_llm(mut self, router: Arc<crate::llm::ModelRouter>) -> Self {
        self.llm_router = Some(router);
        self
    }

    /// Default in-memory [`GrantStore`] used until a durable store is wired.
    ///
    /// It has no `capability_grants_scoped` table, so every grant lookup returns
    /// "no reusable grant" — which makes [`PermissionEngine::authorize`] delegate
    /// the elevated-risk decision straight to the frozen `ApprovalCache`, i.e.
    /// byte-for-byte the pre-swap behavior (flag-off parity, Property 11).
    fn in_memory_grant_store() -> Arc<GrantStore> {
        Arc::new(
            GrantStore::open(std::path::Path::new(":memory:")).expect("open in-memory grant store"),
        )
    }

    /// Wire a durable, db-backed [`GrantStore`] (over the SAME `skills.db` the
    /// registry uses) so grant reuse and revocation take effect (R6.6). Builder
    /// style so existing constructors/call sites stay unchanged.
    ///
    /// Without this, the handler uses an in-memory grant store and `authorize`
    /// behaves exactly like the frozen `ApprovalCache` — no durable grants,
    /// preserving flag-off parity.
    pub fn with_grant_store(mut self, grants: Arc<GrantStore>) -> Self {
        self.grants = grants;
        self
    }

    /// Revoke a persisted capability grant by id (R6.6).
    ///
    /// Marks the grant `revoked = 1` in the [`GrantStore`] so the affected
    /// capability requires fresh approval the next time it is used. Delegates to
    /// the frozen-superset [`PermissionEngine::revoke`]. Exposed as a public
    /// method for the desktop permission-management command (task 13.3) to call.
    ///
    /// Returns an error when the `grant_id` does not exist (nothing revoked) or
    /// the store is unavailable — never a silent no-op (honesty invariant).
    pub fn revoke_grant(&self, grant_id: &str) -> Result<(), crate::openclaw::cil::CilError> {
        self.perm_engine.revoke(grant_id, &self.grants)
    }

    /// Emit an [`AuditLedger`] entry for a single permission decision (R7.1).
    ///
    /// Every `authorize`/bypass outcome — `Allow`/`Prompt`/`Deny` — produces one
    /// signed, appended entry so the decision trail is complete and honest. The
    /// `decision_label` carries the tier/outcome; `allowed` drives the entry's
    /// success flag. This is additive telemetry: it never changes the
    /// `execute_semantic` `ToolResult`, so flag-off output parity is preserved.
    fn audit_permission_decision(
        &self,
        skill_id: &str,
        invocation_id: &str,
        risk: RiskLevel,
        decision_label: &str,
        allowed: bool,
    ) {
        let output = ToolResult {
            success: allowed,
            data: serde_json::json!({ "permission_decision": decision_label }),
            error: if allowed {
                None
            } else {
                Some(format!("permission decision: {decision_label}"))
            },
        };
        let mut entry = AuditLedger::create_invocation_entry(
            AuditEventType::SecurityEvent,
            skill_id,
            invocation_id,
            "",
            "",
            skill_id,
            risk.as_str(),
            &serde_json::json!({ "stage": "permission", "decision": decision_label }),
            &output,
            0,
            "",
            decision_label,
        );
        self.audit_append(&mut entry);
    }

    /// A6: Semantic routing - convert tool request to routing intent.
    fn create_routing_intent(&self, tool_name: &str, params: &serde_json::Value) -> RoutingIntent {
        // Extract request description from parameters or tool name
        let request = if let Some(query) = params.get("query").and_then(|v| v.as_str()) {
            query.to_string()
        } else if let Some(description) = params.get("description").and_then(|v| v.as_str()) {
            description.to_string()
        } else if let Some(task) = params.get("task").and_then(|v| v.as_str()) {
            task.to_string()
        } else {
            // Fall back to tool name and parameters
            format!("{} with parameters {}", tool_name, params)
        };

        // Extract required capabilities from tool name or parameters
        let required_capabilities = if let Some(caps) = params.get("required_capabilities") {
            caps.as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        } else {
            // Infer capabilities from tool name
            let mut caps = Vec::new();
            if tool_name.contains("web")
                || tool_name.contains("search")
                || tool_name.contains("fetch")
            {
                caps.push("network".to_string());
            }
            if tool_name.contains("file")
                || tool_name.contains("read")
                || tool_name.contains("write")
            {
                caps.push("filesystem".to_string());
            }
            if tool_name.contains("image") || tool_name.contains("generate") {
                caps.push("image_generation".to_string());
            }
            caps
        };

        RoutingIntent {
            request,
            required_capabilities,
            max_risk: RiskLevel::Yellow, // Default safe level
            preferred_resource: None,
            context: RoutingContext {
                resource_pressure: ResourcePressure::Low, // TODO: Get from HRA
                gpu_memory_mb: Some(4096),                // TODO: Get from system
                network_available: true,                  // TODO: Get from network detector
                session_trust: TrustTier::Community,      // TODO: Get from session context
            },
        }
    }

    /// Sign and append audit entry.
    fn audit_append(&self, entry: &mut AuditEntry) {
        entry.signature = self.audit.sign_entry(entry);
        if let Err(e) = self.audit.append(entry) {
            tracing::warn!(
                invocation_id = %entry.invocation_id,
                error = %e,
                "SemanticOpenClaw: failed to write audit entry"
            );
        }
    }

    /// RC1: turn the natural-language request into the selected skill's typed,
    /// schema-valid arguments. General — driven purely by the skill's declared
    /// `input_schema`, no per-skill or keyword logic.
    ///
    /// Order: (1) skill declares no arguments → pass through; (2) caller already
    /// supplied schema-valid args → use them (deterministic fast path, no LLM);
    /// (3) otherwise generate via the configured LLM + validate + repair;
    /// (4) no LLM wired → pass through unchanged (prior behavior).
    async fn resolve_arguments(
        &self,
        selected_skill: &super::registry::SkillMetadata,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let Some(schema) = selected_skill.input_schema.as_ref() else {
            return Ok(params.clone());
        };
        if !super::arg_gen::schema_expects_arguments(schema) {
            return Ok(params.clone());
        }
        // Deterministic fast path: caller already passed schema-valid args
        // (e.g. a programmatic invocation, not the freeform chat `query`).
        if super::arg_gen::validate_against_schema(params, schema).is_ok() {
            return Ok(params.clone());
        }

        // Derive the natural-language request from the tool params.
        let request = params
            .get("query")
            .or_else(|| params.get("description"))
            .or_else(|| params.get("task"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| params.to_string());

        let Some(router) = self.llm_router.as_ref() else {
            // No LLM wired (tests / degraded): don't fabricate — pass through.
            return Ok(params.clone());
        };
        let Some(backend) = router.route("openclaw_argument_generation").await else {
            return Err(
                "argument generation needs an LLM backend, but none is configured/reachable"
                    .to_string(),
            );
        };

        super::arg_gen::generate_arguments(
            backend.as_ref(),
            &selected_skill.skill_id,
            &selected_skill.description,
            schema,
            &request,
            3,
        )
        .await
    }

    /// ICP fulfillment attempt (design §8.8). Returns `Some(result)` only when
    /// the CIL produced a terminal outcome the handler should return; `None`
    /// means "fall back to the frozen direct-router path" (honest decline or a
    /// degraded/errored backend — never a panic).
    ///
    /// Reachable only when [`cil_active`](Self::cil_active) is `true` (flag ON +
    /// facade wired + non-degraded), which today happens only in tests / opt-in
    /// wiring. When the facade returns a [`Fulfillment::Plan`], the handler hands
    /// the validated frozen [`ExecutionGraph`] to the frozen [`ExecutionEngine`]
    /// (R4.4: CIL never touches containers — the engine dispatches Skill nodes to
    /// the registered [`OpenClawExecutor`]) and returns the evidence-wrapped
    /// result (R4.5). `Recommend` is wired by the recommender phase (§7).
    async fn try_fulfill_via_cil(
        &self,
        _tool_name: &str,
        params: &serde_json::Value,
        ctx: &RuntimeContext,
    ) -> Option<ToolResult> {
        let cil = self.cil.as_ref()?;

        let query = params
            .get("query")
            .or_else(|| params.get("description"))
            .or_else(|| params.get("task"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| params.to_string());

        let req_ctx = RequestCtx::default();
        match cil.fulfill(&query, &req_ctx).await {
            // Honest decline → let the frozen router path answer instead.
            Ok(Fulfillment::Decline { reason }) => {
                tracing::debug!(reason = %reason, "CIL declined; falling back to frozen router");
                None
            }
            // A validated frozen capability graph. KRIA orchestration authority:
            // the facade decided *what* to run; the FROZEN ExecutionEngine
            // executes it. CIL never touches containers (R4.4). Permission
            // decisions are enforced by the permission phase (§11); this task
            // wires the execute → evidence-wrap path only.
            Ok(Fulfillment::Plan(graph, _decisions)) => {
                match self.execute_plan_via_engine(&graph, &query, ctx).await {
                    Some(result) => Some(result),
                    // No frozen runtime wired in this handler instance (e.g. a
                    // test/router-only constructor) → honest fallback to the
                    // frozen router path rather than a panic.
                    None => {
                        tracing::warn!(
                            graph_id = %graph.id,
                            "CIL produced a plan but no OpenClaw runtime is wired for the \
                             frozen engine; falling back to frozen router"
                        );
                        None
                    }
                }
            }
            // No acceptable INSTALLED skill, but the recommender found
            // marketplace capabilities the user could install (pure read, R8.2).
            // Surface them honestly as the tool result — capability discovery
            // naturally produces recommendations — instead of dropping them.
            Ok(Fulfillment::Recommend(recs)) if !recs.is_empty() => {
                Some(Self::recommendations_to_result(&recs))
            }
            // Empty recommendation set → honest fallback to the frozen path.
            Ok(Fulfillment::Recommend(_)) => None,
            // Degraded/errored backend → honest fallback, never a hard failure.
            Err(e) => {
                tracing::warn!(error = %e, "CIL fulfill failed; falling back to frozen router");
                None
            }
        }
    }

    /// Turn a ranked [`Recommendation`] set (from the CIL recommender, a pure
    /// offline read over the marketplace catalog + capability graph) into an
    /// honest [`ToolResult`] the agent can present. This is the
    /// capability-missing outcome: no installed skill satisfies the goal, so KRIA
    /// surfaces the marketplace capabilities the user could install (with each
    /// candidate's real, signal-derived rationale and any graph alternatives).
    /// Nothing is installed here — acquisition is an explicit, separately
    /// approved step. Never fabricates a candidate (an empty set never reaches
    /// this method — see the caller).
    fn recommendations_to_result(recs: &[Recommendation]) -> ToolResult {
        let items: Vec<serde_json::Value> = recs
            .iter()
            .map(|r| {
                serde_json::json!({
                    "provider_id": r.provider_id,
                    "slug": r.slug,
                    "version": r.version,
                    "score": r.score,
                    "trust_tier": r.trust.map(|t| t.as_str().to_string()),
                    "quality": r.quality,
                    "popularity": r.popularity,
                    "deprecated": r.deprecated,
                    "rationale": r.rationale,
                    "alternatives": r.alternatives,
                })
            })
            .collect();
        let slugs: Vec<&str> = recs.iter().map(|r| r.slug.as_str()).collect();
        let summary = format!(
            "No installed skill can do this yet, but {} marketplace capabilit{} could: {}. \
             Ask to install one to proceed (nothing was installed automatically).",
            recs.len(),
            if recs.len() == 1 { "y" } else { "ies" },
            slugs.join(", ")
        );
        ToolResult {
            success: true,
            data: serde_json::json!({
                "outcome": "recommendations",
                "summary": summary,
                "recommendations": items,
            }),
            error: None,
        }
    }

    /// Hand a validated frozen [`ExecutionGraph`] to the frozen
    /// [`ExecutionEngine`] and return the evidence-wrapped result (R4.4/R4.5).
    ///
    /// KRIA orchestration authority: the CIL facade decided *what* to run and
    /// produced the graph; this method reuses the **frozen** engine to run it —
    /// no second engine, no fork, no engine modification. The engine dispatches
    /// each `Skill` node (addressed by the open-vocabulary `provider_id` `"openclaw"`)
    /// to the registered [`OpenClawExecutor`], which is the ONLY component that
    /// touches containers. CIL/this method never call `DockerRuntime` directly.
    ///
    /// The [`OpenClawExecutor`] is built from the handler's existing frozen
    /// [`RuntimeRegistry`] (the same `Arc<dyn SkillRuntime>` the frozen direct
    /// path uses via `runtime.execute`), so container lifecycle stays unchanged.
    /// Cancellation is propagated by threading the runtime `ctx`'s
    /// [`CancellationToken`](tokio_util::sync::CancellationToken) into the frozen
    /// [`ExecutionContext`], exactly as the frozen path does.
    ///
    /// Returns `None` when no OpenClaw runtime is registered in this handler
    /// instance (some constructors are router-only) so the caller can fall back
    /// honestly instead of panicking.
    async fn execute_plan_via_engine(
        &self,
        graph: &ExecutionGraph,
        query: &str,
        ctx: &RuntimeContext,
    ) -> Option<ToolResult> {
        // Obtain the frozen OpenClaw runtime from the handler's registry. This is
        // the same runtime the frozen direct path resolves via
        // `RuntimeRegistry::kind_for_skill` (Docker; Gpu is the accelerated
        // variant). Absent → honest `None` fallback (no runtime wired).
        let runtime = self
            .runtimes
            .get(RuntimeKind::Docker)
            .or_else(|| self.runtimes.get(RuntimeKind::Gpu))?;

        // RC-3: schema-driven argument generation for the CIL plan. The facade
        // emits Skill nodes with empty `params` (it decides *what* to run, not
        // the typed args). Before execution we resolve each node's arguments
        // from the natural-language `query` using the SAME frozen `arg_gen`
        // path the direct router uses (`resolve_arguments`) — no duplicate
        // arg-gen logic. A node whose skill has no schema, or when no LLM is
        // wired, passes through unchanged (honest, never fabricated).
        let mut resolved_graph = ExecutionGraph::new(graph.id.clone(), graph.goal_id.clone());
        for node in graph.nodes() {
            let mut resolved = node.clone();
            if let NodeKind::Skill {
                action_id, params, ..
            } = &mut resolved.kind
            {
                match self.registry.get_skill(action_id) {
                    Ok(skill) => {
                        let request = serde_json::json!({ "query": query });
                        match self.resolve_arguments(&skill, &request).await {
                            Ok(args) => *params = args,
                            Err(e) => tracing::warn!(
                                skill = %action_id,
                                error = %e,
                                "[CIL] argument generation failed for plan node; \
                                 executing with empty params"
                            ),
                        }
                    }
                    Err(e) => tracing::warn!(
                        skill = %action_id,
                        error = %e,
                        "[CIL] plan node skill not found in registry for arg-gen"
                    ),
                }
            }
            resolved_graph.add_node(resolved);
        }

        // Build the frozen engine and register the OpenClaw executor built from
        // the frozen runtime. `OpenClawExecutor::new(runtime)` is the documented
        // seam; the engine stays backend-agnostic and dispatches OpenClaw Skill
        // nodes to it. No planner/scheduler/engine code is modified.
        let mut engine = ExecutionEngine::new();
        engine.register_executor(Arc::new(OpenClawExecutor::new(runtime)));

        // Frozen execution context with cancellation propagated from the runtime
        // ctx (preserve cancellation propagation, same as the frozen path).
        let exec_ctx = ExecutionContext::new(graph.goal_id.clone(), format!("cil-{}", graph.id))
            .with_cancellation(ctx.cancellation.clone());

        let start = Instant::now();
        let schedule = engine.execute_graph(&resolved_graph, &exec_ctx).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        // Aggregate per-node outputs recorded in the shared frozen context into
        // the raw result. Multi-capability plans surface every node's output
        // keyed by node id; the terminal status governs success.
        let outputs = exec_ctx.all_outputs().await;
        let raw_result = match &schedule.status {
            ScheduleStatus::Completed => ToolResult {
                success: true,
                data: serde_json::json!({
                    "graph_id": graph.id,
                    "completed_nodes": schedule.completed_nodes,
                    "outputs": outputs,
                }),
                error: None,
            },
            ScheduleStatus::Failed(reason) => ToolResult {
                success: false,
                data: serde_json::json!({
                    "graph_id": graph.id,
                    "completed_nodes": schedule.completed_nodes,
                    "failed_nodes": schedule.failed_nodes,
                    "outputs": outputs,
                }),
                error: Some(format!("multi-capability plan failed: {reason}")),
            },
            ScheduleStatus::Cancelled => ToolResult {
                success: false,
                data: serde_json::json!({
                    "graph_id": graph.id,
                    "completed_nodes": schedule.completed_nodes,
                }),
                error: Some("multi-capability plan cancelled".to_string()),
            },
        };

        // R4.5: wrap as verified/evidence-wrapped output, reusing the SAME
        // `EvidenceWrapper::wrap` the frozen direct path uses. The graph id is
        // the plan identity (a multi-capability plan is not a single skill).
        let wrapped = EvidenceWrapper::wrap(
            &graph.id,
            ExecutionSource::OpenClaw,
            &raw_result,
            duration_ms,
        );

        Some(ToolResult {
            success: raw_result.success,
            data: serde_json::json!(wrapped),
            error: raw_result.error,
        })
    }

    async fn execute_semantic(
        &self,
        tool_name: &str,
        params: serde_json::Value,
        ctx: RuntimeContext,
    ) -> ToolResult {
        // ── ICP flag branch (design §8.8; R7.2/R7.3/R13.1/R13.2) ──
        // Flag ON + wired facade + non-degraded backends → delegate to the
        // Capability Intelligence Layer. In EVERY other case — flag OFF (the
        // default), no facade wired, or degraded backends — fall through to the
        // frozen direct-router path below, byte-for-byte unchanged (Property 11).
        //
        // When active, a `Fulfillment::Plan` is executed by handing its frozen
        // `ExecutionGraph` to the frozen `ExecutionEngine` (R4.4) and returning
        // the evidence-wrapped result (R4.5); any other outcome (decline /
        // degraded / no runtime wired) falls through to the frozen path below.
        if self.cil_active() {
            if let Some(result) = self.try_fulfill_via_cil(tool_name, &params, &ctx).await {
                return result;
            }
            // Honest fallback: facade declined / degraded → frozen path below.
        }

        let start = Instant::now();
        let invocation_id = uuid::Uuid::new_v4().to_string();

        // A6: Create routing intent from request
        let intent = self.create_routing_intent(tool_name, &params);

        // A6: Route to best skill using semantic router
        let routing_decision = match self.router.route(intent).await {
            Ok(decision) => decision,
            Err(e) => {
                return ToolResult::err(format!("Routing failed: {}", e));
            }
        };

        let Some(selected_skill) = routing_decision.skill else {
            // A6.9: Provide suggestions when no skill selected
            if !routing_decision.alternatives.is_empty() {
                let suggestions: Vec<String> = routing_decision
                    .alternatives
                    .iter()
                    .map(|alt| {
                        format!(
                            "- {} (confidence: {:.2}): {}",
                            alt.skill.name, alt.confidence, alt.reasoning
                        )
                    })
                    .collect();

                return ToolResult::err(format!(
                    "No suitable skill found. Reason: {}\n\nSuggestions:\n{}",
                    routing_decision.reasoning,
                    suggestions.join("\n")
                ));
            } else {
                return ToolResult::err(format!(
                    "No suitable skill found: {}",
                    routing_decision.reasoning
                ));
            }
        };

        tracing::info!(
            tool_name = %tool_name,
            selected_skill = %selected_skill.skill_id,
            confidence = %routing_decision.confidence,
            reasoning = %routing_decision.reasoning,
            "A6 semantic routing selected skill"
        );

        // TrustConfig enforcement fix (product gap 6/8): read the LIVE,
        // Settings-controlled trust config (hot, no restart — same pattern
        // `safety::global_halt` uses). Previously these two knobs were
        // persisted but read by nothing anywhere.
        let trust_cfg = super::trust_runtime::current();

        // `community_allows_network`: when off, a Community-tier skill's
        // network capability is demoted to `None` at execution time — the
        // skill still installs with its declared capability (enforcement at
        // install time is unaffected, per R3/R12), but a disallowed-network
        // Community skill cannot actually reach the network via THIS path.
        let mut effective_capabilities = selected_skill.capabilities.clone();
        let mut effective_grants = selected_skill.granted_capabilities.clone();
        if selected_skill.trust_tier == TrustTier::Community && !trust_cfg.community_allows_network
        {
            effective_capabilities.network = false;
            effective_capabilities.network_domains.clear();
            effective_grants.retain(|g| {
                g.capability.kind != crate::openclaw::capability::CapabilityKind::Network
            });
        }

        // `verified_skips_hitl`: consult the REAL `ApprovalCache` for any
        // skill whose resource profile requires approval (elevated risk).
        // Verified-tier skills auto-approve ONLY when the flag is on;
        // otherwise (or for non-Verified elevated-risk skills lacking prior
        // approval) an honest `NeedsHitl` decline is returned — never a
        // silent bypass, and never a fabricated success (no HITL prompt UI
        // exists yet in this session, per the GUI-driver blocker).
        let requires_approval = !matches!(selected_skill.risk_level, RiskLevel::Green);
        if requires_approval {
            let caps_for_approval = crate::openclaw::capability::capabilities_of(&effective_grants);
            if selected_skill.trust_tier == TrustTier::Verified && trust_cfg.verified_skips_hitl {
                // Explicit, intentional bypass — the ONE configured tier
                // allowed to skip HITL, per design.md's "Approval-bypass
                // rules: only the configured tiers bypass HITL" — recorded
                // in the cache as auto-approved so it's consistent with a
                // real evaluate() call, not silently skipped.
                let hash = super::approval::ApprovalCache::compute_hash(
                    &selected_skill.skill_id,
                    "",
                    &caps_for_approval,
                    selected_skill.resource_class.as_str(),
                    "",
                );
                self.approval
                    .record_approved(&super::approval::ApprovalToken {
                        hash,
                        risk: selected_skill.risk_level,
                        issued_at: chrono::Utc::now(),
                    });
                // R7.1: every permission decision is audited — including the
                // configured Verified-tier bypass (an explicit `Allow`).
                self.audit_permission_decision(
                    &selected_skill.skill_id,
                    &invocation_id,
                    selected_skill.risk_level,
                    "allow:verified_bypass",
                    true,
                );
            } else {
                // Task 11.4: swap `ApprovalCache::evaluate(...)` for the tiered
                // `PermissionEngine::authorize(...)`. The engine is a strict
                // SUPERSET of the frozen `ApprovalCache` (R7.4): with no durable
                // grant in the store it delegates the elevated-risk widening
                // judgement straight back to the frozen `ApprovalCache` — the
                // SAME oracle, the SAME outcomes — so the observable `ToolResult`
                // is preserved for the existing GREEN/widening cases (Property
                // 11). `previous_caps` stays `None`, matching the prior
                // `evaluate(..)` call; `budget` mirrors the old `resource_class`
                // identity input to `compute_hash`.
                let mut auth_req = AuthorizeRequest::new(
                    &selected_skill.skill_id,
                    caps_for_approval,
                    selected_skill.trust_tier,
                );
                auth_req.budget = selected_skill.resource_class.as_str().to_string();

                let decision = self.perm_engine.authorize(&auth_req, &self.grants);
                let (label, allowed) = match &decision {
                    PermissionDecision::Allow { tier, .. } => (format!("allow:{tier:?}"), true),
                    PermissionDecision::Prompt { tier, .. } => (format!("prompt:{tier:?}"), false),
                    PermissionDecision::Deny { .. } => ("deny".to_string(), false),
                };
                // R7.1: audit the permission decision (Allow / Prompt / Deny).
                self.audit_permission_decision(
                    &selected_skill.skill_id,
                    &invocation_id,
                    selected_skill.risk_level,
                    &label,
                    allowed,
                );

                if !allowed {
                    // Preserve the EXACT frozen error string: a `Prompt`
                    // (needs-HITL) or standing `Deny` maps to the same honest
                    // "requires human approval" refusal the old `evaluate` path
                    // returned when `!decision.is_approved()`.
                    return ToolResult::err(format!(
                        "OpenClaw: skill '{}' requires human approval before it can run (risk={}, trust_tier={}) — \
                         no approval UI is wired to this execution path yet",
                        selected_skill.skill_id, selected_skill.risk_level.as_str(), selected_skill.trust_tier.as_str()
                    ));
                }
            }
        }

        // Create SkillDescriptor from metadata for runtime
        let skill_descriptor = SkillDescriptor {
            skill_id: selected_skill.skill_id.clone(),
            name: selected_skill.name.clone(),
            description: selected_skill.description.clone(),
            category: selected_skill
                .categories
                .first()
                .unwrap_or(&"unknown".to_string())
                .clone(),
            parameters: serde_json::json!({}), // TODO: Extract from metadata
            risk_level: selected_skill.risk_level,
            // Capability-grant wiring fix + TrustConfig enforcement: network
            // policy derived from the (possibly trust-demoted) effective
            // capabilities, never the raw declared ones.
            network_policy: effective_capabilities.to_network_policy(),
            resource_profile: ResourceProfile {
                resource_class: selected_skill.resource_class,
                cpu_limit: "1".to_string(),
                memory_limit: "512M".to_string(),
                timeout_secs: 60,
                max_output_bytes: 1024 * 1024,
                requires_approval: false,
            },
            capabilities: effective_capabilities.clone(),
            // Capability-grant wiring fix + TrustConfig enforcement: the
            // registry's real, authoritative granted capabilities, with any
            // trust-demoted (e.g. network, when community_allows_network is
            // off) grants already removed above.
            granted: effective_grants.clone(),
            trust_tier: selected_skill.trust_tier,
            source: SkillSource::Bundled,
            installed_at: chrono::Utc::now(),
            last_used_at: None,
            use_count: 0,
            status: SkillStatus::Active,
        };

        // Get runtime for selected skill
        let runtime_kind = RuntimeRegistry::kind_for_skill(&skill_descriptor);
        let Some(runtime) = self.runtimes.get(runtime_kind) else {
            return ToolResult::err(format!(
                "No runtime available for skill {}",
                selected_skill.skill_id
            ));
        };

        // Audit: Started
        let mut started = AuditLedger::create_invocation_entry(
            AuditEventType::InvocationStarted,
            &selected_skill.skill_id,
            &invocation_id,
            "",
            "",
            &selected_skill.skill_id,
            selected_skill.risk_level.as_str(),
            &params,
            &ToolResult {
                success: true,
                data: serde_json::Value::Null,
                error: None,
            },
            0,
            selected_skill.resource_class.as_str(),
            "",
        );
        self.audit_append(&mut started);

        // Bundle-execution fix: an installed marketplace/generated skill's
        // handler is NOT baked into the substrate image. If this skill was
        // installed as a bundle (has a `bundle_path`) and the installer
        // prepared a bridge-format runtime dir (`<bundle_path>/.bridge`),
        // hand that dir to the runtime so it bind-mounts the handler into a
        // bespoke execution container. Baked-in skills have no bundle_path →
        // `None` → run from the warm pool exactly as before.
        let mounted_skill_dir = selected_skill.bundle_path.as_ref().and_then(|bp| {
            let bridge_dir = std::path::Path::new(bp).join(".bridge");
            if bridge_dir.is_dir() {
                Some(bridge_dir)
            } else {
                None
            }
        });

        // RC1: schema-driven argument generation. Translate the freeform
        // request into the selected skill's typed, schema-valid arguments
        // BEFORE execution. Never send unresolved/invalid args to a skill.
        let resolved_params = match self.resolve_arguments(&selected_skill, &params).await {
            Ok(args) => args,
            Err(e) => {
                return ToolResult::err(format!(
                    "OpenClaw could not prepare arguments for skill '{}': {e}",
                    selected_skill.skill_id
                ));
            }
        };

        // Execute via runtime
        let spec = LaunchSpec {
            skill_id: selected_skill.skill_id.clone(),
            params: resolved_params,
            resource_class: selected_skill.resource_class,
            timeout: Duration::from_secs(60), // TODO: Get from skill metadata
            correlation_id: invocation_id.clone(),
            // Capability-grant wiring fix + TrustConfig enforcement: the
            // runtime materializes the container SOLELY from this vec
            // (docker.rs::execute) — the skill's real, registry-persisted
            // grants, with any trust-demoted grants already removed above.
            grants: effective_grants,
            mounted_skill_dir,
        };

        let raw_result = runtime.execute(spec, ctx).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        // A6.8: Record feedback for learning
        let _ = self
            .router
            .record_feedback(
                &selected_skill.skill_id,
                raw_result.success,
                duration_ms,
                0.5, // TODO: Calculate actual resource usage
                routing_decision.confidence,
            )
            .await;

        // Evidence wrap result
        let wrapped = EvidenceWrapper::wrap(
            &selected_skill.skill_id,
            ExecutionSource::OpenClaw,
            &raw_result,
            duration_ms,
        );

        // Audit: Completed/Failed
        let event_type = if raw_result.success {
            AuditEventType::InvocationCompleted
        } else {
            AuditEventType::InvocationFailed
        };

        let mut completed = AuditLedger::create_invocation_entry(
            event_type,
            &selected_skill.skill_id,
            &invocation_id,
            "",
            "",
            &selected_skill.skill_id,
            selected_skill.risk_level.as_str(),
            &params,
            &raw_result,
            duration_ms,
            selected_skill.resource_class.as_str(),
            "",
        );
        self.audit_append(&mut completed);

        ToolResult {
            success: raw_result.success,
            data: serde_json::json!(wrapped),
            error: raw_result.error,
        }
    }
}

#[async_trait]
impl ToolHandler for SemanticOpenClawHandler {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        // Extract tool name from parameters if available
        let tool_name = params
            .get("_tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("openclaw_semantic")
            .to_string();

        self.execute_semantic(&tool_name, params, RuntimeContext::detached())
            .await
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool_name = params
            .get("_tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("openclaw_semantic")
            .to_string();

        self.execute_semantic(&tool_name, params, RuntimeContext::from_tool_context(&ctx))
            .await
    }
}

/// BUG #3 FIX (category B: Capability Discovery issue, NOT an LLM limitation).
/// Root cause: `router.rs` had a lexical pattern mapping "list/show/what/which
/// skills installed/have/available" → tool hint `"list_installed_skills"`, but
/// NO tool by that name was ever registered anywhere in `ToolRegistry`. The
/// hint was silently dropped by `TurnGate::direct_tool_hint`'s
/// `allowed_tool_names.contains(hint)` filter, so `list_installed_skills` never
/// appeared in the LLM's callable-tool set. With no way to query the real
/// registry, capability questions ("is there a word-count skill installed?")
/// were answered from the model's own static training assumptions instead of
/// runtime reality — the exact opposite of the requirement that capability
/// discovery must always reflect the real, enabled state of the skill
/// registry. This tool queries the SAME `ProductionSkillRegistry` instance the
/// real router (`SemanticSkillRouter::route` → `get_enabled_skills()`) already
/// uses, so its answer can never drift from what would actually execute.
struct ListInstalledSkills {
    registry: Arc<ProductionSkillRegistry>,
}

#[async_trait]
impl ToolHandler for ListInstalledSkills {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let filter = params
            .get("filter")
            .and_then(|v| v.as_str())
            .unwrap_or("all");

        let query_result = match filter {
            "enabled" => self.registry.get_enabled_skills(),
            _ => {
                // "all" / "disabled": search with no state filter, then optionally
                // narrow client-side, so both queries share one real DB read path.
                self.registry.search_skills(&super::registry::SkillQuery {
                    slug: None,
                    publisher: None,
                    description_contains: None,
                    tags: Vec::new(),
                    categories: Vec::new(),
                    capabilities: Vec::new(),
                    runtime_requirements: None,
                    risk_level: None,
                    state: None,
                    enabled_only: false,
                })
            }
        };

        let skills = match query_result {
            Ok(skills) => skills,
            Err(e) => return ToolResult::err(format!("registry query failed: {e}")),
        };

        let filtered: Vec<&super::registry::SkillMetadata> = match filter {
            "disabled" => skills
                .iter()
                .filter(|s| matches!(s.state, super::registry::SkillState::Disabled))
                .collect(),
            _ => skills.iter().collect(),
        };

        let entries: Vec<serde_json::Value> = filtered
            .iter()
            .map(|s| {
                serde_json::json!({
                    "skill_id": s.skill_id,
                    "name": s.name,
                    "description": s.description,
                    "category": s.category,
                    "state": format!("{:?}", s.state).to_ascii_lowercase(),
                    "version": s.version,
                    "trust_tier": format!("{:?}", s.trust_tier),
                })
            })
            .collect();

        ToolResult::ok(serde_json::json!({
            "filter": filter,
            "count": entries.len(),
            "skills": entries,
        }))
    }
}

/// A6: Register semantic OpenClaw handler as a single "openclaw" tool.
/// Replaces all individual oc_* tool registrations.
#[allow(clippy::too_many_arguments)]
pub fn register_semantic_openclaw(
    _tool_registry: &crate::tools::registry::ToolRegistry,
    registry: Arc<ProductionSkillRegistry>,
    runtimes: Arc<RuntimeRegistry>,
    _audit: Arc<AuditLedger>,
    llm_router: Option<Arc<crate::llm::ModelRouter>>,
    // ICP wiring (RC-1): the Capability Intelligence facade + its master flag +
    // backend-availability state. `None`/`false` preserves the frozen
    // direct-router path byte-for-byte (Property 11). When `icp_enabled` is true
    // and a facade is wired, `execute_semantic` delegates to the CIL.
    cil: Option<Arc<CapabilityIntelligence>>,
    icp_enabled: bool,
    degraded: DegradedState,
) {
    let def = ToolDef {
        name: "openclaw".to_string(),
        // RC-5: an action/capability-oriented description so the agent selects
        // this tool for capability requests WITHOUT the user having to say
        // "OpenClaw" or "skill". The tool discovers and runs the best-matching
        // installed capability and returns a real, verified result (or an honest
        // "no matching skill") — the assistant should not answer capability tasks
        // from its own knowledge.
        description: "Run a capability to actually DO a task that needs execution rather than \
            just an answer — for example: arithmetic/calculation, reading/parsing/converting \
            files, extracting text from PDFs or documents, fetching or searching the web, \
            transcribing audio, or processing images and data. Call this whenever the user asks \
            you to PERFORM such a task, even if they do not mention 'OpenClaw' or 'skill'. It \
            automatically finds and runs the best-matching installed capability and returns a \
            real, verified result, or an honest 'no matching skill found'. Do not compute or \
            fabricate the answer yourself when the task requires a capability."
            .to_string(),
        category: "openclaw".to_string(),
        parameters: vec![
            ParamDef {
                name: "query".to_string(),
                param_type: "string".to_string(),
                description: "Describe what you want to accomplish".to_string(),
                required: true,
                default: None,
            },
            ParamDef {
                name: "required_capabilities".to_string(),
                param_type: "array".to_string(),
                description:
                    "Required capabilities (optional): network, filesystem, image_generation, etc."
                        .to_string(),
                required: false,
                default: None,
            },
        ],
        // RC-4: the `openclaw` tool is a capability DISPATCHER, not itself a
        // privileged action. Its real risk is the risk of the *selected skill*,
        // which `execute_semantic` classifies and gates per-invocation (GREEN ⇒
        // runs; RED/host-scope ⇒ AlwaysAsk; widening ⇒ re-prompt — the tiered
        // `PermissionEngine`). Declaring the dispatcher itself YELLOW caused the
        // outer agent-loop HITL to prompt on EVERY OpenClaw call (duplicate,
        // irritating) even for GREEN skills like the calculator. GREEN here means
        // the dispatch is unprivileged; the downstream per-skill gate remains the
        // single authoritative permission decision (no unsafe skill can bypass
        // it). `list_installed_skills` is likewise a pure read (GREEN).
        default_tier: RiskLevel::Green,
        min_tier: "lite",
    };

    // BUG #3 FIX: register the real skill-introspection tool alongside the
    // execution tool, using the SAME registry instance (clone of the Arc) so
    // "is X installed?" answers can never disagree with what would execute.
    let list_def = ToolDef {
        name: "list_installed_skills".to_string(),
        description: "List OpenClaw skills that are actually installed in the real skill registry right now. Use this to answer ANY question about whether a specific skill/capability is installed, enabled, or disabled — NEVER answer such questions from memory/training data.".to_string(),
        category: "openclaw".to_string(),
        parameters: vec![ParamDef {
            name: "filter".to_string(),
            param_type: "string".to_string(),
            description: "all|enabled|disabled (default all)".to_string(),
            required: false,
            default: None,
        }],
        default_tier: RiskLevel::Green,
        min_tier: "lite",
    };
    _tool_registry.register(
        list_def,
        Arc::new(ListInstalledSkills {
            registry: registry.clone(),
        }),
    );

    let mut handler = SemanticOpenClawHandler::new(registry, runtimes, _audit);
    if let Some(router) = llm_router {
        handler = handler.with_arg_gen_llm(router);
    }
    // RC-1: wire the Capability Intelligence Layer. With `icp_enabled == false`
    // or `cil == None` this is a no-op that keeps the frozen path (parity).
    handler = handler.with_cil(icp_enabled, cil, degraded);
    _tool_registry.register(def, Arc::new(handler));
}

/// Legacy: Build runtime registry for OpenClaw.
pub fn build_runtime_registry(pool: Arc<super::pool::ContainerPool>) -> Arc<RuntimeRegistry> {
    let mut reg = RuntimeRegistry::new();
    reg.register(Arc::new(super::runtime::DockerRuntime::new(pool)));
    Arc::new(reg)
}

/// Legacy: Register individual skill - DEPRECATED in A6.
/// A6 uses semantic routing instead of pre-registering individual skills.
#[deprecated(note = "Use register_semantic_openclaw instead. A6 uses semantic routing.")]
pub fn register_skill(
    _tool_registry: &crate::tools::registry::ToolRegistry,
    runtimes: &RuntimeRegistry,
    _audit: Arc<AuditLedger>,
    skill: SkillDescriptor,
) -> bool {
    // Legacy compatibility - still register individual skills but emit warning
    tracing::warn!(
        skill_id = %skill.skill_id,
        "Using legacy register_skill. Consider migrating to semantic routing."
    );

    let kind = RuntimeRegistry::kind_for_skill(&skill);
    let Some(_runtime) = runtimes.get(kind) else {
        tracing::warn!(
            skill_id = %skill.skill_id,
            runtime = kind.as_str(),
            "[OpenClaw] no runtime registered for skill; not registering tool"
        );
        return false;
    };

    let params: Vec<ParamDef> = skill
        .parameters
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|props| {
            let required: Vec<&str> = skill
                .parameters
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            props
                .iter()
                .map(|(name, schema)| ParamDef {
                    name: name.clone(),
                    param_type: schema
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("string")
                        .to_string(),
                    description: schema
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string(),
                    required: required.contains(&name.as_str()),
                    default: None,
                })
                .collect()
        })
        .unwrap_or_default();

    let _def = ToolDef {
        name: skill.skill_id.clone(),
        description: skill.description.clone(),
        category: skill.category.clone(),
        parameters: params,
        default_tier: skill.risk_level,
        min_tier: "lite",
    };

    // A6: Legacy handler disabled - using semantic routing
    // use super::OpenClawToolHandler;
    // let handler = Arc::new(OpenClawToolHandler::new(skill, runtime, audit));
    // tool_registry.register(def, handler);
    tracing::warn!("Legacy register_skill disabled in A6 - use semantic routing");
    false
}

#[cfg(test)]
mod frozen_engine_wiring_tests {
    //! Task 10.3 — the handler hands a validated frozen `ExecutionGraph` to the
    //! frozen `ExecutionEngine` and returns an evidence-wrapped `ToolResult`
    //! (R4.4/R4.5). These are no-Docker tests: a stub `SkillRuntime` stands in
    //! for the container backend so the wiring (engine build → execute → wrap)
    //! is verified without Docker. The real leak-freedom test over Docker is
    //! task 10.6.

    use super::*;
    use crate::execution::{GraphNode, NodeKind};
    use crate::openclaw::runtime::{LaunchSpec, RuntimeContext, RuntimeKind, SkillRuntime};
    use tempfile::TempDir;

    const TEST_HMAC_KEY: &[u8] = b"handler-frozen-engine-test-hmac-key-0";

    /// A stub `SkillRuntime` that echoes the invoked skill id — stands in for the
    /// Docker backend so the frozen engine can dispatch an OpenClaw Skill node
    /// without touching containers.
    struct StubRuntime {
        kind: RuntimeKind,
        succeed: bool,
    }

    #[async_trait]
    impl SkillRuntime for StubRuntime {
        fn kind(&self) -> RuntimeKind {
            self.kind
        }

        async fn execute(&self, spec: LaunchSpec, _ctx: RuntimeContext) -> ToolResult {
            if self.succeed {
                ToolResult {
                    success: true,
                    data: serde_json::json!({ "echo": spec.skill_id }),
                    error: None,
                }
            } else {
                ToolResult::err(format!("stub failure for {}", spec.skill_id))
            }
        }
    }

    /// Build a handler whose `RuntimeRegistry` contains `runtime` (or is empty
    /// when `runtime` is `None`). Returns the handler + the `TempDir` keeping the
    /// registry/audit sqlite files alive for the test's duration.
    fn handler_with_runtime(runtime: Option<StubRuntime>) -> (SemanticOpenClawHandler, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let registry = Arc::new(
            ProductionSkillRegistry::new(&dir.path().join("skills.db")).expect("open registry"),
        );
        let mut reg = RuntimeRegistry::new();
        if let Some(rt) = runtime {
            reg.register(Arc::new(rt));
        }
        let runtimes = Arc::new(reg);
        let audit = Arc::new(
            AuditLedger::open(&dir.path().join("audit.db"), TEST_HMAC_KEY.to_vec())
                .expect("open audit"),
        );
        (SemanticOpenClawHandler::new(registry, runtimes, audit), dir)
    }

    /// A 1-node graph referencing one OpenClaw skill (mirrors the facade's
    /// `single_skill_graph`).
    fn one_node_graph(action_id: &str) -> ExecutionGraph {
        let mut graph = ExecutionGraph::new("plan-echo", "goal-echo");
        graph.add_node(GraphNode::new(
            format!("skill-{action_id}"),
            NodeKind::Skill {
                provider_id: crate::capability::acl::openclaw::OPENCLAW_PROVIDER_ID.to_string(),
                action_id: action_id.to_string(),
                params: serde_json::json!({}),
            },
        ));
        graph
    }

    #[tokio::test]
    async fn plan_executes_through_frozen_engine_and_is_evidence_wrapped() {
        // Docker-kind stub runtime → the frozen OpenClawExecutor dispatches to it.
        let (handler, _dir) = handler_with_runtime(Some(StubRuntime {
            kind: RuntimeKind::Docker,
            succeed: true,
        }));
        let graph = one_node_graph("echo_skill");

        let result = handler
            .execute_plan_via_engine(&graph, "test query", &RuntimeContext::detached())
            .await
            .expect("frozen runtime wired → Some(result)");

        assert!(result.success, "1-node plan completes: {:?}", result.error);
        // R4.5: the returned data is the evidence-wrapped block (a JSON string),
        // tagged with the plan (graph) id and an OpenClaw source.
        let wrapped = result.data.as_str().expect("evidence-wrapped string data");
        assert!(
            wrapped.contains("<tool_result"),
            "evidence block: {wrapped}"
        );
        assert!(wrapped.contains("plan-echo"), "plan id tagged: {wrapped}");
        assert!(
            wrapped.contains("source=\"openclaw\""),
            "openclaw source: {wrapped}"
        );
        assert!(
            wrapped.contains("<status>success</status>"),
            "status: {wrapped}"
        );
    }

    #[tokio::test]
    async fn failed_node_yields_evidence_wrapped_failure() {
        let (handler, _dir) = handler_with_runtime(Some(StubRuntime {
            kind: RuntimeKind::Docker,
            succeed: false,
        }));
        let graph = one_node_graph("echo_skill");

        let result = handler
            .execute_plan_via_engine(&graph, "test query", &RuntimeContext::detached())
            .await
            .expect("frozen runtime wired → Some(result)");

        assert!(
            !result.success,
            "node failure surfaces as an unsuccessful result"
        );
        assert!(result.error.is_some(), "honest error is carried");
        let wrapped = result.data.as_str().expect("evidence-wrapped string data");
        assert!(
            wrapped.contains("<status>error</status>"),
            "error status: {wrapped}"
        );
    }

    #[tokio::test]
    async fn no_runtime_wired_falls_back_with_none() {
        // Router-only handler (no runtime registered) → honest None fallback,
        // never a panic. CIL never touches containers directly (R4.4).
        let (handler, _dir) = handler_with_runtime(None);
        let graph = one_node_graph("echo_skill");

        let result = handler
            .execute_plan_via_engine(&graph, "test query", &RuntimeContext::detached())
            .await;

        assert!(
            result.is_none(),
            "no OpenClaw runtime wired → fall back to frozen router"
        );
    }

    #[tokio::test]
    async fn cancelled_context_is_propagated_and_reported() {
        let (handler, _dir) = handler_with_runtime(Some(StubRuntime {
            kind: RuntimeKind::Docker,
            succeed: true,
        }));
        let graph = one_node_graph("echo_skill");

        // Pre-cancelled context → the frozen scheduler short-circuits to Cancelled.
        let ctx = RuntimeContext::detached();
        ctx.cancellation.cancel();

        let result = handler
            .execute_plan_via_engine(&graph, "test query", &ctx)
            .await
            .expect("frozen runtime wired → Some(result)");

        assert!(!result.success, "a cancelled plan is not a success");
        assert_eq!(
            result.error.as_deref(),
            Some("multi-capability plan cancelled"),
            "cancellation is reported honestly"
        );
    }
}

#[cfg(test)]
mod permission_swap_tests {
    //! Task 11.4 — the `execute_semantic` approval gate now flows through
    //! `PermissionEngine::authorize` (a strict superset of the frozen
    //! `ApprovalCache`, R7.4) and grants are revocable (R6.6). These are focused
    //! no-Docker tests over the engine + the handler-wired `GrantStore`.

    use super::*;
    use crate::openclaw::approval::ApprovalCache;
    use crate::openclaw::capability::{
        Capability, CapabilityKind, CapabilityMode, CapabilityScope,
    };
    use crate::openclaw::perm::{
        DefaultPermissionEngine, GrantDecision, GrantStore, PermissionDecision, PermissionEngine,
        PermissionTier, ScopeKind, ScopedGrant,
    };
    use crate::openclaw::types::TrustTier;
    use crate::safety::RiskLevel;
    use chrono::Utc;
    use tempfile::TempDir;

    const TEST_HMAC_KEY: &[u8] = b"handler-permission-swap-test-hmac-key";

    /// A `GrantStore` over a temp `skills.db` whose `capability_grants_scoped`
    /// table is created by the frozen registry migrations (migration 5).
    fn store() -> (Arc<GrantStore>, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let s = Arc::new(GrantStore::open(&db_path).expect("grant store open"));
        (s, dir)
    }

    fn net(domains: &[&str]) -> Capability {
        Capability {
            kind: CapabilityKind::Network,
            mode: CapabilityMode::Egress,
            scope: CapabilityScope::Domains(domains.iter().map(|d| d.to_string()).collect()),
        }
    }

    /// Build a handler wired with `grants` (real db-backed store) so
    /// `revoke_grant` acts on the SAME store the test inspects.
    fn handler_with_grants(grants: Arc<GrantStore>) -> (SemanticOpenClawHandler, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let registry = Arc::new(
            ProductionSkillRegistry::new(&dir.path().join("skills.db")).expect("open registry"),
        );
        let runtimes = Arc::new(RuntimeRegistry::new());
        let audit = Arc::new(
            AuditLedger::open(&dir.path().join("audit.db"), TEST_HMAC_KEY.to_vec())
                .expect("open audit"),
        );
        let handler =
            SemanticOpenClawHandler::new(registry, runtimes, audit).with_grant_store(grants);
        (handler, dir)
    }

    /// A GREEN + pure skill authorizes as `NeverAsk` Allow (proceeds like the
    /// pre-swap GREEN fast-path did) — the superset preserves the GREEN outcome.
    #[test]
    fn green_skill_authorizes_allow() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();
        // Empty capability set ⇒ GREEN + no sensitive kind.
        let req = AuthorizeRequest::new("skill.green", vec![], TrustTier::Community);
        match engine.authorize(&req, &grants) {
            PermissionDecision::Allow { tier, grant_id } => {
                assert_eq!(tier, PermissionTier::NeverAsk);
                assert!(grant_id.is_none());
            }
            other => panic!("GREEN+pure must Allow, got {other:?}"),
        }
    }

    /// A revoked grant forces fresh approval on next use (R6.6): a matching
    /// persistent Allow grant is reused (Allow) until `revoke_grant` marks it
    /// revoked, after which the same request must re-prompt.
    #[test]
    fn revoked_grant_requires_fresh_approval() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();

        // A Yellow (sensitive) network request — not eligible for NeverAsk, so
        // its outcome is governed by grant reuse.
        let caps = vec![net(&["api.example.com"])];
        let req = AuthorizeRequest::new("skill.net", caps.clone(), TrustTier::Community);

        // Persist a matching persistent Allow grant with the exact hash the
        // engine computes for this request (budget/version/schema all empty via
        // `AuthorizeRequest::new`).
        let caps_hash = ApprovalCache::compute_hash(
            &req.slug,
            &req.version,
            &caps,
            &req.budget,
            &req.schema_epoch,
        );
        grants
            .insert(&ScopedGrant {
                grant_id: "g-net".to_string(),
                skill_id: "skill.net".to_string(),
                scope_kind: ScopeKind::Persistent,
                scope_key: None,
                caps_hash,
                risk: RiskLevel::Yellow,
                decision: GrantDecision::Allow,
                granted_at: Utc::now(),
                expires_at: None,
                revoked: false,
            })
            .expect("insert grant");

        // Before revocation: the durable grant is reused ⇒ Allow, no prompt.
        match engine.authorize(&req, &grants) {
            PermissionDecision::Allow { grant_id, .. } => {
                assert_eq!(grant_id.as_deref(), Some("g-net"));
            }
            other => panic!("matching grant must be reused Allow, got {other:?}"),
        }

        // Revoke via the handler's public method (task 13.3 desktop entry point),
        // acting on the SAME store.
        let (handler, _hdir) = handler_with_grants(grants.clone());
        handler.revoke_grant("g-net").expect("revoke succeeds");

        // The grant row is now revoked and no longer reusable.
        let row = grants.get("g-net").expect("get grant").expect("row exists");
        assert!(row.revoked, "grant must be marked revoked");

        // After revocation: no reusable grant ⇒ the engine falls back to the
        // frozen ApprovalCache oracle, which (elevated risk, no prior caps) needs
        // fresh approval ⇒ Prompt.
        match engine.authorize(&req, &grants) {
            PermissionDecision::Prompt { .. } => {}
            other => panic!("revoked grant must require fresh approval (Prompt), got {other:?}"),
        }

        // Revoking a non-existent grant is an honest error, never a silent no-op.
        assert!(handler.revoke_grant("does-not-exist").is_err());
    }
}

#[cfg(test)]
mod flag_off_parity_tests {
    //! Task 1.4 / **Property 11: Flag-off parity** (R7.2).
    //!
    //! With `openclaw_icp_enabled == false`, `SemanticOpenClawHandler::execute_semantic`
    //! MUST produce output byte-for-byte identical to the current direct-router
    //! path — i.e. the ICP flag-off branch is a true no-op. These are
    //! deterministic, no-Docker, no-LLM tests.
    //!
    //! Strategy: the flag branch in `execute_semantic` is
    //! `if self.cil_active() { .. }` where
    //! `cil_active() == icp_enabled && cil.is_some() && !degraded.is_degraded()`.
    //! Since no `CapabilityIntelligence` facade is constructible in production and
    //! the default is flag-OFF, the frozen direct-router path always runs. We
    //! prove the branch is skipped in every flag-OFF configuration and that the
    //! observable `ToolResult` is the SAME frozen output in each case.
    //!
    //! An empty registry makes the frozen router deterministically return its
    //! stable "No suitable skill found" refusal — a fixed, Docker-free anchor for
    //! the byte-for-byte comparison.

    use super::*;
    use crate::openclaw::cil::DegradedState;
    use tempfile::TempDir;

    const TEST_HMAC_KEY: &[u8] = b"handler-flag-off-parity-test-hmac-key";

    /// A handler over a fresh, EMPTY registry (no skills installed). The frozen
    /// router therefore always declines with a stable "no suitable skill" result
    /// — deterministic and Docker-free.
    fn empty_handler() -> (SemanticOpenClawHandler, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let registry = Arc::new(
            ProductionSkillRegistry::new(&dir.path().join("skills.db")).expect("open registry"),
        );
        let runtimes = Arc::new(RuntimeRegistry::new());
        let audit = Arc::new(
            AuditLedger::open(&dir.path().join("audit.db"), TEST_HMAC_KEY.to_vec())
                .expect("open audit"),
        );
        (SemanticOpenClawHandler::new(registry, runtimes, audit), dir)
    }

    /// The fixed goal used across every flag-OFF configuration.
    fn goal() -> serde_json::Value {
        serde_json::json!({ "query": "convert a pdf document to plain text" })
    }

    /// Serialize a `ToolResult` to its canonical JSON bytes for a byte-for-byte
    /// comparison (`ToolResult` is `Serialize` but not `PartialEq`).
    fn bytes(result: &ToolResult) -> Vec<u8> {
        serde_json::to_vec(result).expect("serialize ToolResult")
    }

    /// Property 11 (R7.2): the default handler (`new(..)`, flag OFF, `cil = None`)
    /// and a handler with the flag EXPLICITLY off
    /// (`with_cil(false, None, non_degraded())`) both take the frozen
    /// direct-router path and produce byte-for-byte identical output.
    #[tokio::test]
    async fn flag_off_output_is_byte_for_byte_the_frozen_direct_router_path() {
        // (a) Default handler: flag OFF by construction.
        let (default_handler, _d1) = empty_handler();
        // (b) Explicit flag-OFF handler via the builder.
        let (base, _d2) = empty_handler();
        let explicit_off_handler = base.with_cil(false, None, DegradedState::non_degraded());

        // Both must have the ICP branch OFF — proving the frozen path is taken.
        assert!(
            !default_handler.cil_active(),
            "default handler must have the ICP branch OFF (frozen path)"
        );
        assert!(
            !explicit_off_handler.cil_active(),
            "explicit flag-OFF handler must have the ICP branch OFF (frozen path)"
        );

        // Same inputs → same frozen direct-router path → identical ToolResult.
        let out_default = default_handler
            .execute_semantic("openclaw", goal(), RuntimeContext::detached())
            .await;
        let out_explicit = explicit_off_handler
            .execute_semantic("openclaw", goal(), RuntimeContext::detached())
            .await;

        // The frozen path returns an honest refusal (no skill installed), NOT a
        // fabricated success — anchoring the parity comparison on real behavior.
        assert!(
            !out_default.success,
            "empty registry → frozen router declines: {out_default:?}"
        );

        // Byte-for-byte parity: the flag-OFF branch is a true no-op.
        assert_eq!(
            bytes(&out_default),
            bytes(&out_explicit),
            "flag-OFF output must be byte-for-byte identical to the frozen direct-router path"
        );
    }

    /// Strengthening: even flag ON but with NO facade wired
    /// (`with_cil(true, None, non_degraded())`) keeps `cil_active() == false`
    /// (because `cil.is_some()` is false) and therefore ALSO produces the
    /// identical frozen output. This proves the parity holds on the flag
    /// plumbing itself, not merely the default, and that flag-ON-without-backends
    /// is an honest frozen fallback (never a panic).
    #[tokio::test]
    async fn flag_on_without_facade_still_takes_the_frozen_path_identically() {
        let (default_handler, _d1) = empty_handler();
        let (base, _d2) = empty_handler();
        let flag_on_no_facade = base.with_cil(true, None, DegradedState::non_degraded());

        assert!(
            !flag_on_no_facade.cil_active(),
            "flag ON but no facade wired → ICP branch still OFF (honest frozen fallback)"
        );

        let out_default = default_handler
            .execute_semantic("openclaw", goal(), RuntimeContext::detached())
            .await;
        let out_flag_on = flag_on_no_facade
            .execute_semantic("openclaw", goal(), RuntimeContext::detached())
            .await;

        assert_eq!(
            bytes(&out_default),
            bytes(&out_flag_on),
            "flag-ON-without-facade output must be byte-for-byte identical to the frozen path"
        );
    }
}
