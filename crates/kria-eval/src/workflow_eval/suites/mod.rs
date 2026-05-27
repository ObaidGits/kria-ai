//! Workflow eval suite registry.
//!
//! All suites are registered here. Use `all_suites()` to get every case,
//! or call individual suite functions for targeted runs.

pub mod browser;
pub mod coding;
pub mod human_expectation;
pub mod interruption;
pub mod long_horizon;
pub mod productivity;
pub mod stress;

use crate::workflow_eval::types::WorkflowEvalCase;

/// Every workflow eval case across all suites.
pub fn all_suites() -> Vec<WorkflowEvalCase> {
    let mut cases = Vec::new();
    cases.extend(coding::coding_suite());
    cases.extend(browser::browser_suite());
    cases.extend(productivity::productivity_suite());
    cases.extend(interruption::interruption_suite());
    cases.extend(human_expectation::human_expectation_suite());
    cases.extend(stress::stress_suite());
    cases.extend(long_horizon::long_horizon_suite());
    cases
}

/// Auto-safe subset: Safe + Reversible only, no daemon or live opt-in required.
pub fn auto_safe_suite() -> Vec<WorkflowEvalCase> {
    use crate::workflow_eval::safety_filter::SafetyFilter;
    let all = all_suites();
    SafetyFilter::filter_auto_safe(&all)
        .into_iter()
        .cloned()
        .collect()
}

/// Suite summary for reporting.
pub fn suite_manifest() -> Vec<SuiteManifestEntry> {
    vec![
        SuiteManifestEntry {
            name: "coding",
            count: coding::coding_suite().len(),
            requires_daemon: true,
        },
        SuiteManifestEntry {
            name: "browser",
            count: browser::browser_suite().len(),
            requires_daemon: true,
        },
        SuiteManifestEntry {
            name: "productivity",
            count: productivity::productivity_suite().len(),
            requires_daemon: false,
        },
        SuiteManifestEntry {
            name: "interruption",
            count: interruption::interruption_suite().len(),
            requires_daemon: true,
        },
        SuiteManifestEntry {
            name: "human_expectation",
            count: human_expectation::human_expectation_suite().len(),
            requires_daemon: true,
        },
        SuiteManifestEntry {
            name: "stress",
            count: stress::stress_suite().len(),
            requires_daemon: true,
        },
        SuiteManifestEntry {
            name: "long_horizon",
            count: long_horizon::long_horizon_suite().len(),
            requires_daemon: true,
        },
    ]
}

#[derive(Debug, Clone)]
pub struct SuiteManifestEntry {
    pub name: &'static str,
    pub count: usize,
    pub requires_daemon: bool,
}
