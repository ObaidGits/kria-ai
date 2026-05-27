//! Phase 10 production-readiness snapshot for GUI cognition evals.
//!
//! This module aggregates the latest bounded eval reports into one readiness
//! snapshot. It is intentionally read-only: it does not run evals, schedule
//! work, or implement dashboards.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const REPORT_DIR: &str = "tests-logs/eval_reports";
const GUI_REPORT_PATH: &str = "tests-logs/eval_reports/gui_latest_run.json";
const HITL_REPORT_PATH: &str = "tests-logs/eval_reports/hitl_timeline_latest_run.json";
const LLM_REPORT_PATH: &str = "tests-logs/eval_reports/llm_cognition_latest_run.json";
const DESTRUCTIVE_REPORT_PATH: &str = "tests-logs/eval_reports/destructive_safety_latest_run.json";
const OBSERVABILITY_REPORT_PATH: &str = "tests-logs/eval_reports/observability_latest_run.json";
const TREND_LIMIT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessSource {
    GuiAutomation,
    HitlTimeline,
    LlmCognition,
    DestructiveSafety,
    Observability,
}

impl ReadinessSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GuiAutomation => "gui_automation",
            Self::HitlTimeline => "hitl_timeline",
            Self::LlmCognition => "llm_cognition",
            Self::DestructiveSafety => "destructive_safety",
            Self::Observability => "observability",
        }
    }

    pub fn report_path(self) -> &'static str {
        match self {
            Self::GuiAutomation => GUI_REPORT_PATH,
            Self::HitlTimeline => HITL_REPORT_PATH,
            Self::LlmCognition => LLM_REPORT_PATH,
            Self::DestructiveSafety => DESTRUCTIVE_REPORT_PATH,
            Self::Observability => OBSERVABILITY_REPORT_PATH,
        }
    }

    fn missing_severity(self) -> ReadinessGateSeverity {
        match self {
            Self::LlmCognition => ReadinessGateSeverity::Advisory,
            _ => ReadinessGateSeverity::StopShip,
        }
    }
}

const EXPECTED_SOURCES: [ReadinessSource; 5] = [
    ReadinessSource::GuiAutomation,
    ReadinessSource::HitlTimeline,
    ReadinessSource::LlmCognition,
    ReadinessSource::DestructiveSafety,
    ReadinessSource::Observability,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessGateStatus {
    Pass,
    Fail,
    Blocked,
    Advisory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessGateSeverity {
    StopShip,
    Environment,
    Advisory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessVerdict {
    Ready,
    ReadyWithAdvisory,
    BlockedByEnvironment,
    NotReady,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadinessGate {
    pub name: String,
    pub source: ReadinessSource,
    pub status: ReadinessGateStatus,
    pub severity: ReadinessGateSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadinessSourceSnapshot {
    pub source: ReadinessSource,
    pub report_path: String,
    pub report_present: bool,
    pub run_id: Option<String>,
    pub generated_at: Option<String>,
    pub total_items: usize,
    pub passed_items: usize,
    pub failed_items: usize,
    pub blocked_items: usize,
    pub advisory_items: usize,
    pub summary: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadinessTrendEntry {
    pub source: ReadinessSource,
    pub report_path: String,
    pub run_id: Option<String>,
    pub generated_at: Option<String>,
    pub total_items: usize,
    pub passed_items: usize,
    pub failed_items: usize,
    pub blocked_items: usize,
    pub false_success_count: usize,
    pub retrieval_leakage_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullEvalRunbook {
    pub one_shot_command: String,
    pub phase_commands: Vec<String>,
    pub display_matrix_commands: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadinessSummary {
    pub verdict: ReadinessVerdict,
    pub total_sources: usize,
    pub present_sources: usize,
    pub total_gates: usize,
    pub passed_gates: usize,
    pub failed_gates: usize,
    pub environment_blocked_gates: usize,
    pub advisory_gates: usize,
    pub stop_ship_failures: usize,
    pub full_eval_ready_to_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductionReadinessReport {
    pub generated_at: String,
    pub report_version: String,
    pub summary: ReadinessSummary,
    pub source_snapshots: Vec<ReadinessSourceSnapshot>,
    pub gates: Vec<ReadinessGate>,
    pub trend_entries: Vec<ReadinessTrendEntry>,
    pub full_eval_runbook: FullEvalRunbook,
}

#[derive(Debug, Clone)]
struct LoadedSource {
    source: ReadinessSource,
    path: String,
    value: Option<Value>,
}

pub fn build_latest_readiness_report() -> ProductionReadinessReport {
    let sources = EXPECTED_SOURCES
        .iter()
        .map(|source| load_source(*source, source.report_path()))
        .collect::<Vec<_>>();
    let trend_entries = collect_gui_trend_entries(REPORT_DIR, TREND_LIMIT);
    build_report(sources, trend_entries)
}

pub fn build_readiness_report_from_values(
    values: Vec<(ReadinessSource, Value)>,
) -> ProductionReadinessReport {
    let values = values.into_iter().collect::<BTreeMap<_, _>>();
    let sources = EXPECTED_SOURCES
        .iter()
        .map(|source| LoadedSource {
            source: *source,
            path: source.report_path().to_string(),
            value: values.get(source).cloned(),
        })
        .collect::<Vec<_>>();
    build_report(sources, Vec::new())
}

pub fn print_readiness_report(report: &ProductionReadinessReport) {
    println!("── GUI Cognition Production Readiness ────────────────────────");
    println!("  Verdict:              {:?}", report.summary.verdict);
    println!(
        "  Sources:              {}/{}",
        report.summary.present_sources, report.summary.total_sources
    );
    println!("  Gates:                {}", report.summary.total_gates);
    println!("  Passed Gates:         {}", report.summary.passed_gates);
    println!(
        "  Stop-Ship Failures:   {}",
        report.summary.stop_ship_failures
    );
    println!(
        "  Environment Blocks:   {}",
        report.summary.environment_blocked_gates
    );
    println!("  Advisory Gates:       {}", report.summary.advisory_gates);
    println!(
        "  Full Eval Command:    {}",
        report.full_eval_runbook.one_shot_command
    );

    for gate in &report.gates {
        if gate.status == ReadinessGateStatus::Pass {
            continue;
        }
        println!(
            "  {:?} [{}] {}: {}",
            gate.status,
            gate.source.as_str(),
            gate.name,
            gate.message
        );
    }
    println!();
}

pub fn write_readiness_markdown(
    report: &ProductionReadinessReport,
    path: impl AsRef<Path>,
) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str("# KRIA GUI Cognition Readiness Snapshot\n\n");
    out.push_str(&format!("Verdict: `{:?}`\n\n", report.summary.verdict));
    out.push_str("## Gates\n\n");
    out.push_str("| Gate | Source | Status | Severity | Message |\n");
    out.push_str("|---|---|---|---|---|\n");
    for gate in &report.gates {
        out.push_str(&format!(
            "| `{}` | `{}` | `{:?}` | `{:?}` | {} |\n",
            gate.name,
            gate.source.as_str(),
            gate.status,
            gate.severity,
            escape_markdown_cell(&gate.message)
        ));
    }

    out.push_str("\n## Latest Sources\n\n");
    out.push_str("| Source | Present | Total | Passed | Failed | Blocked | Advisory |\n");
    out.push_str("|---|---:|---:|---:|---:|---:|---:|\n");
    for source in &report.source_snapshots {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} | {} |\n",
            source.source.as_str(),
            source.report_present,
            source.total_items,
            source.passed_items,
            source.failed_items,
            source.blocked_items,
            source.advisory_items
        ));
    }

    out.push_str("\n## Full Eval Runbook\n\n");
    out.push_str(&format!(
        "One-shot command:\n\n```bash\n{}\n```\n\n",
        report.full_eval_runbook.one_shot_command
    ));
    out.push_str("Expanded commands:\n\n");
    for command in &report.full_eval_runbook.phase_commands {
        out.push_str(&format!("- `{}`\n", command));
    }
    out.push_str("\nDisplay matrix commands:\n\n");
    for command in &report.full_eval_runbook.display_matrix_commands {
        out.push_str(&format!("- `{}`\n", command));
    }

    if !report.trend_entries.is_empty() {
        out.push_str("\n## Recent GUI Trend\n\n");
        out.push_str(
            "| Report | Total | Passed | Failed | Blocked | False Success | Retrieval Leakage |\n",
        );
        out.push_str("|---|---:|---:|---:|---:|---:|---:|\n");
        for trend in &report.trend_entries {
            out.push_str(&format!(
                "| `{}` | {} | {} | {} | {} | {} | {} |\n",
                trend.report_path,
                trend.total_items,
                trend.passed_items,
                trend.failed_items,
                trend.blocked_items,
                trend.false_success_count,
                trend.retrieval_leakage_count
            ));
        }
    }

    std::fs::write(path, out)
}

fn build_report(
    loaded_sources: Vec<LoadedSource>,
    trend_entries: Vec<ReadinessTrendEntry>,
) -> ProductionReadinessReport {
    let source_snapshots = loaded_sources
        .iter()
        .map(source_snapshot)
        .collect::<Vec<_>>();
    let mut gates = Vec::new();

    for source in &loaded_sources {
        gates.extend(source_presence_gate(source));
        if let Some(value) = &source.value {
            gates.extend(source_gates(source.source, value));
        }
    }

    let total_sources = source_snapshots.len();
    let present_sources = source_snapshots
        .iter()
        .filter(|source| source.report_present)
        .count();
    let total_gates = gates.len();
    let passed_gates = gates
        .iter()
        .filter(|gate| gate.status == ReadinessGateStatus::Pass)
        .count();
    let failed_gates = gates
        .iter()
        .filter(|gate| gate.status == ReadinessGateStatus::Fail)
        .count();
    let environment_blocked_gates = gates
        .iter()
        .filter(|gate| gate.status == ReadinessGateStatus::Blocked)
        .count();
    let advisory_gates = gates
        .iter()
        .filter(|gate| gate.status == ReadinessGateStatus::Advisory)
        .count();
    let stop_ship_failures = gates
        .iter()
        .filter(|gate| {
            gate.severity == ReadinessGateSeverity::StopShip
                && gate.status == ReadinessGateStatus::Fail
        })
        .count();
    let verdict = if stop_ship_failures > 0 {
        ReadinessVerdict::NotReady
    } else if environment_blocked_gates > 0 {
        ReadinessVerdict::BlockedByEnvironment
    } else if advisory_gates > 0 {
        ReadinessVerdict::ReadyWithAdvisory
    } else {
        ReadinessVerdict::Ready
    };

    ProductionReadinessReport {
        generated_at: unix_now(),
        report_version: "phase10_readiness_v1".to_string(),
        summary: ReadinessSummary {
            verdict,
            total_sources,
            present_sources,
            total_gates,
            passed_gates,
            failed_gates,
            environment_blocked_gates,
            advisory_gates,
            stop_ship_failures,
            full_eval_ready_to_run: true,
        },
        source_snapshots,
        gates,
        trend_entries,
        full_eval_runbook: full_eval_runbook(),
    }
}

fn load_source(source: ReadinessSource, path: &str) -> LoadedSource {
    let value = std::fs::read_to_string(path)
        .ok()
        .and_then(|payload| serde_json::from_str::<Value>(&payload).ok());
    LoadedSource {
        source,
        path: path.to_string(),
        value,
    }
}

fn source_presence_gate(source: &LoadedSource) -> Vec<ReadinessGate> {
    let passed = source.value.is_some();
    let severity = source.source.missing_severity();
    vec![ReadinessGate {
        name: "source_report_present".to_string(),
        source: source.source,
        status: if passed {
            ReadinessGateStatus::Pass
        } else if severity == ReadinessGateSeverity::Advisory {
            ReadinessGateStatus::Advisory
        } else {
            ReadinessGateStatus::Fail
        },
        severity,
        message: if passed {
            format!("{} report is present", source.source.as_str())
        } else {
            format!("missing report at {}", source.path)
        },
    }]
}

fn source_gates(source: ReadinessSource, value: &Value) -> Vec<ReadinessGate> {
    match source {
        ReadinessSource::GuiAutomation => gui_gates(value),
        ReadinessSource::HitlTimeline => hitl_gates(value),
        ReadinessSource::LlmCognition => llm_gates(value),
        ReadinessSource::DestructiveSafety => destructive_gates(value),
        ReadinessSource::Observability => observability_gates(value),
    }
}

fn gui_gates(value: &Value) -> Vec<ReadinessGate> {
    let failed = usize_at(value, &["summary", "failed"]);
    let skipped = usize_at(value, &["summary", "skipped"]);
    let environment_blocked = usize_at(value, &["summary", "environment_blocked"]);
    let invariant_violations = usize_at(value, &["summary", "invariant_violation_cases"]);
    let false_success = usize_at(value, &["summary", "false_success_count"]);
    let retrieval_leakage = usize_at(value, &["summary", "retrieval_leakage_count"]);
    let entropy = value
        .pointer("/governance/entropy/entropy_score")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);

    vec![
        stop_ship_gate(
            ReadinessSource::GuiAutomation,
            "gui_no_runtime_failures",
            failed == 0,
            format!("GUI failed cases: {failed}"),
        ),
        stop_ship_gate(
            ReadinessSource::GuiAutomation,
            "gui_no_false_success",
            false_success == 0,
            format!("GUI false-success incidents: {false_success}"),
        ),
        stop_ship_gate(
            ReadinessSource::GuiAutomation,
            "gui_no_retrieval_leakage",
            retrieval_leakage == 0,
            format!("GUI retrieval leakage incidents: {retrieval_leakage}"),
        ),
        stop_ship_gate(
            ReadinessSource::GuiAutomation,
            "gui_no_invariant_violations",
            invariant_violations == 0,
            format!("GUI invariant violation cases: {invariant_violations}"),
        ),
        environment_gate(
            ReadinessSource::GuiAutomation,
            "gui_environment_coverage",
            environment_blocked == 0 && skipped == 0,
            format!(
                "GUI environment-blocked cases: {environment_blocked}; skipped cases: {skipped}"
            ),
        ),
        advisory_gate(
            ReadinessSource::GuiAutomation,
            "gui_eval_entropy_bounded",
            entropy <= 0.35,
            format!("GUI eval entropy score: {entropy:.2}"),
        ),
    ]
}

fn hitl_gates(value: &Value) -> Vec<ReadinessGate> {
    let failed = usize_at(value, &["summary", "failed"]);
    let stale_or_invalidated = usize_at(value, &["summary", "stale_or_invalidated"]);
    let blocked_by_policy = usize_at(value, &["summary", "blocked_by_policy"]);
    vec![
        stop_ship_gate(
            ReadinessSource::HitlTimeline,
            "hitl_all_cases_pass",
            failed == 0,
            format!("HITL failed cases: {failed}"),
        ),
        stop_ship_gate(
            ReadinessSource::HitlTimeline,
            "hitl_stale_invalidation_covered",
            stale_or_invalidated > 0,
            format!("HITL stale/invalidated cases exercised: {stale_or_invalidated}"),
        ),
        stop_ship_gate(
            ReadinessSource::HitlTimeline,
            "hitl_policy_block_covered",
            blocked_by_policy > 0,
            format!("HITL policy-block cases exercised: {blocked_by_policy}"),
        ),
    ]
}

fn llm_gates(value: &Value) -> Vec<ReadinessGate> {
    let cells = array_at(value, &["cells"]);
    let advisory_ready = usize_at(value, &["summary", "advisory_ready"]);
    let blocked_by_provider = usize_at(value, &["summary", "blocked_by_provider"]);
    let blocked_by_budget = usize_at(value, &["summary", "blocked_by_budget"]);
    let structural_blocked = usize_at(value, &["summary", "structural_blocked"]);
    vec![
        stop_ship_gate(
            ReadinessSource::LlmCognition,
            "llm_cells_remain_advisory_only",
            cells.iter().all(|cell| {
                cell.pointer("/advisory_only").and_then(Value::as_bool) == Some(true)
                    && string_at(cell, &["structural_authority"]).as_deref()
                        == Some("policy_verifier_gui_oracle_wins")
            }),
            "LLM cognition cells must be advisory and structurally bounded".to_string(),
        ),
        advisory_gate(
            ReadinessSource::LlmCognition,
            "llm_provider_coverage",
            blocked_by_provider == 0 && blocked_by_budget == 0,
            format!(
                "LLM advisory-ready cells: {advisory_ready}; provider-blocked: {blocked_by_provider}; budget-blocked: {blocked_by_budget}"
            ),
        ),
        advisory_gate(
            ReadinessSource::LlmCognition,
            "llm_no_structural_blocks",
            structural_blocked == 0,
            format!("LLM structural-blocked advisory cells: {structural_blocked}"),
        ),
    ]
}

fn destructive_gates(value: &Value) -> Vec<ReadinessGate> {
    let failed = usize_at(value, &["summary", "failed"]);
    let attempted_execution = usize_at(value, &["summary", "attempted_execution"]);
    let host_execution_possible = usize_at(value, &["summary", "host_execution_possible"]);
    let vm_dry_run_eligible = usize_at(value, &["summary", "vm_dry_run_eligible"]);
    let dry_run_only = value.pointer("/dry_run_only").and_then(Value::as_bool) == Some(true);

    vec![
        stop_ship_gate(
            ReadinessSource::DestructiveSafety,
            "destructive_all_cases_pass",
            failed == 0,
            format!("destructive safety failed cases: {failed}"),
        ),
        stop_ship_gate(
            ReadinessSource::DestructiveSafety,
            "destructive_no_execution_attempted",
            attempted_execution == 0 && host_execution_possible == 0,
            format!(
                "attempted execution: {attempted_execution}; host execution possible: {host_execution_possible}"
            ),
        ),
        stop_ship_gate(
            ReadinessSource::DestructiveSafety,
            "destructive_dry_run_only",
            dry_run_only,
            "destructive suite must remain dry-run only in Phase 10".to_string(),
        ),
        advisory_gate(
            ReadinessSource::DestructiveSafety,
            "destructive_vm_snapshot_not_exercised",
            vm_dry_run_eligible > 0,
            format!("VM dry-run eligible destructive cases: {vm_dry_run_eligible}"),
        ),
    ]
}

fn observability_gates(value: &Value) -> Vec<ReadinessGate> {
    let release_missing = usize_at(value, &["summary", "release_blocking_missing_fields"]);
    let failed_sources = usize_at(value, &["summary", "failed_sources"]);
    let replay_scope = string_at(value, &["summary", "replay_scope"]).unwrap_or_default();
    vec![
        stop_ship_gate(
            ReadinessSource::Observability,
            "observability_no_release_missing_fields",
            release_missing == 0,
            format!("observability release-blocking missing fields: {release_missing}"),
        ),
        stop_ship_gate(
            ReadinessSource::Observability,
            "observability_all_sources_pass",
            failed_sources == 0,
            format!("observability failed sources: {failed_sources}"),
        ),
        stop_ship_gate(
            ReadinessSource::Observability,
            "observability_replay_scope_bounded",
            replay_scope == "decision_evidence_reconstruction_only",
            format!("observability replay scope: {replay_scope}"),
        ),
    ]
}

fn source_snapshot(source: &LoadedSource) -> ReadinessSourceSnapshot {
    let Some(value) = &source.value else {
        return ReadinessSourceSnapshot {
            source: source.source,
            report_path: source.path.clone(),
            report_present: false,
            run_id: None,
            generated_at: None,
            total_items: 0,
            passed_items: 0,
            failed_items: 0,
            blocked_items: 0,
            advisory_items: 0,
            summary: Value::Null,
        };
    };

    let summary = value.get("summary").cloned().unwrap_or(Value::Null);
    let (total, passed, failed, blocked, advisory) = match source.source {
        ReadinessSource::GuiAutomation => (
            usize_at(value, &["summary", "total_cases"]),
            usize_at(value, &["summary", "passed"]),
            usize_at(value, &["summary", "failed"]),
            usize_at(value, &["summary", "environment_blocked"])
                + usize_at(value, &["summary", "skipped"]),
            0,
        ),
        ReadinessSource::HitlTimeline => (
            usize_at(value, &["summary", "total"]),
            usize_at(value, &["summary", "passed"]),
            usize_at(value, &["summary", "failed"]),
            usize_at(value, &["summary", "blocked_by_policy"]),
            0,
        ),
        ReadinessSource::LlmCognition => (
            usize_at(value, &["summary", "total_cells"]),
            usize_at(value, &["summary", "advisory_ready"]),
            usize_at(value, &["summary", "structural_blocked"]),
            usize_at(value, &["summary", "blocked_by_provider"])
                + usize_at(value, &["summary", "blocked_by_budget"]),
            usize_at(value, &["summary", "blocked_by_provider"])
                + usize_at(value, &["summary", "blocked_by_budget"]),
        ),
        ReadinessSource::DestructiveSafety => (
            usize_at(value, &["summary", "total"]),
            usize_at(value, &["summary", "passed"]),
            usize_at(value, &["summary", "failed"]),
            usize_at(value, &["summary", "blocked_by_policy"])
                + usize_at(value, &["summary", "blocked_by_isolation"]),
            if usize_at(value, &["summary", "vm_dry_run_eligible"]) == 0 {
                1
            } else {
                0
            },
        ),
        ReadinessSource::Observability => (
            usize_at(value, &["summary", "total_sources"]),
            usize_at(value, &["summary", "passed_sources"]),
            usize_at(value, &["summary", "failed_sources"]),
            0,
            usize_at(value, &["summary", "warning_missing_fields"]),
        ),
    };

    ReadinessSourceSnapshot {
        source: source.source,
        report_path: source.path.clone(),
        report_present: true,
        run_id: string_at(value, &["run_id"]),
        generated_at: string_at(value, &["generated_at"]),
        total_items: total,
        passed_items: passed,
        failed_items: failed,
        blocked_items: blocked,
        advisory_items: advisory,
        summary,
    }
}

fn collect_gui_trend_entries(report_dir: &str, limit: usize) -> Vec<ReadinessTrendEntry> {
    let mut paths = match std::fs::read_dir(report_dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(is_timestamped_gui_report)
            .collect::<Vec<_>>(),
        Err(_) => return Vec::new(),
    };
    paths.sort();

    let start = paths.len().saturating_sub(limit);
    paths[start..]
        .iter()
        .filter_map(|path| trend_entry_from_gui_report(path))
        .collect()
}

fn trend_entry_from_gui_report(path: &Path) -> Option<ReadinessTrendEntry> {
    let payload = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&payload).ok()?;
    Some(ReadinessTrendEntry {
        source: ReadinessSource::GuiAutomation,
        report_path: path.display().to_string(),
        run_id: string_at(&value, &["run_id"]),
        generated_at: string_at(&value, &["generated_at"]),
        total_items: usize_at(&value, &["summary", "total_cases"]),
        passed_items: usize_at(&value, &["summary", "passed"]),
        failed_items: usize_at(&value, &["summary", "failed"]),
        blocked_items: usize_at(&value, &["summary", "environment_blocked"])
            + usize_at(&value, &["summary", "skipped"]),
        false_success_count: usize_at(&value, &["summary", "false_success_count"]),
        retrieval_leakage_count: usize_at(&value, &["summary", "retrieval_leakage_count"]),
    })
}

fn is_timestamped_gui_report(path: &PathBuf) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    file_name.starts_with("gui_")
        && file_name.ends_with(".json")
        && file_name != "gui_latest_run.json"
}

fn stop_ship_gate(
    source: ReadinessSource,
    name: &str,
    passed: bool,
    message: String,
) -> ReadinessGate {
    ReadinessGate {
        name: name.to_string(),
        source,
        status: if passed {
            ReadinessGateStatus::Pass
        } else {
            ReadinessGateStatus::Fail
        },
        severity: ReadinessGateSeverity::StopShip,
        message,
    }
}

fn environment_gate(
    source: ReadinessSource,
    name: &str,
    passed: bool,
    message: String,
) -> ReadinessGate {
    ReadinessGate {
        name: name.to_string(),
        source,
        status: if passed {
            ReadinessGateStatus::Pass
        } else {
            ReadinessGateStatus::Blocked
        },
        severity: ReadinessGateSeverity::Environment,
        message,
    }
}

fn advisory_gate(
    source: ReadinessSource,
    name: &str,
    passed: bool,
    message: String,
) -> ReadinessGate {
    ReadinessGate {
        name: name.to_string(),
        source,
        status: if passed {
            ReadinessGateStatus::Pass
        } else {
            ReadinessGateStatus::Advisory
        },
        severity: ReadinessGateSeverity::Advisory,
        message,
    }
}

fn full_eval_runbook() -> FullEvalRunbook {
    FullEvalRunbook {
        one_shot_command: "cargo run -p kria-eval -- --gui-full".to_string(),
        phase_commands: vec![
            "cargo run -p kria-eval -- --gui".to_string(),
            "cargo run -p kria-eval -- --gui-hitl".to_string(),
            "cargo run -p kria-eval -- --gui-llm".to_string(),
            "cargo run -p kria-eval -- --gui-destructive".to_string(),
            "cargo run -p kria-eval -- --gui-observability".to_string(),
            "cargo run -p kria-eval -- --gui-readiness".to_string(),
        ],
        display_matrix_commands: vec![
            "KRIA_EVAL_GUI_MATRIX_PROFILE=display-critical cargo run -p kria-eval -- --gui"
                .to_string(),
            "KRIA_EVAL_GUI_MATRIX_PROFILE=x11-critical cargo run -p kria-eval -- --gui"
                .to_string(),
            "KRIA_EVAL_GUI_MATRIX_PROFILE=wayland-critical cargo run -p kria-eval -- --gui"
                .to_string(),
        ],
        notes: vec![
            "LLM cognition remains advisory and cannot override structural failures.".to_string(),
            "Destructive safety remains dry-run only unless VM/snapshot guards are explicitly configured.".to_string(),
            "Environment-blocked GUI cases are not counted as production-ready passes.".to_string(),
        ],
    }
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

fn usize_at(value: &Value, path: &[&str]) -> usize {
    let mut current = value;
    for key in path {
        let Some(next) = current.get(*key) else {
            return 0;
        };
        current = next;
    }
    current.as_u64().unwrap_or(0) as usize
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(ToString::to_string)
}

fn escape_markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
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
    fn clean_reports_are_ready_with_advisory_when_llm_provider_missing() {
        let report = build_readiness_report_from_values(vec![
            (
                ReadinessSource::GuiAutomation,
                gui_report(0, 0, 0, 0, 0, 0.10),
            ),
            (ReadinessSource::HitlTimeline, hitl_report(0, 2, 1)),
            (ReadinessSource::LlmCognition, llm_report(0, 4, 0, true)),
            (
                ReadinessSource::DestructiveSafety,
                destructive_report(0, 0, 0, true, 0),
            ),
            (ReadinessSource::Observability, observability_report(0, 0)),
        ]);

        assert_eq!(report.summary.verdict, ReadinessVerdict::ReadyWithAdvisory);
        assert_eq!(report.summary.stop_ship_failures, 0);
        assert_eq!(report.summary.environment_blocked_gates, 0);
        assert!(report.summary.advisory_gates > 0);
    }

    #[test]
    fn environment_blocked_gui_report_blocks_production_claim_without_stop_ship_failure() {
        let report = build_readiness_report_from_values(vec![
            (
                ReadinessSource::GuiAutomation,
                gui_report(0, 3, 0, 0, 0, 0.10),
            ),
            (ReadinessSource::HitlTimeline, hitl_report(0, 2, 1)),
            (
                ReadinessSource::DestructiveSafety,
                destructive_report(0, 0, 0, true, 1),
            ),
            (ReadinessSource::Observability, observability_report(0, 0)),
        ]);

        assert_eq!(
            report.summary.verdict,
            ReadinessVerdict::BlockedByEnvironment
        );
        assert_eq!(report.summary.stop_ship_failures, 0);
        assert_eq!(report.summary.environment_blocked_gates, 1);
    }

    #[test]
    fn false_success_is_stop_ship_failure() {
        let report = build_readiness_report_from_values(vec![
            (
                ReadinessSource::GuiAutomation,
                gui_report(0, 0, 1, 0, 0, 0.10),
            ),
            (ReadinessSource::HitlTimeline, hitl_report(0, 2, 1)),
            (
                ReadinessSource::DestructiveSafety,
                destructive_report(0, 0, 0, true, 1),
            ),
            (ReadinessSource::Observability, observability_report(0, 0)),
        ]);

        assert_eq!(report.summary.verdict, ReadinessVerdict::NotReady);
        assert!(report.summary.stop_ship_failures > 0);
    }

    #[test]
    fn missing_non_llm_source_is_not_ready() {
        let report = build_readiness_report_from_values(vec![
            (
                ReadinessSource::GuiAutomation,
                gui_report(0, 0, 0, 0, 0, 0.10),
            ),
            (ReadinessSource::HitlTimeline, hitl_report(0, 2, 1)),
            (
                ReadinessSource::DestructiveSafety,
                destructive_report(0, 0, 0, true, 1),
            ),
        ]);

        assert_eq!(report.summary.verdict, ReadinessVerdict::NotReady);
        assert!(report.summary.stop_ship_failures > 0);
    }

    #[test]
    fn llm_structural_authority_violation_is_stop_ship_failure() {
        let report = build_readiness_report_from_values(vec![
            (
                ReadinessSource::GuiAutomation,
                gui_report(0, 0, 0, 0, 0, 0.10),
            ),
            (ReadinessSource::HitlTimeline, hitl_report(0, 2, 1)),
            (ReadinessSource::LlmCognition, llm_report(1, 0, 0, false)),
            (
                ReadinessSource::DestructiveSafety,
                destructive_report(0, 0, 0, true, 1),
            ),
            (ReadinessSource::Observability, observability_report(0, 0)),
        ]);

        assert_eq!(report.summary.verdict, ReadinessVerdict::NotReady);
        assert!(report
            .gates
            .iter()
            .any(|gate| gate.name == "llm_cells_remain_advisory_only"
                && gate.status == ReadinessGateStatus::Fail));
    }

    fn gui_report(
        failed: usize,
        environment_blocked: usize,
        false_success: usize,
        retrieval_leakage: usize,
        invariant_violations: usize,
        entropy: f64,
    ) -> Value {
        json!({
            "run_id": "gui",
            "generated_at": "1",
            "summary": {
                "total_cases": 4,
                "passed": 4usize.saturating_sub(failed + environment_blocked),
                "failed": failed,
                "skipped": 0,
                "environment_blocked": environment_blocked,
                "invariant_violation_cases": invariant_violations,
                "false_success_count": false_success,
                "retrieval_leakage_count": retrieval_leakage
            },
            "governance": {
                "entropy": {
                    "entropy_score": entropy
                }
            }
        })
    }

    fn hitl_report(failed: usize, stale: usize, policy_blocked: usize) -> Value {
        json!({
            "run_id": "hitl",
            "generated_at": "1",
            "summary": {
                "total": 8,
                "passed": 8usize.saturating_sub(failed),
                "failed": failed,
                "stale_or_invalidated": stale,
                "blocked_by_policy": policy_blocked
            }
        })
    }

    fn llm_report(
        structural_blocked: usize,
        blocked_by_provider: usize,
        blocked_by_budget: usize,
        advisory_only: bool,
    ) -> Value {
        json!({
            "run_id": "llm",
            "generated_at": "1",
            "summary": {
                "total_cells": 4,
                "advisory_ready": 4usize.saturating_sub(blocked_by_provider + blocked_by_budget + structural_blocked),
                "blocked_by_provider": blocked_by_provider,
                "blocked_by_budget": blocked_by_budget,
                "structural_blocked": structural_blocked
            },
            "cells": [{
                "advisory_only": advisory_only,
                "structural_authority": if advisory_only {
                    "policy_verifier_gui_oracle_wins"
                } else {
                    "llm_wins"
                }
            }]
        })
    }

    fn destructive_report(
        failed: usize,
        attempted_execution: usize,
        host_execution_possible: usize,
        dry_run_only: bool,
        vm_dry_run_eligible: usize,
    ) -> Value {
        json!({
            "run_id": "destructive",
            "generated_at": "1",
            "dry_run_only": dry_run_only,
            "summary": {
                "total": 8,
                "passed": 8usize.saturating_sub(failed),
                "failed": failed,
                "blocked_by_policy": 3,
                "blocked_by_isolation": 5,
                "vm_dry_run_eligible": vm_dry_run_eligible,
                "attempted_execution": attempted_execution,
                "host_execution_possible": host_execution_possible
            }
        })
    }

    fn observability_report(release_missing: usize, failed_sources: usize) -> Value {
        json!({
            "run_id": "obs",
            "generated_at": "1",
            "summary": {
                "total_sources": 4,
                "passed_sources": 4usize.saturating_sub(failed_sources),
                "failed_sources": failed_sources,
                "release_blocking_missing_fields": release_missing,
                "warning_missing_fields": 0,
                "replay_scope": "decision_evidence_reconstruction_only"
            }
        })
    }
}
