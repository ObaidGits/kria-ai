//! Dynamic VRAM threshold computation — replaces hardcoded MB constants
//! with percentage-based scaling derived from total GPU memory at boot.
//!
//! # Why
//! Hardcoded thresholds (e.g. `yield = 512 MB`, `emergency = 128 MB`) break
//! across GPU tiers:
//! - 4 GB GPU: 512 MB = 12.5% — reasonable
//! - 12 GB GPU: 512 MB = 4.2% — dangerously tight
//! - 24 GB GPU: 512 MB = 2.1% — essentially no headroom
//!
//! Percentage-based thresholds auto-scale to the detected hardware.

use crate::config::OrchestratorConfig;

/// Pre-computed VRAM thresholds scaled to the detected GPU's total memory.
///
/// Created once during orchestrator boot from the initial telemetry snapshot.
/// The watchdog loop reads these instead of the raw config values.
#[derive(Debug, Clone, Copy)]
pub struct ThresholdProfile {
    /// Free VRAM (MB) below which a yield/downshift swap is considered.
    pub yield_mb: u64,
    /// Free VRAM (MB) below which an emergency swap fires.
    pub emergency_mb: u64,
    /// Free VRAM (MB) above which recovery/upscaling is attempted.
    pub recover_mb: u64,
    /// Deadband (MB) required above a threshold to exit that state,
    /// preventing oscillation at the boundary.
    pub hysteresis_mb: u64,
    /// VRAM (MB) held back from the layer budget to keep the system stable.
    pub safety_margin_mb: u64,
    /// The total VRAM these thresholds were computed from.
    pub total_vram_mb: u64,
}

/// Default scaling percentages. Each can be tuned independently.
const DEFAULT_EMERGENCY_PCT: f64 = 0.03; // 3% of total
const DEFAULT_YIELD_PCT: f64 = 0.10; // 10%
const DEFAULT_RECOVER_PCT: f64 = 0.35; // 35%
const DEFAULT_HYSTERESIS_PCT: f64 = 0.05; // 5%
const DEFAULT_SAFETY_PCT: f64 = 0.08; // 8%

/// Hard floors so thresholds never drop below sane minimums on tiny GPUs.
const FLOOR_EMERGENCY_MB: u64 = 64;
const FLOOR_YIELD_MB: u64 = 256;
const FLOOR_RECOVER_MB: u64 = 1024;
const FLOOR_HYSTERESIS_MB: u64 = 128;
const FLOOR_SAFETY_MB: u64 = 256;

impl ThresholdProfile {
    /// Compute thresholds as percentages of total VRAM, with hard floors.
    ///
    /// If `total_vram_mb` is zero (CPU-only or detection failure), returns
    /// fallback values from the config defaults.
    pub fn from_total_vram(total_vram_mb: u64) -> Self {
        if total_vram_mb == 0 {
            return Self::from_config_defaults();
        }
        let t = total_vram_mb as f64;
        Self {
            emergency_mb: (t * DEFAULT_EMERGENCY_PCT).max(FLOOR_EMERGENCY_MB as f64) as u64,
            yield_mb: (t * DEFAULT_YIELD_PCT).max(FLOOR_YIELD_MB as f64) as u64,
            recover_mb: (t * DEFAULT_RECOVER_PCT).max(FLOOR_RECOVER_MB as f64) as u64,
            hysteresis_mb: (t * DEFAULT_HYSTERESIS_PCT).max(FLOOR_HYSTERESIS_MB as f64) as u64,
            safety_margin_mb: (t * DEFAULT_SAFETY_PCT).max(FLOOR_SAFETY_MB as f64) as u64,
            total_vram_mb,
        }
    }

    /// Merge with config overrides: if the config has a non-zero value, use it.
    /// Otherwise use the dynamically computed value.
    ///
    /// This lets power users lock specific thresholds in `config.toml` while
    /// leaving others auto-scaled.
    pub fn with_config_overrides(mut self, config: &OrchestratorConfig) -> Self {
        if config.yield_threshold_mb > 0 {
            self.yield_mb = config.yield_threshold_mb;
        }
        if config.emergency_threshold_mb > 0 {
            self.emergency_mb = config.emergency_threshold_mb;
        }
        if config.recover_threshold_mb > 0 {
            self.recover_mb = config.recover_threshold_mb;
        }
        if config.hysteresis_band_mb > 0 {
            self.hysteresis_mb = config.hysteresis_band_mb;
        }
        if config.safety_margin_mb > 0 {
            self.safety_margin_mb = config.safety_margin_mb;
        }
        self
    }

    /// Fallback profile using the hardcoded defaults from `OrchestratorConfig`.
    fn from_config_defaults() -> Self {
        let defaults = OrchestratorConfig::default();
        Self {
            yield_mb: defaults.yield_threshold_mb,
            emergency_mb: defaults.emergency_threshold_mb,
            recover_mb: defaults.recover_threshold_mb,
            hysteresis_mb: defaults.hysteresis_band_mb,
            safety_margin_mb: defaults.safety_margin_mb,
            total_vram_mb: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_4gb_gpu() {
        let p = ThresholdProfile::from_total_vram(4096);
        assert_eq!(p.emergency_mb, 122); // 4096 * 0.03 = 122.88 → 122
        assert_eq!(p.yield_mb, 409); // 4096 * 0.10 = 409.6  → 409
        assert_eq!(p.recover_mb, 1433); // 4096 * 0.35 = 1433.6 → 1433
        assert!(p.hysteresis_mb >= FLOOR_HYSTERESIS_MB);
        assert!(p.safety_margin_mb >= FLOOR_SAFETY_MB);
    }

    #[test]
    fn scaling_6gb_gpu() {
        let p = ThresholdProfile::from_total_vram(6144);
        assert_eq!(p.emergency_mb, 184); // 6144 * 0.03 = 184.32
        assert_eq!(p.yield_mb, 614); // 6144 * 0.10 = 614.4
        assert_eq!(p.recover_mb, 2150); // 6144 * 0.35 = 2150.4
    }

    #[test]
    fn scaling_12gb_gpu() {
        let p = ThresholdProfile::from_total_vram(12288);
        assert_eq!(p.emergency_mb, 368);
        assert_eq!(p.yield_mb, 1228);
        assert_eq!(p.recover_mb, 4300);
    }

    #[test]
    fn scaling_24gb_gpu() {
        let p = ThresholdProfile::from_total_vram(24576);
        assert_eq!(p.emergency_mb, 737);
        assert_eq!(p.yield_mb, 2457);
        assert_eq!(p.recover_mb, 8601);
    }

    #[test]
    fn floor_enforced_on_tiny_gpu() {
        let p = ThresholdProfile::from_total_vram(512); // 512 MB toy GPU
        assert!(p.emergency_mb >= FLOOR_EMERGENCY_MB);
        assert!(p.yield_mb >= FLOOR_YIELD_MB);
        assert!(p.recover_mb >= FLOOR_RECOVER_MB);
    }

    #[test]
    fn zero_vram_falls_back_to_defaults() {
        let p = ThresholdProfile::from_total_vram(0);
        let defaults = OrchestratorConfig::default();
        assert_eq!(p.yield_mb, defaults.yield_threshold_mb);
        assert_eq!(p.emergency_mb, defaults.emergency_threshold_mb);
        assert_eq!(p.recover_mb, defaults.recover_threshold_mb);
    }

    #[test]
    fn config_override_takes_precedence() {
        let mut config = OrchestratorConfig::default();
        config.yield_threshold_mb = 999;
        config.emergency_threshold_mb = 0; // 0 = use dynamic

        let p = ThresholdProfile::from_total_vram(6144).with_config_overrides(&config);
        assert_eq!(p.yield_mb, 999); // overridden
        assert_eq!(p.emergency_mb, 184); // dynamic (6144 * 0.03)
    }
}
