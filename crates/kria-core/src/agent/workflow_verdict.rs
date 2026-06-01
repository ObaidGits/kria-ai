//! Workflow Verdict Computation — Single Source of Completion Truth.
//!
//! This module replaces the dual-truth contradiction where:
//! - HTN executor declares structural success
//! - ObservableCompletionEngine re-derives outcomes and contradicts
//!
//! # Authority
//!
//! The `compute_verdict` function is the ONLY place where a workflow's
//! final completion state is determined. It takes:
//! - Structural step results (from the executor)
//! - Verification results (from the verifier, using plan-bound contract)
//! - The outcome contract (from the planner, never re-derived)
//!
//! And produces ONE `WorkflowVerdict` that the frontend renders.
//!
//! # Design Rules
//!
//! - Deterministic: same inputs → same verdict always
//! - No LLM calls, no I/O
//! - Never claims `Complete` without verification evidence
//! - Never claims `Failed` when structural execution succeeded
//! - Honest about what it can and cannot verify

use crate::agent::workflow_types::{
    OutcomeContract, OutcomeFailurePolicy, VisibilityConfidence, WorkflowVerdict,
};

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Structural Step Result (Input from Executor)
// ═══════════════════════════════════════════════════════════════════════════════

/// Result of executing a single workflow step.
#[derive(Debug, Clone)]
pub struct StepResult {
    pub step_index: u32,
    pub action: String,
    pub success: bool,
    pub error: Option<String>,
    pub artifacts: Vec<String>,
    pub duration_ms: u64,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Verification Result (Input from Verifier)
// ═══════════════════════════════════════════════════════════════════════════════

/// Result of verifying a single planned outcome.
#[derive(Debug, Clone)]
pub struct OutcomeVerificationResult {
    pub outcome_description: String,
    pub verified: bool,
    pub confidence: f32,
    pub evidence: String,
    pub method: String,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Verdict Computation (The Single Source of Truth)
// ═══════════════════════════════════════════════════════════════════════════════

/// Compute the canonical workflow verdict from structural + verification results.
///
/// This is the ONLY function that determines workflow completion state.
/// It replaces the contradictory dual-path in loop_engine where:
/// - `result.success` says "done"
/// - `observable_narrative_requires_partial_completion` says "not done"
///
/// # Rules
///
/// 1. If any required structural step failed → `Failed`
/// 2. If all structural steps passed AND all desired outcomes verified → `Complete`
/// 3. If all structural steps passed BUT some desired outcomes unverified → `StructurallyComplete`
/// 4. If some structural steps passed → `Partial`
pub fn compute_verdict(
    step_results: &[StepResult],
    verification_results: &[OutcomeVerificationResult],
    contract: &OutcomeContract,
    total_steps: u32,
) -> VerdictComputation {
    // ── Check structural success ──────────────────────────────────────────
    let all_structural_passed = step_results.iter().all(|r| r.success);
    let completed_steps = step_results.iter().filter(|r| r.success).count() as u32;

    if !all_structural_passed {
        let failed_step = step_results
            .iter()
            .find(|r| !r.success)
            .map(|r| r.step_index)
            .unwrap_or(0);
        let error = step_results
            .iter()
            .find(|r| !r.success)
            .and_then(|r| r.error.clone())
            .unwrap_or_else(|| "Unknown step failure".to_string());

        return VerdictComputation {
            verdict: WorkflowVerdict::Failed {
                step: failed_step,
                reason: error,
                recovery: None,
            },
            visibility_confidence: VisibilityConfidence::NotApplicable,
            narrative: format!("Workflow failed at step {}/{}.", failed_step, total_steps),
        };
    }

    // ── All structural steps passed — check verification ──────────────────
    if contract.required.is_empty() && contract.desired.is_empty() {
        // No outcome contract → structural success is sufficient
        return VerdictComputation {
            verdict: WorkflowVerdict::Complete,
            visibility_confidence: VisibilityConfidence::NotApplicable,
            narrative: format!(
                "Completed {} step{} successfully.",
                completed_steps,
                if completed_steps == 1 { "" } else { "s" }
            ),
        };
    }

    // Check required outcomes
    let required_failures: Vec<&str> = contract
        .required
        .iter()
        .filter(|outcome| {
            !verification_results.iter().any(|vr| {
                vr.outcome_description == outcome.description
                    && vr.verified
                    && vr.confidence >= outcome.min_confidence
            })
        })
        .filter(|outcome| outcome.on_failure == OutcomeFailurePolicy::FailWorkflow)
        .map(|o| o.description.as_str())
        .collect();

    if !required_failures.is_empty() {
        return VerdictComputation {
            verdict: WorkflowVerdict::Failed {
                step: total_steps,
                reason: format!(
                    "Required outcomes not verified: {}",
                    required_failures.join(", ")
                ),
                recovery: None,
            },
            visibility_confidence: VisibilityConfidence::StructuralOnly {
                reason: "Required verification failed".into(),
            },
            narrative: format!(
                "Steps completed but required outcomes not verified: {}",
                required_failures.join(", ")
            ),
        };
    }

    // Check desired outcomes (visibility expectations)
    let unverified_desired: Vec<String> = contract
        .desired
        .iter()
        .filter(|outcome| {
            !verification_results.iter().any(|vr| {
                vr.outcome_description == outcome.description
                    && vr.verified
                    && vr.confidence >= outcome.min_confidence
            })
        })
        .map(|o| o.description.clone())
        .collect();

    if unverified_desired.is_empty() {
        // All desired outcomes verified — full success
        let avg_confidence = if verification_results.is_empty() {
            1.0
        } else {
            verification_results
                .iter()
                .map(|r| r.confidence)
                .sum::<f32>()
                / verification_results.len() as f32
        };

        VerdictComputation {
            verdict: WorkflowVerdict::Complete,
            visibility_confidence: VisibilityConfidence::Confirmed {
                confidence: avg_confidence,
                evidence: "All planned outcomes verified".into(),
            },
            narrative: format!(
                "Completed {} step{} with all outcomes verified.",
                completed_steps,
                if completed_steps == 1 { "" } else { "s" }
            ),
        }
    } else {
        // Some desired outcomes unverified — structurally complete
        VerdictComputation {
            verdict: WorkflowVerdict::StructurallyComplete {
                unverified_outcomes: unverified_desired.clone(),
            },
            visibility_confidence: VisibilityConfidence::StructuralOnly {
                reason: format!(
                    "{} visibility expectation{} could not be verified",
                    unverified_desired.len(),
                    if unverified_desired.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                ),
            },
            narrative: format!(
                "All {} steps completed structurally. Visibility unverified for: {}",
                completed_steps,
                unverified_desired.join(", ")
            ),
        }
    }
}

/// The complete verdict computation result.
#[derive(Debug, Clone)]
pub struct VerdictComputation {
    /// The canonical verdict
    pub verdict: WorkflowVerdict,
    /// Overall visibility confidence
    pub visibility_confidence: VisibilityConfidence,
    /// Human-readable narrative (for backward compatibility with existing UI)
    pub narrative: String,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — Legacy Adapter: Convert existing WorkflowResult to Verdict
// ═══════════════════════════════════════════════════════════════════════════════

/// Convert a legacy `htn_executor::WorkflowResult` + observable narrative
/// into a canonical `WorkflowVerdict`.
///
/// This adapter allows the existing GUI workflow path to produce canonical
/// verdicts without rewriting the executor. It replaces the contradictory
/// `format_gui_workflow_partial_for_user` / `format_gui_workflow_success_for_user`
/// dual-path with a single deterministic verdict.
pub fn verdict_from_legacy_result(
    success: bool,
    completed_steps: usize,
    total_steps: usize,
    error: Option<&str>,
    observable_narrative: Option<&str>,
) -> VerdictComputation {
    if !success {
        let reason = error.unwrap_or("Unknown error").to_string();
        return VerdictComputation {
            verdict: WorkflowVerdict::Failed {
                step: (completed_steps + 1) as u32,
                reason: reason.clone(),
                recovery: None,
            },
            visibility_confidence: VisibilityConfidence::NotApplicable,
            narrative: format!(
                "Workflow failed at step {}/{}. {}",
                completed_steps + 1,
                total_steps,
                reason
            ),
        };
    }

    // Structural success — check observable narrative for visibility truth
    let visibility_failed = observable_narrative
        .map(|n| {
            n.starts_with('⚠')
                || n.contains("Expected outcome not yet visible")
                || n.contains("not yet visible")
        })
        .unwrap_or(false);

    if visibility_failed {
        // Structural success + visibility failure = StructurallyComplete
        let unverified = observable_narrative
            .map(|n| {
                // Extract the outcome descriptions from the narrative
                n.lines()
                    .filter(|l| l.contains('(') && l.contains(')'))
                    .map(|l| l.trim().to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let unverified_outcomes = if unverified.is_empty() {
            vec!["Visibility verification timed out or unavailable".to_string()]
        } else {
            unverified
        };

        VerdictComputation {
            verdict: WorkflowVerdict::StructurallyComplete {
                unverified_outcomes,
            },
            visibility_confidence: VisibilityConfidence::StructuralOnly {
                reason: "Observable completion check reported visibility failure".into(),
            },
            narrative: format!(
                "All {} steps completed structurally, but visible outcomes could not be verified.",
                completed_steps
            ),
        }
    } else {
        // Full success
        VerdictComputation {
            verdict: WorkflowVerdict::Complete,
            visibility_confidence: if observable_narrative.is_some() {
                VisibilityConfidence::Confirmed {
                    confidence: 0.85,
                    evidence: observable_narrative.unwrap_or("").to_string(),
                }
            } else {
                VisibilityConfidence::NotApplicable
            },
            narrative: format!(
                "Completed {} step{} successfully.",
                completed_steps,
                if completed_steps == 1 { "" } else { "s" }
            ),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §5 — Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::workflow_types::{
        OutcomeContract, OutcomeExpectation, OutcomeFailurePolicy, PlannedOutcome,
    };

    #[test]
    fn all_steps_pass_no_contract_gives_complete() {
        let steps = vec![
            StepResult {
                step_index: 1,
                action: "write_file".into(),
                success: true,
                error: None,
                artifacts: vec![],
                duration_ms: 100,
            },
            StepResult {
                step_index: 2,
                action: "open_app".into(),
                success: true,
                error: None,
                artifacts: vec![],
                duration_ms: 200,
            },
        ];
        let result = compute_verdict(&steps, &[], &OutcomeContract::empty(), 2);
        assert!(matches!(result.verdict, WorkflowVerdict::Complete));
    }

    #[test]
    fn step_failure_gives_failed_verdict() {
        let steps = vec![
            StepResult {
                step_index: 1,
                action: "write_file".into(),
                success: true,
                error: None,
                artifacts: vec![],
                duration_ms: 100,
            },
            StepResult {
                step_index: 2,
                action: "open_app".into(),
                success: false,
                error: Some("app not found".into()),
                artifacts: vec![],
                duration_ms: 50,
            },
        ];
        let result = compute_verdict(&steps, &[], &OutcomeContract::empty(), 2);
        match result.verdict {
            WorkflowVerdict::Failed { step, reason, .. } => {
                assert_eq!(step, 2);
                assert!(reason.contains("app not found"));
            }
            _ => panic!("Expected Failed verdict"),
        }
    }

    #[test]
    fn structural_pass_with_unverified_desired_gives_structurally_complete() {
        let steps = vec![StepResult {
            step_index: 1,
            action: "write_file".into(),
            success: true,
            error: None,
            artifacts: vec![],
            duration_ms: 100,
        }];
        let contract = OutcomeContract {
            required: vec![],
            desired: vec![PlannedOutcome {
                description: "VS Code window visible".into(),
                expectation: OutcomeExpectation::AppWindowVisible {
                    app: "code".into(),
                    title_hint: None,
                },
                min_confidence: 0.7,
                on_failure: OutcomeFailurePolicy::DowngradeFidelity,
            }],
        };
        // No verification results → desired outcome unverified
        let result = compute_verdict(&steps, &[], &contract, 1);
        match result.verdict {
            WorkflowVerdict::StructurallyComplete {
                unverified_outcomes,
            } => {
                assert_eq!(unverified_outcomes.len(), 1);
                assert!(unverified_outcomes[0].contains("VS Code"));
            }
            _ => panic!(
                "Expected StructurallyComplete verdict, got {:?}",
                result.verdict
            ),
        }
    }

    #[test]
    fn structural_pass_with_verified_desired_gives_complete() {
        let steps = vec![StepResult {
            step_index: 1,
            action: "write_file".into(),
            success: true,
            error: None,
            artifacts: vec![],
            duration_ms: 100,
        }];
        let contract = OutcomeContract {
            required: vec![],
            desired: vec![PlannedOutcome {
                description: "VS Code window visible".into(),
                expectation: OutcomeExpectation::AppWindowVisible {
                    app: "code".into(),
                    title_hint: None,
                },
                min_confidence: 0.7,
                on_failure: OutcomeFailurePolicy::DowngradeFidelity,
            }],
        };
        let verifications = vec![OutcomeVerificationResult {
            outcome_description: "VS Code window visible".into(),
            verified: true,
            confidence: 0.85,
            evidence: "AT-SPI found window".into(),
            method: "atspi".into(),
        }];
        let result = compute_verdict(&steps, &verifications, &contract, 1);
        assert!(matches!(result.verdict, WorkflowVerdict::Complete));
    }

    #[test]
    fn legacy_adapter_success_no_narrative_gives_complete() {
        let result = verdict_from_legacy_result(true, 3, 3, None, None);
        assert!(matches!(result.verdict, WorkflowVerdict::Complete));
    }

    #[test]
    fn legacy_adapter_success_with_visibility_failure_gives_structurally_complete() {
        let narrative = "⚠ Expected outcome not yet visible: code is open (Visibility probe timed out after 750ms)";
        let result = verdict_from_legacy_result(true, 3, 3, None, Some(narrative));
        assert!(matches!(
            result.verdict,
            WorkflowVerdict::StructurallyComplete { .. }
        ));
    }

    #[test]
    fn legacy_adapter_failure_gives_failed() {
        let result = verdict_from_legacy_result(false, 1, 3, Some("app not found"), None);
        match result.verdict {
            WorkflowVerdict::Failed { step, reason, .. } => {
                assert_eq!(step, 2); // completed_steps + 1
                assert!(reason.contains("app not found"));
            }
            _ => panic!("Expected Failed verdict"),
        }
    }

    #[test]
    fn never_claims_complete_without_evidence_when_contract_has_desired() {
        // If there's a desired outcome and NO verification results,
        // the verdict must be StructurallyComplete, never Complete.
        let steps = vec![StepResult {
            step_index: 1,
            action: "open_app".into(),
            success: true,
            error: None,
            artifacts: vec![],
            duration_ms: 100,
        }];
        let contract = OutcomeContract {
            required: vec![],
            desired: vec![PlannedOutcome {
                description: "Browser at localhost".into(),
                expectation: OutcomeExpectation::BrowserAtUrl {
                    url_contains: "localhost".into(),
                },
                min_confidence: 0.6,
                on_failure: OutcomeFailurePolicy::DowngradeFidelity,
            }],
        };
        let result = compute_verdict(&steps, &[], &contract, 1);
        assert!(
            !matches!(result.verdict, WorkflowVerdict::Complete),
            "Must NOT claim Complete when desired outcomes are unverified"
        );
    }
}
