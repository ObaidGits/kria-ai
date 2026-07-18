//! Desktop command surface for the Capability Provider Platform (CPP).
//!
//! Provider-neutral Tauri commands backing the first-class Capabilities area:
//! provider list + health, cross-provider discovery, the capability catalog,
//! marketplace recommendations, the Descriptor Viewer, the durable permission
//! grant list + revoke, the live approval flow (authorize → approve → execute),
//! and the observability timeline / runtime monitor.
//!
//! They build a single [`CapabilityPlatform`] from the live app state (the
//! OpenClaw provider from the skill registry + container pool, and any
//! config-declared MCP providers) plus a durable [`GrantStore`], a
//! [`DefaultPermissionEngine`], and a [`CapabilityEventBus`] with a bounded
//! in-process ring buffer that feeds the Timeline / Runtime Monitor / Recovery
//! surfaces. All of this is cached process-globally and built lazily on first
//! CPP command so boot stays fast and Docker-free until a Capabilities surface
//! is opened.
//!
//! These are ADDITIVE commands (new names, `cpp_*`); no existing OpenClaw command
//! is renamed or removed. The platform is provider-neutral — this module contains
//! no provider-specific branching beyond adapter *construction*, which is
//! inherently where an adapter is chosen from config.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use kria_core::capability::acl::mcp::McpProvider;
use kria_core::capability::acl::openclaw::OpenClawProvider;
use kria_core::capability::events::{CapabilityEvent, CapabilityEventBus};
use kria_core::capability::grants::{GrantDecision, GrantStore, ScopeKind};
use kria_core::capability::index::{InMemoryFederatedIndex, MemoryEmbedder};
use kria_core::capability::intelligence::DefaultLifecycleManager;
use kria_core::capability::permission::{
    approval_grant, AuthorizeRequest, DefaultPermissionEngine, PermissionDecision,
    PermissionEngine, PermissionTier,
};
use kria_core::capability::platform::CapabilityPlatform;
use kria_core::capability::provider::{CapabilityOutcome, CapabilityRequest, RequestContext};
use kria_core::capability::registry::ProviderRegistry;
use kria_core::openclaw::runtime::{DockerRuntime, SkillRuntime};
use serde::Serialize;
use tauri::{command, AppHandle, State};
use tokio::sync::OnceCell;

use crate::commands::app_state::AppStateCell;

/// How many recent capability events the in-process ring buffer retains for the
/// Timeline / Runtime Monitor. Bounded so a long-running desktop never grows
/// unbounded; the durable record remains the `AuditLedger`/tracing.
const EVENT_RING_CAP: usize = 2_000;

/// Everything the CPP desktop surface needs, built once and cached.
struct CppState {
    /// The provider-neutral composition root (discovery + execution).
    platform: Arc<CapabilityPlatform>,
    /// Durable, scoped, revocable permission grants (M4).
    grants: Arc<GrantStore>,
    /// The descriptor-effects permission engine (M4).
    engine: DefaultPermissionEngine,
    /// Bounded snapshot of recent events for the Timeline/Runtime Monitor. The
    /// live [`CapabilityEventBus`] itself is kept alive by the platform (which
    /// holds a clone); a background task drains it into this ring.
    ring: Arc<Mutex<VecDeque<CapabilityEvent>>>,
}

/// Process-global cached CPP state (one desktop app instance).
static CPP: OnceCell<Arc<CppState>> = OnceCell::const_new();

/// Process-global continuous-discovery engine handle (Wave 10). Spawned once
/// (at boot by the runtime, or lazily by the first capability command) so there
/// is exactly ONE background loop, and the status/scan commands read it.
static DISCOVERY: std::sync::OnceLock<
    Arc<kria_core::capability::intelligence::ContinuousDiscoveryEngine>,
> = std::sync::OnceLock::new();

/// The live discovery engine, if the loop has been spawned.
pub(crate) fn discovery_engine(
) -> Option<Arc<kria_core::capability::intelligence::ContinuousDiscoveryEngine>> {
    DISCOVERY.get().cloned()
}

/// Process-global job manager (Wave 11). One durable job lifecycle for the app.
static JOBS: std::sync::OnceLock<Arc<kria_core::capability::intelligence::JobManager>> =
    std::sync::OnceLock::new();

/// The live job manager, if wired.
pub(crate) fn job_manager() -> Option<Arc<kria_core::capability::intelligence::JobManager>> {
    JOBS.get().cloned()
}

/// Wire the durable job manager over `platform` + `store` exactly once
/// (idempotent) and resume any jobs left active by a prior run (restart
/// recovery, spec R28.1). No-op if already wired.
pub(crate) fn ensure_jobs_spawned(
    platform: Arc<CapabilityPlatform>,
    store: Arc<dyn kria_core::capability::intelligence::JobStore>,
    max_concurrency: usize,
) {
    use kria_core::capability::intelligence::{JobManager, RetryPolicy};
    if JOBS.get().is_some() {
        return;
    }
    let mgr = Arc::new(JobManager::new(
        platform,
        store,
        RetryPolicy::default(),
        max_concurrency,
    ));
    if JOBS.set(mgr.clone()).is_ok() {
        // Resume active jobs from the durable store (background, non-blocking).
        let resume = mgr.clone();
        tokio::spawn(async move {
            match resume.resume_all().await {
                Ok(ids) if !ids.is_empty() => {
                    tracing::info!(
                        "[CPP] Job manager resumed {} job(s) after restart",
                        ids.len()
                    )
                }
                _ => {}
            }
        });
        tracing::info!("[CPP] Job manager wired (durable resumable jobs, spec R28)");
    }
}

/// Spawn the continuous-discovery loop over `platform` exactly once (idempotent).
/// Reuses the platform's evolution + marketplace + CKB; writes reversible
/// proposals to the oversight feed under `autonomy`. No-op if already spawned.
pub(crate) fn ensure_discovery_spawned(
    platform: Arc<CapabilityPlatform>,
    autonomy: kria_core::capability::intelligence::AutonomyLevel,
) {
    use kria_core::capability::intelligence::{ContinuousDiscoveryEngine, DiscoveryPolicy};
    if DISCOVERY.get().is_some() {
        return;
    }
    let engine = Arc::new(ContinuousDiscoveryEngine::new(
        platform,
        DiscoveryPolicy::default(),
        autonomy,
    ));
    if DISCOVERY.set(engine.clone()).is_ok() {
        engine.spawn();
        tracing::info!(
            "[CPP] Continuous discovery loop spawned (autonomy {})",
            autonomy.as_str()
        );
    }
}

/// Build (once) the whole CPP state from live app state + config.
async fn cpp(state: &State<'_, AppStateCell>) -> Result<Arc<CppState>, String> {
    let app = state.get().ok_or("runtime not ready")?;

    if let Some(s) = CPP.get() {
        return Ok(s.clone());
    }

    // Federated index over the shared embedding backend.
    let embedder = Arc::new(MemoryEmbedder::load().map_err(|e| format!("embedder: {e}"))?);
    let index = Arc::new(InMemoryFederatedIndex::new(embedder));
    let registry = Arc::new(ProviderRegistry::new(index));

    let data_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".kria");
    let _ = std::fs::create_dir_all(&data_dir);

    // OpenClaw provider — only when a container pool exists (Docker available).
    // The LIFECYCLE facet (marketplace acquire/remove) is wired from the live
    // OpenClaw registry config so the desktop can acquire capabilities on a goal
    // miss through the same frozen installer the manual install path uses.
    if let Some(pool) = &app.container_pool {
        let runtime: Arc<dyn SkillRuntime> = Arc::new(DockerRuntime::new(pool.clone()));
        let oc_cfg = app.config.read().await.openclaw.clone();
        let store_dir = data_dir.join("openclaw_skills");
        let _ = std::fs::create_dir_all(&store_dir);
        let mut oc = OpenClawProvider::new(app.skill_registry.clone(), runtime);
        if let Ok(audit) = kria_core::openclaw::audit::AuditLedger::open(
            &data_dir.join("skills.db"),
            b"kria-openclaw-dev-audit-key-0001".to_vec(),
        ) {
            oc = oc.with_lifecycle(
                oc_cfg.registry.index_url.clone(),
                oc_cfg.registry.allowed_hosts.clone(),
                Arc::new(audit),
                store_dir,
            );
        }
        registry.register(Arc::new(oc));
    }

    // Config-declared providers (data-driven; e.g. MCP servers). No hardcoding:
    // the set of providers comes entirely from `[capability].providers`.
    let cap_cfg = app.config.read().await.capability.clone();
    for pc in cap_cfg.providers.iter().filter(|p| p.enabled) {
        if pc.kind == "mcp" {
            let cmd = pc
                .settings
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let args: Vec<String> = pc
                .settings
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if !cmd.is_empty() {
                match McpProvider::connect(pc.id.clone(), &cmd, &args).await {
                    Ok(p) => registry.register(Arc::new(p)),
                    Err(e) => tracing::warn!("[CPP] mcp provider '{}' unavailable: {e}", pc.id),
                }
            }
        }
    }

    // P9: the synthesizing provider (generate a capability when none exists),
    // flag-gated + off critical path (spec R7.4). Lowest trust, audited-primitive
    // bounded. Registered like any provider — the Brain treats generate as acquire.
    if cap_cfg.intelligence.synthesis {
        let syn_store = data_dir.join("cpp_synthesis");
        match kria_core::capability::acl::synthesis::SynthesisProvider::new("synthesis", &syn_store)
        {
            Ok(p) => {
                registry.register(Arc::new(p));
                tracing::info!("[CPP] Synthesis provider wired (generate capabilities, spec R7)");
            }
            Err(e) => tracing::warn!("[CPP] synthesis provider unavailable: {e}"),
        }
    }

    // Observability bus + a bounded ring buffer subscriber that snapshots recent
    // events for the Timeline / Runtime Monitor (the bus itself is lossy/live).
    let events = Arc::new(CapabilityEventBus::new(EVENT_RING_CAP));
    let ring: Arc<Mutex<VecDeque<CapabilityEvent>>> =
        Arc::new(Mutex::new(VecDeque::with_capacity(EVENT_RING_CAP)));
    {
        let mut rx = events.subscribe();
        let ring_bg = ring.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        if let Ok(mut buf) = ring_bg.lock() {
                            if buf.len() == EVENT_RING_CAP {
                                buf.pop_front();
                            }
                            buf.push_back(ev);
                        }
                    }
                    // Lagged: keep draining. Closed: bus dropped, stop.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    let mut platform_builder = CapabilityPlatform::new(registry).with_events(events.clone());
    // P9: enable synthesis fall-through when the synthesis provider is registered.
    if cap_cfg.intelligence.synthesis {
        platform_builder = platform_builder.with_synthesis("synthesis");
        // P9 (W9-R11): LLM-assisted IR proposer, flag-gated. The model only
        // proposes; the validator + golden gate own correctness, and a bad/absent
        // model falls back to the deterministic proposer (never fabricates).
        if cap_cfg.intelligence.synthesis_llm {
            let generator = SynthesisLlmGenerator::new(app.model_router.clone());
            let proposer = Arc::new(
                kria_core::capability::intelligence::LlmIrProposer::new(generator)
                    .with_code(cap_cfg.intelligence.synthesis_code),
            );
            platform_builder = platform_builder.with_ir_proposer(proposer);
            tracing::info!("[CPP] LLM-assisted IR proposer wired (synthesis_llm)");
        }
        // P9 (W9-R13 / BLOCKER 2/3): Tier-3 hardened code sandbox, flag-gated.
        // Generated code runs ONLY inside the seccomp/no-network Docker sandbox.
        if cap_cfg.intelligence.synthesis_code {
            platform_builder = platform_builder.with_code_runner(Arc::new(
                kria_core::capability::acl::code_sandbox::CodeSandbox::default(),
            ));
            tracing::info!("[CPP] Tier-3 code sandbox wired (synthesis_code)");
        }
    }
    // Wire the durable CKB + its EvolutionStore facet so the Capabilities UI's
    // health/proposals surface reads/writes the one learned layer (spec R6/R18).
    // The concrete CKB Arc is also captured for the Wave-11 JobStore.
    let mut jobs_store: Option<
        Arc<kria_core::capability::intelligence::SqliteCapabilityKnowledge>,
    > = None;
    if cap_cfg.intelligence.ckb || cap_cfg.intelligence.evolution {
        match kria_core::capability::intelligence::SqliteCapabilityKnowledge::open(
            &data_dir.join("cpp_knowledge.db"),
        ) {
            Ok(ckb) => {
                let ckb = Arc::new(ckb);
                platform_builder = platform_builder.with_knowledge(ckb.clone());
                if cap_cfg.intelligence.evolution {
                    platform_builder = platform_builder.with_evolution_store(ckb.clone());
                }
                jobs_store = Some(ckb.clone());
            }
            Err(e) => tracing::warn!("[CPP] UI CKB open failed (continuing without): {e}"),
        }
    }
    let platform = Arc::new(platform_builder);
    platform.refresh().await;

    // Wave 11: wire the durable job manager (background, off-by-default). Reuses
    // the platform's reliable execution path + the CKB as the JobStore; resumes
    // any jobs left active by a prior run. Idempotent global.
    if cap_cfg.intelligence.jobs {
        if let Some(store) = &jobs_store {
            ensure_jobs_spawned(platform.clone(), store.clone(), 8);
        } else {
            tracing::warn!("[CPP] jobs enabled but CKB unavailable — job manager not wired");
        }
    }

    // Wave 10: ensure the continuous discovery/maintenance loop is running
    // (background, off-by-default, autonomy-gated). Idempotent — a no-op if the
    // runtime already spawned it at boot. Reuses the platform's evolution +
    // marketplace + CKB; writes proposals to the oversight feed.
    if cap_cfg.intelligence.continuous_discovery {
        let autonomy = kria_core::capability::intelligence::AutonomyLevel::parse(
            &cap_cfg.intelligence.autonomy_level,
        )
        .unwrap_or(kria_core::capability::intelligence::AutonomyLevel::ProposeOnly);
        ensure_discovery_spawned(platform.clone(), autonomy);
    }

    // Durable grant store (shared skills.db companion file, additive table).
    let grants = Arc::new(
        GrantStore::open(&data_dir.join("cpp_grants.db"))
            .map_err(|e| format!("grant store: {e}"))?,
    );

    let cpp_state = Arc::new(CppState {
        platform,
        grants,
        engine: DefaultPermissionEngine,
        ring,
    });
    let _ = CPP.set(cpp_state.clone());
    Ok(cpp_state)
}

// ── View DTOs (frontend contract) ───────────────────────────────────────────

/// CPP platform status for the desktop header/dashboard.
#[derive(Debug, Serialize)]
pub struct CppStatus {
    pub enabled: bool,
    pub provider_count: usize,
    pub healthy_providers: usize,
    pub descriptor_count: usize,
}

/// One provider's live state for the Provider Manager view.
#[derive(Debug, Serialize)]
pub struct CppProviderView {
    pub provider_id: String,
    pub health: String,
    pub state: String,
    pub version: Option<String>,
    pub descriptor_count: usize,
    pub error: Option<String>,
}

/// A ranked/discovered capability for the Capability Browser + discovery.
#[derive(Debug, Serialize)]
pub struct CppCapabilityView {
    pub provider_id: String,
    pub capability_id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub elevated: bool,
    pub score: f32,
}

/// A durable permission grant for the Approval Center grant list.
#[derive(Debug, Serialize)]
pub struct CppGrantView {
    pub grant_id: String,
    pub provider_id: String,
    pub capability_id: String,
    pub scope: String,
    pub scope_key: Option<String>,
    pub effects: Vec<String>,
    pub decision: String,
    pub granted_at: String,
    pub expires_at: Option<String>,
}

/// The result of authorizing a capability (drives the approval modal).
#[derive(Debug, Serialize)]
pub struct CppAuthDecision {
    /// `allow` | `prompt` | `deny`.
    pub kind: String,
    /// Permission tier label (e.g. `never_ask`, `ask_per_session`, `always_ask`).
    pub tier: String,
    /// Effect classes being requested (surfaced in the modal).
    pub effects: Vec<String>,
    /// Coarse risk label (`low`/`medium`/`high`) when a prompt is required.
    pub risk: Option<String>,
    /// Human-readable reason (prompt) or deny reason.
    pub reason: Option<String>,
    /// The grant id that backed an allow, if any.
    pub grant_id: Option<String>,
}

/// The outcome of a gated execute: either an approval is required first, or the
/// capability ran and produced a result.
#[derive(Debug, Serialize)]
pub struct CppExecuteResult {
    /// `ok` | `declined` | `needs_approval` | `denied`.
    pub status: String,
    /// The permission decision when `needs_approval`/`denied`.
    pub decision: Option<CppAuthDecision>,
    /// The JSON result value when `ok`.
    pub value: Option<serde_json::Value>,
    /// A reason string for `declined`/`denied`.
    pub reason: Option<String>,
}

/// One capability event for the Timeline / Runtime Monitor / Recovery views.
#[derive(Debug, Serialize)]
pub struct CppEventView {
    pub correlation_id: String,
    pub provider_id: String,
    pub capability_id: Option<String>,
    pub stage: String,
    pub outcome: String,
    pub detail: String,
    pub timestamp: String,
}

/// The full descriptor for the Descriptor Viewer.
#[derive(Debug, Serialize)]
pub struct CppDescriptorView {
    pub provider_id: String,
    pub capability_id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub schema_version: String,
    pub tags: Vec<String>,
    pub io_modality: Vec<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub effect_classes: Vec<String>,
    pub reversible: String,
    pub idempotent: bool,
    pub elevated: bool,
    pub trust_tier: Option<String>,
    pub signed: bool,
    /// Guidance / expectations serialized as JSON for a rich viewer.
    pub guidance: Option<serde_json::Value>,
    pub expectations: Option<serde_json::Value>,
    pub input_schema: serde_json::Value,
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn to_view(sd: &kria_core::capability::ScoredDescriptor) -> CppCapabilityView {
    let d = &sd.descriptor;
    CppCapabilityView {
        provider_id: d.provider_id.clone(),
        capability_id: d.capability_id.clone(),
        name: d.name.clone(),
        description: d.description.clone(),
        tags: d.tags.iter().map(|t| t.id.clone()).collect(),
        elevated: d.effects.is_elevated(),
        score: sd.score,
    }
}

fn tier_str(t: PermissionTier) -> &'static str {
    match t {
        PermissionTier::NeverAsk => "never_ask",
        PermissionTier::AskOnce => "ask_once",
        PermissionTier::AskPerSession => "ask_per_session",
        PermissionTier::AskPerWorkspace => "ask_per_workspace",
        PermissionTier::Persistent => "persistent",
        PermissionTier::Silent => "silent",
        PermissionTier::AlwaysAsk => "always_ask",
    }
}

fn decision_to_view(d: &PermissionDecision) -> CppAuthDecision {
    match d {
        PermissionDecision::Allow { tier, grant_id } => CppAuthDecision {
            kind: "allow".into(),
            tier: tier_str(*tier).into(),
            effects: Vec::new(),
            risk: None,
            reason: None,
            grant_id: grant_id.clone(),
        },
        PermissionDecision::Prompt { tier, prompt } => CppAuthDecision {
            kind: "prompt".into(),
            tier: tier_str(*tier).into(),
            effects: prompt.effects.clone(),
            risk: Some(prompt.risk.clone()),
            reason: Some(prompt.reason.clone()),
            grant_id: None,
        },
        PermissionDecision::Deny { reason } => CppAuthDecision {
            kind: "deny".into(),
            tier: "deny".into(),
            effects: Vec::new(),
            risk: None,
            reason: Some(reason.clone()),
            grant_id: None,
        },
    }
}

/// Load a descriptor by key or return an actionable error.
async fn require_descriptor(
    cpp: &CppState,
    provider_id: &str,
    capability_id: &str,
) -> Result<kria_core::capability::descriptor::CapabilityDescriptor, String> {
    cpp.platform
        .descriptor(provider_id, capability_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("capability '{provider_id}/{capability_id}' not found"))
}

// ── Discovery / catalog commands (M9 A) ─────────────────────────────────────

/// Status: is the CPP flag on, and how many providers/capabilities are federated.
#[command]
pub async fn cpp_status(state: State<'_, AppStateCell>) -> Result<CppStatus, String> {
    let app = state.get().ok_or("runtime not ready")?;
    let enabled = app.config.read().await.capability.enabled;
    let cpp = cpp(&state).await?;
    let report = cpp.platform.refresh().await;
    Ok(CppStatus {
        enabled,
        provider_count: report.providers.len(),
        healthy_providers: report.healthy_count(),
        descriptor_count: report.total_descriptors,
    })
}

/// The Provider Manager list: every registered provider + its negotiated state.
#[command]
pub async fn cpp_list_providers(
    state: State<'_, AppStateCell>,
) -> Result<Vec<CppProviderView>, String> {
    let cpp = cpp(&state).await?;
    let report = cpp.platform.refresh().await;
    Ok(report
        .providers
        .into_iter()
        .map(|p| CppProviderView {
            provider_id: p.provider_id,
            health: p.health.as_str().to_string(),
            state: p.state.as_str().to_string(),
            version: p.version.map(|v| format!("{}.{}", v.major, v.minor)),
            descriptor_count: p.descriptor_count,
            error: p.error,
        })
        .collect())
}

/// Cross-provider discovery for a goal query (Capability Browser search).
#[command]
pub async fn cpp_discover(
    query: String,
    k: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<Vec<CppCapabilityView>, String> {
    let cpp = cpp(&state).await?;
    let hits = cpp
        .platform
        .discover(&query, k.unwrap_or(20))
        .map_err(|e| e.to_string())?;
    Ok(hits.iter().map(to_view).collect())
}

/// The full federated capability catalog (Capability Browser default listing).
#[command]
pub async fn cpp_catalog(state: State<'_, AppStateCell>) -> Result<Vec<CppCapabilityView>, String> {
    let cpp = cpp(&state).await?;
    let hits = cpp
        .platform
        .discover("", 10_000)
        .map_err(|e| e.to_string())?;
    Ok(hits.iter().map(to_view).collect())
}

/// Marketplace recommendations for a goal (installable, not-yet-installed
/// capabilities across all providers' catalogs).
#[command]
pub async fn cpp_recommend(
    query: String,
    k: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<Vec<CppCapabilityView>, String> {
    let cpp = cpp(&state).await?;
    let hits = cpp
        .platform
        .recommend(&query, k.unwrap_or(10))
        .await
        .map_err(|e| e.to_string())?;
    Ok(hits.iter().map(to_view).collect())
}

/// One quarantined capability (failed the Brain's trust/integrity gate, R8.3).
#[derive(serde::Serialize)]
pub struct CppQuarantineView {
    pub provider_id: String,
    pub capability_id: String,
    pub reason: String,
}

/// List capabilities quarantined by the marketplace trust/integrity gate
/// (spec R8.3). Empty unless `marketplace_v2` is enabled.
#[command]
pub async fn cpp_quarantined(
    state: State<'_, AppStateCell>,
) -> Result<Vec<CppQuarantineView>, String> {
    let cpp = cpp(&state).await?;
    Ok(cpp
        .platform
        .quarantined()
        .into_iter()
        .map(|(provider_id, capability_id, reason)| CppQuarantineView {
            provider_id,
            capability_id,
            reason,
        })
        .collect())
}

/// Release a capability from quarantine after operator review / re-verification
/// (spec R8.3). Returns whether it was quarantined.
#[command]
pub async fn cpp_release_quarantine(
    provider_id: String,
    capability_id: String,
    state: State<'_, AppStateCell>,
) -> Result<bool, String> {
    let cpp = cpp(&state).await?;
    Ok(cpp
        .platform
        .release_quarantine(&provider_id, &capability_id))
}

/// Durable generated/discovered tool quarantine record for Governance review.
#[derive(Debug, Serialize)]
pub struct QuarantineToolView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub risk_level: kria_core::safety::RiskLevel,
    pub status: kria_core::tools::quarantine::QuarantineStatus,
    pub source: kria_core::tools::quarantine::ToolSource,
    pub success_count: i64,
    pub consecutive_failures: i64,
    pub total_executions: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_tested: chrono::DateTime<chrono::Utc>,
    pub review_notes: Option<String>,
    pub parameters_schema: Option<serde_json::Value>,
}

impl From<kria_core::tools::quarantine::QuarantinedTool> for QuarantineToolView {
    fn from(tool: kria_core::tools::quarantine::QuarantinedTool) -> Self {
        Self {
            id: tool.name.clone(),
            name: tool.name,
            description: "Generated or discovered tool awaiting runtime trust review".to_string(),
            risk_level: tool.risk_level,
            status: tool.status,
            source: tool.source,
            success_count: tool.success_count,
            consecutive_failures: tool.consecutive_failures,
            total_executions: tool.total_executions,
            created_at: tool.created_at,
            last_tested: tool.last_tested,
            review_notes: tool.review_notes,
            parameters_schema: None,
        }
    }
}

#[command]
pub async fn list_quarantined_tools(
    state: State<'_, AppStateCell>,
) -> Result<Vec<QuarantineToolView>, String> {
    let app = state.get().ok_or("runtime not ready")?;
    let registry = app.quarantine_registry.clone();
    tokio::task::spawn_blocking(move || registry.all())
        .await
        .map_err(|error| error.to_string())?
        .map(|tools| tools.into_iter().map(QuarantineToolView::from).collect())
        .map_err(|error| error.to_string())
}

#[command]
pub async fn approve_quarantined_tool(
    tool_id: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let app = state.get().ok_or("runtime not ready")?;
    let registry = app.quarantine_registry.clone();
    tokio::task::spawn_blocking(move || {
        registry.approve(&tool_id, Some("Approved from Capabilities governance"))
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

#[command]
pub async fn reject_quarantined_tool(
    tool_id: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let app = state.get().ok_or("runtime not ready")?;
    let registry = app.quarantine_registry.clone();
    tokio::task::spawn_blocking(move || {
        registry.reject(&tool_id, Some("Rejected from Capabilities governance"))
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

/// The full descriptor for the Descriptor Viewer.
#[command]
pub async fn cpp_descriptor(
    provider_id: String,
    capability_id: String,
    state: State<'_, AppStateCell>,
) -> Result<CppDescriptorView, String> {
    let cpp = cpp(&state).await?;
    let d = require_descriptor(&cpp, &provider_id, &capability_id).await?;
    Ok(CppDescriptorView {
        provider_id: d.provider_id.clone(),
        capability_id: d.capability_id.clone(),
        name: d.name.clone(),
        description: d.description.clone(),
        version: d.version.clone(),
        schema_version: format!("{}.{}", d.schema_version.major, d.schema_version.minor),
        tags: d.tags.iter().map(|t| t.id.clone()).collect(),
        io_modality: d.io_modality.clone(),
        inputs: d.inputs.clone(),
        outputs: d.outputs.clone(),
        effect_classes: d.effects.classes.clone(),
        reversible: format!("{:?}", d.effects.reversible),
        idempotent: d.effects.idempotent,
        elevated: d.effects.is_elevated(),
        trust_tier: d.trust.tier.clone(),
        signed: d.trust.signed,
        guidance: d
            .guidance
            .as_ref()
            .and_then(|g| serde_json::to_value(g).ok()),
        expectations: d
            .expectations
            .as_ref()
            .and_then(|e| serde_json::to_value(e).ok()),
        input_schema: d.input_schema.clone(),
    })
}

// ── Permission / grants commands (M4) ───────────────────────────────────────

/// The Approval Center grant list: all active durable grants.
#[command]
pub async fn cpp_list_grants(state: State<'_, AppStateCell>) -> Result<Vec<CppGrantView>, String> {
    let cpp = cpp(&state).await?;
    let grants = cpp
        .grants
        .active_grants(chrono::Utc::now())
        .map_err(|e| e.to_string())?;
    Ok(grants
        .into_iter()
        .map(|g| CppGrantView {
            grant_id: g.grant_id,
            provider_id: g.provider_id,
            capability_id: g.capability_id,
            scope: g.scope_kind.as_str().to_string(),
            scope_key: g.scope_key,
            effects: g.effects,
            decision: g.decision.as_str().to_string(),
            granted_at: g.granted_at.to_rfc3339(),
            expires_at: g.expires_at.map(|e| e.to_rfc3339()),
        })
        .collect())
}

/// Revoke a durable grant by id (forces fresh approval next use).
#[command]
pub async fn cpp_revoke_grant(
    grant_id: String,
    state: State<'_, AppStateCell>,
) -> Result<bool, String> {
    let cpp = cpp(&state).await?;
    cpp.engine
        .revoke(&grant_id, &cpp.grants)
        .map(|_| true)
        .map_err(|e| e.to_string())
}

/// Authorize a capability without executing it — the modal calls this to learn
/// whether a prompt is needed and what effects to surface.
#[command]
pub async fn cpp_authorize(
    provider_id: String,
    capability_id: String,
    session_id: Option<String>,
    workspace_id: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<CppAuthDecision, String> {
    let cpp = cpp(&state).await?;
    let d = require_descriptor(&cpp, &provider_id, &capability_id).await?;
    let req = AuthorizeRequest::from_descriptor(&d, session_id, workspace_id);
    let decision = cpp.engine.authorize(&req, &cpp.grants);
    let mut view = decision_to_view(&decision);
    // Ensure the requested effect classes are always surfaced (even on allow).
    if view.effects.is_empty() {
        view.effects = d.effects.classes.clone();
    }
    Ok(view)
}

/// Persist an approval decision (approve/deny at a scope) after the user acts on
/// the modal. Returns the new grant id.
#[command]
pub async fn cpp_approve(
    provider_id: String,
    capability_id: String,
    scope: String,
    allow: bool,
    session_id: Option<String>,
    workspace_id: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<String, String> {
    let cpp = cpp(&state).await?;
    let d = require_descriptor(&cpp, &provider_id, &capability_id).await?;
    let req = AuthorizeRequest::from_descriptor(&d, session_id, workspace_id);
    let scope_kind = ScopeKind::from_name(&scope).unwrap_or(ScopeKind::Once);
    let decision = if allow {
        GrantDecision::Allow
    } else {
        GrantDecision::Deny
    };
    let grant = approval_grant(&req, scope_kind, decision);
    let id = grant.grant_id.clone();
    cpp.grants.insert(&grant).map_err(|e| e.to_string())?;
    Ok(id)
}

// ── Gated execution (M4 + default-on migration) ─────────────────────────────

/// Execute a capability through the permission gate. If approval is required the
/// command returns `needs_approval` with the decision (the UI raises the modal,
/// calls `cpp_approve`, then re-invokes this). On allow, it runs the capability
/// through the owning provider and returns the result.
#[command]
pub async fn cpp_execute(
    provider_id: String,
    capability_id: String,
    args: serde_json::Value,
    session_id: Option<String>,
    workspace_id: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<CppExecuteResult, String> {
    let cpp = cpp(&state).await?;
    let d = require_descriptor(&cpp, &provider_id, &capability_id).await?;

    // 1) Permission gate.
    let auth = AuthorizeRequest::from_descriptor(&d, session_id.clone(), workspace_id.clone());
    match cpp.engine.authorize(&auth, &cpp.grants) {
        PermissionDecision::Deny { reason } => {
            return Ok(CppExecuteResult {
                status: "denied".into(),
                decision: None,
                value: None,
                reason: Some(reason),
            });
        }
        PermissionDecision::Prompt { .. } => {
            let decision = cpp.engine.authorize(&auth, &cpp.grants);
            let mut view = decision_to_view(&decision);
            if view.effects.is_empty() {
                view.effects = d.effects.classes.clone();
            }
            return Ok(CppExecuteResult {
                status: "needs_approval".into(),
                decision: Some(view),
                value: None,
                reason: None,
            });
        }
        PermissionDecision::Allow { .. } => {}
    }

    // 2) Approved → build the neutral request and execute through the platform.
    let mut ctx = RequestContext::new();
    ctx.session_id = session_id;
    ctx.workspace_id = workspace_id;
    let req = CapabilityRequest {
        provider_id: provider_id.clone(),
        capability_id: capability_id.clone(),
        args,
        context: ctx,
        // The provider must not exceed the descriptor's declared effect classes.
        granted_effects: d.effects.classes.clone(),
    };

    match cpp.platform.execute(req).await {
        Ok(CapabilityOutcome::Value(v)) => Ok(CppExecuteResult {
            status: "ok".into(),
            decision: None,
            value: Some(v),
            reason: None,
        }),
        Ok(CapabilityOutcome::Declined { reason }) => Ok(CppExecuteResult {
            status: "declined".into(),
            decision: None,
            value: None,
            reason: Some(reason),
        }),
        Ok(CapabilityOutcome::Stream(_)) => Ok(CppExecuteResult {
            // Streaming results are surfaced via events; the command returns ok.
            status: "ok".into(),
            decision: None,
            value: Some(serde_json::json!({ "streaming": true })),
            reason: None,
        }),
        Err(e) => Err(e.to_string()),
    }
}

// ── Observability (M8 / M9-C) ───────────────────────────────────────────────

/// The Timeline / Runtime Monitor feed: the most recent CPP events, newest last.
/// Optionally filtered to one `correlation_id` (a single goal's lifecycle).
#[command]
pub async fn cpp_timeline(
    correlation_id: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<Vec<CppEventView>, String> {
    let cpp = cpp(&state).await?;
    let buf = cpp.ring.lock().map_err(|e| format!("ring lock: {e}"))?;
    let limit = limit.unwrap_or(500);
    // Filter (preserving chronological order), then keep only the last `limit`
    // events so the newest are always returned, oldest-to-newest.
    let mut matched: Vec<&CapabilityEvent> = buf
        .iter()
        .filter(|e| {
            correlation_id
                .as_ref()
                .is_none_or(|c| &e.correlation_id == c)
        })
        .collect();
    if matched.len() > limit {
        matched.drain(0..matched.len() - limit);
    }
    Ok(matched
        .into_iter()
        .map(|e| CppEventView {
            correlation_id: e.correlation_id.clone(),
            provider_id: e.provider_id.clone(),
            capability_id: e.capability_id.clone(),
            stage: e.stage.as_str().to_string(),
            outcome: e.outcome.as_str().to_string(),
            detail: e.detail.clone(),
            timestamp: e.timestamp.to_rfc3339(),
        })
        .collect())
}

// ── Wave 8: Evolution / Health oversight (spec R6, R18, R29) ────────────────

/// One capability's health for the Health tab.
#[derive(Debug, Serialize)]
pub struct CppHealthView {
    pub provider_id: String,
    pub capability_id: String,
    pub family: String,
    pub status: String,
    pub success_rate: Option<f32>,
    pub total: u64,
    pub consecutive_failures: u32,
    pub last_failure: Option<String>,
}

/// One evolution proposal for the Oversight/Proposals feed.
#[derive(Debug, Serialize)]
pub struct CppProposalView {
    pub id: String,
    pub kind: String,
    pub provider_id: String,
    pub capability_id: String,
    pub replacement: Option<(String, String)>,
    pub rationale: String,
    pub confidence: f32,
    pub requires_approval: bool,
    pub status: String,
    pub created_at: String,
}

async fn autonomy_level(
    state: &State<'_, AppStateCell>,
) -> kria_core::capability::intelligence::AutonomyLevel {
    use kria_core::capability::intelligence::AutonomyLevel;
    let Some(app) = state.get() else {
        return AutonomyLevel::default();
    };
    let level = app
        .config
        .read()
        .await
        .capability
        .intelligence
        .autonomy_level
        .clone();
    AutonomyLevel::parse(&level).unwrap_or_default()
}

/// Capability health snapshots (spec R6.1). Empty unless the CKB/evolution store
/// is wired.
#[command]
pub async fn cpp_health(state: State<'_, AppStateCell>) -> Result<Vec<CppHealthView>, String> {
    use kria_core::capability::intelligence::{health, HealthPolicy};
    let cpp = cpp(&state).await?;
    let Some(store) = cpp.platform.evolution_store() else {
        return Ok(vec![]);
    };
    let snaps = store.health_snapshots().await.map_err(|e| e.to_string())?;
    let classified = health::classify(&HealthPolicy::default(), snaps);
    Ok(classified
        .into_iter()
        .map(|h| CppHealthView {
            success_rate: h.success_rate(),
            provider_id: h.provider_id,
            capability_id: h.capability_id,
            family: h.family,
            status: h.status.as_str().to_string(),
            total: h.total,
            consecutive_failures: h.consecutive_failures,
            last_failure: h.last_failure,
        })
        .collect())
}

fn proposal_to_view(p: kria_core::capability::intelligence::EvolutionProposal) -> CppProposalView {
    CppProposalView {
        id: p.id,
        kind: p.kind.as_str().to_string(),
        provider_id: p.provider_id,
        capability_id: p.capability_id,
        replacement: p.replacement,
        rationale: p.rationale,
        confidence: p.confidence,
        requires_approval: p.requires_approval,
        status: p.status.as_str().to_string(),
        created_at: p.created_at,
    }
}

/// Analyze health + return the current evolution proposals (spec R6/R29.1). This
/// runs the neutral Evolution Engine (which persists new proposals) then returns
/// the full auditable list, newest first.
#[command]
pub async fn cpp_proposals(state: State<'_, AppStateCell>) -> Result<Vec<CppProposalView>, String> {
    use kria_core::capability::intelligence::DefaultEvolutionEngine;
    let autonomy = autonomy_level(&state).await;
    let cpp = cpp(&state).await?;
    let Some(store) = cpp.platform.evolution_store() else {
        return Ok(vec![]);
    };
    // Generate fresh proposals from current health (idempotent per trigger).
    let engine = DefaultEvolutionEngine::new(store.clone(), autonomy);
    let _ = engine.analyze().await.map_err(|e| e.to_string())?;
    let all = store
        .list_proposals(None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(all.into_iter().map(proposal_to_view).collect())
}

/// Approve + mark a proposal Applied (spec R29.1 — the actual lifecycle action
/// is driven by the existing LifecycleManager; this records the auditable
/// decision + status). Returns the updated proposal.
/// Build a lifecycle manager over the UI platform + its CKB (for real apply/undo).
fn lifecycle_for(cpp: &CppState) -> DefaultLifecycleManager {
    let mut mgr = DefaultLifecycleManager::new(cpp.platform.clone());
    if let Some(ckb) = cpp.platform.knowledge() {
        mgr = mgr.with_knowledge(ckb.clone());
    }
    mgr
}

#[command]
pub async fn cpp_proposal_apply(
    id: String,
    state: State<'_, AppStateCell>,
) -> Result<CppProposalView, String> {
    use kria_core::capability::intelligence::{DefaultEvolutionEngine, EvolutionProposal};
    let autonomy = autonomy_level(&state).await;
    let cpp = cpp(&state).await?;
    let store = cpp
        .platform
        .evolution_store()
        .ok_or("evolution store not enabled")?;
    let proposal: EvolutionProposal = store
        .get_proposal(&id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("proposal not found")?;

    // REAL application through the neutral LifecycleManager (not a status flip).
    let lifecycle = lifecycle_for(&cpp);
    let engine = DefaultEvolutionEngine::new(store.clone(), autonomy);
    engine
        .apply(&proposal, &lifecycle)
        .await
        .map_err(|e| e.to_string())?;

    let updated = store
        .get_proposal(&id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("proposal not found after apply")?;
    Ok(proposal_to_view(updated))
}

/// Undo an applied proposal (spec R29.1) — **real reversal** via the lifecycle
/// manager (recover archived capability), then mark Undone.
#[command]
pub async fn cpp_proposal_undo(
    id: String,
    state: State<'_, AppStateCell>,
) -> Result<CppProposalView, String> {
    use kria_core::capability::intelligence::{DefaultEvolutionEngine, EvolutionProposal};
    let autonomy = autonomy_level(&state).await;
    let cpp = cpp(&state).await?;
    let store = cpp
        .platform
        .evolution_store()
        .ok_or("evolution store not enabled")?;
    let proposal: EvolutionProposal = store
        .get_proposal(&id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("proposal not found")?;

    let lifecycle = lifecycle_for(&cpp);
    let engine = DefaultEvolutionEngine::new(store.clone(), autonomy);
    engine
        .undo(&proposal, &lifecycle)
        .await
        .map_err(|e| e.to_string())?;

    let updated = store
        .get_proposal(&id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("proposal not found after undo")?;
    Ok(proposal_to_view(updated))
}

/// Get the configured autonomy level (spec R29.2).
#[command]
pub async fn cpp_get_autonomy(state: State<'_, AppStateCell>) -> Result<String, String> {
    Ok(autonomy_level(&state).await.as_str().to_string())
}

/// Set the capability-evolution autonomy level. Validates the enum, routes
/// elevated levels through canonical settings approval, then persists and
/// hot-swaps the complete intelligence config atomically through ConfigService.
#[command]
pub async fn cpp_set_autonomy(
    level: String,
    app_handle: AppHandle,
    state: State<'_, AppStateCell>,
) -> Result<String, String> {
    use kria_core::capability::intelligence::AutonomyLevel;
    use kria_core::config::ChangeSource;
    use kria_core::safety::{hitl::ApprovalResponse, RiskLevel};

    let parsed =
        AutonomyLevel::parse(&level).ok_or_else(|| format!("invalid autonomy level '{level}'"))?;
    let app = state.get().ok_or("runtime not ready")?;
    let mut intelligence = app.config.read().await.capability.intelligence.clone();
    intelligence.autonomy_level = parsed.as_str().to_string();
    let value = serde_json::to_value(&intelligence).map_err(|error| error.to_string())?;
    let risk = match parsed {
        AutonomyLevel::Manual | AutonomyLevel::ProposeOnly => RiskLevel::Green,
        AutonomyLevel::AutoWithNotice => RiskLevel::Yellow,
        AutonomyLevel::FullAuto => RiskLevel::Red,
    };

    if risk != RiskLevel::Green {
        match super::config_prompt::request_settings_approval(
            &app_handle,
            &app.hitl,
            "capability",
            "intelligence",
            &value,
            risk,
        )
        .await
        {
            ApprovalResponse::Approved => {}
            ApprovalResponse::Denied => return Err("capability autonomy change was denied".into()),
            ApprovalResponse::Timeout => {
                return Err("capability autonomy approval timed out".into())
            }
        }
    }

    app.config_service
        .patch("capability", "intelligence", value, ChangeSource::Ui, None)
        .await
        .map_err(|error| error.to_string())?;
    Ok(parsed.as_str().to_string())
}

// ── Capability Synthesis / Generate (M9 / Wave 9, W9-R12) ───────────────────

/// A dry-run preview of the capability KRIA would synthesize for a goal — shown
/// in the Generate tab BEFORE anything is installed (spec R7/R27). Pure: derives
/// the deterministic Capability-Graph IR and reports it; installs nothing.
#[derive(serde::Serialize)]
pub struct CppSynthesisPreview {
    /// Whether the goal is expressible from the audited primitive set.
    pub synthesizable: bool,
    pub capability_id: Option<String>,
    pub name: Option<String>,
    /// The ordered audited-primitive pipeline the IR composes.
    pub pipeline: Vec<String>,
    /// Number of IR nodes.
    pub node_count: usize,
    /// Content-addressed IR hash (provenance/reproducibility).
    pub ir_hash: Option<String>,
    /// The declared golden case (input → expected output) — the liveness proof.
    pub golden_input: Option<String>,
    pub golden_output: Option<String>,
    /// Honest message when the goal is not synthesizable (no fabrication).
    pub message: Option<String>,
}

/// Preview the capability KRIA would synthesize for `goal` — no install, no side
/// effects (W9-R12). The Generate tab calls this as the user types, then calls
/// [`cpp_synthesize`] to actually generate + smoke + activate.
#[command]
pub async fn cpp_synthesis_preview(goal: String) -> Result<CppSynthesisPreview, String> {
    use kria_core::capability::intelligence::CapabilitySpecification;
    match CapabilitySpecification::from_goal(&goal) {
        Some(spec) => {
            let node_count = spec.normalized_graph().map(|g| g.nodes.len()).unwrap_or(0);
            Ok(CppSynthesisPreview {
                synthesizable: true,
                capability_id: Some(spec.capability_id.clone()),
                name: Some(spec.name.clone()),
                pipeline: spec.pipeline.clone(),
                node_count,
                ir_hash: spec.ir_hash(),
                golden_input: Some(spec.golden_input.clone()),
                golden_output: Some(spec.golden_output.clone()),
                message: None,
            })
        }
        None => Ok(CppSynthesisPreview {
            synthesizable: false,
            capability_id: None,
            name: None,
            pipeline: Vec::new(),
            node_count: 0,
            ir_hash: None,
            golden_input: None,
            golden_output: None,
            message: Some(format!(
                "'{goal}' is not expressible from the audited primitive set — KRIA will not \
                 fabricate a capability (honest decline)."
            )),
        }),
    }
}

/// Synthesize (generate + smoke-gate + trust-gate + activate) a capability for a
/// goal, through the SAME neutral acquisition path as marketplace install
/// (W9-R12, spec R7). Returns the installed descriptor view. Honest error when
/// the goal is not synthesizable — nothing is fabricated.
#[command]
pub async fn cpp_synthesize(
    goal: String,
    state: State<'_, AppStateCell>,
) -> Result<CppCapabilityView, String> {
    let cpp = cpp(&state).await?;
    let descriptor = cpp
        .platform
        .acquire_for_goal(&goal)
        .await
        .map_err(|e| e.to_string())?;
    let (pid, cid) = descriptor.key();
    // Return the freshly-installed descriptor as a browser view.
    let hits = cpp
        .platform
        .discover("", 10_000)
        .map_err(|e| e.to_string())?;
    hits.iter()
        .find(|s| s.descriptor.provider_id == pid && s.descriptor.capability_id == cid)
        .map(to_view)
        .ok_or_else(|| format!("synthesized '{cid}' not found after activation"))
}

// ── LLM-assisted IR proposer adapter (Wave 9, W9-R11) ───────────────────────

/// Neutral [`TextGenerator`] adapter over the app's real `ModelRouter`. This is
/// the ACL seam that lets the provider-neutral `LlmIrProposer` (in kria-core)
/// use KRIA's live LLM without the capability Brain depending on any LLM type.
/// The model only ever *proposes* an IR — the validator + golden gate in core
/// own correctness, and a bad/absent model falls back to the deterministic
/// proposer (never fabricates).
pub struct SynthesisLlmGenerator {
    router: Arc<kria_core::llm::model_router::ModelRouter>,
    label: String,
}

impl SynthesisLlmGenerator {
    pub fn new(router: Arc<kria_core::llm::model_router::ModelRouter>) -> Self {
        Self {
            router,
            label: "router".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl kria_core::capability::intelligence::TextGenerator for SynthesisLlmGenerator {
    async fn generate(&self, system: &str, user: &str) -> Result<String, String> {
        use kria_core::llm::ChatMessage;
        let backend = self
            .router
            .route("capability_synthesis")
            .await
            .ok_or_else(|| "no LLM backend available for synthesis".to_string())?;
        let messages = vec![
            ChatMessage {
                role: "system".into(),
                content: system.to_string(),
                name: None,
                images: None,
            },
            ChatMessage {
                role: "user".into(),
                content: user.to_string(),
                name: None,
                images: None,
            },
        ];
        // Low temperature for deterministic-ish structured output; small budget —
        // the response is a short JSON pipeline, not prose.
        let resp = backend
            .chat(&messages, None, 0.0, 256)
            .await
            .map_err(|e| format!("synthesis LLM call failed: {e}"))?;
        Ok(resp.content)
    }

    fn model_label(&self) -> &str {
        &self.label
    }
}

// ── Continuous Discovery & Maintenance (Wave 10, spec R20/R29 §34) ──────────

/// Live status of the background discovery loop (Discovery Dashboard). Returns a
/// disabled snapshot when the `continuous_discovery` flag is off.
#[command]
pub async fn cpp_discovery_status(
    state: State<'_, AppStateCell>,
) -> Result<kria_core::capability::intelligence::DiscoveryStatus, String> {
    // Ensure the surface is built (may lazily spawn the loop if enabled).
    let _ = cpp(&state).await?;
    match discovery_engine() {
        Some(engine) => Ok(engine.status()),
        None => Ok(kria_core::capability::intelligence::DiscoveryStatus::default()),
    }
}

/// Manually trigger one discovery scan now (Discovery Dashboard "Scan" button).
/// Runs the SAME `scan_once` the background loop runs; returns the report. Errors
/// honestly when the loop is not enabled.
#[command]
pub async fn cpp_discovery_scan(
    state: State<'_, AppStateCell>,
) -> Result<kria_core::capability::intelligence::DiscoveryReport, String> {
    let _ = cpp(&state).await?;
    match discovery_engine() {
        Some(engine) => Ok(engine.scan_once().await),
        None => Err("continuous discovery is disabled (enable capability.intelligence.continuous_discovery)".into()),
    }
}

// ── Long-running jobs (Wave 11, spec R28) ───────────────────────────────────

/// The Execution Monitor feed: recent jobs (newest first) with their durable
/// state. Empty when the `jobs` flag is off.
#[command]
pub async fn cpp_jobs(
    limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<Vec<kria_core::capability::intelligence::Job>, String> {
    let _ = cpp(&state).await?;
    match job_manager() {
        Some(mgr) => mgr
            .list(limit.unwrap_or(200))
            .await
            .map_err(|e| e.to_string()),
        None => Ok(Vec::new()),
    }
}

/// Submit a new durable job (queued) and run it to a terminal state through the
/// reliable execution path. Returns the final job state. Errors if jobs are off.
#[command]
pub async fn cpp_job_submit(
    provider_id: String,
    capability_id: String,
    args: serde_json::Value,
    priority: Option<i64>,
    state: State<'_, AppStateCell>,
) -> Result<String, String> {
    let _ = cpp(&state).await?;
    let mgr = job_manager().ok_or("jobs are disabled (enable capability.intelligence.jobs)")?;
    let id = mgr
        .submit(&provider_id, &capability_id, args, priority.unwrap_or(0))
        .await
        .map_err(|e| e.to_string())?;
    // Run in the background so the command returns immediately (long-running).
    let run_mgr = mgr.clone();
    let run_id = id.clone();
    tokio::spawn(async move {
        let _ = run_mgr.run(&run_id).await;
    });
    Ok(id)
}

/// Control a job: `action` = "cancel" (user/shutdown/dependency cancellation).
#[command]
pub async fn cpp_job_control(
    id: String,
    action: String,
    state: State<'_, AppStateCell>,
) -> Result<String, String> {
    let _ = cpp(&state).await?;
    let mgr = job_manager().ok_or("jobs are disabled")?;
    match action.as_str() {
        "cancel" => {
            mgr.cancel(&id).await.map_err(|e| e.to_string())?;
            Ok("cancelled".into())
        }
        "pause" => {
            let ok = mgr.pause(&id).await.map_err(|e| e.to_string())?;
            Ok(if ok {
                "paused".into()
            } else {
                "not_pausable".into()
            })
        }
        "resume" => {
            let mgr2 = mgr.clone();
            let rid = id.clone();
            tokio::spawn(async move {
                let _ = mgr2.resume(&rid).await;
            });
            Ok("resuming".into())
        }
        other => Err(format!("unknown job action '{other}'")),
    }
}
