//! Data-only configuration for the Capability Provider Platform.
//!
//! Loaded as the additive `[capability]` section of `kria_config.toml`. The
//! single most important default is [`CapabilityPlatformConfig::enabled`] =
//! `false` (the `capability_provider_platform_enabled` master flag): with the
//! flag OFF, KRIA behaves byte-for-byte as it does today (the existing CIL /
//! OpenClaw path) and none of the CPP wiring is reachable.
//!
//! Provider identity is data, not code: providers are listed under
//! `[[capability.providers]]` by open-vocabulary [`ProviderConfig::id`], so
//! enabling/adding a provider is a config change, never a KRIA-core edit. This
//! section is deliberately separate from the existing `[providers]`
//! (LLM model providers) to avoid any collision.

use serde::{Deserialize, Serialize};

/// Root config for the Capability Provider Platform (`[capability]`).
///
/// The derived [`Default`] gives `enabled = false` (the
/// `capability_provider_platform_enabled` master flag OFF) and no providers, so
/// an absent `[capability]` section preserves the current behavior exactly.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CapabilityPlatformConfig {
    /// Master flag (`capability_provider_platform_enabled`). Default `false`:
    /// flag-OFF is byte-for-byte the current behavior.
    pub enabled: bool,

    /// Configured capability providers, by open-vocabulary id. Empty by default.
    /// The set of providers is data — the platform never hardcodes a provider.
    pub providers: Vec<ProviderConfig>,
}

impl CapabilityPlatformConfig {
    /// Look up a configured provider by id.
    pub fn provider(&self, id: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// The ids of enabled providers.
    pub fn enabled_provider_ids(&self) -> Vec<String> {
        self.providers
            .iter()
            .filter(|p| p.enabled)
            .map(|p| p.id.clone())
            .collect()
    }
}

/// Configuration for one capability provider. Generic by design: `id` and
/// `kind` are open strings and `settings` is a free-form map, so a new provider
/// kind needs no new config type.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    /// Open-vocabulary provider id (e.g. `"openclaw"`, `"mcp:github"`).
    pub id: String,
    /// Whether this provider is enabled.
    pub enabled: bool,
    /// Open provider-kind hint used by the registry to pick an adapter
    /// (e.g. `"openclaw"`, `"mcp"`). Not a closed enum.
    pub kind: String,
    /// Free-form, provider-specific settings passed to the adapter. Opaque to
    /// KRIA-core.
    pub settings: serde_json::Map<String, serde_json::Value>,
}
