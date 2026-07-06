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
use kria_core::capability::permission::{
    approval_grant, AuthorizeRequest, DefaultPermissionEngine, PermissionDecision,
    PermissionEngine, PermissionTier,
};
use kria_core::capability::platform::CapabilityPlatform;
use kria_core::capability::provider::{CapabilityOutcome, CapabilityRequest, RequestContext};
use kria_core::capability::registry::ProviderRegistry;
use kria_core::openclaw::runtime::{DockerRuntime, SkillRuntime};
use serde::Serialize;
use tauri::{command, State};
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

    let platform = Arc::new(CapabilityPlatform::new(registry).with_events(events.clone()));
    platform.refresh().await;

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
