//! Core types for the OpenClaw skill substrate.
//!
//! These types define the contract between KRIA's sovereign core and the
//! OpenClaw execution sandbox. They are designed to be:
//! - Serializable for SQLite persistence
//! - Serializable for LLM function-calling schema generation
//! - Auditable via the `AuditLedger`

use crate::safety::RiskLevel;
use serde::{Deserialize, Serialize};

// ─── Trust Tiers ──────────────────────────────────────────────────────────────

/// Trust tiers for skills. Ordered from most to least trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TrustTier {
    /// Curated by KRIA team. Verified source code, tested, signed.
    Verified,
    /// Community skill with positive reputation on ClawHub.
    Community,
    /// User-installed from local path or unknown source.
    Local,
    /// Explicitly untrusted (known security concerns).
    Untrusted,
}

impl TrustTier {
    /// Maximum resource class allowed per trust tier.
    pub fn max_resource_class(&self) -> ResourceClass {
        match self {
            Self::Verified => ResourceClass::Heavy,
            Self::Community => ResourceClass::Medium,
            Self::Local => ResourceClass::Light,
            Self::Untrusted => ResourceClass::Light,
        }
    }

    /// Whether the skill can request network access.
    pub fn allows_network(&self) -> bool {
        matches!(self, Self::Verified | Self::Community)
    }

    /// Whether the skill needs HITL approval to install.
    pub fn requires_hitl_approval(&self) -> bool {
        !matches!(self, Self::Verified)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Community => "community",
            Self::Local => "local",
            Self::Untrusted => "untrusted",
        }
    }
}

impl std::fmt::Display for TrustTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for TrustTier {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "verified" => Ok(Self::Verified),
            "community" => Ok(Self::Community),
            "local" => Ok(Self::Local),
            "untrusted" => Ok(Self::Untrusted),
            _ => Err(format!("unknown trust tier: {}", s)),
        }
    }
}

// ─── Resource Classes ─────────────────────────────────────────────────────────

/// Resource classes for container sizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ResourceClass {
    /// 256MB, 0.5 CPU — web search, productivity tools.
    Light,
    /// 512MB, 1.0 CPU — general tools.
    Medium,
    /// 2GB, 2.0 CPU — media generation, code compilation.
    Heavy,
}

impl ResourceClass {
    pub fn memory_limit(&self) -> &'static str {
        match self {
            Self::Light => "256m",
            Self::Medium => "512m",
            Self::Heavy => "2g",
        }
    }

    pub fn cpu_limit(&self) -> &'static str {
        match self {
            Self::Light => "0.5",
            Self::Medium => "1.0",
            Self::Heavy => "2.0",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Medium => "medium",
            Self::Heavy => "heavy",
        }
    }
}

impl std::fmt::Display for ResourceClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ResourceClass {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "light" => Ok(Self::Light),
            "medium" => Ok(Self::Medium),
            "heavy" => Ok(Self::Heavy),
            _ => Err(format!("unknown resource class: {}", s)),
        }
    }
}

// ─── Resource Profile ─────────────────────────────────────────────────────────

/// Per-skill resource profile, approved during installation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceProfile {
    /// Memory limit string (e.g., "256m", "2g").
    pub memory_limit: String,
    /// CPU limit string (e.g., "0.5", "2.0").
    pub cpu_limit: String,
    /// Timeout per invocation in seconds.
    pub timeout_secs: u64,
    /// Maximum output bytes.
    pub max_output_bytes: usize,
    /// Whether this profile requires elevated HITL approval.
    pub requires_approval: bool,
    /// The resource class this profile maps to.
    pub resource_class: ResourceClass,
}

impl ResourceProfile {
    /// Create a default profile for a skill category.
    pub fn for_category(category: &str) -> Self {
        match category {
            "web" | "search" | "productivity" => Self {
                memory_limit: "256m".into(),
                cpu_limit: "0.5".into(),
                timeout_secs: 30,
                max_output_bytes: 512 * 1024,
                requires_approval: false,
                resource_class: ResourceClass::Light,
            },
            "media" | "music" | "video" | "image" => Self {
                memory_limit: "2g".into(),
                cpu_limit: "2.0".into(),
                timeout_secs: 120,
                max_output_bytes: 10 * 1024 * 1024,
                requires_approval: true,
                resource_class: ResourceClass::Heavy,
            },
            "code" | "compilation" => Self {
                memory_limit: "1g".into(),
                cpu_limit: "2.0".into(),
                timeout_secs: 60,
                max_output_bytes: 2 * 1024 * 1024,
                requires_approval: true,
                resource_class: ResourceClass::Heavy,
            },
            _ => Self {
                memory_limit: "512m".into(),
                cpu_limit: "1.0".into(),
                timeout_secs: 30,
                max_output_bytes: 1024 * 1024,
                requires_approval: false,
                resource_class: ResourceClass::Medium,
            },
        }
    }
}

// ─── Network Policy ───────────────────────────────────────────────────────────

/// Network policy for OpenClaw tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OpenClawNetworkPolicy {
    /// No network access (default).
    None,
    /// Allowlist of domains the tool may access.
    DomainAllowlist(Vec<String>),
    /// Full network access (requires RED-level approval).
    Unrestricted,
}

impl OpenClawNetworkPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::DomainAllowlist(_) => "allowlist",
            Self::Unrestricted => "unrestricted",
        }
    }
}

// ─── Skill Capabilities ──────────────────────────────────────────────────────

/// What a skill needs from the sandbox environment.
/// Declared in YAML frontmatter, validated against runtime behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillCapabilities {
    /// Needs filesystem read access.
    pub filesystem_read: bool,
    /// Needs filesystem write access.
    pub filesystem_write: bool,
    /// Needs subprocess/shell execution.
    pub subprocess: bool,
    /// Needs browser automation.
    pub browser: bool,
    /// Needs network access.
    pub network: bool,
    /// Specific domains needed (if network=true).
    pub network_domains: Vec<String>,
    /// Needs image generation.
    pub image_generation: bool,
    /// Needs audio/media processing.
    pub media: bool,
}

impl Default for SkillCapabilities {
    fn default() -> Self {
        Self {
            filesystem_read: false,
            filesystem_write: false,
            subprocess: false,
            browser: false,
            network: false,
            network_domains: Vec::new(),
            image_generation: false,
            media: false,
        }
    }
}

impl SkillCapabilities {
    /// Derive network policy from capabilities.
    pub fn to_network_policy(&self) -> OpenClawNetworkPolicy {
        if !self.network {
            OpenClawNetworkPolicy::None
        } else if self.network_domains.is_empty() {
            OpenClawNetworkPolicy::Unrestricted
        } else {
            OpenClawNetworkPolicy::DomainAllowlist(self.network_domains.clone())
        }
    }

    /// Derive risk level from capabilities.
    /// This is KRIA's assessment — never trust the skill author's claim.
    pub fn classify_risk(&self) -> RiskLevel {
        if self.subprocess || self.filesystem_write {
            RiskLevel::Red
        } else if self.network || self.browser || self.image_generation {
            RiskLevel::Yellow
        } else {
            RiskLevel::Green
        }
    }
}

// ─── Skill Source ─────────────────────────────────────────────────────────────

/// Where a skill was installed from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkillSource {
    /// Installed from ClawHub (slug + version).
    ClawHub { slug: String, version: String },
    /// Installed from a local path.
    Local { path: String },
    /// Bundled with KRIA.
    Bundled,
}

impl SkillSource {
    pub fn as_str(&self) -> String {
        match self {
            Self::ClawHub { slug, version } => format!("clawhub:{}@{}", slug, version),
            Self::Local { path } => format!("local:{}", path),
            Self::Bundled => "bundled".into(),
        }
    }
}

// ─── Skill Status ─────────────────────────────────────────────────────────────

/// Runtime status of an installed skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillStatus {
    /// Active and available for use.
    Active,
    /// Disabled by user or lifecycle policy.
    Disabled,
    /// Auto-disabled due to staleness.
    StaleDisabled,
    /// Pending HITL approval.
    PendingApproval,
    /// Quarantined due to failures.
    Quarantined,
}

impl SkillStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::StaleDisabled => "stale_disabled",
            Self::PendingApproval => "pending_approval",
            Self::Quarantined => "quarantined",
        }
    }
}

impl std::str::FromStr for SkillStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "disabled" => Ok(Self::Disabled),
            "stale_disabled" => Ok(Self::StaleDisabled),
            "pending_approval" => Ok(Self::PendingApproval),
            "quarantined" => Ok(Self::Quarantined),
            _ => Err(format!("unknown skill status: {}", s)),
        }
    }
}

// ─── Skill Descriptor ─────────────────────────────────────────────────────────

/// A transpiled, safe representation of an OpenClaw skill.
///
/// This is what KRIA's LLM sees — never raw SKILL.md markdown.
/// Descriptions are rewritten by KRIA's local LLM at installation time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDescriptor {
    /// Unique skill identifier (e.g., "oc_web_search").
    pub skill_id: String,
    /// Human-readable name.
    pub name: String,
    /// Rewritten description (safe verb-noun sentence, max 100 chars).
    pub description: String,
    /// Skill category (e.g., "web", "media", "productivity").
    pub category: String,
    /// Tool parameter schema (JSON Schema format for LLM function calling).
    pub parameters: serde_json::Value,
    /// Risk level assigned by KRIA's policy engine.
    pub risk_level: RiskLevel,
    /// Network policy required by this skill.
    pub network_policy: OpenClawNetworkPolicy,
    /// Resource profile (memory, CPU, timeout).
    pub resource_profile: ResourceProfile,
    /// Capability requirements.
    pub capabilities: SkillCapabilities,
    /// Trust tier.
    pub trust_tier: TrustTier,
    /// Source (ClawHub, local, bundled).
    pub source: SkillSource,
    /// Installation timestamp.
    pub installed_at: chrono::DateTime<chrono::Utc>,
    /// Last time this skill was used.
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Number of times this skill has been invoked.
    pub use_count: u64,
    /// Current status.
    pub status: SkillStatus,
}

impl SkillDescriptor {
    /// Convert to a ToolDef-compatible JSON schema for LLM function calling.
    pub fn to_tool_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.skill_id,
                "description": self.description,
                "parameters": self.parameters,
            }
        })
    }

    /// Whether this skill is currently usable.
    pub fn is_usable(&self) -> bool {
        matches!(self.status, SkillStatus::Active)
    }
}

// ─── Execution Source ─────────────────────────────────────────────────────────

/// Where a tool execution originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionSource {
    /// KRIA's native Rust tools.
    Native,
    /// MCP server tools.
    Mcp,
    /// OpenClaw sandbox skills.
    OpenClaw,
    /// Cloud API tools.
    Cloud,
}

impl ExecutionSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Mcp => "mcp",
            Self::OpenClaw => "openclaw",
            Self::Cloud => "cloud",
        }
    }

    /// Trust level for evidence wrapping.
    pub fn trust_label(&self) -> &'static str {
        match self {
            Self::Native => "trusted",
            Self::Mcp => "semi-trusted",
            Self::OpenClaw => "untrusted",
            Self::Cloud => "untrusted",
        }
    }
}

// ─── Audit Event Types ────────────────────────────────────────────────────────

/// Types of events recorded in the audit ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditEventType {
    SkillInstalled,
    SkillUpdated,
    SkillUninstalled,
    SkillAutoDisabled,
    InvocationStarted,
    InvocationCompleted,
    InvocationFailed,
    ContainerRecycled,
    SecurityEvent,
    PolicyViolation,
}

impl AuditEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SkillInstalled => "skill_installed",
            Self::SkillUpdated => "skill_updated",
            Self::SkillUninstalled => "skill_uninstalled",
            Self::SkillAutoDisabled => "skill_auto_disabled",
            Self::InvocationStarted => "invocation_started",
            Self::InvocationCompleted => "invocation_completed",
            Self::InvocationFailed => "invocation_failed",
            Self::ContainerRecycled => "container_recycled",
            Self::SecurityEvent => "security_event",
            Self::PolicyViolation => "policy_violation",
        }
    }
}

impl std::str::FromStr for AuditEventType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "skill_installed" => Ok(Self::SkillInstalled),
            "skill_updated" => Ok(Self::SkillUpdated),
            "skill_uninstalled" => Ok(Self::SkillUninstalled),
            "skill_auto_disabled" => Ok(Self::SkillAutoDisabled),
            "invocation_started" => Ok(Self::InvocationStarted),
            "invocation_completed" => Ok(Self::InvocationCompleted),
            "invocation_failed" => Ok(Self::InvocationFailed),
            "container_recycled" => Ok(Self::ContainerRecycled),
            "security_event" => Ok(Self::SecurityEvent),
            "policy_violation" => Ok(Self::PolicyViolation),
            _ => Err(format!("unknown audit event type: {}", s)),
        }
    }
}

// ─── Lifecycle Actions ────────────────────────────────────────────────────────

/// Actions taken by the lifecycle maintenance process.
#[derive(Debug, Clone)]
pub enum LifecycleAction {
    AutoDisabled {
        skill_id: String,
        days_unused: i64,
    },
    FlaggedStale {
        skill_id: String,
        days_unused: i64,
    },
    UpdateAvailable {
        skill_id: String,
        new_version: String,
    },
}

// ─── Skill Update Diff ───────────────────────────────────────────────────────

/// Shows what changed when a skill is updated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillUpdateDiff {
    pub skill_id: String,
    pub old_version: String,
    pub new_version: String,
    pub capability_changes: Vec<CapabilityChange>,
    pub resource_changes: Vec<ResourceChange>,
    pub requires_reapproval: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CapabilityChange {
    Added { capability: String },
    Removed { capability: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResourceChange {
    MemoryIncreased { from: String, to: String },
    MemoryDecreased { from: String, to: String },
    CpuChanged { from: String, to: String },
    TimeoutChanged { from_secs: u64, to_secs: u64 },
}

// ─── Transpile Errors ─────────────────────────────────────────────────────────

/// Errors during SKILL.md transpilation.
#[derive(Debug, thiserror::Error)]
pub enum TranspileError {
    #[error("no YAML frontmatter found (missing --- delimiters)")]
    NoFrontmatter,
    #[error("invalid YAML in frontmatter: {0}")]
    InvalidYaml(#[from] serde_yaml::Error),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("invalid skill name (must be alphanumeric + underscore, max 64 chars)")]
    InvalidName,
    #[error("invalid description (max 200 chars, no control characters)")]
    InvalidDescription,
    #[error("description rewrite failed")]
    DescriptionRewriteFailed,
    #[error("invalid parameter schema: {0}")]
    InvalidParameters(String),
    #[error("trust tier '{0}' does not allow resource class '{1}'")]
    TrustTierViolation(String, String),
}
