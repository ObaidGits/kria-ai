//! OpenClaw substrate configuration.

use serde::{Deserialize, Serialize};

/// OpenClaw integration configuration.
///
/// Lives under `[openclaw]` in `~/.kria/config.toml`.
/// Disabled by default — user must explicitly enable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenClawConfig {
    /// Whether the OpenClaw substrate is enabled.
    pub enabled: bool,
    /// Docker image to use for the substrate container.
    pub image: String,
    /// Container name prefix.
    pub container_name: String,
    /// Default memory limit for containers.
    pub default_memory_limit: String,
    /// Default CPU limit for containers.
    pub default_cpu_limit: String,
    /// Default tool timeout in seconds.
    pub default_timeout_secs: u64,
    /// Maximum output bytes from a single tool invocation.
    pub max_output_bytes: usize,
    /// Maximum OpenClaw tools exposed per turn (anti-tool-soup).
    pub max_tools_per_turn: usize,
    /// Minimum similarity threshold for skill matching.
    pub similarity_threshold: f32,
    /// Maximum container restart attempts before disabling substrate.
    pub max_restart_attempts: u32,
    /// Egress proxy port.
    pub egress_proxy_port: u16,
    /// Number of pre-warmed containers per resource class.
    pub warm_per_class: usize,
    /// Maximum age (seconds) of a warm container before recycling.
    pub max_warm_age_secs: u64,
    /// Maximum concurrent tool invocations.
    pub max_concurrent_invocations: usize,
    /// Whether to rewrite skill descriptions via local LLM at install time.
    pub rewrite_descriptions: bool,
    /// Trust tier configuration.
    pub trust: TrustConfig,
    /// Lifecycle policy.
    pub lifecycle: LifecycleConfig,
    /// Remote registry configuration.
    pub registry: RegistryConfig,
    /// Capability Intelligence Layer (ICP) configuration.
    ///
    /// Additive `[openclaw.cil]` section (data-only). Gated behind
    /// `cil.openclaw_icp_enabled` (default `false`), which preserves current
    /// behavior byte-for-byte. See [`crate::openclaw::cil::CilConfig`].
    pub cil: crate::openclaw::cil::CilConfig,
}

impl Default for OpenClawConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            image: "kria/openclaw-substrate:latest".into(),
            container_name: "kria-openclaw-substrate".into(),
            default_memory_limit: "512m".into(),
            default_cpu_limit: "1.0".into(),
            default_timeout_secs: 30,
            max_output_bytes: 1024 * 1024, // 1MB
            max_tools_per_turn: 8,
            similarity_threshold: 0.70,
            max_restart_attempts: 3,
            egress_proxy_port: 18800,
            warm_per_class: 2,
            max_warm_age_secs: 300,
            max_concurrent_invocations: 4,
            rewrite_descriptions: true,
            trust: TrustConfig::default(),
            lifecycle: LifecycleConfig::default(),
            registry: RegistryConfig::default(),
            cil: crate::openclaw::cil::CilConfig::default(),
        }
    }
}

/// Trust tier configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TrustConfig {
    /// Whether to allow Community-tier skills to request network access.
    pub community_allows_network: bool,
    /// Whether Verified skills skip HITL approval.
    pub verified_skips_hitl: bool,
    /// Default trust tier for skills installed from unknown sources.
    pub default_unknown_tier: String,
}

impl Default for TrustConfig {
    fn default() -> Self {
        Self {
            community_allows_network: true,
            verified_skips_hitl: true,
            default_unknown_tier: "local".into(),
        }
    }
}

/// Skill lifecycle configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LifecycleConfig {
    /// Days since last use before a skill is flagged as stale.
    pub stale_after_days: u32,
    /// Days since last use before a skill is auto-disabled.
    pub auto_disable_after_days: u32,
    /// Whether to periodically check for skill updates.
    pub check_updates: bool,
}

/// Remote registry configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RegistryConfig {
    /// URL of the remote index.json.
    pub index_url: String,
    /// Extra download hosts permitted beyond the built-in allowlist.
    pub allowed_hosts: Vec<String>,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            index_url: crate::openclaw::clawhub::DEFAULT_REGISTRY_URL.to_string(),
            allowed_hosts: Vec::new(),
        }
    }
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            stale_after_days: 30,
            auto_disable_after_days: 90,
            check_updates: true,
        }
    }
}
