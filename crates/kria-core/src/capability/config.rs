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

    /// Capability-intelligence layer flags (CKB, reasoner, planner, lifecycle,
    /// evolution, synthesis, ...). All OFF by default so flag-off is byte-for-byte
    /// the current behavior (spec R2/R10.3; OpenClaw Intelligence Enhancements P0).
    pub intelligence: CapabilityIntelligenceConfig,
}

/// Feature flags for the capability-intelligence layer. Every flag defaults to
/// `false`; with all OFF, KRIA behaves exactly as the pre-enhancement CPP + the
/// existing execution runtimes (flag-off parity, spec Property 1).
///
/// These are additive and orthogonal so phases can be enabled independently in
/// the documented dependency order (CKB → reasoner → planner → lifecycle → ...).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CapabilityIntelligenceConfig {
    /// P1: persistent Capability Knowledge Base (durable learned layer).
    pub ckb: bool,
    /// P2: Capability Reasoner (goal taxonomy, inference, strategy, confidence).
    pub reasoner: bool,
    /// P2: Cost Model (latency/resource/token/money estimation + calibration).
    pub cost_model: bool,
    /// P3: Capability Planner + single planning authority/arbitration.
    pub planner: bool,
    /// P4: full Lifecycle Manager (verify/smoke/upgrade/replace/rollback/retire).
    pub lifecycle: bool,
    /// P5: plan-level effects + plan permission.
    pub plan_permission: bool,
    /// P6: marketplace intelligence v2 (neutral ranking, integrity, versioning).
    pub marketplace_v2: bool,
    /// P8: Evolution Engine (benchmark-driven, gated, reversible).
    pub evolution: bool,
    /// P8: Benchmark Harness (golden/synthetic proxy scoring).
    pub benchmark: bool,
    /// P9: Capability Synthesis (generation) provider.
    pub synthesis: bool,
    /// P9: LLM-assisted IR proposer for synthesis (spec R7.1/R3.4). When OFF
    /// (default), synthesis uses the deterministic goal→IR proposer. When ON, an
    /// LLM proposes the IR but the validator + golden gate still own correctness
    /// (a bad model falls back to deterministic — never fabricates). Requires
    /// `synthesis` to also be enabled.
    pub synthesis_llm: bool,
    /// P9: Tier-3 generated-code node (raw code compiled + run in the seccomp-
    /// bound Docker sandbox). OFF by default and gated separately because it is
    /// the only synthesis tier that depends on a reliable code-generation model.
    pub synthesis_code: bool,
    /// P10: background Continuous Discovery & Maintenance loop.
    pub continuous_discovery: bool,
    /// P4/P11: long-running / resumable jobs.
    pub jobs: bool,
    /// Cross-cutting: the neutral prompt-intent / planning-arbitration gate.
    pub routing_gate: bool,
    /// P8: autonomy level governing when evolution/discovery act without asking
    /// (spec R29.2): `manual` | `propose_only` | `auto_with_notice` | `full_auto`.
    /// Conservative default: `propose_only` (never auto-apply elevated actions).
    #[serde(default = "default_autonomy_level")]
    pub autonomy_level: String,
}

fn default_autonomy_level() -> String {
    "propose_only".to_string()
}

#[cfg(test)]
mod flag_tests {
    use super::*;

    #[test]
    fn default_intelligence_config_is_all_disabled_including_synthesis_flags() {
        // Flag-off parity (spec Property 1): the default must report all-disabled
        // so byte-identical legacy behavior holds — including the Wave 9
        // synthesis_llm + synthesis_code toggles added in this wave.
        let cfg = CapabilityIntelligenceConfig::default();
        assert!(cfg.all_disabled());
        assert!(!cfg.synthesis_llm);
        assert!(!cfg.synthesis_code);
        // Turning on either Wave-9 toggle breaks all_disabled (they are toggles).
        let mut c2 = CapabilityIntelligenceConfig::default();
        c2.synthesis_llm = true;
        assert!(!c2.all_disabled());
        let mut c3 = CapabilityIntelligenceConfig::default();
        c3.synthesis_code = true;
        assert!(!c3.all_disabled());
    }
}

impl CapabilityIntelligenceConfig {
    /// True when no intelligence FEATURE toggle is enabled (pure legacy CPP
    /// path). `autonomy_level` is an operational setting, not a feature toggle,
    /// so it does not affect this — flag-off parity depends only on the toggles.
    pub fn all_disabled(&self) -> bool {
        !self.ckb
            && !self.reasoner
            && !self.cost_model
            && !self.planner
            && !self.lifecycle
            && !self.plan_permission
            && !self.marketplace_v2
            && !self.evolution
            && !self.benchmark
            && !self.synthesis
            && !self.synthesis_llm
            && !self.synthesis_code
            && !self.continuous_discovery
            && !self.jobs
            && !self.routing_gate
    }
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
