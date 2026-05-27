//! Bounded observability and flake classification for GUI evals.
//!
//! This is not a telemetry platform. It writes compact failure bundles that are
//! useful for triage and keeps flake classification explicit instead of hiding
//! instability behind retries.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::invariants::InvariantReport;
use super::report::GuiEvalReport;
use super::types::{FailureCategory, GuiEvalCase, GuiEvalObservation, GuiEvalVerdict};

pub const FAILURE_BUNDLE_SIZE_BUDGET_BYTES: usize = 64 * 1024;
const PREVIEW_LIMIT: usize = 2_000;
const EVIDENCE_LIMIT: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlakeClassification {
    StablePassCandidate,
    StableFailureCandidate,
    EnvironmentBlocked,
    InvariantViolation,
    PotentialRuntimeFlake,
    PotentialModelVariance,
    NotAssessed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseObservability {
    pub root_cause_category: String,
    pub flake_classification: FlakeClassification,
    pub debug_signal: String,
    pub reproduction_command: String,
    pub failure_bundle_recommended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureBundleIndexEntry {
    pub case_id: String,
    pub bundle_path: String,
    pub root_cause_category: String,
    pub flake_classification: FlakeClassification,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureBundleSummary {
    pub bundle_dir: String,
    pub size_budget_bytes: usize,
    pub written: usize,
    pub omitted: usize,
    pub entries: Vec<FailureBundleIndexEntry>,
}

impl Default for FailureBundleSummary {
    fn default() -> Self {
        Self {
            bundle_dir: String::new(),
            size_budget_bytes: FAILURE_BUNDLE_SIZE_BUDGET_BYTES,
            written: 0,
            omitted: 0,
            entries: Vec::new(),
        }
    }
}

pub fn empty_failure_bundle_summary(bundle_dir: impl Into<String>) -> FailureBundleSummary {
    FailureBundleSummary {
        bundle_dir: bundle_dir.into(),
        size_budget_bytes: FAILURE_BUNDLE_SIZE_BUDGET_BYTES,
        written: 0,
        omitted: 0,
        entries: Vec::new(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FailureBundle {
    schema_version: u8,
    run_id: String,
    case_id: String,
    verdict: String,
    failure_category: Option<String>,
    root_cause_category: String,
    flake_classification: FlakeClassification,
    reproduction_command: String,
    display_server: String,
    capability_ids: Vec<String>,
    failure_mode_ids: Vec<String>,
    required_environment_profile: Option<String>,
    missing_capabilities: Vec<String>,
    blocking_reasons: Vec<String>,
    tools_called: Vec<String>,
    retrieval_tools_called: Vec<String>,
    substrate_used: Option<String>,
    artifact_count: usize,
    invariant_release_blocking_violations: usize,
    invariant_failures: Vec<String>,
    evidence: Vec<String>,
    final_response_preview: String,
}

pub fn classify_case_observability(
    case: &GuiEvalCase,
    obs: &GuiEvalObservation,
    verdict: &GuiEvalVerdict,
    invariants: &InvariantReport,
) -> CaseObservability {
    let root_cause_category = verdict
        .failure_category
        .as_ref()
        .map(|category| category.as_str().to_string())
        .unwrap_or_else(|| "none".to_string());
    let flake_classification = classify_flake(verdict, invariants);
    let debug_signal = debug_signal_for(obs, verdict, invariants);
    let reproduction_command = reproduction_command_for(case);
    let failure_bundle_recommended = (verdict.kind.as_str() != "PASS"
        && verdict.kind.as_str() != "SKIP")
        || invariants.has_release_blocking_violation();

    CaseObservability {
        root_cause_category,
        flake_classification,
        debug_signal,
        reproduction_command,
        failure_bundle_recommended,
    }
}

pub fn write_failure_bundles(
    report: &GuiEvalReport,
    base_dir: impl AsRef<Path>,
) -> std::io::Result<FailureBundleSummary> {
    let bundle_dir = base_dir.as_ref().join(&report.run_id);
    std::fs::create_dir_all(&bundle_dir)?;

    let mut entries = Vec::new();
    let mut omitted = 0usize;

    for case in &report.case_results {
        if !case.observability.failure_bundle_recommended {
            continue;
        }

        let bundle = FailureBundle {
            schema_version: 1,
            run_id: report.run_id.clone(),
            case_id: case.case_id.clone(),
            verdict: case.verdict.clone(),
            failure_category: case.failure_category.clone(),
            root_cause_category: case.observability.root_cause_category.clone(),
            flake_classification: case.observability.flake_classification.clone(),
            reproduction_command: case.observability.reproduction_command.clone(),
            display_server: report.environment.display_server.clone(),
            capability_ids: case.governance.capability_ids.clone(),
            failure_mode_ids: case.governance.failure_mode_ids.clone(),
            required_environment_profile: case
                .preflight
                .required_environment_profile
                .as_ref()
                .map(|profile| profile.as_str().to_string()),
            missing_capabilities: case.preflight.missing_capabilities.clone(),
            blocking_reasons: case.preflight.blocking_reasons.clone(),
            tools_called: case.tools_called.clone(),
            retrieval_tools_called: case.retrieval_tools_called.clone(),
            substrate_used: case.substrate_used.clone(),
            artifact_count: case.artifacts_found,
            invariant_release_blocking_violations: case.invariants.release_blocking_violations,
            invariant_failures: case.invariants.release_blocking_messages(),
            evidence: case
                .evidence
                .iter()
                .take(EVIDENCE_LIMIT)
                .map(|value| redacted_preview(value, PREVIEW_LIMIT))
                .collect(),
            final_response_preview: redacted_preview(&case.explanation, PREVIEW_LIMIT),
        };

        let payload = serde_json::to_vec_pretty(&bundle)?;
        if payload.len() > FAILURE_BUNDLE_SIZE_BUDGET_BYTES {
            omitted += 1;
            continue;
        }

        let path = bundle_path(&bundle_dir, &case.case_id);
        std::fs::write(&path, payload)?;
        let size_bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        entries.push(FailureBundleIndexEntry {
            case_id: case.case_id.clone(),
            bundle_path: path.display().to_string(),
            root_cause_category: case.observability.root_cause_category.clone(),
            flake_classification: case.observability.flake_classification.clone(),
            size_bytes,
        });
    }

    Ok(FailureBundleSummary {
        bundle_dir: bundle_dir.display().to_string(),
        size_budget_bytes: FAILURE_BUNDLE_SIZE_BUDGET_BYTES,
        written: entries.len(),
        omitted,
        entries,
    })
}

fn classify_flake(verdict: &GuiEvalVerdict, invariants: &InvariantReport) -> FlakeClassification {
    if invariants.has_release_blocking_violation() {
        return FlakeClassification::InvariantViolation;
    }

    match verdict.kind.as_str() {
        "PASS" => FlakeClassification::StablePassCandidate,
        "ENVIRONMENT_BLOCKED" => FlakeClassification::EnvironmentBlocked,
        "SKIP" => FlakeClassification::NotAssessed,
        "RETRIEVAL_LEAKAGE" | "FALSE_SUCCESS" => FlakeClassification::StableFailureCandidate,
        _ => match verdict.failure_category.as_ref() {
            Some(FailureCategory::Timeout)
            | Some(FailureCategory::WindowManagement)
            | Some(FailureCategory::VerificationFailure)
            | Some(FailureCategory::MissingRecovery) => FlakeClassification::PotentialRuntimeFlake,
            Some(FailureCategory::CloudLlmLeakage) => FlakeClassification::PotentialModelVariance,
            _ => FlakeClassification::StableFailureCandidate,
        },
    }
}

fn debug_signal_for(
    obs: &GuiEvalObservation,
    verdict: &GuiEvalVerdict,
    invariants: &InvariantReport,
) -> String {
    if invariants.has_release_blocking_violation() {
        return format!(
            "release_blocking_invariant:{}",
            invariants.release_blocking_messages().join("; ")
        );
    }
    if !obs.preflight.blocking_reasons.is_empty() {
        return format!(
            "preflight_blocked:{}",
            obs.preflight.blocking_reasons.join("; ")
        );
    }
    if let Some(category) = &verdict.failure_category {
        return format!("failure_category:{}", category.as_str());
    }
    "no_failure".to_string()
}

fn reproduction_command_for(case: &GuiEvalCase) -> String {
    if let Some(profile) = std::env::var("KRIA_EVAL_GUI_MATRIX_PROFILE")
        .ok()
        .or_else(|| std::env::var("KRIA_EVAL_GUI_PROFILE").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return format!(
            "KRIA_EVAL_GUI_MATRIX_PROFILE={} cargo run -p kria-eval -- --gui",
            profile
        );
    }

    let first_tag = case
        .tags
        .first()
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "KRIA_EVAL_GUI_TAG={} cargo run -p kria-eval -- --gui",
        first_tag
    )
}

fn bundle_path(bundle_dir: &Path, case_id: &str) -> PathBuf {
    let safe_id: String = case_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    bundle_dir.join(format!("{safe_id}.json"))
}

fn truncate(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }

    let mut out: String = value.chars().take(limit).collect();
    out.push_str("...[truncated]");
    out
}

fn redacted_preview(value: &str, limit: usize) -> String {
    let truncated = truncate(value, limit);
    let words = truncated
        .split_whitespace()
        .map(|word| {
            if word.starts_with("sk-") || word.starts_with("Bearer ") {
                "[redacted]".to_string()
            } else if word.len() > 48
                && word
                    .chars()
                    .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
                    .count()
                    > 40
            {
                "[redacted-long-token]".to_string()
            } else {
                word.to_string()
            }
        })
        .collect::<Vec<_>>();
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui_eval::judge::GuiEvalJudge;
    use crate::gui_eval::runner::GuiEvalRunner;
    use crate::gui_eval::suites::semantic_parsing_suite;

    #[tokio::test]
    async fn environment_blocked_case_gets_failure_bundle() {
        if std::env::var("KRIA_EVAL_GUI").as_deref() == Ok("1") {
            eprintln!("[SKIP] KRIA_EVAL_GUI=1 disables this blocked-case assertion");
            return;
        }

        let case = semantic_parsing_suite()
            .into_iter()
            .find(|case| case.id == "parse-001-open-gedit-simple")
            .expect("case exists");
        let obs = GuiEvalRunner::new().run(&case).await;
        let verdict = GuiEvalJudge.evaluate(&case, &obs);
        let invariants = crate::gui_eval::invariants::evaluate_invariants(&case, &obs);
        let observability = classify_case_observability(&case, &obs, &verdict, &invariants);

        assert_eq!(
            observability.flake_classification,
            FlakeClassification::EnvironmentBlocked
        );
        assert!(observability.failure_bundle_recommended);
        assert!(observability
            .debug_signal
            .contains("desktop eval requires KRIA_EVAL_GUI=1"));
    }

    #[test]
    fn truncate_caps_large_values() {
        let long = "a".repeat(PREVIEW_LIMIT + 100);
        let truncated = truncate(&long, PREVIEW_LIMIT);
        assert!(truncated.len() < long.len());
        assert!(truncated.ends_with("...[truncated]"));
    }
}
