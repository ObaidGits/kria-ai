//! ToolStats — Per-tool success rate tracking using Beta distribution.
//!
//! # Statistical Approach: Beta(α, β) Posterior
//!
//! Previous approach: raw percentage (success / total)
//! Problem: 1 success = 100%, massive small-sample bias
//!
//! New approach: Beta(α, β) posterior estimation
//! - Prior: Beta(1, 1) — uniform prior, starts at 0.50
//! - Update: success → α += 1, failure → β += 1
//! - Posterior mean: α / (α + β)
//!
//! This naturally handles:
//! - Unknown tools start at 0.50 (neutral)
//! - 1 success → (1+1)/(1+2) = 0.67 (not 1.00)
//! - 10 successes, 0 failures → (10+1)/(10+2) = 0.92 (high confidence)
//! - 1 success, 1 failure → (1+1)/(1+1+2) = 0.50 (neutral)
//!
//! # Why Beta Distribution (Not Just Laplace Smoothing)?
//!
//! Laplace smoothing (P = (S+1)/(N+2)) is the posterior mean of a Beta(1,1) prior.
//! But Beta gives us more:
//!
//! 1. **Adjustable priors:** Beta(2,1) encodes "probably good" for trusted tools
//! 2. **Confidence intervals:** We can report uncertainty, not just point estimates
//! 3. **Conjugate prior:** Beta is the conjugate prior for Bernoulli — updates are exact
//! 4. **Natural regularization:** Few observations → prior dominates; many → data dominates

use std::collections::HashMap;
use std::time::Duration;

/// Per-tool success rate tracking using Beta distribution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolStats {
    /// Tool name (e.g., "systemctl", "git", "curl").
    pub tool_name: String,
    /// Beta distribution alpha parameter (successes + prior).
    pub alpha: f64,
    /// Beta distribution beta parameter (failures + prior).
    pub beta: f64,
    /// Total number of calls.
    pub total_calls: u64,
    /// Exponential moving average of latency.
    pub avg_latency_ms: f64,
    /// Last time this tool was used.
    pub last_used_epoch: u64,
    /// Known failure modes (e.g., "nginx config invalid → restart crashes site").
    pub known_failure_modes: Vec<String>,
}

impl ToolStats {
    /// Create a new tool with Beta(1,1) prior (neutral 0.50 score).
    pub fn new(tool_name: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            alpha: 1.0,  // Prior: 1 success
            beta: 1.0,   // Prior: 1 failure
            total_calls: 0,
            avg_latency_ms: 0.0,
            last_used_epoch: 0,
            known_failure_modes: Vec::new(),
        }
    }

    /// Create a tool with an adjustable prior (for trusted built-in tools).
    pub fn with_prior(tool_name: impl Into<String>, alpha: f64, beta: f64) -> Self {
        Self {
            tool_name: tool_name.into(),
            alpha,
            beta,
            total_calls: 0,
            avg_latency_ms: 0.0,
            last_used_epoch: 0,
            known_failure_modes: Vec::new(),
        }
    }

    /// Posterior mean: α / (α + β).
    /// This is the expected success rate.
    pub fn success_rate(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Record a successful execution.
    pub fn record_success(&mut self, latency: Duration) {
        self.alpha += 1.0;
        self.total_calls += 1;
        self.update_latency(latency);
    }

    /// Record a failed execution.
    pub fn record_failure(&mut self, latency: Duration) {
        self.beta += 1.0;
        self.total_calls += 1;
        self.update_latency(latency);
    }

    /// Record an outcome (success = true/false).
    pub fn record(&mut self, success: bool, latency: Duration) {
        if success {
            self.record_success(latency);
        } else {
            self.record_failure(latency);
        }
    }

    /// Confidence interval width (95% CI approximation).
    /// Narrower = more confident in the estimate.
    pub fn confidence_width(&self) -> f64 {
        let n = self.alpha + self.beta - 2.0; // Effective sample size
        if n <= 0.0 {
            return 1.0; // Maximum uncertainty
        }
        let p = self.success_rate();
        2.0 * 1.96 * (p * (1.0 - p) / n).sqrt()
    }

    /// Add a known failure mode.
    pub fn add_failure_mode(&mut self, mode: impl Into<String>) {
        self.known_failure_modes.push(mode.into());
    }

    /// Update exponential moving average of latency.
    fn update_latency(&mut self, latency: Duration) {
        let alpha = 0.1; // Smoothing factor
        let new_ms = latency.as_secs_f64() * 1000.0;
        if self.total_calls <= 1 {
            self.avg_latency_ms = new_ms;
        } else {
            self.avg_latency_ms = self.avg_latency_ms * (1.0 - alpha) + new_ms * alpha;
        }
    }
}

/// SelfModel — capability awareness with historical success rates.
///
/// Tracks per-tool success rates and per-domain routing accuracy.
/// Used by the Structured Branching Planner to score paths.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SelfModel {
    /// Per-tool success rates.
    tool_stats: HashMap<String, ToolStats>,
    /// Per-domain routing accuracy (e.g., "system_admin" → 0.85).
    domain_accuracy: HashMap<String, f64>,
}

impl SelfModel {
    /// Create a new empty SelfModel.
    pub fn new() -> Self {
        Self {
            tool_stats: HashMap::new(),
            domain_accuracy: HashMap::new(),
        }
    }

    /// Get or create stats for a tool.
    pub fn get_or_create(&mut self, tool_name: &str) -> &mut ToolStats {
        self.tool_stats
            .entry(tool_name.to_string())
            .or_insert_with(|| ToolStats::new(tool_name))
    }

    /// Record an outcome for a tool.
    pub fn record_outcome(&mut self, tool_name: &str, success: bool, latency: Duration) {
        let stats = self.get_or_create(tool_name);
        stats.record(success, latency);
    }

    /// Get the success rate for a tool.
    pub fn success_rate(&self, tool_name: &str) -> f64 {
        self.tool_stats
            .get(tool_name)
            .map(|s| s.success_rate())
            .unwrap_or(0.5) // Unknown tools get neutral Beta(1,1) prior
    }

    /// Get stats for a specific tool.
    pub fn get_stats(&self, tool_name: &str) -> Option<&ToolStats> {
        self.tool_stats.get(tool_name)
    }

    /// Get all tool stats.
    pub fn all_stats(&self) -> &HashMap<String, ToolStats> {
        &self.tool_stats
    }

    /// Score a path (sequence of tool names) using geometric mean.
    /// The geometric mean ensures that if ANY tool has a low success rate,
    /// the overall path score is low (path fails if any step fails).
    pub fn score_path(&self, tool_names: &[&str]) -> f64 {
        if tool_names.is_empty() {
            return 0.5;
        }

        let product: f64 = tool_names.iter()
            .map(|name| self.success_rate(name))
            .product();

        product.powf(1.0 / tool_names.len() as f64)
    }

    /// Record domain accuracy.
    pub fn record_domain_accuracy(&mut self, domain: &str, accuracy: f64) {
        self.domain_accuracy.insert(domain.to_string(), accuracy);
    }

    /// Get domain accuracy.
    pub fn domain_accuracy(&self, domain: &str) -> f64 {
        self.domain_accuracy.get(domain).copied().unwrap_or(0.5)
    }

    /// Get all domain accuracies.
    pub fn all_domain_accuracies(&self) -> &HashMap<String, f64> {
        &self.domain_accuracy
    }

    /// Merge another SelfModel into this one (for persistence loading).
    pub fn merge(&mut self, other: SelfModel) {
        for (name, stats) in other.tool_stats {
            self.tool_stats.entry(name).or_insert(stats);
        }
        for (domain, accuracy) in other.domain_accuracy {
            self.domain_accuracy.entry(domain).or_insert(accuracy);
        }
    }
}

impl Default for SelfModel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beta_prior_starts_at_0_5() {
        let stats = ToolStats::new("test_tool");
        assert!((stats.success_rate() - 0.5).abs() < 0.001,
            "Beta(1,1) prior should give 0.50, got {}", stats.success_rate());
    }

    #[test]
    fn one_success_gives_0_67() {
        let mut stats = ToolStats::new("test_tool");
        stats.record_success(Duration::from_millis(100));
        // (1+1)/(1+1+2) = 2/3 ≈ 0.667
        assert!((stats.success_rate() - 0.6667).abs() < 0.01,
            "1 success should give ~0.67, got {}", stats.success_rate());
    }

    #[test]
    fn ten_successes_gives_0_92() {
        let mut stats = ToolStats::new("test_tool");
        for _ in 0..10 {
            stats.record_success(Duration::from_millis(100));
        }
        // (10+1)/(10+1+2) = 11/12 ≈ 0.917
        assert!((stats.success_rate() - 0.9167).abs() < 0.01,
            "10 successes should give ~0.92, got {}", stats.success_rate());
    }

    #[test]
    fn one_failure_gives_0_33() {
        let mut stats = ToolStats::new("test_tool");
        stats.record_failure(Duration::from_millis(100));
        // 1/(1+1+2) = 1/3 ≈ 0.333
        assert!((stats.success_rate() - 0.3333).abs() < 0.01,
            "1 failure should give ~0.33, got {}", stats.success_rate());
    }

    #[test]
    fn balanced_outcomes_give_0_5() {
        let mut stats = ToolStats::new("test_tool");
        stats.record_success(Duration::from_millis(100));
        stats.record_failure(Duration::from_millis(100));
        // (1+1)/(1+1+1+1+2) = wait... let me recalculate
        // After 1 success: α=2, β=1
        // After 1 failure: α=2, β=2
        // Posterior: 2/(2+2) = 0.5
        assert!((stats.success_rate() - 0.5).abs() < 0.001,
            "Balanced outcomes should give 0.5, got {}", stats.success_rate());
    }

    #[test]
    fn confidence_interval_narrows_with_data() {
        let mut stats = ToolStats::new("test_tool");
        let ci_before = stats.confidence_width();

        for _ in 0..100 {
            stats.record_success(Duration::from_millis(100));
        }
        let ci_after = stats.confidence_width();

        assert!(ci_after < ci_before,
            "CI should narrow with more data: before={}, after={}", ci_before, ci_after);
    }

    #[test]
    fn unknown_tool_gets_0_5() {
        let model = SelfModel::new();
        assert!((model.success_rate("nonexistent_tool") - 0.5).abs() < 0.001,
            "Unknown tool should get 0.5, got {}", model.success_rate("nonexistent_tool"));
    }

    #[test]
    fn score_path_geometric_mean() {
        let mut model = SelfModel::new();
        // Tool A: 0.8 success rate
        for _ in 0..7 { model.record_outcome("tool_a", true, Duration::from_millis(100)); }
        for _ in 0..2 { model.record_outcome("tool_a", false, Duration::from_millis(100)); }

        // Tool B: 0.8 success rate
        for _ in 0..7 { model.record_outcome("tool_b", true, Duration::from_millis(100)); }
        for _ in 0..2 { model.record_outcome("tool_b", false, Duration::from_millis(100)); }

        let score = model.score_path(&["tool_a", "tool_b"]);
        // Geometric mean of 0.8 and 0.8 = 0.8
        assert!((score - 0.8).abs() < 0.1,
            "Path score should be ~0.8, got {}", score);
    }

    #[test]
    fn score_path_fails_if_any_tool_fails() {
        let mut model = SelfModel::new();
        // Tool A: good (0.9)
        for _ in 0..9 { model.record_outcome("good_tool", true, Duration::from_millis(100)); }
        model.record_outcome("good_tool", false, Duration::from_millis(100));

        // Tool B: bad (0.2)
        model.record_outcome("bad_tool", true, Duration::from_millis(100));
        for _ in 0..4 { model.record_outcome("bad_tool", false, Duration::from_millis(100)); }

        let score = model.score_path(&["good_tool", "bad_tool"]);
        // Geometric mean of ~0.9 and ~0.2 = ~0.42
        assert!(score < 0.5,
            "Path with a bad tool should score low, got {}", score);
    }

    #[test]
    fn latency_ema_updates() {
        let mut stats = ToolStats::new("test_tool");
        stats.record_success(Duration::from_millis(100));
        assert!((stats.avg_latency_ms - 100.0).abs() < 1.0);

        stats.record_success(Duration::from_millis(200));
        // EMA: 100 * 0.9 + 200 * 0.1 = 110
        assert!((stats.avg_latency_ms - 110.0).abs() < 1.0,
            "Latency EMA should be ~110, got {}", stats.avg_latency_ms);
    }

    #[test]
    fn adjustable_prior() {
        // Trusted built-in tool with Beta(2,1) prior (starts at 0.67)
        let stats = ToolStats::with_prior("ls", 2.0, 1.0);
        assert!((stats.success_rate() - 0.6667).abs() < 0.01,
            "Beta(2,1) prior should give ~0.67, got {}", stats.success_rate());
    }

    #[test]
    fn failure_mode_tracking() {
        let mut stats = ToolStats::new("nginx");
        stats.add_failure_mode("restart without config check → site crash");
        assert_eq!(stats.known_failure_modes.len(), 1);
        assert!(stats.known_failure_modes[0].contains("config check"));
    }

    #[test]
    fn self_model_merge() {
        let mut model1 = SelfModel::new();
        model1.record_outcome("tool_a", true, Duration::from_millis(100));

        let mut model2 = SelfModel::new();
        model2.record_outcome("tool_b", true, Duration::from_millis(100));

        model1.merge(model2);
        assert!(model1.get_stats("tool_a").is_some());
        assert!(model1.get_stats("tool_b").is_some());
    }
}
