//! Workflow Eval Runner — Executes scenarios against the real desktop.
//!
//! This runner sends prompts to the KRIA agent loop and collects results.
//! It is designed to run as a batch evaluation, discovering ALL failures
//! in one pass rather than debugging one-by-one.

use super::failure_classifier::ClassifiedFailure;
use super::scenarios::EvalCategory;
use serde::{Deserialize, Serialize};

/// Result of running a single eval scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub scenario_id: String,
    pub category: EvalCategory,
    pub prompt: String,
    pub success: bool,
    pub verdict: String,
    pub duration_ms: u64,
    pub steps_completed: u32,
    pub steps_total: u32,
    pub error: Option<String>,
    pub failure_classification: Option<ClassifiedFailure>,
    pub criteria_met: Vec<(String, bool)>,
    pub telemetry_events: u32,
    pub environment_snapshot: EnvironmentSnapshot,
}

/// Snapshot of the environment at eval time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentSnapshot {
    pub session_type: String,
    pub compositor: Option<String>,
    pub display: Option<String>,
    pub chrome_available: bool,
    pub code_available: bool,
    pub nautilus_available: bool,
}

impl EnvironmentSnapshot {
    pub fn capture() -> Self {
        let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
        let display = std::env::var("DISPLAY").ok();
        let compositor = std::env::var("XDG_CURRENT_DESKTOP").ok();

        Self {
            session_type,
            compositor,
            display,
            chrome_available: which_exists("google-chrome") || which_exists("chromium"),
            code_available: which_exists("code"),
            nautilus_available: which_exists("nautilus"),
        }
    }
}

fn which_exists(binary: &str) -> bool {
    std::process::Command::new("which")
        .arg(binary)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Batch eval run result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalBatchResult {
    pub total_scenarios: usize,
    pub passed: usize,
    pub failed: usize,
    pub timed_out: usize,
    pub results: Vec<EvalResult>,
    pub failure_summary: FailureSummary,
    pub environment: EnvironmentSnapshot,
    pub total_duration_ms: u64,
    pub timestamp: String,
}

/// Aggregated failure summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureSummary {
    pub by_category: Vec<(String, usize)>,
    pub by_classification: Vec<(String, usize)>,
    pub by_severity: Vec<(String, usize)>,
    pub top_failure_patterns: Vec<String>,
}

impl EvalBatchResult {
    pub fn compute_summary(results: &[EvalResult]) -> FailureSummary {
        use std::collections::HashMap;

        let mut by_category: HashMap<String, usize> = HashMap::new();
        let mut by_classification: HashMap<String, usize> = HashMap::new();
        let mut by_severity: HashMap<String, usize> = HashMap::new();
        let mut patterns: HashMap<String, usize> = HashMap::new();

        for result in results.iter().filter(|r| !r.success) {
            *by_category
                .entry(format!("{:?}", result.category))
                .or_default() += 1;

            if let Some(ref fc) = result.failure_classification {
                *by_classification
                    .entry(format!("{:?}", fc.classification))
                    .or_default() += 1;
                *by_severity.entry(format!("{:?}", fc.severity)).or_default() += 1;
                *patterns.entry(fc.probable_cause.clone()).or_default() += 1;
            }
        }

        let mut top_patterns: Vec<(String, usize)> = patterns.into_iter().collect();
        top_patterns.sort_by(|a, b| b.1.cmp(&a.1));

        FailureSummary {
            by_category: by_category.into_iter().collect(),
            by_classification: by_classification.into_iter().collect(),
            by_severity: by_severity.into_iter().collect(),
            top_failure_patterns: top_patterns
                .into_iter()
                .take(10)
                .map(|(p, c)| format!("{} ({}x)", p, c))
                .collect(),
        }
    }

    /// Generate a human-readable report.
    pub fn to_report(&self) -> String {
        let mut report = String::new();
        report.push_str("═══════════════════════════════════════════════════════════════\n");
        report.push_str("  KRIA GUI COGNITION OPERATIONAL EVAL REPORT\n");
        report.push_str("═══════════════════════════════════════════════════════════════\n\n");
        report.push_str(&format!("Timestamp: {}\n", self.timestamp));
        report.push_str(&format!(
            "Environment: {} ({:?})\n",
            self.environment.session_type, self.environment.compositor
        ));
        report.push_str(&format!("Total Duration: {}ms\n\n", self.total_duration_ms));
        report.push_str(&format!(
            "RESULTS: {} passed / {} failed / {} timed out (of {})\n\n",
            self.passed, self.failed, self.timed_out, self.total_scenarios
        ));

        if !self.failure_summary.top_failure_patterns.is_empty() {
            report.push_str("TOP FAILURE PATTERNS:\n");
            for pattern in &self.failure_summary.top_failure_patterns {
                report.push_str(&format!("  • {}\n", pattern));
            }
            report.push_str("\n");
        }

        report.push_str("DETAILED RESULTS:\n");
        report.push_str("─────────────────────────────────────────────────────────────\n");
        for result in &self.results {
            let status = if result.success {
                "✓ PASS"
            } else {
                "✗ FAIL"
            };
            report.push_str(&format!(
                "[{}] {} ({}ms)\n",
                status, result.scenario_id, result.duration_ms
            ));
            if let Some(ref err) = result.error {
                report.push_str(&format!("    Error: {}\n", err));
            }
            if let Some(ref fc) = result.failure_classification {
                report.push_str(&format!(
                    "    Class: {:?} | Severity: {:?}\n",
                    fc.classification, fc.severity
                ));
                report.push_str(&format!("    Cause: {}\n", fc.probable_cause));
                report.push_str(&format!("    Fix:   {}\n", fc.suggested_fix));
            }
            report.push_str("\n");
        }

        report
    }
}
