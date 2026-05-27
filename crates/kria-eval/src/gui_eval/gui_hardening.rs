//! Phase 10 GUI workflow hardening audits.
//!
//! This module is intentionally report-driven. It does not add runtime
//! behavior, execute desktop actions, call models, or create another planner.
//! It audits the Phase 1-9 metadata pipeline for boundedness, ownership,
//! latency, stale-state handling, fallback honesty, and anti-hardcoding drift.

use crate::gui_eval::production_gui_workflows::{
    production_gui_workflow_suite, run_production_gui_case, run_production_gui_workflow_suite,
};
use kria_core::agent::execution_mode_reasoner::{
    EnvironmentCapabilities, ExecutionMode, ExecutionModeReasoner, PolicyContext, RequiredVerifier,
    RuntimeTruthOwnership,
};
use kria_core::agent::hybrid_synchronization::HybridSynchronizationOverallVerdict;
use kria_core::agent::intent_compiler::{ContentClass, GuiTaskSpec, TargetRef, Verb};
use kria_core::agent::semantic_workflow::analyze_semantic_workflow;
use kria_core::agent::workflow_intent_contract::ForbiddenDegradation;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Instant;

const SEMANTIC_WORKFLOW_SRC: &str =
    include_str!("../../../kria-core/src/agent/semantic_workflow.rs");
const EXECUTION_MODE_REASONER_SRC: &str =
    include_str!("../../../kria-core/src/agent/execution_mode_reasoner.rs");
const WORKFLOW_CONTRACT_SRC: &str =
    include_str!("../../../kria-core/src/agent/workflow_intent_contract.rs");
const VERIFIER_AUTHORITY_SRC: &str =
    include_str!("../../../kria-core/src/agent/verifier_authority.rs");
const HYBRID_SYNC_SRC: &str =
    include_str!("../../../kria-core/src/agent/hybrid_synchronization.rs");
const BROWSER_MEDIA_SRC: &str =
    include_str!("../../../kria-core/src/agent/browser_media_governance.rs");

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiHardeningDimension {
    PhasePrerequisites,
    LatencyBudget,
    PlannerBoundary,
    AuthorityOwnership,
    StaleStateAudit,
    SynchronizationAudit,
    FallbackHonesty,
    AppAdapterBoundary,
    AntiHardcoding,
    TraceCompleteness,
}

impl GuiHardeningDimension {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PhasePrerequisites => "phase_prerequisites",
            Self::LatencyBudget => "latency_budget",
            Self::PlannerBoundary => "planner_boundary",
            Self::AuthorityOwnership => "authority_ownership",
            Self::StaleStateAudit => "stale_state_audit",
            Self::SynchronizationAudit => "synchronization_audit",
            Self::FallbackHonesty => "fallback_honesty",
            Self::AppAdapterBoundary => "app_adapter_boundary",
            Self::AntiHardcoding => "anti_hardcoding",
            Self::TraceCompleteness => "trace_completeness",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiHardeningPrerequisiteStatus {
    pub phase9_total: usize,
    pub phase9_passed: usize,
    pub phase9_failed: usize,
    pub phase9_passed_all: bool,
    pub phase9_prerequisites_satisfied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiHardeningLatencyObservation {
    pub case_id: String,
    pub mode: ExecutionMode,
    pub elapsed_ms: u128,
    pub budget_ms: u128,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiHardeningAuditResult {
    pub id: String,
    pub dimension: GuiHardeningDimension,
    pub passed: bool,
    pub explanation: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiHardeningDimensionSummary {
    pub dimension: GuiHardeningDimension,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiHardeningSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub latency_cases: usize,
    pub latency_passed: usize,
    pub dimension_summaries: Vec<GuiHardeningDimensionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiHardeningReport {
    pub run_id: String,
    pub generated_at: String,
    pub report_version: String,
    pub prerequisite_status: GuiHardeningPrerequisiteStatus,
    pub latency_observations: Vec<GuiHardeningLatencyObservation>,
    pub summary: GuiHardeningSummary,
    pub audit_results: Vec<GuiHardeningAuditResult>,
}

pub fn run_gui_hardening_suite(run_id: impl Into<String>) -> GuiHardeningReport {
    let run_id = run_id.into();
    let prerequisite_status = phase9_prerequisite_status(&run_id);
    let latency_observations = latency_observations();
    let mut audit_results = Vec::new();

    audit_results.push(phase_prerequisite_audit(&prerequisite_status));
    audit_results.push(latency_budget_audit(&latency_observations));
    audit_results.push(planner_boundary_audit());
    audit_results.push(authority_ownership_audit());
    audit_results.push(stale_state_audit());
    audit_results.push(synchronization_audit());
    audit_results.push(fallback_honesty_audit());
    audit_results.push(app_adapter_boundary_audit());
    audit_results.push(anti_hardcoding_audit());
    audit_results.push(trace_completeness_audit());

    let total = audit_results.len();
    let passed = audit_results.iter().filter(|result| result.passed).count();
    let latency_passed = latency_observations
        .iter()
        .filter(|observation| observation.passed)
        .count();
    let summary = GuiHardeningSummary {
        total,
        passed,
        failed: total.saturating_sub(passed),
        latency_cases: latency_observations.len(),
        latency_passed,
        dimension_summaries: dimension_summaries(&audit_results),
    };

    GuiHardeningReport {
        run_id,
        generated_at: unix_now(),
        report_version: "phase10_gui_hardening_v1".to_string(),
        prerequisite_status,
        latency_observations,
        summary,
        audit_results,
    }
}

pub fn print_gui_hardening_report(report: &GuiHardeningReport) {
    println!("GUI Hardening Audit");
    println!(
        "  prerequisites.phase9_production_gui={}/{} passed",
        report.prerequisite_status.phase9_passed, report.prerequisite_status.phase9_total
    );
    println!(
        "  audits total={} passed={} failed={}",
        report.summary.total, report.summary.passed, report.summary.failed
    );
    println!(
        "  latency cases={}/{} passed",
        report.summary.latency_passed, report.summary.latency_cases
    );
    for summary in &report.summary.dimension_summaries {
        println!(
            "  {}: {}/{} passed",
            summary.dimension.as_str(),
            summary.passed,
            summary.total
        );
    }
    for result in &report.audit_results {
        let status = if result.passed { "PASS" } else { "FAIL" };
        println!("  {} {:34} {}", status, result.id, result.explanation);
    }
}

pub fn write_gui_hardening_markdown(
    report: &GuiHardeningReport,
    path: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    let mut markdown = String::new();
    markdown.push_str("# GUI Hardening Audit\n\n");
    markdown.push_str(&format!("**Run ID:** `{}`\n\n", report.run_id));
    markdown.push_str(&format!(
        "**Phase 9 prerequisite:** {}/{} production GUI workflow cases passed\n\n",
        report.prerequisite_status.phase9_passed, report.prerequisite_status.phase9_total
    ));
    markdown.push_str(&format!(
        "**Summary:** {}/{} audits passed\n\n",
        report.summary.passed, report.summary.total
    ));

    markdown.push_str("## Dimensions\n\n");
    markdown.push_str("| Dimension | Passed | Total |\n");
    markdown.push_str("|---|---:|---:|\n");
    for summary in &report.summary.dimension_summaries {
        markdown.push_str(&format!(
            "| {} | {} | {} |\n",
            summary.dimension.as_str(),
            summary.passed,
            summary.total
        ));
    }

    markdown.push_str("\n## Latency\n\n");
    markdown.push_str("| Case | Mode | Elapsed ms | Budget ms | Verdict |\n");
    markdown.push_str("|---|---|---:|---:|---|\n");
    for observation in &report.latency_observations {
        markdown.push_str(&format!(
            "| `{}` | `{:?}` | {} | {} | {} |\n",
            observation.case_id,
            observation.mode,
            observation.elapsed_ms,
            observation.budget_ms,
            if observation.passed { "pass" } else { "fail" }
        ));
    }

    markdown.push_str("\n## Audits\n\n");
    markdown.push_str("| Audit | Dimension | Verdict | Explanation |\n");
    markdown.push_str("|---|---|---|---|\n");
    for result in &report.audit_results {
        markdown.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            result.id,
            result.dimension.as_str(),
            if result.passed { "pass" } else { "fail" },
            result.explanation.replace('|', "\\|")
        ));
    }
    std::fs::write(path, markdown)
}

fn phase9_prerequisite_status(run_id: &str) -> GuiHardeningPrerequisiteStatus {
    let phase9 = run_production_gui_workflow_suite(format!("{run_id}:phase9-prerequisite"));
    GuiHardeningPrerequisiteStatus {
        phase9_total: phase9.summary.total,
        phase9_passed: phase9.summary.passed,
        phase9_failed: phase9.summary.failed,
        phase9_passed_all: phase9.summary.failed == 0 && phase9.summary.total > 0,
        phase9_prerequisites_satisfied: phase9.summary.prerequisites_satisfied,
    }
}

fn latency_observations() -> Vec<GuiHardeningLatencyObservation> {
    production_gui_workflow_suite()
        .into_iter()
        .map(|case| {
            let started = Instant::now();
            let result = run_production_gui_case(case);
            let elapsed_ms = started.elapsed().as_millis();
            let budget_ms = latency_budget_ms(result.observation.mode);
            GuiHardeningLatencyObservation {
                case_id: result.case.id,
                mode: result.observation.mode,
                elapsed_ms,
                budget_ms,
                passed: elapsed_ms <= budget_ms,
            }
        })
        .collect()
}

fn latency_budget_ms(mode: ExecutionMode) -> u128 {
    match mode {
        ExecutionMode::StructuralExecution | ExecutionMode::SilentAutomationWorkflow => 500,
        ExecutionMode::VisibleAppWorkflow
        | ExecutionMode::HybridWorkflow
        | ExecutionMode::VerificationVisibleWorkflow => 1_000,
        ExecutionMode::HumanCollaborativeWorkflow => 2_000,
    }
}

fn phase_prerequisite_audit(status: &GuiHardeningPrerequisiteStatus) -> GuiHardeningAuditResult {
    audit(
        "hardening-001-phase-prerequisites",
        GuiHardeningDimension::PhasePrerequisites,
        status.phase9_passed_all && status.phase9_prerequisites_satisfied,
        "Phase 9 production GUI evals and inherited Phase 8 prerequisite are green.",
        "Phase 9 production GUI prerequisite failed.",
        vec![
            format!("phase9_total::{}", status.phase9_total),
            format!("phase9_passed::{}", status.phase9_passed),
            format!(
                "phase9_prerequisites_satisfied::{}",
                status.phase9_prerequisites_satisfied
            ),
        ],
    )
}

fn latency_budget_audit(
    observations: &[GuiHardeningLatencyObservation],
) -> GuiHardeningAuditResult {
    let failed = observations
        .iter()
        .filter(|observation| !observation.passed)
        .map(|observation| {
            format!(
                "{}:{}ms>{}ms",
                observation.case_id, observation.elapsed_ms, observation.budget_ms
            )
        })
        .collect::<Vec<_>>();
    audit(
        "hardening-002-latency-budgets",
        GuiHardeningDimension::LatencyBudget,
        failed.is_empty() && !observations.is_empty(),
        "Semantic workflow reasoning stays within deterministic planning budgets.",
        "One or more semantic workflow cases exceeded the planning budget.",
        if failed.is_empty() {
            observations
                .iter()
                .map(|observation| {
                    format!(
                        "{}::{}ms/{}ms",
                        observation.case_id, observation.elapsed_ms, observation.budget_ms
                    )
                })
                .collect()
        } else {
            failed
        },
    )
}

fn planner_boundary_audit() -> GuiHardeningAuditResult {
    let checks = [
        (
            "semantic_workflow",
            source_before_tests(SEMANTIC_WORKFLOW_SRC),
            &[
                "ExecutionModeReasoner",
                "WorkflowIntentContractRegistry",
                "VerifierAuthorityEvaluator",
                "SubstratePlanner",
                "Command::",
                ".await",
            ][..],
        ),
        (
            "workflow_intent_contract",
            source_before_tests(WORKFLOW_CONTRACT_SRC),
            &[
                "SubstratePlanner",
                "GuiExecutor",
                "execute_bash",
                "Command::",
                "tokio::spawn",
                ".await",
            ][..],
        ),
        (
            "verifier_authority",
            source_before_tests(VERIFIER_AUTHORITY_SRC),
            &[
                "SubstratePlanner",
                "RecoveryManager",
                "ExecutionModeReasoner.decide",
                "Command::",
                "tokio::spawn",
                ".await",
            ][..],
        ),
    ];
    let violations = source_violations(&checks);
    audit(
        "hardening-003-planner-boundaries",
        GuiHardeningDimension::PlannerBoundary,
        violations.is_empty(),
        "Semantic, contract, and verifier modules do not contain hidden planner/executor hooks.",
        "Planner or executor responsibility leaked into semantic/contract/verifier modules.",
        if violations.is_empty() {
            vec![
                "semantic_owner::SemanticWorkflowFrame".to_string(),
                "mode_owner::ExecutionModeDecision".to_string(),
                "truth_owner::Verifier".to_string(),
            ]
        } else {
            violations
        },
    )
}

fn authority_ownership_audit() -> GuiHardeningAuditResult {
    let ownership = sample_decision().ownership;
    let expected = RuntimeTruthOwnership::canonical();
    let passed = ownership == expected;
    audit(
        "hardening-004-authority-ownership",
        GuiHardeningDimension::AuthorityOwnership,
        passed,
        "Runtime truth ownership matches the canonical one-owner-per-object model.",
        "Runtime truth ownership drifted from the canonical owner map.",
        vec![
            format!(
                "workflow_semantics_owner::{}",
                ownership.workflow_semantics_owner
            ),
            format!("execution_style_owner::{}", ownership.execution_style_owner),
            format!("invariants_owner::{}", ownership.invariants_owner),
            format!("execution_steps_owner::{}", ownership.execution_steps_owner),
            format!(
                "completion_truth_owner::{}",
                ownership.completion_truth_owner
            ),
        ],
    )
}

fn stale_state_audit() -> GuiHardeningAuditResult {
    let result = production_gui_workflow_suite()
        .into_iter()
        .find(|case| case.id == "prod-gui-013-hybrid-stale-state")
        .map(run_production_gui_case);
    let passed = result
        .as_ref()
        .and_then(|result| result.observation.hybrid_sync_verdict)
        == Some(HybridSynchronizationOverallVerdict::Invalidated);
    audit(
        "hardening-005-stale-state-invalidation",
        GuiHardeningDimension::StaleStateAudit,
        passed,
        "Stale visible IDE state invalidates hybrid completion.",
        "Stale visible state was not rejected by hybrid synchronization.",
        result
            .map(|result| result.observation.evidence)
            .unwrap_or_else(|| vec!["missing_case::prod-gui-013-hybrid-stale-state".to_string()]),
    )
}

fn synchronization_audit() -> GuiHardeningAuditResult {
    let result = production_gui_workflow_suite()
        .into_iter()
        .find(|case| case.id == "prod-gui-001-visible-coding-hybrid")
        .map(run_production_gui_case);
    let passed = result.as_ref().is_some_and(|result| {
        result
            .observation
            .required_verifiers
            .contains(&RequiredVerifier::IdeFileVisible)
            && result
                .observation
                .required_verifiers
                .contains(&RequiredVerifier::OutputSurfaced)
            && result.observation.mode == ExecutionMode::HybridWorkflow
    });
    audit(
        "hardening-006-hybrid-sync-coverage",
        GuiHardeningDimension::SynchronizationAudit,
        passed,
        "Hybrid visible coding retains file-visible and output-surfaced verifier requirements.",
        "Hybrid visible coding lacks required synchronization verifier coverage.",
        result
            .map(|result| result.observation.evidence)
            .unwrap_or_else(
                || vec!["missing_case::prod-gui-001-visible-coding-hybrid".to_string()],
            ),
    )
}

fn fallback_honesty_audit() -> GuiHardeningAuditResult {
    let result = production_gui_workflow_suite()
        .into_iter()
        .find(|case| case.id == "prod-gui-011-fallback-honesty")
        .map(run_production_gui_case);
    let passed = result.as_ref().is_some_and(|result| {
        result
            .observation
            .forbidden_degradations
            .contains(&ForbiddenDegradation::SilentVisibleToStructural)
            && result.observation.partial_completion_required
            && result.observation.recovery_pause_or_disclosure_required
    });
    audit(
        "hardening-007-fallback-honesty",
        GuiHardeningDimension::FallbackHonesty,
        passed,
        "Visible-to-structural downgrade is reported as partial completion requiring disclosure.",
        "Visible-to-structural downgrade was not captured as honest fallback.",
        result
            .map(|result| result.observation.evidence)
            .unwrap_or_else(|| vec!["missing_case::prod-gui-011-fallback-honesty".to_string()]),
    )
}

fn app_adapter_boundary_audit() -> GuiHardeningAuditResult {
    let checks = [
        (
            "browser_media_governance",
            source_before_tests(BROWSER_MEDIA_SRC),
            &[
                "Command::",
                "std::process",
                "execute_bash",
                "open_url",
                "play_media",
                "SubstratePlanner",
                "GuiExecutor",
                ".await",
            ][..],
        ),
        (
            "hybrid_synchronization",
            source_before_tests(HYBRID_SYNC_SRC),
            &[
                "Command::",
                "std::process",
                "execute_bash",
                "open_url",
                "SubstratePlanner",
                "GuiExecutor",
                ".await",
            ][..],
        ),
    ];
    let violations = source_violations(&checks);
    audit(
        "hardening-008-app-adapter-boundaries",
        GuiHardeningDimension::AppAdapterBoundary,
        violations.is_empty(),
        "Capability/governance helpers remain metadata providers, not workflow brains.",
        "Capability/governance helper contains execution or planning hooks.",
        if violations.is_empty() {
            vec![
                "browser_media_governance::metadata_only".to_string(),
                "hybrid_synchronization::metadata_only".to_string(),
            ]
        } else {
            violations
        },
    )
}

fn anti_hardcoding_audit() -> GuiHardeningAuditResult {
    let checks = [
        (
            "semantic_workflow",
            source_before_tests(SEMANTIC_WORKFLOW_SRC),
            &[
                "pascal triangle",
                "open code and write",
                "run it and show output",
                "prod-gui-",
                "fidelity-",
            ][..],
        ),
        (
            "execution_mode_reasoner",
            source_before_tests(EXECUTION_MODE_REASONER_SRC),
            &[
                "pascal triangle",
                "open code and write",
                "run it and show output",
                "prod-gui-",
                "fidelity-",
            ][..],
        ),
        (
            "workflow_intent_contract",
            source_before_tests(WORKFLOW_CONTRACT_SRC),
            &[
                "pascal triangle",
                "open code and write",
                "run it and show output",
                "prod-gui-",
                "fidelity-",
            ][..],
        ),
    ];
    let violations = source_violations(&checks);
    audit(
        "hardening-009-anti-hardcoding",
        GuiHardeningDimension::AntiHardcoding,
        violations.is_empty(),
        "Runtime semantic/mode/contract modules avoid eval prompt-specific routing strings.",
        "Runtime module contains eval prompt-specific routing text.",
        if violations.is_empty() {
            vec![
                "runtime_modules_scanned::3".to_string(),
                "prompt_specific_runtime_routes::0".to_string(),
            ]
        } else {
            violations
        },
    )
}

fn trace_completeness_audit() -> GuiHardeningAuditResult {
    let results = production_gui_workflow_suite()
        .into_iter()
        .map(run_production_gui_case)
        .collect::<Vec<_>>();
    let missing = results
        .iter()
        .filter(|result| {
            result.observation.trace_labels.is_empty() || result.observation.evidence.is_empty()
        })
        .map(|result| result.case.id.clone())
        .collect::<Vec<_>>();
    audit(
        "hardening-010-trace-completeness",
        GuiHardeningDimension::TraceCompleteness,
        missing.is_empty() && !results.is_empty(),
        "Every production GUI workflow eval emits trace labels and evidence.",
        "One or more production GUI workflow evals lacks trace or evidence metadata.",
        if missing.is_empty() {
            vec![format!("cases_with_trace::{}", results.len())]
        } else {
            missing
        },
    )
}

fn audit(
    id: &str,
    dimension: GuiHardeningDimension,
    passed: bool,
    success: &str,
    failure: &str,
    evidence: Vec<String>,
) -> GuiHardeningAuditResult {
    GuiHardeningAuditResult {
        id: id.to_string(),
        dimension,
        passed,
        explanation: if passed {
            success.to_string()
        } else {
            failure.to_string()
        },
        evidence,
    }
}

fn sample_decision() -> kria_core::agent::execution_mode_reasoner::ExecutionModeDecision {
    let spec = GuiTaskSpec {
        primary_verb: Verb::Open,
        targets: vec![TargetRef::App("VS Code".to_string())],
        content: Some(ContentClass::Generated {
            hint: "small python script".to_string(),
            language: Some("python".to_string()),
        }),
        declared_preconditions: Vec::new(),
        declared_success_criteria: Vec::new(),
        ambiguities: Vec::new(),
    };
    let analysis = analyze_semantic_workflow(
        &spec,
        "open code and write a program, run it and show output",
    );
    ExecutionModeReasoner.decide(
        &spec,
        &analysis,
        &EnvironmentCapabilities::unchecked_default(),
        &PolicyContext::default(),
    )
}

fn source_before_tests(source: &'static str) -> &'static str {
    source.split("#[cfg(test)]").next().unwrap_or(source)
}

fn source_violations(checks: &[(&str, &str, &[&str])]) -> Vec<String> {
    let mut violations = Vec::new();
    for (module, source, forbidden_patterns) in checks {
        for pattern in *forbidden_patterns {
            if source.contains(pattern) {
                violations.push(format!("{module}::{pattern}"));
            }
        }
    }
    violations
}

fn dimension_summaries(results: &[GuiHardeningAuditResult]) -> Vec<GuiHardeningDimensionSummary> {
    let mut counts = BTreeMap::<GuiHardeningDimension, (usize, usize)>::new();
    for result in results {
        let entry = counts.entry(result.dimension).or_insert((0, 0));
        entry.0 += 1;
        if result.passed {
            entry.1 += 1;
        }
    }
    counts
        .into_iter()
        .map(
            |(dimension, (total, passed))| GuiHardeningDimensionSummary {
                dimension,
                total,
                passed,
                failed: total.saturating_sub(passed),
            },
        )
        .collect()
}

fn unix_now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardening_suite_passes_all_audits() {
        let report = run_gui_hardening_suite("phase10-test-run");

        assert!(report.prerequisite_status.phase9_passed_all);
        assert!(report.prerequisite_status.phase9_prerequisites_satisfied);
        assert_eq!(report.summary.failed, 0);
        assert_eq!(report.summary.latency_cases, report.summary.latency_passed);
    }

    #[test]
    fn hardening_suite_covers_required_dimensions() {
        let report = run_gui_hardening_suite("phase10-test-run");
        let dimensions = report
            .summary
            .dimension_summaries
            .iter()
            .map(|summary| summary.dimension)
            .collect::<Vec<_>>();

        for expected in [
            GuiHardeningDimension::PhasePrerequisites,
            GuiHardeningDimension::LatencyBudget,
            GuiHardeningDimension::PlannerBoundary,
            GuiHardeningDimension::AuthorityOwnership,
            GuiHardeningDimension::StaleStateAudit,
            GuiHardeningDimension::SynchronizationAudit,
            GuiHardeningDimension::FallbackHonesty,
            GuiHardeningDimension::AppAdapterBoundary,
            GuiHardeningDimension::AntiHardcoding,
            GuiHardeningDimension::TraceCompleteness,
        ] {
            assert!(dimensions.contains(&expected), "missing {expected:?}");
        }
    }

    #[test]
    fn source_boundary_audits_pass_individually() {
        assert!(planner_boundary_audit().passed);
        assert!(app_adapter_boundary_audit().passed);
        assert!(anti_hardcoding_audit().passed);
    }

    #[test]
    fn report_serializes_to_json() {
        let report = run_gui_hardening_suite("phase10-test-run");
        let json = serde_json::to_string_pretty(&report).expect("serialize");
        let roundtrip: GuiHardeningReport = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(roundtrip.summary.total, report.summary.total);
        assert!(json.contains("phase10_gui_hardening_v1"));
    }

    #[test]
    fn markdown_report_contains_latency_and_audits() {
        let report = run_gui_hardening_suite("phase10-test-run");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("gui_hardening.md");
        write_gui_hardening_markdown(&report, &path).expect("markdown write");
        let markdown = std::fs::read_to_string(path).expect("read markdown");

        assert!(markdown.contains("Phase 9 prerequisite"));
        assert!(markdown.contains("Latency"));
        assert!(markdown.contains("hardening-010-trace-completeness"));
    }
}
