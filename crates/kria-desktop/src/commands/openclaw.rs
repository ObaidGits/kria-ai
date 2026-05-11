use crate::commands::app_state::AppStateCell;
use kria_core::openclaw::clawhub::{ClawHubClient, RemoteSkillEntry};
use kria_core::openclaw::{SkillCapabilities, SkillDescriptor};
use serde::{Deserialize, Serialize};
use tauri::{command, State};

/// Frontend-facing substrate status payload.
#[derive(Debug, Clone, Serialize)]
pub struct SubstrateStatusPayload {
    pub status: String,
    pub details: String,
    pub active_invocations: u32,
    pub warm_pool_count: u32,
}

/// Lightweight skill card for the frontend marketplace view.
#[derive(Debug, Clone, Serialize)]
pub struct SkillCard {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub trust_tier: String,
    pub installed: bool,
    pub enabled: bool,
}

impl From<&SkillDescriptor> for SkillCard {
    fn from(sd: &SkillDescriptor) -> Self {
        Self {
            slug: sd.skill_id.clone(),
            name: sd.name.clone(),
            description: sd.description.clone(),
            category: sd.category.clone(),
            trust_tier: sd.trust_tier.as_str().to_string(),
            installed: true,
            enabled: sd.is_usable(),
        }
    }
}

impl From<&RemoteSkillEntry> for SkillCard {
    fn from(entry: &RemoteSkillEntry) -> Self {
        Self {
            slug: entry.slug.clone(),
            name: entry.name.clone(),
            description: entry.description.clone(),
            category: entry.category.clone(),
            trust_tier: entry.trust_tier.clone(),
            installed: false,
            enabled: false,
        }
    }
}

/// Extended remote skill card — carries manifest_url and capabilities_summary
/// for the permission modal.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteSkillCard {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub category: String,
    /// Always "community" for remote skills.
    pub trust_tier: String,
    pub version: String,
    pub manifest_url: String,
    pub capabilities_summary: Vec<String>,
    pub installed: bool,
}

impl RemoteSkillCard {
    fn from_entry(entry: &RemoteSkillEntry, installed: bool) -> Self {
        Self {
            slug: entry.slug.clone(),
            name: entry.name.clone(),
            description: entry.description.clone(),
            category: entry.category.clone(),
            trust_tier: "community".into(),
            version: entry.version.clone(),
            manifest_url: entry.manifest_url.clone(),
            capabilities_summary: entry.capabilities_summary.clone(),
            installed,
        }
    }
}

/// Install request from the frontend permission modal.
#[derive(Debug, Deserialize)]
pub struct RemoteInstallRequest {
    pub manifest_url: String,
    pub slug: String,
    /// User-approved capability set from the permission modal.
    /// Kept for future HITL policy enforcement; not yet validated against
    /// the transpiled descriptor.
    #[allow(dead_code)]
    pub approved_capabilities: Option<SkillCapabilities>,
}

/// List all installed skills from the live SQLite registry.
#[command]
pub fn clawhub_list_skills(state: State<'_, AppStateCell>) -> Result<Vec<SkillCard>, String> {
    let app = state.get().ok_or("runtime not ready")?;
    let skills = app
        .skill_registry
        .list_installed()
        .map_err(|e| e.to_string())?;
    Ok(skills.iter().map(SkillCard::from).collect())
}

/// Search installed skills by name/description substring.
/// Remote ClawHub search is intentionally omitted until a real endpoint exists.
#[command]
pub fn clawhub_search_skills(
    query: String,
    _category: Option<String>,
    _limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<Vec<SkillCard>, String> {
    let app = state.get().ok_or("runtime not ready")?;
    let all = app
        .skill_registry
        .list_installed()
        .map_err(|e| e.to_string())?;
    let q = query.to_lowercase();
    let matched: Vec<SkillCard> = all
        .iter()
        .filter(|s| {
            q.is_empty()
                || s.name.to_lowercase().contains(&q)
                || s.description.to_lowercase().contains(&q)
                || s.category.to_lowercase().contains(&q)
        })
        .map(SkillCard::from)
        .collect();
    Ok(matched)
}

/// Fetch skills from the remote GitHub registry index.
///
/// Returns remote entries enriched with `installed: true/false` by cross-
/// referencing the local registry. Passes through `query` and `category`
/// filters server-side (index is small enough to filter locally).
#[command]
pub async fn clawhub_fetch_remote_skills(
    query: String,
    category: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<Vec<RemoteSkillCard>, String> {
    let app = state.get().ok_or("runtime not ready")?;
    let cfg = app.config.read().await.openclaw.clone();
    let client = ClawHubClient::new(&cfg.registry.index_url, cfg.registry.allowed_hosts.clone());

    let entries = client
        .search_remote(&query, category.as_deref())
        .await
        .map_err(|e| e.to_string())?;

    let cards = entries
        .iter()
        .map(|e| {
            let installed = app.skill_registry.get(&e.slug).is_ok();
            RemoteSkillCard::from_entry(e, installed)
        })
        .collect();

    Ok(cards)
}

/// Install a skill from a remote manifest URL.
///
/// Full pipeline:
/// 1. Validate manifest URL via `DomainValidator` (HTTPS + allowlist).
/// 2. Download the raw `SKILL.md` (≤ 64 KiB).
/// 3. Transpile through `transpiler::transpile_skill()` — enforces safe name,
///    description, and capabilities. Sets `TrustTier::Community`.
/// 4. Verify network_domains against PSL via `DomainValidator`.
/// 5. Persist to `SkillRegistry`.
/// 6. Write HMAC-signed `SkillInstalled` entry to `AuditLedger`.
#[command]
pub async fn clawhub_install_skill(
    request: RemoteInstallRequest,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    use kria_core::openclaw::audit::AuditLedger;
    use kria_core::openclaw::clawhub::DomainValidator;
    use kria_core::openclaw::transpiler::transpile_skill;
    use kria_core::openclaw::types::{SkillSource, TrustTier};

    let app = state.get().ok_or("runtime not ready")?;

    // No-op if already installed.
    if app.skill_registry.get(&request.slug).is_ok() {
        return Ok(());
    }

    let cfg = app.config.read().await.openclaw.clone();

    // 1. Validate manifest URL.
    let validator = DomainValidator::new(cfg.registry.allowed_hosts.clone());
    validator
        .validate(&request.manifest_url)
        .map_err(|e| format!("URL rejected: {e}"))?;

    // 2. Download manifest.
    let client = ClawHubClient::new(&cfg.registry.index_url, cfg.registry.allowed_hosts.clone());
    let raw_manifest = client
        .download_skill_manifest(&request.manifest_url)
        .await
        .map_err(|e| format!("Download failed: {e}"))?;

    // 3. Transpile — enforces name/desc validation; assigns Community tier.
    let source = SkillSource::ClawHub {
        slug: request.slug.clone(),
        version: "remote".into(),
    };
    let mut descriptor = transpile_skill(&raw_manifest, source, false)
        .map_err(|e| format!("Transpile failed: {e}"))?;

    // 4. Security enforcement: remote skills are ALWAYS Community, never Verified.
    descriptor.trust_tier = TrustTier::Community;

    // 5. Validate declared network_domains via DomainValidator.
    if let kria_core::openclaw::types::OpenClawNetworkPolicy::DomainAllowlist(ref domains) =
        descriptor.network_policy
    {
        for domain in domains {
            let test_url = format!("https://{}/", domain);
            validator
                .validate(&test_url)
                .map_err(|e| format!("Network domain '{}' rejected: {e}", domain))?;
        }
    }

    // 6. Persist to registry.
    app.skill_registry
        .install(&descriptor)
        .map_err(|e| format!("Registry install failed: {e}"))?;

    // 7. Write HMAC-signed SkillInstalled audit entry.
    // AppState doesn't carry AuditLedger directly — open it from the default path.
    let data_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".kria");
    if let Ok(ledger) = kria_core::openclaw::audit::AuditLedger::open(
        &data_dir.join("skills.db"),
        b"kria-openclaw-dev-audit-key-0001".to_vec(),
    ) {
        let mut entry = AuditLedger::create_skill_install_entry(
            &descriptor.skill_id,
            &descriptor.name,
            descriptor.trust_tier.as_str(),
            &request.manifest_url,
        );
        entry.signature = ledger.sign_entry(&entry);
        let _ = ledger.append(&entry);
    }

    tracing::info!(
        skill_id = %descriptor.skill_id,
        trust_tier = %descriptor.trust_tier,
        source_url = %request.manifest_url,
        "[OpenClaw] remote skill installed"
    );

    Ok(())
}

/// Uninstall a skill from the registry.
#[command]
pub fn clawhub_uninstall_skill(
    skill_id: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let app = state.get().ok_or("runtime not ready")?;
    app.skill_registry
        .uninstall(&skill_id)
        .map_err(|e| e.to_string())
}

/// Toggle a skill enabled/disabled.
#[command]
pub fn clawhub_toggle_skill(
    skill_id: String,
    enabled: bool,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let app = state.get().ok_or("runtime not ready")?;
    app.skill_registry
        .toggle(&skill_id, enabled)
        .map_err(|e| e.to_string())
}

/// Return current substrate health — reads live pool counts when Docker is available.
#[command]
pub async fn openclaw_substrate_status(
    state: State<'_, AppStateCell>,
) -> Result<SubstrateStatusPayload, String> {
    let app = state.get().ok_or("runtime not ready")?;
    match &app.container_pool {
        Some(pool) => {
            let active = pool.active_count().await as u32;
            let warm = pool.warm_count_total().await as u32;
            let status = if active > 0 { "busy" } else { "running" };
            let details = format!(
                "Docker substrate healthy — {} active, {} warm",
                active, warm
            );
            Ok(SubstrateStatusPayload {
                status: status.into(),
                details,
                active_invocations: active,
                warm_pool_count: warm,
            })
        }
        None => Ok(SubstrateStatusPayload {
            status: "unavailable".into(),
            details: "Docker not detected — container substrate offline".into(),
            active_invocations: 0,
            warm_pool_count: 0,
        }),
    }
}

/// Drain and re-warm the container pool.
#[command]
pub async fn openclaw_substrate_restart(state: State<'_, AppStateCell>) -> Result<(), String> {
    let app = state.get().ok_or("runtime not ready")?;
    if let Some(pool) = &app.container_pool {
        pool.shutdown().await.map_err(|e| e.to_string())?;
        pool.initialize().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}
