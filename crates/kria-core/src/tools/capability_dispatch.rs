//! CPP-backed chat/agent capability dispatcher (Option-A migration, M12).
//!
//! This is THE single execution entry point the agent loop uses for capability
//! requests. It replaces the legacy `SemanticOpenClawHandler` on the chat path:
//! instead of `SemanticSkillRouter` + `ExecutionEngine` + `ApprovalCache`, a chat
//! capability request now flows through the one architecture —
//!
//! ```text
//! query → CapabilityPlatform.discover → descriptor ranking
//!       → capability::permission (effects → tier, durable grants)
//!       → CapabilityPlatform.execute (owning provider adapter) → result
//! ```
//!
//! It is registered as the `openclaw` tool (name kept so the agent's tool
//! contract is unchanged) but is provider-neutral: it dispatches to whatever
//! provider owns the best-matching capability (OpenClaw, MCP, future providers).
//! There is exactly ONE permission engine ([`DefaultPermissionEngine`]) and ONE
//! grant store ([`GrantStore`]) — the same ones the desktop Capabilities panel
//! uses — so a grant approved anywhere is honored here (no repeated prompts).
//!
//! Argument generation (natural language → the capability's typed
//! `input_schema`) reuses the neutral schema-driven [`arg_gen`] helper.

use std::sync::Arc;

use async_trait::async_trait;

use crate::capability::grants::GrantStore;
use crate::capability::intelligence::arg_gen;
use crate::capability::permission::{
    AuthorizeRequest, DefaultPermissionEngine, PermissionDecision, PermissionEngine,
};
use crate::capability::platform::CapabilityPlatform;
use crate::capability::provider::{CapabilityOutcome, CapabilityRequest, RequestContext};
use crate::infra::isolation::ToolResult;
use crate::tools::registry::ToolHandler;

/// The single CPP dispatcher behind the `openclaw` chat tool.
pub struct CapabilityDispatchHandler {
    /// The provider-neutral composition root (discovery + execution).
    platform: Arc<CapabilityPlatform>,
    /// The one durable grant store (shared with the desktop Capabilities panel).
    grants: Arc<GrantStore>,
    /// The one permission engine (descriptor-effects → tier → grant reuse).
    engine: DefaultPermissionEngine,
    /// Model router for natural-language → typed-argument generation. `None` in
    /// tests / when no LLM is reachable — then a capability that needs typed args
    /// is honestly declined rather than fed fabricated input.
    arg_llm: Option<Arc<crate::llm::ModelRouter>>,
    /// How many top capabilities to consider for a query.
    discover_k: usize,
    /// Confidence-based selector (P2). Used only when [`Self::use_reasoner`] is
    /// set, so flag-off is byte-identical to the legacy overlap ranking.
    selector: crate::capability::intelligence::DefaultCapabilitySelector,
    /// Whether to route selection through the confidence selector (spec R3).
    /// Gated by the `capability.intelligence.reasoner` flag at construction.
    use_reasoner: bool,
}

impl CapabilityDispatchHandler {
    /// Build the dispatcher over the shared platform + grant store.
    pub fn new(platform: Arc<CapabilityPlatform>, grants: Arc<GrantStore>) -> Self {
        Self {
            platform,
            grants,
            engine: DefaultPermissionEngine,
            arg_llm: None,
            discover_k: 5,
            selector:
                crate::capability::intelligence::DefaultCapabilitySelector::with_default_policy(),
            use_reasoner: false,
        }
    }

    /// Attach the model router used for schema-driven argument generation.
    pub fn with_arg_llm(mut self, router: Arc<crate::llm::ModelRouter>) -> Self {
        self.arg_llm = Some(router);
        self
    }

    /// Enable confidence-based selection (P2, spec R3). When enabled, candidate
    /// choice + the native-sufficiency gate come from the [`DefaultCapabilitySelector`]
    /// (consulting the CKB); when disabled, the legacy overlap ranking is used
    /// (flag-off parity).
    pub fn with_reasoner(mut self, enabled: bool) -> Self {
        self.use_reasoner = enabled;
        self
    }

    /// Resolve the capability's typed arguments from the tool params:
    /// 1. if the schema needs no args → `{}`;
    /// 2. if the params already satisfy the schema → use them as-is;
    /// 3. otherwise derive them from the natural-language `query` via the LLM.
    async fn resolve_args(
        &self,
        descriptor: &crate::capability::descriptor::CapabilityDescriptor,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let schema = &descriptor.input_schema;
        if !arg_gen::schema_expects_arguments(schema) {
            return Ok(serde_json::json!({}));
        }
        // Caller-supplied params that already satisfy the schema win (no LLM cost).
        if arg_gen::validate_against_schema(params, schema).is_ok() {
            return Ok(params.clone());
        }
        let query = params
            .get("query")
            .or_else(|| params.get("description"))
            .or_else(|| params.get("task"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| params.to_string());

        let Some(router) = self.arg_llm.as_ref() else {
            return Err(format!(
                "capability '{}' needs typed arguments but no LLM is available to derive them",
                descriptor.capability_id
            ));
        };
        let Some(backend) = router.route("openclaw_argument_generation").await else {
            return Err("argument generation needs an LLM backend, but none is reachable".into());
        };
        arg_gen::generate_arguments(
            backend.as_ref(),
            &descriptor.capability_id,
            &descriptor.description,
            schema,
            &query,
            3,
        )
        .await
    }
}

#[async_trait]
impl ToolHandler for CapabilityDispatchHandler {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let query = params
            .get("query")
            .or_else(|| params.get("description"))
            .or_else(|| params.get("task"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if query.is_empty() {
            return ToolResult::err("no query provided to the capability dispatcher");
        }

        // 1) Discover the best-matching capability across all providers, then
        //    re-rank by descriptor-token overlap with the query and apply a
        //    relevance floor. This makes routing robust even when the embedding
        //    backend is the deterministic hash fallback (no ONNX model), whose
        //    semantic signal is noisy — the lexical overlap of the query against
        //    the capability's id/name/tags/description then decides, and an
        //    irrelevant query (no overlap + low score) honestly declines instead
        //    of executing an unrelated capability.
        let hits = match self.platform.discover(&query, self.discover_k) {
            Ok(h) => h,
            Err(e) => return ToolResult::err(format!("capability discovery failed: {e}")),
        };
        let q_tokens = tokenize(&query);
        // Honest-miss floor (kept in BOTH paths): require some descriptor-token
        // overlap OR a non-trivial fused score, so an irrelevant query declines
        // instead of mis-routing — robust even under a degraded (hash) embedder.
        let best_overlap = hits
            .iter()
            .map(|h| descriptor_overlap(&q_tokens, &h.descriptor))
            .fold(0.0f32, f32::max);
        let best_score = hits.iter().map(|h| h.score).fold(0.0f32, f32::max);
        let honest_miss = hits.is_empty() || (best_overlap <= 0.0 && best_score < 0.15);
        if honest_miss {
            return ToolResult::ok_text(format!(
                "No installed capability matches: '{query}'. Try the Marketplace to install one."
            ));
        }

        let descriptor = if self.use_reasoner {
            // P2: confidence-based selection + native-sufficiency gate (spec R3),
            // consulting the CKB learned-success signal.
            let selection = self.selector.select(&hits, self.platform.knowledge()).await;

            // Explainability (spec R16): persist a Decision Record so "why X / why
            // not Y" is answerable from durable state, not model recall.
            if let Some(ckb) = self.platform.knowledge() {
                let candidates: Vec<(String, String, f32)> = selection
                    .candidates
                    .iter()
                    .map(|c| {
                        (
                            c.descriptor.provider_id.clone(),
                            c.descriptor.capability_id.clone(),
                            c.confidence,
                        )
                    })
                    .collect();
                let rejected: Vec<(String, String, String)> = selection
                    .candidates
                    .iter()
                    .filter(|c| {
                        selection
                            .chosen
                            .as_ref()
                            .map(|(p, i)| {
                                &c.descriptor.provider_id != p || &c.descriptor.capability_id != i
                            })
                            .unwrap_or(true)
                    })
                    .map(|c| {
                        (
                            c.descriptor.provider_id.clone(),
                            c.descriptor.capability_id.clone(),
                            format!("lower confidence {:.2}", c.confidence),
                        )
                    })
                    .collect();
                let record = crate::capability::intelligence::DecisionRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    goal: query.clone(),
                    goal_class: crate::capability::intelligence::GoalClass::Other(
                        "unclassified".into(),
                    ),
                    candidates,
                    chosen: selection.chosen.clone(),
                    rejected,
                    path: selection.path.clone(),
                    confidence: selection.confidence,
                    policy_version: selection.policy_version,
                    created_at: chrono::Utc::now().to_rfc3339(),
                };
                let _ = ckb.record_decision(&record).await;
            }

            use crate::capability::intelligence::ExecutionPath;
            match (&selection.path, &selection.chosen) {
                (ExecutionPath::Ask, _) | (_, None) => {
                    return ToolResult::ok_text(format!(
                        "I'm not confident which capability fits '{query}' ({}). \
                         Could you clarify, or try the Marketplace?",
                        selection.rationale
                    ));
                }
                (ExecutionPath::Acquire, _) => {
                    return ToolResult::ok_text(format!(
                        "No installed capability is sufficient for '{query}'. \
                         Search the Marketplace to install one. ({})",
                        selection.rationale
                    ));
                }
                (_, Some((pid, cid))) => {
                    match hits.iter().find(|h| {
                        &h.descriptor.provider_id == pid && &h.descriptor.capability_id == cid
                    }) {
                        Some(h) => h.descriptor.clone(),
                        None => hits[0].descriptor.clone(),
                    }
                }
            }
        } else {
            // Legacy overlap ranking (flag-off parity): lexical overlap dominates
            // (robust under a degraded embedder), fused score as tiebreak.
            let mut best: Option<(f32, crate::capability::ScoredDescriptor)> = None;
            for h in hits {
                let overlap = descriptor_overlap(&q_tokens, &h.descriptor);
                let route = overlap * 2.0 + h.score;
                if best.as_ref().map(|(r, _)| route > *r).unwrap_or(true) {
                    best = Some((route, h));
                }
            }
            best.map(|(_, h)| h.descriptor)
                .unwrap_or_else(|| unreachable!("honest_miss guard ensures at least one hit"))
        };

        // 2) Permission gate — the ONE engine + ONE grant store. Chat has no
        //    session id; scope grants to the workspace so a single approval is
        //    remembered across the whole session (no repeated prompts).
        let auth =
            AuthorizeRequest::from_descriptor(&descriptor, None, Some("default".to_string()));
        match self.engine.authorize(&auth, &self.grants) {
            PermissionDecision::Deny { reason } => {
                return ToolResult::err(format!("permission denied: {reason}"));
            }
            PermissionDecision::Prompt { prompt, .. } => {
                // No inline modal on the tool path yet: surface an honest,
                // actionable message. Once approved in the Capabilities panel the
                // durable grant makes subsequent calls run without prompting.
                return ToolResult::err(format!(
                    "'{}' requires approval (effects: {}). Approve it once in the Capabilities → \
                     Approval Center, then retry.",
                    descriptor.name,
                    if prompt.effects.is_empty() {
                        descriptor.effects.classes.join(", ")
                    } else {
                        prompt.effects.join(", ")
                    }
                ));
            }
            PermissionDecision::Allow { .. } => {}
        }

        // 3) Resolve typed args + execute through the owning provider adapter.
        let args = match self.resolve_args(&descriptor, &params).await {
            Ok(a) => a,
            Err(e) => return ToolResult::err(e),
        };
        let req = CapabilityRequest {
            provider_id: descriptor.provider_id.clone(),
            capability_id: descriptor.capability_id.clone(),
            args,
            context: RequestContext::new(),
            granted_effects: descriptor.effects.classes.clone(),
        };
        match self.platform.execute(req).await {
            Ok(CapabilityOutcome::Value(v)) => ToolResult::ok(v),
            Ok(CapabilityOutcome::Declined { reason }) => {
                ToolResult::err(format!("capability declined: {reason}"))
            }
            Ok(CapabilityOutcome::Stream(_)) => {
                ToolResult::ok_text("capability produced a stream (surfaced via the timeline)")
            }
            Err(e) => ToolResult::err(format!("capability execution failed: {e}")),
        }
    }
}

/// Lowercase alphanumeric tokens of a string (shared by the routing re-rank).
fn tokenize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1)
        .map(|t| t.to_string())
        .collect()
}

/// Fraction of the capability's identity tokens (capability_id + name + tags)
/// that appear in the query. High when the query names what the capability does
/// (e.g. "minify this json" ↔ `oc_json_tool` / "Json Tool"), so routing is
/// robust even when the embedding backend is degraded to the hash fallback.
fn descriptor_overlap(
    q_tokens: &[String],
    d: &crate::capability::descriptor::CapabilityDescriptor,
) -> f32 {
    let mut cap_tokens = tokenize(&d.capability_id);
    cap_tokens.extend(tokenize(&d.name));
    for t in &d.tags {
        cap_tokens.extend(tokenize(&t.id));
    }
    // Drop generic noise tokens that don't discriminate between skills.
    cap_tokens.retain(|t| !matches!(t.as_str(), "oc" | "tool" | "skill" | "openclaw"));
    if cap_tokens.is_empty() {
        return 0.0;
    }
    // A capability token matches a query token on equality or a shared prefix
    // of length >= 4 (so "hash" ↔ "hashing", "compress" ↔ "compression").
    let matched = cap_tokens
        .iter()
        .filter(|c| {
            q_tokens.iter().any(|q| {
                q == *c
                    || (c.len() >= 4
                        && q.len() >= 4
                        && (q.starts_with(c.as_str()) || c.starts_with(q)))
            })
        })
        .count();
    matched as f32 / cap_tokens.len() as f32
}

/// CPP-backed `list_installed_skills` tool — replaces the legacy
/// `ListInstalledSkills`. Lists the capabilities every provider currently
/// describes (i.e. installed + available) via the one platform. Provider-neutral:
/// OpenClaw, MCP, and any future provider's installed capabilities appear here.
pub struct CapabilityListHandler {
    platform: Arc<CapabilityPlatform>,
}

impl CapabilityListHandler {
    pub fn new(platform: Arc<CapabilityPlatform>) -> Self {
        Self { platform }
    }
}

#[async_trait]
impl ToolHandler for CapabilityListHandler {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        // `filter`: all|enabled|disabled (default all). The platform only
        // describes installed/available capabilities, so "disabled" is empty.
        let filter = params
            .get("filter")
            .and_then(|v| v.as_str())
            .unwrap_or("all");
        if filter == "disabled" {
            return ToolResult::ok(serde_json::json!({ "skills": [], "count": 0 }));
        }
        let hits = match self.platform.discover("", 10_000) {
            Ok(h) => h,
            Err(e) => return ToolResult::err(format!("capability listing failed: {e}")),
        };
        let skills: Vec<serde_json::Value> = hits
            .into_iter()
            .map(|s| {
                let d = s.descriptor;
                serde_json::json!({
                    "provider_id": d.provider_id,
                    "capability_id": d.capability_id,
                    "name": d.name,
                    "description": d.description,
                    "elevated": d.effects.is_elevated(),
                    "tags": d.tags.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
                })
            })
            .collect();
        let count = skills.len();
        ToolResult::ok(serde_json::json!({ "skills": skills, "count": count }))
    }
}

/// CPP-backed `search_marketplace` tool — the agent's provider-neutral view of
/// the marketplace (installable-but-not-yet-installed capabilities across every
/// provider's catalog). This is what a natural request like "search the
/// marketplace for a PDF extractor" resolves to — NOT the OS package manager and
/// NOT the installed-skills list. Returns ranked remote candidates the user (or
/// the agent) can then install with `install_capability`.
pub struct MarketplaceSearchHandler {
    platform: Arc<CapabilityPlatform>,
    k: usize,
}

impl MarketplaceSearchHandler {
    pub fn new(platform: Arc<CapabilityPlatform>) -> Self {
        Self { platform, k: 8 }
    }
}

#[async_trait]
impl ToolHandler for MarketplaceSearchHandler {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let query = params
            .get("query")
            .or_else(|| params.get("description"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if query.is_empty() {
            return ToolResult::err("no query provided for marketplace search");
        }
        match self.platform.recommend(&query, self.k).await {
            Ok(hits) => {
                let results: Vec<serde_json::Value> = hits
                    .into_iter()
                    .map(|h| {
                        let d = h.descriptor;
                        serde_json::json!({
                            "provider_id": d.provider_id,
                            "capability_id": d.capability_id,
                            "name": d.name,
                            "description": d.description,
                            "score": h.score,
                            "tags": d.tags.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                if results.is_empty() {
                    return ToolResult::ok_text(format!(
                        "No marketplace capability matches '{query}'."
                    ));
                }
                let count = results.len();
                ToolResult::ok(serde_json::json!({
                    "query": query,
                    "results": results,
                    "count": count,
                }))
            }
            Err(e) => ToolResult::err(format!("marketplace search failed: {e}")),
        }
    }
}

/// CPP-backed `install_capability` tool — installs the best marketplace match
/// for a natural-language goal via the owning provider's lifecycle facet, then
/// refreshes discovery so the capability is immediately usable. No hardcoded
/// skill names: the user says what they want ("install a PDF extractor",
/// "install a zip compressor") and the provider resolves the best match.
pub struct MarketplaceInstallHandler {
    platform: Arc<CapabilityPlatform>,
}

impl MarketplaceInstallHandler {
    pub fn new(platform: Arc<CapabilityPlatform>) -> Self {
        Self { platform }
    }
}

#[async_trait]
impl ToolHandler for MarketplaceInstallHandler {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let goal = params
            .get("query")
            .or_else(|| params.get("description"))
            .or_else(|| params.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if goal.is_empty() {
            return ToolResult::err("no capability described to install");
        }
        match self.platform.acquire_for_goal(&goal).await {
            Ok(d) => ToolResult::ok(serde_json::json!({
                "installed": true,
                "provider_id": d.provider_id,
                "capability_id": d.capability_id,
                "name": d.name,
                "description": d.description,
                "message": format!(
                    "Installed '{}'. It is now available — ask me to use it.",
                    d.name
                ),
            })),
            Err(e) => ToolResult::err(format!("could not install a capability for '{goal}': {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::{CapabilityDescriptor, Effects};
    use crate::capability::index::{InMemoryFederatedIndex, MemoryEmbedder};
    use crate::capability::protocol::{
        ClientCapabilities, FeatureSet, ProtocolSession, ProtocolVersion, ProviderHealth,
    };
    use crate::capability::provider::CapabilityProvider;
    use crate::capability::registry::ProviderRegistry;
    use crate::capability::CapError;
    use crate::capability::ProviderId;

    /// A tiny in-test provider that echoes a fixed value — proves the dispatcher
    /// wiring (discover → permission → execute) without Docker.
    struct EchoProvider {
        id: ProviderId,
    }

    #[async_trait]
    impl CapabilityProvider for EchoProvider {
        fn provider_id(&self) -> &ProviderId {
            &self.id
        }
        async fn negotiate(
            &self,
            client: &ClientCapabilities,
        ) -> Result<ProtocolSession, CapError> {
            Ok(client.negotiate(
                self.id.clone(),
                ProtocolVersion::CURRENT,
                FeatureSet::mandatory(),
                serde_json::Map::new(),
            ))
        }
        async fn describe(
            &self,
            _s: &ProtocolSession,
        ) -> Result<Vec<CapabilityDescriptor>, CapError> {
            let mut d = CapabilityDescriptor::minimal(
                self.id.clone(),
                "echo_upper",
                "Echo Upper",
                "Converts the given text to upper case.",
                serde_json::json!({
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"]
                }),
            );
            // GREEN, reversible, no elevated effects → NeverAsk (no prompt).
            d.effects = Effects {
                classes: vec![],
                reversible: crate::capability::descriptor::Reversibility::Reversible,
                idempotent: true,
                resource_class: Default::default(),
            };
            Ok(vec![d])
        }
        async fn execute(&self, req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
            let text = req
                .args
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            Ok(CapabilityOutcome::Value(serde_json::json!({
                "output": text.to_uppercase()
            })))
        }
        async fn health(&self) -> ProviderHealth {
            ProviderHealth::Ready
        }
    }

    #[tokio::test]
    async fn dispatcher_green_capability_runs_without_prompt() {
        let embedder = Arc::new(MemoryEmbedder::load().unwrap());
        let index = Arc::new(InMemoryFederatedIndex::new(embedder));
        let registry = Arc::new(ProviderRegistry::new(index));
        registry.register(Arc::new(EchoProvider {
            id: "test".to_string(),
        }));
        let platform = Arc::new(CapabilityPlatform::new(registry));
        platform.refresh().await;
        let grants = Arc::new(GrantStore::in_memory().unwrap());
        let handler = CapabilityDispatchHandler::new(platform, grants);

        // Params already satisfy the schema → no LLM needed.
        let out = handler
            .execute(serde_json::json!({ "query": "upper case hello", "text": "hello" }))
            .await;
        assert!(out.success, "expected success, got {out:?}");
        assert_eq!(
            out.data.get("output").and_then(|v| v.as_str()),
            Some("HELLO")
        );
    }

    #[tokio::test]
    async fn dispatcher_no_match_is_honest() {
        let embedder = Arc::new(MemoryEmbedder::load().unwrap());
        let index = Arc::new(InMemoryFederatedIndex::new(embedder));
        let registry = Arc::new(ProviderRegistry::new(index));
        let platform = Arc::new(CapabilityPlatform::new(registry));
        platform.refresh().await;
        let grants = Arc::new(GrantStore::in_memory().unwrap());
        let handler = CapabilityDispatchHandler::new(platform, grants);
        let out = handler
            .execute(serde_json::json!({ "query": "do a thing" }))
            .await;
        assert!(out.success);
        assert!(out
            .data
            .as_str()
            .unwrap_or_default()
            .contains("No installed capability"));
    }
}
