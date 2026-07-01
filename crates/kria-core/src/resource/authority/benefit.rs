//! Benefit Evaluation Engine (HRA Task 69 / redesign G5).
//!
//! Pure decision: is a Restart-class optimization (kill+respawn llama-server to change
//! `n_gpu_layers`) WORTH the disruption? The governing law biases hard toward *not* restarting:
//! a restart is only `WorthIt` when the expected speedup is meaningful, the user cannot be
//! interrupted (DeepIdle), the predicted failure probability is low, and the reload cost is bounded
//! (or hidden by idle). For a session already GPU-resident at a good size, expected speedup ≈ 1.0
//! → `NotWorthIt` → no restart. Uncertainty always resolves to `NotWorthIt`.

use serde::{Deserialize, Serialize};

use super::activity::ActivityState;

/// Tunable thresholds (overridable on target hardware). Defaults are conservative.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BenefitThresholds {
    /// Minimum speedup ratio (target/current) to justify a restart, e.g. 1.3 = 30% faster.
    pub speedup_min: f32,
    /// Maximum acceptable predicted failure probability (0..=1).
    pub fail_max: f32,
    /// Maximum acceptable reload cost in seconds when NOT hidden by idle.
    pub cost_max_s: f32,
}

impl Default for BenefitThresholds {
    fn default() -> Self {
        Self {
            speedup_min: 1.3,
            fail_max: 0.10,
            cost_max_s: 8.0,
        }
    }
}

/// Inputs to a benefit evaluation. All fields are estimates produced by cheap models
/// (per-tier throughput table refined by observed throughput; simulator margin → failure prob).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BenefitInputs {
    /// Estimated tokens/sec at the proposed target configuration.
    pub target_tok_per_s: f32,
    /// Estimated tokens/sec at the current configuration.
    pub current_tok_per_s: f32,
    /// Estimated cost (seconds) of the restart: model reload + warmup.
    pub restart_cost_s: f32,
    /// Predicted probability (0..=1) that the new size fails to load / OOMs.
    pub failure_prob: f32,
    /// Current user activity — interruption risk is infinite unless DeepIdle.
    pub activity: ActivityState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Benefit {
    WorthIt,
    NotWorthIt,
}

/// Why a benefit evaluation resolved the way it did (decision-grade logging, G11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenefitReason {
    /// User is not DeepIdle → any restart would interrupt; forbidden.
    InterruptionRisk,
    /// Expected speedup below the minimum threshold (already at a good size).
    InsufficientSpeedup,
    /// Predicted failure probability too high (tight VRAM fit).
    FailureRisk,
    /// Reload cost too high and not hidden by idle.
    CostTooHigh,
    /// All gates passed.
    WorthIt,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BenefitEval {
    pub benefit: Benefit,
    pub reason: BenefitReason,
    pub expected_speedup: f32,
}

/// Evaluate whether a Restart-class optimization is worth it. Order of checks is also the priority
/// of the failure reason reported (interruption first — it is the governing-law gate).
pub fn evaluate(inputs: &BenefitInputs, th: &BenefitThresholds) -> BenefitEval {
    let expected_speedup = if inputs.current_tok_per_s > 0.0 {
        inputs.target_tok_per_s / inputs.current_tok_per_s
    } else {
        // Current ≈ 0 (e.g. failed/stalled) → treat as large speedup but still gated below.
        f32::INFINITY
    };

    // 1. Interruption risk: a restart is only ever permitted while DeepIdle.
    if !inputs.activity.allows_perf_restart() {
        return BenefitEval {
            benefit: Benefit::NotWorthIt,
            reason: BenefitReason::InterruptionRisk,
            expected_speedup,
        };
    }
    // 2. Speedup must clear the bar (resident-at-good-size ⇒ ≈1.0 ⇒ rejected).
    //    (target/current tok-per-s are both ≥ 1.0 here, so `expected_speedup` is never NaN.)
    if expected_speedup < th.speedup_min {
        return BenefitEval {
            benefit: Benefit::NotWorthIt,
            reason: BenefitReason::InsufficientSpeedup,
            expected_speedup,
        };
    }
    // 3. Failure probability ceiling (tight fit → reject; bias to stability).
    if inputs.failure_prob > th.fail_max {
        return BenefitEval {
            benefit: Benefit::NotWorthIt,
            reason: BenefitReason::FailureRisk,
            expected_speedup,
        };
    }
    // 4. Cost ceiling. In DeepIdle the cost is largely hidden, but a pathologically long reload is
    //    still rejected so a returning user is never caught mid-restart for too long.
    if inputs.restart_cost_s > th.cost_max_s {
        return BenefitEval {
            benefit: Benefit::NotWorthIt,
            reason: BenefitReason::CostTooHigh,
            expected_speedup,
        };
    }
    BenefitEval {
        benefit: Benefit::WorthIt,
        reason: BenefitReason::WorthIt,
        expected_speedup,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deepidle(target: f32, current: f32) -> BenefitInputs {
        BenefitInputs {
            target_tok_per_s: target,
            current_tok_per_s: current,
            restart_cost_s: 4.0,
            failure_prob: 0.02,
            activity: ActivityState::DeepIdle,
        }
    }

    #[test]
    fn resident_at_good_size_is_not_worth_it() {
        // target ≈ current → speedup ≈ 1.0 → reject.
        let e = evaluate(&deepidle(30.0, 29.0), &BenefitThresholds::default());
        assert_eq!(e.benefit, Benefit::NotWorthIt);
        assert_eq!(e.reason, BenefitReason::InsufficientSpeedup);
    }

    #[test]
    fn cpu_to_gpu_deepidle_safe_is_worth_it() {
        // big speedup, deep idle, low failure prob, bounded cost.
        let e = evaluate(&deepidle(30.0, 10.0), &BenefitThresholds::default());
        assert_eq!(e.benefit, Benefit::WorthIt);
        assert_eq!(e.reason, BenefitReason::WorthIt);
        assert!(e.expected_speedup >= 1.3);
    }

    #[test]
    fn never_worth_it_while_active_even_with_huge_speedup() {
        let mut inp = deepidle(60.0, 5.0);
        inp.activity = ActivityState::Active;
        let e = evaluate(&inp, &BenefitThresholds::default());
        assert_eq!(e.benefit, Benefit::NotWorthIt);
        assert_eq!(e.reason, BenefitReason::InterruptionRisk);
    }

    #[test]
    fn never_worth_it_while_idle_not_deepidle() {
        let mut inp = deepidle(60.0, 5.0);
        inp.activity = ActivityState::Idle;
        assert_eq!(
            evaluate(&inp, &BenefitThresholds::default()).reason,
            BenefitReason::InterruptionRisk
        );
    }

    #[test]
    fn high_failure_prob_rejected() {
        let mut inp = deepidle(30.0, 10.0);
        inp.failure_prob = 0.5;
        let e = evaluate(&inp, &BenefitThresholds::default());
        assert_eq!(e.benefit, Benefit::NotWorthIt);
        assert_eq!(e.reason, BenefitReason::FailureRisk);
    }

    #[test]
    fn excessive_reload_cost_rejected() {
        let mut inp = deepidle(30.0, 10.0);
        inp.restart_cost_s = 60.0;
        let e = evaluate(&inp, &BenefitThresholds::default());
        assert_eq!(e.benefit, Benefit::NotWorthIt);
        assert_eq!(e.reason, BenefitReason::CostTooHigh);
    }

    #[test]
    fn stalled_current_throughput_treated_as_large_speedup() {
        let mut inp = deepidle(30.0, 0.0);
        inp.current_tok_per_s = 0.0;
        let e = evaluate(&inp, &BenefitThresholds::default());
        assert_eq!(e.benefit, Benefit::WorthIt);
    }
}
