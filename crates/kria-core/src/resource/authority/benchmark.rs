//! Benchmark Framework (HRA Task 48 / R30).
//!
//! Resource-efficiency benchmark data model + regression detection. The harness runs fixed
//! scenarios (in `kria-eval`) and emits `BenchResult`s; this module provides before/after
//! comparison and regression detection used as a release gate.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchResult {
    pub scenario: String,
    pub hw_class: String,
    pub vram_peak_mb: u64,
    pub ram_peak_mb: u64,
    pub cpu_pct: f32,
    pub gpu_pct: f32,
    pub p50_ms: u32,
    pub p99_ms: u32,
    pub throughput: f32,
    pub queue_delay_ms: u32,
    pub swaps: u32,
    pub recovery_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Regression {
    pub scenario: String,
    pub metric: String,
    pub baseline: f64,
    pub candidate: f64,
    pub pct_worse: f64,
}

/// Tolerance (fractional) before a worsening counts as a regression. e.g. 0.10 = 10%.
#[derive(Debug, Clone, Copy)]
pub struct RegressionTolerance {
    pub latency: f64,
    pub memory: f64,
    pub swaps: f64,
}

impl Default for RegressionTolerance {
    fn default() -> Self {
        Self {
            latency: 0.10,
            memory: 0.10,
            swaps: 0.0, // any increase in swaps is a regression
        }
    }
}

/// Compare a candidate against a baseline for the same scenario. Returns all detected regressions
/// ("higher is worse" metrics). Empty result = no regression (gate passes).
pub fn detect_regressions(
    baseline: &BenchResult,
    candidate: &BenchResult,
    tol: RegressionTolerance,
) -> Vec<Regression> {
    let mut out = Vec::new();
    let mut check = |metric: &str, base: f64, cand: f64, tolerance: f64| {
        if base <= 0.0 {
            return;
        }
        let pct = (cand - base) / base;
        if pct > tolerance {
            out.push(Regression {
                scenario: candidate.scenario.clone(),
                metric: metric.into(),
                baseline: base,
                candidate: cand,
                pct_worse: pct * 100.0,
            });
        }
    };

    check("p50_ms", baseline.p50_ms as f64, candidate.p50_ms as f64, tol.latency);
    check("p99_ms", baseline.p99_ms as f64, candidate.p99_ms as f64, tol.latency);
    check("vram_peak_mb", baseline.vram_peak_mb as f64, candidate.vram_peak_mb as f64, tol.memory);
    check("ram_peak_mb", baseline.ram_peak_mb as f64, candidate.ram_peak_mb as f64, tol.memory);
    check("queue_delay_ms", baseline.queue_delay_ms as f64, candidate.queue_delay_ms as f64, tol.latency);
    check("swaps", baseline.swaps as f64, candidate.swaps as f64, tol.swaps);
    check("recovery_ms", baseline.recovery_ms as f64, candidate.recovery_ms as f64, tol.latency);
    out
}

/// Release gate: true when the candidate has no regressions vs baseline.
pub fn gate_passes(baseline: &BenchResult, candidate: &BenchResult, tol: RegressionTolerance) -> bool {
    detect_regressions(baseline, candidate, tol).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(scenario: &str, p99: u32, vram: u64, swaps: u32) -> BenchResult {
        BenchResult {
            scenario: scenario.into(),
            hw_class: "medium".into(),
            vram_peak_mb: vram,
            ram_peak_mb: 8000,
            cpu_pct: 40.0,
            gpu_pct: 70.0,
            p50_ms: 100,
            p99_ms: p99,
            throughput: 10.0,
            queue_delay_ms: 50,
            swaps,
            recovery_ms: 500,
        }
    }

    #[test]
    fn no_regression_within_tolerance() {
        let base = r("chat", 1000, 6000, 2);
        let cand = r("chat", 1050, 6100, 2); // +5% p99, +1.6% vram → within 10%
        assert!(gate_passes(&base, &cand, RegressionTolerance::default()));
    }

    #[test]
    fn latency_regression_detected() {
        let base = r("chat", 1000, 6000, 2);
        let cand = r("chat", 1300, 6000, 2); // +30% p99
        let regs = detect_regressions(&base, &cand, RegressionTolerance::default());
        assert!(regs.iter().any(|r| r.metric == "p99_ms"));
        assert!(!gate_passes(&base, &cand, RegressionTolerance::default()));
    }

    #[test]
    fn any_swap_increase_is_regression() {
        let base = r("img", 5000, 9000, 1);
        let cand = r("img", 5000, 9000, 2); // swaps 1→2, tol 0.0
        assert!(!gate_passes(&base, &cand, RegressionTolerance::default()));
    }

    #[test]
    fn improvement_passes_gate() {
        let base = r("chat", 1000, 6000, 3);
        let cand = r("chat", 800, 5000, 1); // everything better
        assert!(gate_passes(&base, &cand, RegressionTolerance::default()));
    }
}
