//! Workflow Cognition Judge.
//!
//! Produces a `WorkflowEvalVerdict` from an observation + eval case by:
//!
//! 1. Running the `SafetyFilter` (skip if unsafe)
//! 2. Computing `WorkflowSuccessLevels` via `WorkflowCognitionScorer`
//! 3. Detecting false-success and silent-completion
//! 4. Mapping success levels to `WorkflowVerdictKind`
//! 5. Capturing failure diagnostics

use super::failure_analysis::FailureDiagnostic;
use super::safety_filter::SafetyFilter;
use super::scoring::WorkflowCognitionScorer;
use super::types::{
    WorkflowEvalCase, WorkflowEvalObservation, WorkflowEvalVerdict, WorkflowSuccessLevels,
    WorkflowVerdictKind,
};

/// Judges a workflow eval case against its observation.
pub struct WorkflowCognitionJudge;

impl WorkflowCognitionJudge {
    /// Evaluate and return a full verdict with all 5 success dimensions scored.
    pub fn evaluate(
        case: &WorkflowEvalCase,
        obs: &WorkflowEvalObservation,
    ) -> (WorkflowEvalVerdict, Option<FailureDiagnostic>) {
        // ── Step 1: Safety filter ─────────────────────────────────────────
        if let Err(reason) = SafetyFilter::validate(case) {
            let verdict = WorkflowEvalVerdict {
                case_id: case.id.clone(),
                success_levels: WorkflowSuccessLevels::default(),
                kind: WorkflowVerdictKind::Skip,
                failure_reason: Some(reason.clone()),
                evidence: vec![format!("SKIPPED: {}", reason)],
                quality_score: 1.0,
                recommended_fix: None,
                explanation: format!("SKIP: {} — safety constraint", case.id),
            };
            return (verdict, None);
        }

        // ── Step 2: Daemon/display environment check ──────────────────────
        if case.requires_daemon && !obs.daemon_alive_at_start {
            let verdict = WorkflowEvalVerdict {
                case_id: case.id.clone(),
                success_levels: WorkflowSuccessLevels::default(),
                kind: WorkflowVerdictKind::Skip,
                failure_reason: Some("uinput daemon not running".to_string()),
                evidence: vec!["SKIPPED: uinput daemon required but not alive".to_string()],
                quality_score: 1.0,
                recommended_fix: None,
                explanation: format!(
                    "SKIP: {} — requires daemon (KRIA_EVAL_LIVE=1 or daemon must be started)",
                    case.id
                ),
            };
            return (verdict, None);
        }

        // ── Step 3: Score all 5 dimensions ───────────────────────────────
        let interruption_expected = case.interruption.is_some();
        let levels = WorkflowCognitionScorer::score(obs, &case.contract, interruption_expected);

        // ── Step 4: Detect false success ─────────────────────────────────
        let false_success = detect_false_success(obs, &levels);

        // ── Step 5: Detect silent completion ─────────────────────────────
        let silent_trigger = case
            .contract
            .forbidden_silent_completion_patterns
            .iter()
            .find(|p| {
                obs.final_response
                    .to_ascii_lowercase()
                    .contains(&p.to_ascii_lowercase())
            })
            .cloned();

        // ── Step 6: Build evidence list ───────────────────────────────────
        let mut evidence = Vec::new();
        evidence.push(format!("Success levels: {}", levels.summary()));
        evidence.push(format!(
            "Tools called: {}",
            if obs.tools_called.is_empty() {
                "none".to_string()
            } else {
                obs.tools_called.join(", ")
            }
        ));
        evidence.push(format!(
            "Stages completed: {}",
            if obs.completed_stage_labels.is_empty() {
                "none".to_string()
            } else {
                obs.completed_stage_labels.join(", ")
            }
        ));
        evidence.push(format!("Artifacts found: {}", obs.artifacts_found.len()));
        if let Some(ref t) = silent_trigger {
            evidence.push(format!("SILENT_COMPLETION pattern: '{}'", t));
        }
        if !obs.stage_errors.is_empty() {
            evidence.push(format!("Stage errors: {}", obs.stage_errors.join("; ")));
        }

        // ── Step 7: Classify verdict kind ─────────────────────────────────
        let kind = classify_verdict(
            &levels,
            false_success,
            &silent_trigger,
            interruption_expected,
        );

        // ── Step 8: Build recommended fix ─────────────────────────────────
        let recommended_fix = if kind.is_passing() {
            None
        } else {
            Some(quick_fix(&kind, &levels))
        };

        let quality_score = levels.weighted_score();
        let explanation = build_explanation(case, &kind, &levels);
        let failure_reason = if kind.is_passing() {
            None
        } else {
            evidence
                .iter()
                .find(|e| e.starts_with("SILENT") || e.starts_with("Stage errors"))
                .cloned()
                .or_else(|| Some(kind.as_str().to_string()))
        };

        let verdict = WorkflowEvalVerdict {
            case_id: case.id.clone(),
            success_levels: levels.clone(),
            kind: kind.clone(),
            failure_reason: failure_reason.clone(),
            evidence: evidence.clone(),
            quality_score,
            recommended_fix: recommended_fix.clone(),
            explanation: explanation.clone(),
        };

        // ── Step 9: Capture failure diagnostic ───────────────────────────
        let diagnostic = if !kind.is_passing() {
            Some(FailureDiagnostic::from_verdict(
                obs,
                &verdict,
                &case.contract,
            ))
        } else {
            None
        };

        (verdict, diagnostic)
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn detect_false_success(obs: &WorkflowEvalObservation, levels: &WorkflowSuccessLevels) -> bool {
    let false_success_phrases = [
        "successfully completed",
        "task completed successfully",
        "done! i completed",
        "i have successfully",
        "automation complete",
    ];
    let response_lower = obs.final_response.to_ascii_lowercase();
    let claims_success = false_success_phrases
        .iter()
        .any(|p| response_lower.contains(p));

    // False success: claims success but semantic or observable failed
    claims_success && (!levels.semantic_success || !levels.observable_success)
}

fn classify_verdict(
    levels: &WorkflowSuccessLevels,
    false_success: bool,
    silent_trigger: &Option<String>,
    interruption_expected: bool,
) -> WorkflowVerdictKind {
    // False success takes highest priority
    if false_success {
        return WorkflowVerdictKind::FalseSuccess;
    }

    // Silent completion: tool ran but output was never surfaced
    if silent_trigger.is_some() {
        return WorkflowVerdictKind::SilentCompletion;
    }

    // Recovery failure
    if interruption_expected {
        if let Some(false) = levels.collaborative_success {
            return WorkflowVerdictKind::RecoveryFail;
        }
    }

    // Pass if semantic + observable both satisfied
    if levels.semantic_success && levels.observable_success {
        return WorkflowVerdictKind::Pass;
    }

    // Semantic failed but tool worked → semantic fail
    if levels.tool_success && !levels.semantic_success {
        return WorkflowVerdictKind::SemanticFail;
    }

    // Semantic ok but not surfaced → observable fail
    if levels.semantic_success && !levels.observable_success {
        return WorkflowVerdictKind::ObservableFail;
    }

    // Nothing worked
    WorkflowVerdictKind::Fail
}

fn quick_fix(kind: &WorkflowVerdictKind, levels: &WorkflowSuccessLevels) -> String {
    match kind {
        WorkflowVerdictKind::FalseSuccess => {
            "Replace unconditional success claims with ObservableCompletionEngine.verify_visible(). \
             See loop_engine/mod.rs result handling."
                .to_string()
        }
        WorkflowVerdictKind::SilentCompletion => {
            "KRIA must surface the output explicitly — remove hollow 'Done!' messages and \
             replace with actual output content in the response."
                .to_string()
        }
        WorkflowVerdictKind::SemanticFail => {
            "Semantic contract not satisfied. Ensure the response contains actual result content, \
             not just action confirmations. Check workflow_expectation for missing outcome templates."
                .to_string()
        }
        WorkflowVerdictKind::ObservableFail => {
            "Semantic success but result not visible. Check ObservableCompletionEngine policies \
             and ensure loop_engine surfaces the output in StreamEvent::Text."
                .to_string()
        }
        WorkflowVerdictKind::RecoveryFail => {
            "Interruption not recovered correctly. Check WorkflowContinuationRuntime.plan_recovery() \
             and ensure the recovery action matches the interruption class."
                .to_string()
        }
        WorkflowVerdictKind::Fail => {
            if !levels.tool_success {
                "Tool execution failed. Check tool registry, daemon health, and stage_executor error propagation.".to_string()
            } else {
                "Workflow execution incomplete. Check stage_executor.execute_actions() and checkpoint validation.".to_string()
            }
        }
        _ => "Review evidence list for specifics.".to_string(),
    }
}

fn build_explanation(
    case: &WorkflowEvalCase,
    kind: &WorkflowVerdictKind,
    levels: &WorkflowSuccessLevels,
) -> String {
    format!(
        "{}: {} [{}] — {}",
        kind.as_str(),
        case.id,
        case.category.as_str(),
        levels.summary()
    )
}
