//! Multi-dimensional workflow cognition scorer.
//!
//! Transforms a `WorkflowEvalObservation` into `WorkflowSuccessLevels`
//! by independently scoring each of the five success dimensions.
//!
//! ## Scoring logic
//!
//! | Dimension     | Signal sources                                              |
//! |---------------|-------------------------------------------------------------|
//! | Tool          | tools_called is non-empty and contains expected tools       |
//! | Workflow      | completed_stage_labels covers required stages               |
//! | Semantic      | response contains contract signals + no silent-completion   |
//! | Observable    | artifact exists OR response contains visible-output signal  |
//! | Collaborative | interruption was detected + recovery matched expected plan  |

use super::types::{SemanticCompletionContract, WorkflowEvalObservation, WorkflowSuccessLevels};

// ─── WorkflowCognitionScorer ──────────────────────────────────────────────────

/// Scores a workflow observation across all five success dimensions.
pub struct WorkflowCognitionScorer;

impl WorkflowCognitionScorer {
    /// Compute `WorkflowSuccessLevels` from an observation and its contract.
    pub fn score(
        obs: &WorkflowEvalObservation,
        contract: &SemanticCompletionContract,
        interruption_expected: bool,
    ) -> WorkflowSuccessLevels {
        let tool = Self::score_tool(obs);
        let workflow = Self::score_workflow(obs, contract);
        let semantic = Self::score_semantic(obs, contract);
        let observable = Self::score_observable(obs, contract);
        let collaborative = if interruption_expected {
            Some(Self::score_collaborative(obs))
        } else {
            None
        };

        WorkflowSuccessLevels {
            tool_success: tool,
            workflow_success: workflow,
            semantic_success: semantic,
            observable_success: observable,
            collaborative_success: collaborative,
        }
    }

    // ── Tool success ──────────────────────────────────────────────────────

    fn score_tool(obs: &WorkflowEvalObservation) -> bool {
        !obs.tools_called.is_empty() && obs.stage_errors.is_empty()
    }

    // ── Workflow success ──────────────────────────────────────────────────

    fn score_workflow(
        obs: &WorkflowEvalObservation,
        contract: &SemanticCompletionContract,
    ) -> bool {
        if contract.required_stage_labels.is_empty() {
            // No specific stage requirements: pass if tool succeeded
            return !obs.tools_called.is_empty();
        }

        let completed_lower: Vec<String> = obs
            .completed_stage_labels
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .collect();

        contract.required_stage_labels.iter().all(|required| {
            let req_lower = required.to_ascii_lowercase();
            completed_lower.iter().any(|c| c.contains(&req_lower))
        })
    }

    // ── Semantic success ──────────────────────────────────────────────────

    fn score_semantic(
        obs: &WorkflowEvalObservation,
        contract: &SemanticCompletionContract,
    ) -> bool {
        let response_lower = obs.final_response.to_ascii_lowercase();

        // Fail if any forbidden silent-completion pattern appears
        for pattern in &contract.forbidden_silent_completion_patterns {
            if response_lower.contains(&pattern.to_ascii_lowercase()) {
                return false;
            }
        }

        // If require_observable_before_success_claim: must also pass observable check
        if contract.require_observable_before_success_claim {
            // At least one semantic signal AND at least one observable output
            let has_signal = contract
                .semantic_success_signals
                .iter()
                .any(|s| response_lower.contains(&s.to_ascii_lowercase()));

            let has_observable = Self::has_any_observable(obs, contract);

            return has_signal && has_observable;
        }

        // Otherwise: just check for semantic signals
        if contract.semantic_success_signals.is_empty() {
            return !obs.tools_called.is_empty();
        }

        contract
            .semantic_success_signals
            .iter()
            .any(|s| response_lower.contains(&s.to_ascii_lowercase()))
    }

    // ── Observable success ────────────────────────────────────────────────

    fn score_observable(
        obs: &WorkflowEvalObservation,
        contract: &SemanticCompletionContract,
    ) -> bool {
        if contract.required_observable_outputs.is_empty() {
            // No explicit observable requirement: pass if response is non-empty
            return !obs.final_response.trim().is_empty();
        }

        // All required outputs must be satisfied
        contract
            .required_observable_outputs
            .iter()
            .filter(|o| o.required)
            .all(|required_output| Self::check_observable(obs, required_output))
    }

    fn check_observable(
        obs: &WorkflowEvalObservation,
        req: &super::types::ObservableOutputContract,
    ) -> bool {
        let response_lower = obs.final_response.to_ascii_lowercase();

        // Check response signal
        let response_ok = req
            .response_must_contain
            .iter()
            .any(|signal| response_lower.contains(&signal.to_ascii_lowercase()));

        // Check artifact (if required)
        let artifact_ok = match &req.artifact_path_glob {
            None => true, // No artifact requirement
            Some(_glob) => {
                // Check if any artifact satisfies size + content requirements
                obs.artifacts_found.iter().any(|a| {
                    let size_ok = req
                        .artifact_min_bytes
                        .map(|min| a.size_bytes >= min)
                        .unwrap_or(true);
                    let content_ok = req
                        .artifact_content_contains
                        .as_ref()
                        .map(|needle| {
                            a.content_preview
                                .to_ascii_lowercase()
                                .contains(&needle.to_ascii_lowercase())
                        })
                        .unwrap_or(true);
                    size_ok && content_ok
                })
            }
        };

        response_ok && artifact_ok
    }

    fn has_any_observable(
        obs: &WorkflowEvalObservation,
        contract: &SemanticCompletionContract,
    ) -> bool {
        if contract.required_observable_outputs.is_empty() {
            return !obs.final_response.trim().is_empty();
        }
        contract
            .required_observable_outputs
            .iter()
            .any(|o| Self::check_observable(obs, o))
    }

    // ── Collaborative success ─────────────────────────────────────────────

    fn score_collaborative(obs: &WorkflowEvalObservation) -> bool {
        // Check if KRIA acknowledged the interruption and recovery
        if let Some(handled) = obs.interruption_handled {
            return handled;
        }
        // If we expected an interruption but the field is None → not handled
        false
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_eval::contracts::coding_contract;
    use crate::workflow_eval::types::ArtifactFound;

    fn obs_with_response(response: &str, tools: &[&str]) -> WorkflowEvalObservation {
        WorkflowEvalObservation {
            case_id: "test".into(),
            final_response: response.to_string(),
            tools_called: tools.iter().map(|s| s.to_string()).collect(),
            completed_stage_labels: vec!["open_application".into()],
            reported_success: true,
            interruption_handled: None,
            artifacts_found: vec![],
            stage_errors: vec![],
            duration_ms: 500,
            daemon_alive_at_start: true,
            daemon_alive_at_end: true,
        }
    }

    fn obs_with_artifact(response: &str, artifact_bytes: u64) -> WorkflowEvalObservation {
        let mut obs = obs_with_response(response, &["type_text", "open_application"]);
        obs.artifacts_found = vec![ArtifactFound {
            path: "/tmp/test.py".into(),
            size_bytes: artifact_bytes,
            content_preview: "def pascal".into(),
        }];
        obs
    }

    #[test]
    fn tool_success_requires_nonempty_tools() {
        let obs = obs_with_response("done", &[]);
        assert!(!WorkflowCognitionScorer::score(&obs, &coding_contract(), false).tool_success);

        let obs = obs_with_response("done", &["type_text"]);
        assert!(WorkflowCognitionScorer::score(&obs, &coding_contract(), false).tool_success);
    }

    #[test]
    fn semantic_fail_on_silent_completion_phrase() {
        let contract = coding_contract();
        let obs = obs_with_response(
            "Done! I completed the task. I've opened VS Code and written the code.",
            &["type_text", "open_application"],
        );
        let levels = WorkflowCognitionScorer::score(&obs, &contract, false);
        // "done!" is a forbidden silent completion pattern in coding contract
        assert!(
            !levels.semantic_success,
            "expected semantic fail for silent phrase"
        );
    }

    #[test]
    fn semantic_success_with_output_signal_and_artifact() {
        let contract = coding_contract();
        // Response must satisfy BOTH required observable outputs:
        // 1) "Source code file exists" → needs "wrote"/"file"/"created"/"saved"
        // 2) "Program output visible" → needs "output"/"ran"/"executed"
        let obs = obs_with_artifact(
            "Wrote the file. Executed the program. Output:\n1\n1 1\n1 2 1",
            100,
        );
        let levels = WorkflowCognitionScorer::score(&obs, &contract, false);
        assert!(levels.tool_success);
        assert!(levels.semantic_success, "summary: {}", levels.summary());
        assert!(levels.observable_success, "summary: {}", levels.summary());
    }

    #[test]
    fn observable_fail_with_no_artifact_and_no_response_signal() {
        let contract = coding_contract();
        let obs = obs_with_response("I've opened the IDE.", &["open_application"]);
        let levels = WorkflowCognitionScorer::score(&obs, &contract, false);
        assert!(!levels.observable_success, "summary: {}", levels.summary());
    }

    #[test]
    fn weighted_score_orders_correctly() {
        let full = WorkflowSuccessLevels {
            tool_success: true,
            workflow_success: true,
            semantic_success: true,
            observable_success: true,
            collaborative_success: None,
        };
        let partial = WorkflowSuccessLevels {
            tool_success: true,
            workflow_success: false,
            semantic_success: false,
            observable_success: false,
            collaborative_success: None,
        };
        assert!(full.weighted_score() > partial.weighted_score());
        assert_eq!(full.weighted_score(), 0.95); // 0.10+0.15+0.40+0.30
    }

    #[test]
    fn is_passing_requires_semantic_and_observable() {
        let levels = WorkflowSuccessLevels {
            tool_success: true,
            workflow_success: true,
            semantic_success: false,
            observable_success: true,
            collaborative_success: None,
        };
        assert!(!levels.is_passing());

        let levels = WorkflowSuccessLevels {
            tool_success: true,
            workflow_success: true,
            semantic_success: true,
            observable_success: true,
            collaborative_success: None,
        };
        assert!(levels.is_passing());
    }
}
