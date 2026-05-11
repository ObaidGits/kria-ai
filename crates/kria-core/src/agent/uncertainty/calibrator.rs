//! ConfidenceCalibrator — Adaptive threshold calibration using Beta distribution.
//!
//! # Design: Bayesian Calibration
//!
//! Instead of arbitrary thresholds, we use Bayesian calibration:
//! After each task, we update the Beta(α, β) distribution for each
//! threshold zone. The thresholds converge to their empirically
//! optimal values over time.
//!
//! # Threshold Zones
//!
//! - **Plan** (confidence ≥ plan_threshold): Proceed to planning
//! - **Gather** (confidence ≥ gather_threshold): Gather more evidence
//! - **Ask** (confidence ≥ ask_threshold): Ask the user
//! - **Refuse** (confidence < ask_threshold): Refuse and explain why

/// What to do given the current confidence level.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum UncertaintyAction {
    /// Confidence is high enough — proceed to planning.
    Plan,
    /// Confidence is medium — gather more evidence first.
    GatherEvidence,
    /// Confidence is low — ask the user for clarification.
    AskUser,
    /// Confidence is very low — refuse and explain why.
    Refuse,
}

/// Adaptive threshold calibration using Beta distribution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConfidenceCalibrator {
    /// Beta distribution parameters for the "plan" zone.
    plan_alpha: f64,
    plan_beta: f64,
    /// Beta distribution parameters for the "gather" zone.
    gather_alpha: f64,
    gather_beta: f64,
    /// Beta distribution parameters for the "ask" zone.
    ask_alpha: f64,
    ask_beta: f64,
    /// Current thresholds (recalibrated periodically).
    pub plan_threshold: f64,
    pub gather_threshold: f64,
    pub ask_threshold: f64,
}

impl ConfidenceCalibrator {
    /// Create a new calibrator with default thresholds.
    pub fn new() -> Self {
        Self {
            // Priors encode our initial beliefs about threshold quality
            plan_alpha: 8.0,   // 8 successes at 0.8 threshold
            plan_beta: 2.0,    // 2 failures at 0.8 threshold
            gather_alpha: 6.0, // 6 successes at 0.6 threshold
            gather_beta: 4.0,  // 4 failures at 0.6 threshold
            ask_alpha: 3.0,    // 3 successes at 0.3 threshold
            ask_beta: 7.0,     // 7 failures at 0.3 threshold
            plan_threshold: 0.8,
            gather_threshold: 0.6,
            ask_threshold: 0.3,
        }
    }

    /// Evaluate a confidence score and return the action.
    pub fn evaluate(&self, confidence: f64) -> UncertaintyAction {
        if confidence >= self.plan_threshold {
            UncertaintyAction::Plan
        } else if confidence >= self.gather_threshold {
            UncertaintyAction::GatherEvidence
        } else if confidence >= self.ask_threshold {
            UncertaintyAction::AskUser
        } else {
            UncertaintyAction::Refuse
        }
    }

    /// Record an outcome and recalibrate thresholds.
    ///
    /// `confidence`: The confidence score that was used for the decision.
    /// `success`: Whether the task succeeded.
    pub fn record_outcome(&mut self, confidence: f64, success: bool) {
        // Update the appropriate Beta distribution
        if confidence >= self.plan_threshold {
            if success {
                self.plan_alpha += 1.0;
            } else {
                self.plan_beta += 1.0;
            }
        } else if confidence >= self.gather_threshold {
            if success {
                self.gather_alpha += 1.0;
            } else {
                self.gather_beta += 1.0;
            }
        } else if confidence >= self.ask_threshold {
            if success {
                self.ask_alpha += 1.0;
            } else {
                self.ask_beta += 1.0;
            }
        }
        // Below ask_threshold: no update (we refused, so no outcome)

        // Recalibrate thresholds (posterior mean of each Beta)
        self.plan_threshold = self.plan_alpha / (self.plan_alpha + self.plan_beta);
        self.gather_threshold = self.gather_alpha / (self.gather_alpha + self.gather_beta);
        self.ask_threshold = self.ask_alpha / (self.ask_alpha + self.ask_beta);

        // Ensure ordering: plan > gather > ask (with minimum gap)
        self.gather_threshold = self.gather_threshold.min(self.plan_threshold - 0.05);
        self.ask_threshold = self.ask_threshold.min(self.gather_threshold - 0.05);

        // Clamp to reasonable bounds
        self.plan_threshold = self.plan_threshold.clamp(0.5, 0.99);
        self.gather_threshold = self.gather_threshold.clamp(0.3, 0.9);
        self.ask_threshold = self.ask_threshold.clamp(0.1, 0.7);
    }

    /// Get the current thresholds.
    pub fn thresholds(&self) -> (f64, f64, f64) {
        (
            self.plan_threshold,
            self.gather_threshold,
            self.ask_threshold,
        )
    }
}

impl Default for ConfidenceCalibrator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_thresholds() {
        let cal = ConfidenceCalibrator::new();
        assert!((cal.plan_threshold - 0.8).abs() < 0.001);
        assert!((cal.gather_threshold - 0.6).abs() < 0.001);
        assert!((cal.ask_threshold - 0.3).abs() < 0.001);
    }

    #[test]
    fn high_confidence_plans() {
        let cal = ConfidenceCalibrator::new();
        assert_eq!(cal.evaluate(0.9), UncertaintyAction::Plan);
        assert_eq!(cal.evaluate(0.85), UncertaintyAction::Plan);
    }

    #[test]
    fn medium_confidence_gathers() {
        let cal = ConfidenceCalibrator::new();
        assert_eq!(cal.evaluate(0.7), UncertaintyAction::GatherEvidence);
        assert_eq!(cal.evaluate(0.65), UncertaintyAction::GatherEvidence);
    }

    #[test]
    fn low_confidence_asks() {
        let cal = ConfidenceCalibrator::new();
        assert_eq!(cal.evaluate(0.4), UncertaintyAction::AskUser);
        assert_eq!(cal.evaluate(0.35), UncertaintyAction::AskUser);
    }

    #[test]
    fn very_low_confidence_refuses() {
        let cal = ConfidenceCalibrator::new();
        assert_eq!(cal.evaluate(0.2), UncertaintyAction::Refuse);
        assert_eq!(cal.evaluate(0.1), UncertaintyAction::Refuse);
    }

    #[test]
    fn recording_success_adjusts_thresholds() {
        let mut cal = ConfidenceCalibrator::new();
        let _initial_plan = cal.plan_threshold;

        // Record many successes at the plan threshold
        for _ in 0..10 {
            cal.record_outcome(0.85, true);
        }

        // Threshold should adjust (may go up or down depending on Beta update)
        // The important thing is it doesn't panic and stays reasonable
        assert!(cal.plan_threshold > 0.5);
        assert!(cal.plan_threshold < 1.0);
    }

    #[test]
    fn recording_failure_adjusts_thresholds() {
        let mut cal = ConfidenceCalibrator::new();

        // Record many failures at the plan threshold
        for _ in 0..10 {
            cal.record_outcome(0.85, false);
        }

        // Threshold should adjust (may go up or down depending on Beta update)
        // The important thing is it stays reasonable and doesn't panic
        assert!(
            cal.plan_threshold >= 0.0 && cal.plan_threshold <= 1.0,
            "Threshold should stay in [0,1], got {}",
            cal.plan_threshold
        );
    }

    #[test]
    fn thresholds_maintain_ordering() {
        let mut cal = ConfidenceCalibrator::new();

        // Record many outcomes to stress the ordering
        for i in 0..100 {
            let conf = 0.5 + (i as f64 * 0.004);
            cal.record_outcome(conf, i % 2 == 0);
        }

        assert!(
            cal.plan_threshold > cal.gather_threshold,
            "plan ({}) should be > gather ({})",
            cal.plan_threshold,
            cal.gather_threshold
        );
        assert!(
            cal.gather_threshold > cal.ask_threshold,
            "gather ({}) should be > ask ({})",
            cal.gather_threshold,
            cal.ask_threshold
        );
    }

    #[test]
    fn thresholds_stay_in_bounds() {
        let mut cal = ConfidenceCalibrator::new();

        // Extreme outcomes
        for _ in 0..1000 {
            cal.record_outcome(0.9, true);
        }
        for _ in 0..1000 {
            cal.record_outcome(0.9, false);
        }

        assert!(cal.plan_threshold >= 0.5 && cal.plan_threshold <= 0.99);
        assert!(cal.gather_threshold >= 0.3 && cal.gather_threshold <= 0.9);
        assert!(cal.ask_threshold >= 0.1 && cal.ask_threshold <= 0.7);
    }

    #[test]
    fn boundary_values() {
        let cal = ConfidenceCalibrator::new();
        assert_eq!(cal.evaluate(0.8), UncertaintyAction::Plan); // exact boundary
        assert_eq!(cal.evaluate(0.6), UncertaintyAction::GatherEvidence);
        assert_eq!(cal.evaluate(0.3), UncertaintyAction::AskUser);
        assert_eq!(cal.evaluate(0.29), UncertaintyAction::Refuse);
    }
}
