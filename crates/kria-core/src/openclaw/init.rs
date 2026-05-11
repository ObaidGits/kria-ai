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
use crate::openclaw::handler::OpenClawToolHandler;
use crate::openclaw::pool::ContainerPool;
use crate::openclaw::registry::SkillRegistry;
use crate::openclaw::types::*;
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolRegistry};
use std::path::Path;
use std::sync::Arc;

/// Holds the fully-initialized OpenClaw subsystem handles.
pub struct OpenClawSubsystem {
    pub registry: Arc<SkillRegistry>,
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

        // 1. Open SkillRegistry (creates `installed_skills` table synchronously)
        let registry = SkillRegistry::open(&db_path)
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

    /// Register all active skills as `oc_*` tool handlers in the `ToolRegistry`.
    ///
    /// Must be called after `ContainerPool` is available. Each active skill
    /// gets a `ToolDef` (for LLM schema) and an `OpenClawToolHandler` (for
    /// execution). Already-registered `oc_*` tools are silently skipped so
    /// this is safe to call multiple times (e.g. after install).
    pub fn register_into_tool_registry(
        &self,
        tool_registry: &ToolRegistry,
        pool: Arc<ContainerPool>,
    ) {
        let skills = match self.registry.list_active() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    "[OpenClaw] failed to list active skills for tool registration: {e}"
                );
                return;
            }
        };

        let mut registered = 0usize;
        for skill in skills {
            // Skip if already registered (idempotent).
            if tool_registry.get_def(&skill.skill_id).is_some() {
                continue;
            }

            // Build parameter defs from the skill's JSON schema.
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

            let def = ToolDef {
                name: skill.skill_id.clone(),
                description: skill.description.clone(),
                category: skill.category.clone(),
                parameters: params,
                default_tier: skill.risk_level,
                min_tier: "lite",
            };

            let handler = Arc::new(OpenClawToolHandler::new(
                skill,
                pool.clone(),
                self.audit.clone(),
            ));

            tool_registry.register(def, handler);
            registered += 1;
        }

        if registered > 0 {
            tracing::info!(
                count = registered,
                "[OpenClaw] registered oc_* tools in ToolRegistry"
            );
        }
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

/// Inject curated skills into the registry if they are not already present.
pub fn initialize_curated_skills(registry: &SkillRegistry) {
    let curated = vec![
        build_curated_skill(
            "oc_calculator",
            "Calculator",
            "Perform arithmetic, unit conversions, and math expressions.",
            "productivity",
        ),
        build_curated_skill(
            "oc_web_search",
            "Web Search",
            "Search the web via privacy-respecting engines.",
            "web",
        ),
        build_curated_skill(
            "oc_web_fetch",
            "Web Fetch",
            "Fetch and extract content from web pages.",
            "web",
        ),
    ];

    for skill in &curated {
        if registry.get(&skill.skill_id).is_err() {
            if let Err(e) = registry.install(skill) {
                tracing::warn!("Failed to seed curated skill {}: {e}", skill.skill_id);
            }
        }
    }
}

fn build_curated_skill(
    skill_id: &str,
    name: &str,
    description: &str,
    category: &str,
) -> SkillDescriptor {
    let resource_profile = ResourceProfile::for_category(category);
    SkillDescriptor {
        skill_id: skill_id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        category: category.to_string(),
        parameters: serde_json::json!({"type": "object", "properties": {}}),
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
        trust_tier: TrustTier::Verified,
        source: SkillSource::Bundled,
        installed_at: chrono::Utc::now(),
        last_used_at: None,
        use_count: 0,
        status: SkillStatus::Active,
    }
}
