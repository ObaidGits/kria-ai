//! OpenClaw subsystem initialization.
//!
//! Provides a single entry-point (`OpenClawSubsystem::boot`) that:
//! 1. Opens `skills.db` (creating it if absent).
//! 2. Synchronously creates `installed_skills` and `audit_log` tables (via
//!    `SkillRegistry::open` and `AuditLedger::open`).
//! 3. Seeds curated built-in skills if they are not already present.
//!
//! This guarantees both tables exist immediately at application startup,
//! never lazily on first write.

use crate::openclaw::audit::AuditLedger;
use crate::openclaw::pool::ContainerPool;
use crate::openclaw::registry::{
    DiscoverySource, ProductionSkillRegistry, SkillMetadata, SkillState,
};
use crate::openclaw::types::*;
use crate::safety::RiskLevel;
use std::path::Path;
use std::sync::Arc;

/// Holds the fully-initialized OpenClaw subsystem handles.
pub struct OpenClawSubsystem {
    pub registry: Arc<ProductionSkillRegistry>,
    pub audit: Arc<AuditLedger>,
}

/// Default HMAC key for development builds.
/// In production this should be derived from a user-specific secret.
const DEV_HMAC_KEY: &[u8] = b"kria-openclaw-dev-audit-key-0001";

impl OpenClawSubsystem {
    /// Boot the OpenClaw subsystem synchronously.
    ///
    /// `data_dir` is the KRIA data directory (e.g. `~/.kria/`).
    /// Both `installed_skills` and `audit_log` tables are created in
    /// `<data_dir>/skills.db` via `CREATE TABLE IF NOT EXISTS` — this is
    /// synchronous and executes immediately, not lazily.
    pub fn boot(data_dir: &Path) -> Result<Self, OpenClawBootError> {
        let db_path = data_dir.join("skills.db");

        // Ensure the parent directory exists.
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| OpenClawBootError::Io(format!("failed to create data dir: {e}")))?;
        }

        // 1. Open ProductionSkillRegistry (A5 architecture)
        let registry = ProductionSkillRegistry::open(&db_path)
            .map_err(|e| OpenClawBootError::Registry(format!("{e}")))?;

        // 2. Open AuditLedger (creates `audit_log` table synchronously)
        let audit = AuditLedger::open(&db_path, DEV_HMAC_KEY.to_vec())
            .map_err(|e| OpenClawBootError::Audit(format!("{e}")))?;

        let registry = Arc::new(registry);
        let audit = Arc::new(audit);

        // 3. Seed curated skills (idempotent — skips if already present)
        initialize_curated_skills(&registry);

        tracing::info!(
            db = %db_path.display(),
            "[OpenClaw] subsystem booted — registry + audit_log tables ready"
        );

        Ok(Self { registry, audit })
    }
}

/// Errors that can occur during OpenClaw subsystem boot.
#[derive(Debug, thiserror::Error)]
pub enum OpenClawBootError {
    #[error("IO error: {0}")]
    Io(String),
    #[error("registry initialization failed: {0}")]
    Registry(String),
    #[error("audit ledger initialization failed: {0}")]
    Audit(String),
}

/// RC2 — synchronize the registry from the container's authoritative
/// `tools/list`. The container is the single source of truth for what can
/// actually execute; this makes EVERY baked/installed skill routable by the
/// semantic router, with its real `inputSchema` persisted for argument
/// generation. General — no hardcoded skill list; any future baked skill
/// appears automatically.
///
/// - New skills (not in the registry) are installed ENABLED with their schema.
/// - Existing skills keep their state/trust, but get their `input_schema`
///   backfilled when missing (brings pre-`input_schema`-column rows — e.g. an
///   upgrading user's curated calculator — in line with the container).
///
/// Non-fatal: returns the count of skills added/updated, or an error string if
/// the container/bridge is unreachable (OpenClaw simply stays on whatever the
/// registry already had).
pub async fn sync_registry_from_container(
    registry: &ProductionSkillRegistry,
    pool: Arc<ContainerPool>,
) -> Result<usize, String> {
    let runtime = crate::openclaw::runtime::DockerRuntime::new(pool);
    let tools = runtime.probe_tools().await?;
    let mut changed = 0usize;

    // The container's authoritative set of executable tool names. Built before
    // the (consuming) add/backfill loop below so the prune pass can quarantine
    // registry skills the substrate does not actually implement (RC-2).
    let container_tool_names: std::collections::HashSet<String> =
        tools.iter().map(|t| t.name.clone()).collect();

    for tool in tools {
        let schema = tool.input_schema.clone();
        match registry.get_skill(&tool.name) {
            Ok(existing) => {
                // Backfill the schema if the registry row lacks one.
                if existing.input_schema.is_none() {
                    if let Some(ref s) = schema {
                        if let Err(e) = registry.set_input_schema(&tool.name, s) {
                            tracing::warn!(skill = %tool.name, "sync: set_input_schema failed: {e}");
                        } else {
                            changed += 1;
                        }
                    }
                }
            }
            Err(_) => {
                // New container-advertised skill → register it enabled.
                let now = chrono::Utc::now();
                let metadata = SkillMetadata {
                    skill_id: tool.name.clone(),
                    name: tool.name.clone(),
                    description: tool.description.clone().unwrap_or_default(),
                    publisher: "KRIA".to_string(),
                    version: "1.0.0".to_string(),
                    category: "utility".to_string(),
                    discovery_source: DiscoverySource::Bundled {
                        path: "container".to_string(),
                    },
                    discovered_at: now,
                    capabilities: SkillCapabilities::default(),
                    runtime_requirements: "{}".to_string(),
                    risk_level: RiskLevel::Green,
                    resource_class: ResourceClass::Light,
                    tags: vec![],
                    categories: vec!["utility".to_string()],
                    semantic_version: "1.0.0".to_string(),
                    dependencies: vec![],
                    compatibility_requirements: vec![],
                    trust_tier: TrustTier::Verified,
                    content_hash: String::new(),
                    signature: None,
                    granted_capabilities: vec![],
                    bundle_path: None,
                    manifest_toml: None,
                    input_schema: schema,
                    state: SkillState::Enabled,
                    state_changed_at: now,
                };
                if let Err(e) = registry.install_skill(&metadata) {
                    tracing::warn!(skill = %tool.name, "sync: install failed: {e}");
                } else {
                    changed += 1;
                }
            }
        }
    }

    // ── Prune pass (RC-2): quarantine phantom baked-in skills ──────────────
    //
    // A skill is executable one of two ways: (a) baked into the substrate image
    // — it MUST appear in the container `tools/list`; or (b) installed as a
    // bundle with its own mounted handler dir (`bundle_path` set) —
    // `execute_semantic` bind-mounts `<bundle_path>/.bridge`, so it does NOT
    // need to be in `tools/list`. Any ENABLED skill that is neither in the
    // container toolset NOR bundle-backed is a phantom: routable but
    // unexecutable, producing the runtime `[-32602] Unknown tool` error. Disable
    // it so the semantic router can never select it (registry exposes only
    // executable capabilities — RC-2 goal).
    //
    // Guard: only prune when the container returned a NON-EMPTY toolset. An
    // empty probe means the container/bridge was unreachable or degraded; in
    // that case we keep whatever the registry had (honest degraded, never a
    // destructive wipe).
    if !container_tool_names.is_empty() {
        match registry.get_enabled_skills() {
            Ok(enabled) => {
                for meta in enabled {
                    let bundle_backed = meta
                        .bundle_path
                        .as_deref()
                        .map(|bp| std::path::Path::new(bp).join(".bridge").is_dir())
                        .unwrap_or(false);
                    if !bundle_backed && !container_tool_names.contains(&meta.skill_id) {
                        match registry.toggle(&meta.skill_id, false) {
                            Ok(()) => {
                                changed += 1;
                                tracing::warn!(
                                    skill = %meta.skill_id,
                                    "[OpenClaw] disabled phantom skill (not in container \
                                     tools/list and not bundle-backed) — it cannot execute in \
                                     the current substrate image"
                                );
                            }
                            Err(e) => tracing::warn!(
                                skill = %meta.skill_id,
                                "sync: failed to disable phantom skill: {e}"
                            ),
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("sync: prune pass skipped (enabled query failed): {e}"),
        }
    }

    tracing::info!(
        changed,
        "[OpenClaw] registry synced from container tools/list"
    );
    Ok(changed)
}

/// Inject curated skills into the registry if they are not already present.
pub fn initialize_curated_skills(registry: &ProductionSkillRegistry) {
    let calculator_params = serde_json::json!({
        "type": "object",
        "properties": {
            "expression": {
                "type": "string",
                "description": "Arithmetic expression, e.g. '2 * (3 + 4)' or '10 / 4'."
            }
        },
        "required": ["expression"]
    });
    let empty_params = serde_json::json!({"type": "object", "properties": {}});
    let curated = vec![
        build_curated_skill(
            "oc_calculator",
            "Calculator",
            "Evaluates an arithmetic expression and returns the numeric result.",
            "productivity",
            calculator_params,
        ),
        build_curated_skill(
            "oc_web_search",
            "Web Search",
            "Search the web via privacy-respecting engines.",
            "web",
            empty_params.clone(),
        ),
        build_curated_skill(
            "oc_web_fetch",
            "Web Fetch",
            "Fetch and extract content from web pages.",
            "web",
            empty_params,
        ),
    ];

    for skill_descriptor in &curated {
        if registry.get_skill(&skill_descriptor.skill_id).is_err() {
            // Convert SkillDescriptor to SkillMetadata for A5 registry
            let metadata = SkillMetadata {
                skill_id: skill_descriptor.skill_id.clone(),
                name: skill_descriptor.name.clone(),
                description: skill_descriptor.description.clone(),
                publisher: "KRIA".to_string(),
                version: "1.0.0".to_string(),
                category: skill_descriptor.category.clone(),
                discovery_source: DiscoverySource::Bundled {
                    path: "bundled".to_string(),
                },
                discovered_at: chrono::Utc::now(),
                capabilities: skill_descriptor.capabilities.clone(),
                runtime_requirements: "{}".to_string(),
                risk_level: skill_descriptor.risk_level,
                resource_class: skill_descriptor.resource_profile.resource_class,
                tags: vec![],
                categories: vec![skill_descriptor.category.clone()],
                semantic_version: "1.0.0".to_string(),
                dependencies: vec![],
                compatibility_requirements: vec![],
                trust_tier: skill_descriptor.trust_tier,
                content_hash: "".to_string(),
                signature: None,
                granted_capabilities: skill_descriptor.granted.clone(),
                bundle_path: None,
                manifest_toml: None,
                // Seed the real input schema so schema-driven argument
                // generation (RC1) has it even before any container tools/list
                // sync runs. `{}`-only params become None (see helper semantics).
                input_schema: if skill_descriptor.parameters.get("properties").is_some()
                    || skill_descriptor.parameters.get("type").is_some()
                {
                    Some(skill_descriptor.parameters.clone())
                } else {
                    None
                },
                state: SkillState::Enabled,
                state_changed_at: chrono::Utc::now(),
            };

            if let Err(e) = registry.install_skill(&metadata) {
                tracing::warn!(
                    "Failed to seed curated skill {}: {e}",
                    skill_descriptor.skill_id
                );
            }
        }
    }
}

fn build_curated_skill(
    skill_id: &str,
    name: &str,
    description: &str,
    category: &str,
    parameters: serde_json::Value,
) -> SkillDescriptor {
    let resource_profile = ResourceProfile::for_category(category);
    SkillDescriptor {
        skill_id: skill_id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        category: category.to_string(),
        parameters,
        risk_level: RiskLevel::Green,
        network_policy: if category == "web" {
            OpenClawNetworkPolicy::DomainAllowlist(vec!["*".to_string()])
        } else {
            OpenClawNetworkPolicy::None
        },
        resource_profile,
        capabilities: SkillCapabilities {
            network: category == "web",
            ..Default::default()
        },
        granted: Vec::new(),
        trust_tier: TrustTier::Verified,
        source: SkillSource::Bundled,
        installed_at: chrono::Utc::now(),
        last_used_at: None,
        use_count: 0,
        status: SkillStatus::Active,
    }
}
