//! Phase 9 observability and replay-scope scoring.
//!
//! This module scores whether existing eval reports are debuggable. It does not
//! implement desktop replay. Replay scope is limited to decision/evidence
//! reconstruction from compact report artifacts.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

pub const OBSERVABILITY_REPLAY_SCOPE: &str = "decision_evidence_reconstruction_only";

const DEFAULT_REPORT_PATHS: &[(&str, ObservabilityReportKind)] = &[
    (
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../eval_reports/gui_latest_run.json"
        ),
        ObservabilityReportKind::GuiAutomation,
    ),
    (
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../eval_reports/hitl_timeline_latest_run.json"
        ),
        ObservabilityReportKind::HitlTimeline,
    ),
    (
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../eval_reports/llm_cognition_latest_run.json"
        ),
        ObservabilityReportKind::LlmCognition,
    ),
    (
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../eval_reports/destructive_safety_latest_run.json"
        ),
        ObservabilityReportKind::DestructiveSafety,
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservabilityReportKind {
    GuiAutomation,
    HitlTimeline,
    LlmCognition,
    DestructiveSafety,
}

impl ObservabilityReportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GuiAutomation => "gui_automation",
            Self::HitlTimeline => "hitl_timeline",
            Self::LlmCognition => "llm_cognition",
            Self::DestructiveSafety => "destructive_safety",
        }
    }

    pub fn reproduction_command(self) -> &'static str {
        match self {
            Self::GuiAutomation => "cargo run -p kria-eval -- --gui",
            Self::HitlTimeline => "cargo run -p kria-eval -- --gui-hitl",
            Self::LlmCognition => "cargo run -p kria-eval -- --gui-llm",
            Self::DestructiveSafety => "cargo run -p kria-eval -- --gui-destructive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservabilityCheckSeverity {
    Critical,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityCheck {
    pub name: String,
    pub passed: bool,
    pub severity: ObservabilityCheckSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilitySourceScore {
    pub report_kind: ObservabilityReportKind,
    pub report_path: String,
    pub report_present: bool,
    pub run_id: Option<String>,
    pub total_items: usize,
    pub failed_items: usize,
    pub observability_score: u8,
    pub release_blocking_missing_fields: Vec<String>,
    pub warning_missing_fields: Vec<String>,
    pub replay_scope: String,
    pub reproduction_command: String,
    pub checks: Vec<ObservabilityCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityScoreSummary {
    pub total_sources: usize,
    pub present_sources: usize,
    pub passed_sources: usize,
    pub failed_sources: usize,
    pub average_score: u8,
    pub release_blocking_missing_fields: usize,
    pub warning_missing_fields: usize,
    pub replay_scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityScoreReport {
    pub generated_at: String,
    pub replay_scope: String,
    pub summary: ObservabilityScoreSummary,
    pub source_scores: Vec<ObservabilitySourceScore>,
}

pub fn score_latest_observability_reports() -> ObservabilityScoreReport {
    let source_scores = DEFAULT_REPORT_PATHS
        .iter()
        .map(|(path, kind)| score_report_path(path, *kind))
        .collect::<Vec<_>>();
    build_report(source_scores)
}

pub fn score_report_path(
    path: impl AsRef<Path>,
    kind: ObservabilityReportKind,
) -> ObservabilitySourceScore {
    let path_ref = path.as_ref();
    let report_path = path_ref.display().to_string();
    match std::fs::read_to_string(path_ref) {
        Ok(payload) => match serde_json::from_str::<Value>(&payload) {
            Ok(value) => score_report_value(kind, report_path, &value),
            Err(error) => missing_report_score(
                kind,
                report_path,
                format!("report JSON is invalid: {error}"),
            ),
        },
        Err(error) => missing_report_score(kind, report_path, format!("report missing: {error}")),
    }
}

pub fn score_report_value(
    kind: ObservabilityReportKind,
    report_path: impl Into<String>,
    value: &Value,
) -> ObservabilitySourceScore {
    let report_path = report_path.into();
    let mut checks = common_checks(kind, value);
    match kind {
        ObservabilityReportKind::GuiAutomation => checks.extend(gui_checks(value)),
        ObservabilityReportKind::HitlTimeline => checks.extend(hitl_checks(value)),
        ObservabilityReportKind::LlmCognition => checks.extend(llm_checks(value)),
        ObservabilityReportKind::DestructiveSafety => checks.extend(destructive_checks(value)),
    }

    let total_items = count_items(kind, value);
    let failed_items = count_failed_items(kind, value);
    let score = score_checks(&checks);
    let release_blocking_missing_fields =
        missing_fields(&checks, ObservabilityCheckSeverity::Critical);
    let warning_missing_fields = missing_fields(&checks, ObservabilityCheckSeverity::Warning);

    ObservabilitySourceScore {
        report_kind: kind,
        report_path,
        report_present: true,
        run_id: string_at(value, &["run_id"]),
        total_items,
        failed_items,
        observability_score: score,
        release_blocking_missing_fields,
        warning_missing_fields,
        replay_scope: OBSERVABILITY_REPLAY_SCOPE.to_string(),
        reproduction_command: kind.reproduction_command().to_string(),
        checks,
    }
}

pub fn print_observability_score_report(report: &ObservabilityScoreReport) {
    println!("── Observability / Replay Scope Score ─────────────────────────");
    println!("  Replay Scope:       {}", report.replay_scope);
    println!("  Sources:            {}", report.summary.total_sources);
    println!("  Present Sources:    {}", report.summary.present_sources);
    println!("  Passed Sources:     {}", report.summary.passed_sources);
    println!("  Failed Sources:     {}", report.summary.failed_sources);
    println!("  Average Score:      {}", report.summary.average_score);
    println!(
        "  Critical Missing:   {}",
        report.summary.release_blocking_missing_fields
    );
    println!(
        "  Warning Missing:    {}",
        report.summary.warning_missing_fields
    );
    for source in &report.source_scores {
        println!(
            "  {} [{}] score={} critical_missing={} warnings={}",
            if source.release_blocking_missing_fields.is_empty() {
                "PASS"
            } else {
                "FAIL"
            },
            source.report_kind.as_str(),
            source.observability_score,
            source.release_blocking_missing_fields.len(),
            source.warning_missing_fields.len()
        );
        if !source.release_blocking_missing_fields.is_empty() {
            println!(
                "     critical: {}",
                source.release_blocking_missing_fields.join(", ")
            );
        }
    }
    println!();
}

pub fn write_observability_markdown(
    report: &ObservabilityScoreReport,
    path: impl AsRef<Path>,
) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str("# KRIA Eval Observability Score\n\n");
    out.push_str(&format!("Replay scope: `{}`\n\n", report.replay_scope));
    out.push_str("| Source | Score | Critical Missing | Warning Missing | Reproduction |\n");
    out.push_str("|---|---:|---:|---:|---|\n");
    for source in &report.source_scores {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | `{}` |\n",
            source.report_kind.as_str(),
            source.observability_score,
            source.release_blocking_missing_fields.len(),
            source.warning_missing_fields.len(),
            source.reproduction_command
        ));
    }
    out.push_str("\n## Missing Fields\n\n");
    for source in &report.source_scores {
        if source.release_blocking_missing_fields.is_empty()
            && source.warning_missing_fields.is_empty()
        {
            continue;
        }
        out.push_str(&format!("### {}\n\n", source.report_kind.as_str()));
        if !source.release_blocking_missing_fields.is_empty() {
            out.push_str(&format!(
                "- critical: {}\n",
                source.release_blocking_missing_fields.join(", ")
            ));
        }
        if !source.warning_missing_fields.is_empty() {
            out.push_str(&format!(
                "- warning: {}\n",
                source.warning_missing_fields.join(", ")
            ));
        }
        out.push('\n');
    }
    std::fs::write(path, out)
}

fn build_report(source_scores: Vec<ObservabilitySourceScore>) -> ObservabilityScoreReport {
    let total_sources = source_scores.len();
    let present_sources = source_scores
        .iter()
        .filter(|source| source.report_present)
        .count();
    let passed_sources = source_scores
        .iter()
        .filter(|source| source.release_blocking_missing_fields.is_empty())
        .count();
    let failed_sources = total_sources.saturating_sub(passed_sources);
    let score_sum: usize = source_scores
        .iter()
        .map(|source| source.observability_score as usize)
        .sum();
    let average_score = if total_sources == 0 {
        0
    } else {
        (score_sum / total_sources) as u8
    };
    let release_blocking_missing_fields = source_scores
        .iter()
        .map(|source| source.release_blocking_missing_fields.len())
        .sum();
    let warning_missing_fields = source_scores
        .iter()
        .map(|source| source.warning_missing_fields.len())
        .sum();

    ObservabilityScoreReport {
        generated_at: unix_now(),
        replay_scope: OBSERVABILITY_REPLAY_SCOPE.to_string(),
        summary: ObservabilityScoreSummary {
            total_sources,
            present_sources,
            passed_sources,
            failed_sources,
            average_score,
            release_blocking_missing_fields,
            warning_missing_fields,
            replay_scope: OBSERVABILITY_REPLAY_SCOPE.to_string(),
        },
        source_scores,
    }
}

fn missing_report_score(
    kind: ObservabilityReportKind,
    report_path: String,
    message: String,
) -> ObservabilitySourceScore {
    let checks = vec![ObservabilityCheck {
        name: "report_present".to_string(),
        passed: false,
        severity: ObservabilityCheckSeverity::Critical,
        message: message.clone(),
    }];
    ObservabilitySourceScore {
        report_kind: kind,
        report_path,
        report_present: false,
        run_id: None,
        total_items: 0,
        failed_items: 0,
        observability_score: 0,
        release_blocking_missing_fields: vec!["report_present".to_string()],
        warning_missing_fields: Vec::new(),
        replay_scope: OBSERVABILITY_REPLAY_SCOPE.to_string(),
        reproduction_command: kind.reproduction_command().to_string(),
        checks,
    }
}

fn common_checks(kind: ObservabilityReportKind, value: &Value) -> Vec<ObservabilityCheck> {
    vec![
        check(
            "run_id_present",
            string_at(value, &["run_id"]).is_some(),
            ObservabilityCheckSeverity::Critical,
            "report must include a stable run id",
        ),
        check(
            "generated_at_present",
            string_at(value, &["generated_at"]).is_some(),
            ObservabilityCheckSeverity::Warning,
            "report should include generation time",
        ),
        check(
            "summary_present",
            value.get("summary").is_some(),
            ObservabilityCheckSeverity::Critical,
            "report must include aggregate summary",
        ),
        check(
            "items_present",
            count_items(kind, value) > 0,
            ObservabilityCheckSeverity::Critical,
            "report must include case results or matrix cells",
        ),
        check(
            "reproduction_command_available",
            !kind.reproduction_command().is_empty(),
            ObservabilityCheckSeverity::Critical,
            "scorer must provide a reproduction command",
        ),
    ]
}

fn gui_checks(value: &Value) -> Vec<ObservabilityCheck> {
    let cases = array_at(value, &["case_results"]);
    let non_pass = cases
        .iter()
        .filter(|case| string_at(case, &["verdict"]).as_deref() != Some("PASS"))
        .collect::<Vec<_>>();
    let failure_bundle_count = value
        .pointer("/failure_bundles/written")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    vec![
        check(
            "governance_present",
            value.get("governance").is_some(),
            ObservabilityCheckSeverity::Critical,
            "GUI report must include capability governance",
        ),
        check(
            "failure_bundle_summary_present",
            value.get("failure_bundles").is_some(),
            ObservabilityCheckSeverity::Critical,
            "GUI report must include failure bundle summary",
        ),
        check(
            "non_pass_cases_have_root_cause",
            non_pass
                .iter()
                .all(|case| string_at(case, &["observability", "root_cause_category"]).is_some()),
            ObservabilityCheckSeverity::Critical,
            "non-pass GUI cases need root cause category",
        ),
        check(
            "non_pass_cases_have_reproduction_command",
            non_pass
                .iter()
                .all(|case| string_at(case, &["observability", "reproduction_command"]).is_some()),
            ObservabilityCheckSeverity::Critical,
            "non-pass GUI cases need reproduction command",
        ),
        check(
            "non_pass_cases_have_failure_bundles",
            non_pass.is_empty() || failure_bundle_count >= non_pass.len(),
            ObservabilityCheckSeverity::Warning,
            "non-pass GUI cases should have bounded failure bundles",
        ),
        check(
            "cases_have_capability_ids",
            cases
                .iter()
                .all(|case| !array_at(case, &["governance", "capability_ids"]).is_empty()),
            ObservabilityCheckSeverity::Critical,
            "GUI cases must include capability ids",
        ),
        check(
            "cases_have_failure_mode_ids",
            cases
                .iter()
                .all(|case| !array_at(case, &["governance", "failure_mode_ids"]).is_empty()),
            ObservabilityCheckSeverity::Critical,
            "GUI cases must include failure mode ids",
        ),
        check(
            "cases_have_preflight_and_invariants",
            cases
                .iter()
                .all(|case| case.get("preflight").is_some() && case.get("invariants").is_some()),
            ObservabilityCheckSeverity::Critical,
            "GUI cases must include preflight and invariant evidence",
        ),
        check(
            "cases_have_evidence",
            cases
                .iter()
                .all(|case| !array_at(case, &["evidence"]).is_empty()),
            ObservabilityCheckSeverity::Critical,
            "GUI cases must include evidence",
        ),
    ]
}

fn hitl_checks(value: &Value) -> Vec<ObservabilityCheck> {
    let cases = array_at(value, &["case_results"]);
    vec![
        check(
            "hitl_cases_have_capability_ids",
            cases
                .iter()
                .all(|case| !array_at(case, &["case", "capability_ids"]).is_empty()),
            ObservabilityCheckSeverity::Critical,
            "HITL cases must include capability ids",
        ),
        check(
            "hitl_cases_have_failure_mode_ids",
            cases
                .iter()
                .all(|case| !array_at(case, &["case", "failure_mode_ids"]).is_empty()),
            ObservabilityCheckSeverity::Critical,
            "HITL cases must include failure mode ids",
        ),
        check(
            "hitl_cases_have_decision_state",
            cases
                .iter()
                .all(hitl_case_has_decision_or_policy_block_state),
            ObservabilityCheckSeverity::Critical,
            "HITL observations need decision state, or explicit pre-decision policy block state",
        ),
        check(
            "hitl_cases_have_execution_safety_flags",
            cases.iter().all(|case| {
                case.pointer("/observation/resume_allowed")
                    .and_then(Value::as_bool)
                    .is_some()
                    && case
                        .pointer("/observation/side_effect_allowed")
                        .and_then(Value::as_bool)
                        .is_some()
            }),
            ObservabilityCheckSeverity::Critical,
            "HITL observations need resume and side-effect flags",
        ),
        check(
            "hitl_cases_have_evidence",
            cases.iter().all(|case| {
                !array_at(case, &["observation", "evidence"]).is_empty()
                    && !array_at(case, &["verdict", "evidence"]).is_empty()
            }),
            ObservabilityCheckSeverity::Critical,
            "HITL cases need observation and verdict evidence",
        ),
    ]
}

fn hitl_case_has_decision_or_policy_block_state(case: &&Value) -> bool {
    let final_state = string_at(case, &["observation", "final_state"]);
    let event_count = case
        .pointer("/observation/event_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let policy_blocked = case
        .pointer("/observation/policy_blocked")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if final_state.as_deref() == Some("blocked_by_policy") {
        return policy_blocked && !array_at(case, &["observation", "evidence"]).is_empty();
    }

    final_state.is_some() && event_count > 0
}

fn llm_checks(value: &Value) -> Vec<ObservabilityCheck> {
    let cells = array_at(value, &["cells"]);
    vec![
        check(
            "llm_budget_present",
            value.get("budget").is_some(),
            ObservabilityCheckSeverity::Critical,
            "LLM cognition matrix must include budget",
        ),
        check(
            "llm_profiles_present",
            !array_at(value, &["profiles"]).is_empty(),
            ObservabilityCheckSeverity::Critical,
            "LLM cognition matrix must include provider profiles",
        ),
        check(
            "llm_cells_advisory_only",
            cells
                .iter()
                .all(|cell| cell.pointer("/advisory_only").and_then(Value::as_bool) == Some(true)),
            ObservabilityCheckSeverity::Critical,
            "LLM cells must remain advisory-only",
        ),
        check(
            "llm_cells_have_structural_authority",
            cells.iter().all(|cell| {
                string_at(cell, &["structural_authority"]).as_deref()
                    == Some("policy_verifier_gui_oracle_wins")
            }),
            ObservabilityCheckSeverity::Critical,
            "LLM cells must record structural authority",
        ),
        check(
            "llm_cells_have_capability_and_failure_ids",
            cells.iter().all(|cell| {
                !array_at(cell, &["capability_ids"]).is_empty()
                    && !array_at(cell, &["failure_mode_ids"]).is_empty()
            }),
            ObservabilityCheckSeverity::Critical,
            "LLM cells need capability and failure mappings",
        ),
        check(
            "llm_cells_have_status_reason",
            cells.iter().all(|cell| {
                string_at(cell, &["status"]).is_some() && string_at(cell, &["reason"]).is_some()
            }),
            ObservabilityCheckSeverity::Critical,
            "LLM cells need status and reason",
        ),
    ]
}

fn destructive_checks(value: &Value) -> Vec<ObservabilityCheck> {
    let cases = array_at(value, &["case_results"]);
    vec![
        check(
            "destructive_dry_run_only",
            value.pointer("/dry_run_only").and_then(Value::as_bool) == Some(true),
            ObservabilityCheckSeverity::Critical,
            "destructive eval must be dry-run only",
        ),
        check(
            "destructive_guard_present",
            value.get("guard").is_some(),
            ObservabilityCheckSeverity::Critical,
            "destructive eval must record isolation guard",
        ),
        check(
            "destructive_cases_have_policy_evidence",
            cases.iter().all(|case| {
                string_at(case, &["observation", "policy_risk"]).is_some()
                    && case
                        .pointer("/observation/policy_blocked")
                        .and_then(Value::as_bool)
                        .is_some()
                    && case
                        .pointer("/observation/policy_requires_approval")
                        .and_then(Value::as_bool)
                        .is_some()
            }),
            ObservabilityCheckSeverity::Critical,
            "destructive observations need policy evidence",
        ),
        check(
            "destructive_cases_have_guard_evidence",
            cases.iter().all(|case| {
                case.pointer("/observation/isolation_guard_complete")
                    .and_then(Value::as_bool)
                    .is_some()
                    && case
                        .get("observation")
                        .and_then(|obs| obs.get("missing_guard_reasons"))
                        .is_some()
            }),
            ObservabilityCheckSeverity::Critical,
            "destructive observations need guard evidence",
        ),
        check(
            "destructive_cases_have_no_execution",
            cases.iter().all(|case| {
                case.pointer("/observation/attempted_execution")
                    .and_then(Value::as_bool)
                    == Some(false)
                    && case
                        .pointer("/observation/host_execution_possible")
                        .and_then(Value::as_bool)
                        == Some(false)
            }),
            ObservabilityCheckSeverity::Critical,
            "destructive eval must prove no execution was attempted",
        ),
        check(
            "destructive_cases_have_capability_and_failure_ids",
            cases.iter().all(|case| {
                !array_at(case, &["case", "capability_ids"]).is_empty()
                    && !array_at(case, &["case", "failure_mode_ids"]).is_empty()
            }),
            ObservabilityCheckSeverity::Critical,
            "destructive cases need capability and failure mappings",
        ),
        check(
            "destructive_cases_have_evidence",
            cases.iter().all(|case| {
                !array_at(case, &["observation", "evidence"]).is_empty()
                    && !array_at(case, &["verdict", "evidence"]).is_empty()
            }),
            ObservabilityCheckSeverity::Critical,
            "destructive cases need observation and verdict evidence",
        ),
    ]
}

fn count_items(kind: ObservabilityReportKind, value: &Value) -> usize {
    match kind {
        ObservabilityReportKind::GuiAutomation
        | ObservabilityReportKind::HitlTimeline
        | ObservabilityReportKind::DestructiveSafety => array_at(value, &["case_results"]).len(),
        ObservabilityReportKind::LlmCognition => array_at(value, &["cells"]).len(),
    }
}

fn count_failed_items(kind: ObservabilityReportKind, value: &Value) -> usize {
    match kind {
        ObservabilityReportKind::GuiAutomation => array_at(value, &["case_results"])
            .iter()
            .filter(|case| {
                let verdict = string_at(case, &["verdict"]).unwrap_or_default();
                verdict != "PASS" && verdict != "SKIP"
            })
            .count(),
        ObservabilityReportKind::HitlTimeline | ObservabilityReportKind::DestructiveSafety => {
            array_at(value, &["case_results"])
                .iter()
                .filter(|case| {
                    !case
                        .pointer("/verdict/passed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
                .count()
        }
        ObservabilityReportKind::LlmCognition => array_at(value, &["cells"])
            .iter()
            .filter(|cell| string_at(cell, &["status"]).as_deref() == Some("structural_blocked"))
            .count(),
    }
}

fn check(
    name: &str,
    passed: bool,
    severity: ObservabilityCheckSeverity,
    message: &str,
) -> ObservabilityCheck {
    ObservabilityCheck {
        name: name.to_string(),
        passed,
        severity,
        message: message.to_string(),
    }
}

fn score_checks(checks: &[ObservabilityCheck]) -> u8 {
    if checks.is_empty() {
        return 0;
    }
    let passed = checks.iter().filter(|check| check.passed).count();
    ((passed * 100) / checks.len()) as u8
}

fn missing_fields(
    checks: &[ObservabilityCheck],
    severity: ObservabilityCheckSeverity,
) -> Vec<String> {
    checks
        .iter()
        .filter(|check| !check.passed && check.severity == severity)
        .map(|check| check.name.clone())
        .collect()
}

fn array_at<'a>(value: &'a Value, path: &[&str]) -> Vec<&'a Value> {
    let mut current = value;
    for key in path {
        let Some(next) = current.get(*key) else {
            return Vec::new();
        };
        current = next;
    }
    current
        .as_array()
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(ToString::to_string)
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
    use serde_json::json;

    #[test]
    fn gui_report_requires_non_pass_debug_fields() {
        let report = json!({
            "run_id": "gui-unit",
            "generated_at": "1",
            "summary": {"total_cases": 1},
            "governance": {},
            "failure_bundles": {"written": 1},
            "case_results": [{
                "case_id": "case-1",
                "verdict": "ENVIRONMENT_BLOCKED",
                "evidence": ["blocked"],
                "observability": {
                    "root_cause_category": "environment_blocked",
                    "reproduction_command": "cargo run"
                },
                "governance": {
                    "capability_ids": ["environment.display_compat"],
                    "failure_mode_ids": ["display_incompatibility"]
                },
                "preflight": {},
                "invariants": {}
            }]
        });

        let score =
            score_report_value(ObservabilityReportKind::GuiAutomation, "unit.json", &report);
        assert!(score.release_blocking_missing_fields.is_empty());
        assert_eq!(score.failed_items, 1);
    }

    #[test]
    fn gui_report_flags_missing_reproduction_command() {
        let report = json!({
            "run_id": "gui-unit",
            "generated_at": "1",
            "summary": {"total_cases": 1},
            "governance": {},
            "failure_bundles": {"written": 0},
            "case_results": [{
                "case_id": "case-1",
                "verdict": "FAIL",
                "evidence": ["failed"],
                "observability": {
                    "root_cause_category": "unknown"
                },
                "governance": {
                    "capability_ids": ["intent.multi_step_gui"],
                    "failure_mode_ids": ["partial_completion"]
                },
                "preflight": {},
                "invariants": {}
            }]
        });

        let score =
            score_report_value(ObservabilityReportKind::GuiAutomation, "unit.json", &report);
        assert!(score
            .release_blocking_missing_fields
            .contains(&"non_pass_cases_have_reproduction_command".to_string()));
    }

    #[test]
    fn hitl_report_requires_decision_evidence() {
        let report = json!({
            "run_id": "hitl-unit",
            "generated_at": "1",
            "summary": {"total": 1},
            "case_results": [{
                "case": {
                    "id": "hitl-1",
                    "capability_ids": ["hitl.timeline"],
                    "failure_mode_ids": ["stale_response"]
                },
                "observation": {
                    "final_state": "invalidated_no_execution",
                    "event_count": 2,
                    "resume_allowed": false,
                    "side_effect_allowed": false,
                    "evidence": ["decision_created"]
                },
                "verdict": {
                    "passed": true,
                    "evidence": ["decision_created"]
                }
            }]
        });

        let score = score_report_value(ObservabilityReportKind::HitlTimeline, "unit.json", &report);
        assert!(score.release_blocking_missing_fields.is_empty());
    }

    #[test]
    fn hitl_policy_block_does_not_require_decision_events() {
        let report = json!({
            "run_id": "hitl-unit",
            "generated_at": "1",
            "summary": {"total": 1},
            "case_results": [{
                "case": {
                    "id": "hitl-policy-block",
                    "capability_ids": ["hitl.timeline"],
                    "failure_mode_ids": ["unsafe_approval_blocked"]
                },
                "observation": {
                    "final_state": "blocked_by_policy",
                    "event_count": 0,
                    "resume_allowed": false,
                    "side_effect_allowed": false,
                    "policy_blocked": true,
                    "evidence": ["policy_blocked=true", "candidate_created=false"]
                },
                "verdict": {
                    "passed": true,
                    "evidence": ["policy_blocked=true"]
                }
            }]
        });

        let score = score_report_value(ObservabilityReportKind::HitlTimeline, "unit.json", &report);
        assert!(score.release_blocking_missing_fields.is_empty());
    }

    #[test]
    fn destructive_report_requires_no_execution() {
        let report = json!({
            "run_id": "destructive-unit",
            "generated_at": "1",
            "dry_run_only": true,
            "summary": {"total": 1},
            "guard": {"isolation_domain": "host"},
            "case_results": [{
                "case": {
                    "id": "destructive-1",
                    "capability_ids": ["safety.destructive_vm_isolation"],
                    "failure_mode_ids": ["root_recursive_delete"]
                },
                "observation": {
                    "policy_risk": "Black",
                    "policy_blocked": true,
                    "policy_requires_approval": false,
                    "isolation_guard_complete": false,
                    "missing_guard_reasons": ["KRIA_EVAL_VM=1 not set"],
                    "attempted_execution": false,
                    "host_execution_possible": false,
                    "evidence": ["policy blocked"]
                },
                "verdict": {
                    "passed": true,
                    "evidence": ["policy blocked"]
                }
            }]
        });

        let score = score_report_value(
            ObservabilityReportKind::DestructiveSafety,
            "unit.json",
            &report,
        );
        assert!(score.release_blocking_missing_fields.is_empty());
    }

    #[test]
    fn llm_report_requires_structural_authority() {
        let report = json!({
            "run_id": "llm-unit",
            "generated_at": "1",
            "summary": {"total_cells": 1},
            "budget": {"max_requests": 1},
            "profiles": [{"id": "local"}],
            "cells": [{
                "case_id": "case",
                "status": "blocked_by_provider",
                "reason": "provider not configured",
                "advisory_only": true,
                "structural_authority": "policy_verifier_gui_oracle_wins",
                "capability_ids": ["llm.cognition_advisory"],
                "failure_mode_ids": ["model_variance"]
            }]
        });

        let score = score_report_value(ObservabilityReportKind::LlmCognition, "unit.json", &report);
        assert!(score.release_blocking_missing_fields.is_empty());
    }
}
