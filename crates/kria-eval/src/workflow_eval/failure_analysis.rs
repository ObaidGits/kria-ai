//! Automatic failure diagnostic capture for workflow eval cases.
//!
//! For every failed workflow eval, this module captures:
//! - Stage lineage (which stages ran, in what order)
//! - Verifier lineage (which checkpoints were checked and passed/failed)
//! - Interruption lineage (whether an interruption occurred and was handled)
//! - Semantic mismatch (which contract signals were missing)
//! - Observable mismatch (which outputs were expected but not found)
//! - Workflow continuation state (pause/resume checkpoints)
//!
//! The output is a human-readable `FailureDiagnostic` that engineers can
//! use to pinpoint the exact cognition failure.

use serde::{Deserialize, Serialize};

use super::types::{
    SemanticCompletionContract, WorkflowEvalObservation, WorkflowEvalVerdict,
    WorkflowSuccessLevels, WorkflowVerdictKind,
};

// ─── Lineage Records ──────────────────────────────────────────────────────────

/// Record of a single stage in the workflow execution lineage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageRecord {
    pub label: String,
    pub completed: bool,
    pub error: Option<String>,
}

/// Record of a single verification checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifierRecord {
    pub stage_label: String,
    pub checkpoint_kind: String,
    pub passed: bool,
    pub confidence: f32,
    pub evidence: String,
}

/// Record of an interruption event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptionRecord {
    pub kind: String,
    pub after_stage: String,
    pub handled: bool,
    pub recovery_plan: String,
}

// ─── Semantic Mismatch ────────────────────────────────────────────────────────

/// A specific semantic signal that was expected but not found.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticMismatch {
    pub expected_signal: String,
    pub found_in_response: bool,
    pub impact: String,
}

/// A required observable output that was not satisfied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservableMismatch {
    pub description: String,
    pub response_signals_checked: Vec<String>,
    pub artifact_path_glob: Option<String>,
    pub artifacts_found: usize,
    pub why_failed: String,
}

// ─── FailureDiagnostic ────────────────────────────────────────────────────────

/// Full failure diagnostic for a single workflow eval case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureDiagnostic {
    pub case_id: String,
    pub verdict_kind: String,
    pub success_summary: String,
    pub stage_lineage: Vec<StageRecord>,
    pub semantic_mismatches: Vec<SemanticMismatch>,
    pub observable_mismatches: Vec<ObservableMismatch>,
    pub interruption_lineage: Option<InterruptionRecord>,
    pub silent_completion_trigger: Option<String>,
    pub recommended_fix: String,
    pub human_readable_summary: String,
}

impl FailureDiagnostic {
    /// Produce a diagnostic from an observation, verdict, and contract.
    pub fn from_verdict(
        obs: &WorkflowEvalObservation,
        verdict: &WorkflowEvalVerdict,
        contract: &SemanticCompletionContract,
    ) -> Self {
        let stage_lineage = build_stage_lineage(obs);
        let semantic_mismatches = find_semantic_mismatches(obs, contract);
        let observable_mismatches = find_observable_mismatches(obs, contract);
        let silent_completion_trigger = find_silent_completion_trigger(obs, contract);
        let interruption_lineage = None; // populated by runner when scenario present

        let recommended_fix = derive_recommended_fix(
            &verdict.kind,
            &verdict.success_levels,
            &semantic_mismatches,
            &observable_mismatches,
            &silent_completion_trigger,
        );

        let human_readable_summary = build_human_summary(
            &obs.case_id,
            &verdict.kind,
            &verdict.success_levels,
            &semantic_mismatches,
            &observable_mismatches,
            &silent_completion_trigger,
        );

        FailureDiagnostic {
            case_id: obs.case_id.clone(),
            verdict_kind: verdict.kind.as_str().to_string(),
            success_summary: verdict.success_levels.summary(),
            stage_lineage,
            semantic_mismatches,
            observable_mismatches,
            interruption_lineage,
            silent_completion_trigger,
            recommended_fix,
            human_readable_summary,
        }
    }
}

// ─── Builders ─────────────────────────────────────────────────────────────────

fn build_stage_lineage(obs: &WorkflowEvalObservation) -> Vec<StageRecord> {
    let mut records: Vec<StageRecord> = obs
        .completed_stage_labels
        .iter()
        .map(|label| StageRecord {
            label: label.clone(),
            completed: true,
            error: None,
        })
        .collect();

    for error in &obs.stage_errors {
        records.push(StageRecord {
            label: "unknown".to_string(),
            completed: false,
            error: Some(error.clone()),
        });
    }

    records
}

fn find_semantic_mismatches(
    obs: &WorkflowEvalObservation,
    contract: &SemanticCompletionContract,
) -> Vec<SemanticMismatch> {
    let response_lower = obs.final_response.to_ascii_lowercase();
    contract
        .semantic_success_signals
        .iter()
        .map(|signal| {
            let found = response_lower.contains(&signal.to_ascii_lowercase());
            SemanticMismatch {
                expected_signal: signal.clone(),
                found_in_response: found,
                impact: if found {
                    "satisfied".to_string()
                } else {
                    "missing — semantic success cannot be confirmed".to_string()
                },
            }
        })
        .collect()
}

fn find_observable_mismatches(
    obs: &WorkflowEvalObservation,
    contract: &SemanticCompletionContract,
) -> Vec<ObservableMismatch> {
    let response_lower = obs.final_response.to_ascii_lowercase();
    contract
        .required_observable_outputs
        .iter()
        .filter_map(|req| {
            let response_signals_satisfied = req
                .response_must_contain
                .iter()
                .any(|s| response_lower.contains(&s.to_ascii_lowercase()));

            let artifact_satisfied =
                req.artifact_path_glob.is_none() || !obs.artifacts_found.is_empty();

            if !response_signals_satisfied || !artifact_satisfied {
                Some(ObservableMismatch {
                    description: req.description.clone(),
                    response_signals_checked: req.response_must_contain.clone(),
                    artifact_path_glob: req.artifact_path_glob.clone(),
                    artifacts_found: obs.artifacts_found.len(),
                    why_failed: if !response_signals_satisfied {
                        "none of the required response signals appeared in KRIA's output".into()
                    } else {
                        "required artifact was not found on disk".into()
                    },
                })
            } else {
                None
            }
        })
        .collect()
}

fn find_silent_completion_trigger(
    obs: &WorkflowEvalObservation,
    contract: &SemanticCompletionContract,
) -> Option<String> {
    let response_lower = obs.final_response.to_ascii_lowercase();
    contract
        .forbidden_silent_completion_patterns
        .iter()
        .find(|p| response_lower.contains(&p.to_ascii_lowercase()))
        .cloned()
}

fn derive_recommended_fix(
    kind: &WorkflowVerdictKind,
    levels: &WorkflowSuccessLevels,
    semantic: &[SemanticMismatch],
    observable: &[ObservableMismatch],
    silent: &Option<String>,
) -> String {
    if let Some(trigger) = silent {
        return format!(
            "SILENT_COMPLETION: response contains forbidden pattern '{}'. \
             KRIA must surface the actual output, not claim completion without evidence. \
             Check observable_completion/mod.rs and loop_engine/mod.rs for result surfacing.",
            trigger
        );
    }
    match kind {
        WorkflowVerdictKind::SemanticFail => {
            let missing: Vec<_> = semantic.iter().filter(|m| !m.found_in_response).collect();
            if missing.is_empty() {
                "Semantic failure with no missing signals — check silent-completion patterns."
                    .into()
            } else {
                format!(
                    "Missing semantic signals in response: {:?}. \
                     Ensure KRIA surfaces the actual result, not just claims it ran.",
                    missing
                        .iter()
                        .map(|m| &m.expected_signal)
                        .collect::<Vec<_>>()
                )
            }
        }
        WorkflowVerdictKind::ObservableFail => {
            let missing_obs: Vec<_> = observable.iter().collect();
            format!(
                "Observable outputs not satisfied: {}. \
                 KRIA must surface the result visibly. \
                 Check ObservableCompletionEngine and loop_engine result display.",
                missing_obs
                    .iter()
                    .map(|o| o.description.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        WorkflowVerdictKind::FalseSuccess => {
            "FALSE SUCCESS: KRIA claimed success with no verifiable evidence. \
             Check that success claims in loop_engine/mod.rs are gated on \
             ObservableCompletionEngine.verify_visible() returning true."
                .into()
        }
        WorkflowVerdictKind::RecoveryFail => {
            "RECOVERY FAIL: interruption was not handled correctly. \
             Check WorkflowContinuationRuntime.classify_interruption() and \
             plan_recovery() return values match the expected recovery plan."
                .into()
        }
        WorkflowVerdictKind::Fail => {
            if !levels.tool_success {
                "Tool execution failed entirely — check tool registry and daemon health.".into()
            } else if !levels.workflow_success {
                "Workflow stages did not complete — check stage executor and checkpoint logic."
                    .into()
            } else {
                "Unknown failure — review stage_errors and event trace.".into()
            }
        }
        _ => "Review the evidence list for specific failure details.".into(),
    }
}

fn build_human_summary(
    case_id: &str,
    kind: &WorkflowVerdictKind,
    levels: &WorkflowSuccessLevels,
    semantic: &[SemanticMismatch],
    observable: &[ObservableMismatch],
    silent: &Option<String>,
) -> String {
    let mut parts = vec![
        format!("Case: {}", case_id),
        format!("Verdict: {}", kind.as_str()),
        format!("Scores: {}", levels.summary()),
    ];

    if let Some(t) = silent {
        parts.push(format!("⚠ Silent completion triggered by: '{}'", t));
    }

    let missing_semantic: Vec<_> = semantic
        .iter()
        .filter(|m| !m.found_in_response)
        .map(|m| m.expected_signal.as_str())
        .collect();
    if !missing_semantic.is_empty() {
        parts.push(format!("Missing semantic signals: {:?}", missing_semantic));
    }

    for obs_miss in observable {
        parts.push(format!(
            "Observable gap: '{}' — {}",
            obs_miss.description, obs_miss.why_failed
        ));
    }

    parts.join("\n")
}
