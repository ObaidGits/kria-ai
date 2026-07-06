//! Capability Intelligence Layer (CIL) configuration — **data only**.
//!
//! Per the ICP design (§8.4, §8.8) and the *no-hardcoding* invariant, every
//! threshold and weight the CIL uses to discover, rank, acquire, and plan is a
//! **config value**, never a constant baked into code. This module defines that
//! configuration surface: [`CilConfig`] plus the multi-signal [`RankWeights`].
//!
//! # Wiring
//!
//! [`CilConfig`] is an **additive** section nested under the existing
//! `[openclaw]` config (see [`crate::openclaw::config::OpenClawConfig`]), so it
//! loads from `kria_config.toml` as `[openclaw.cil]` without changing any
//! existing key. Every field carries a `#[serde(default)]` (via the struct-level
//! attribute) so the whole section is optional and forward-compatible: an old
//! config file with no `[openclaw.cil]` block deserializes to the defaults
//! below, and the defaults preserve current behavior byte-for-byte
//! (`openclaw_icp_enabled = false`).
//!
//! # Flag parity
//!
//! The single most important default is [`CilConfig::openclaw_icp_enabled`] =
//! `false`. With the flag OFF, `SemanticOpenClawHandler::execute_semantic` MUST
//! produce output identical to the current direct-router path. None of the other
//! values here take effect until the flag is turned ON in a later phase.

use serde::{Deserialize, Serialize};

/// Multi-signal ranking weights (design §8.4).
///
/// The [`crate::openclaw::cil`] `CapabilityRanker` combines these signals into a
/// single score. They are **data, not code**: no per-skill or per-category
/// branch may substitute for tuning these weights. Each field is a relative
/// weight in `0.0..` applied to a signal normalized to `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RankWeights {
    /// Dense goal↔capability semantic similarity.
    pub semantic: f32,
    /// Lexical (frozen BM25) overlap.
    pub lexical: f32,
    /// I/O type fit + runtime requirement + dependency satisfiability.
    pub compatibility: f32,
    /// Publisher/trust-tier signal.
    pub trust: f32,
    /// Validator / marketplace quality signal.
    pub quality: f32,
    /// Install / usage popularity (from `SkillStatistics`).
    pub popularity: f32,
    /// Historical success rate (from `SkillStatistics`).
    pub success: f32,
}

impl Default for RankWeights {
    fn default() -> Self {
        // Sums to 1.0; semantic-led with compatibility as the second signal.
        // These are starting weights, tunable via `[openclaw.cil.weights]`.
        Self {
            semantic: 0.35,
            lexical: 0.15,
            compatibility: 0.20,
            trust: 0.10,
            quality: 0.08,
            popularity: 0.06,
            success: 0.06,
        }
    }
}

/// Data-only configuration for the Capability Intelligence Layer.
///
/// Loaded as the additive `[openclaw.cil]` section of `kria_config.toml`. All
/// behavior-affecting thresholds, weights, and caps live here so the CIL has
/// **no hardcoded constants** governing discovery, ranking, acquisition, or
/// planning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CilConfig {
    /// Master feature flag for the OpenClaw ICP. Default `false`: with the flag
    /// OFF, the handler's behavior is byte-for-byte the current direct-router
    /// path (design §7.2 / Property 11).
    pub openclaw_icp_enabled: bool,

    /// Multi-signal ranking weights (design §8.4).
    pub weights: RankWeights,

    /// Minimum trust score (`0.0..=1.0`) a candidate must meet to be accepted
    /// for automatic acquisition/selection. Candidates below this are declined
    /// or surfaced only as recommendations.
    pub trust_threshold: f32,

    /// Minimum compatibility score (`0.0..=1.0`) a candidate must meet to be
    /// considered a usable match for a required capability.
    pub compatibility_threshold: f32,

    /// Maximum plan breadth (fan-out) the `CapabilityPlanner` may emit before it
    /// reduces or rejects the plan (design §Requirement 3.5 / 11.5).
    pub planner_max_breadth: usize,

    /// Maximum plan depth (composition chain length) the `CapabilityPlanner` may
    /// emit before it reduces or rejects the plan.
    pub planner_max_depth: usize,

    /// Whether A9 skill generation is allowed as an acquisition fallback when no
    /// acceptable marketplace candidate exists. Default `false` (conservative):
    /// generation must be explicitly enabled.
    pub generation_allowed: bool,
}

impl Default for CilConfig {
    fn default() -> Self {
        Self {
            openclaw_icp_enabled: false,
            weights: RankWeights::default(),
            trust_threshold: 0.5,
            compatibility_threshold: 0.5,
            planner_max_breadth: 8,
            planner_max_depth: 5,
            generation_allowed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::config::OpenClawConfig;

    /// Safety default (Property 11): the master flag is OFF by default so the
    /// handler's behavior stays byte-for-byte the current direct-router path.
    #[test]
    fn default_flag_is_off() {
        assert!(
            !CilConfig::default().openclaw_icp_enabled,
            "openclaw_icp_enabled MUST default to false (flag OFF)"
        );
    }

    /// The additive `[openclaw.cil]` section is optional: an `[openclaw]` config
    /// with no `cil` block deserializes to the CIL defaults (flag OFF), so old
    /// configs keep current behavior.
    #[test]
    fn missing_cil_section_falls_back_to_defaults() {
        let toml_src = r#"
            enabled = true
            image = "kria/openclaw-substrate:latest"
        "#;
        let cfg: OpenClawConfig = toml::from_str(toml_src).expect("deserialize [openclaw]");
        assert_eq!(cfg.cil, CilConfig::default());
        assert!(
            !cfg.cil.openclaw_icp_enabled,
            "flag defaults OFF when absent"
        );
    }

    /// Weights, thresholds, planner caps, and the generation flag all load from a
    /// `kria_config.toml`-style `[openclaw.cil]` section — proving they are
    /// config *data*, not hardcoded constants. Every asserted value is chosen to
    /// differ from [`CilConfig::default`] so a hardcoded constant would fail.
    #[test]
    fn cil_section_loads_weights_thresholds_and_caps_from_config() {
        // TOML mirrors the `[openclaw.cil]` nesting under `[openclaw]`.
        let toml_src = r#"
            [cil]
            openclaw_icp_enabled = true
            trust_threshold = 0.8
            compatibility_threshold = 0.65
            planner_max_breadth = 16
            planner_max_depth = 9
            generation_allowed = true

            [cil.weights]
            semantic = 0.5
            lexical = 0.1
            compatibility = 0.2
            trust = 0.05
            quality = 0.05
            popularity = 0.05
            success = 0.05
        "#;
        let cfg: OpenClawConfig = toml::from_str(toml_src).expect("deserialize [openclaw.cil]");
        let cil = &cfg.cil;

        // Flag + scalar thresholds/caps loaded from config (all differ from defaults).
        assert!(cil.openclaw_icp_enabled);
        assert_eq!(cil.trust_threshold, 0.8);
        assert_eq!(cil.compatibility_threshold, 0.65);
        assert_eq!(cil.planner_max_breadth, 16);
        assert_eq!(cil.planner_max_depth, 9);
        assert!(cil.generation_allowed);

        // Ranking weights loaded from the nested `[openclaw.cil.weights]` table.
        assert_eq!(cil.weights.semantic, 0.5);
        assert_eq!(cil.weights.lexical, 0.1);
        assert_eq!(cil.weights.compatibility, 0.2);
        assert_eq!(cil.weights.trust, 0.05);
        assert_eq!(cil.weights.quality, 0.05);
        assert_eq!(cil.weights.popularity, 0.05);
        assert_eq!(cil.weights.success, 0.05);

        // No-hardcoding guard: the loaded config is NOT the default constant.
        assert_ne!(*cil, CilConfig::default());
        assert_ne!(cil.weights, RankWeights::default());
    }

    /// `CilConfig` deserializes directly from a bare `[openclaw.cil]`-style table
    /// as well, and partial tables fall back to per-field defaults (forward
    /// compatibility via `#[serde(default)]`).
    #[test]
    fn cil_config_partial_table_uses_field_defaults() {
        // Only override the flag; everything else must take its default value.
        let cil: CilConfig = toml::from_str("openclaw_icp_enabled = true\n").expect("deserialize");
        assert!(cil.openclaw_icp_enabled);
        assert_eq!(cil.trust_threshold, CilConfig::default().trust_threshold);
        assert_eq!(cil.weights, RankWeights::default());
    }
}
