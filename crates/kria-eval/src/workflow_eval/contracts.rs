//! Semantic completion contracts for each workflow category.
//!
//! Each contract is a compiled constant that defines exactly what "done"
//! means for a given workflow category. These are NOT LLM outputs —
//! they are grounded in observable, verifiable signals.
//!
//! ## Contract authority
//!
//! Contracts are the single source of truth for:
//! - What stages must complete
//! - What the user must be able to see
//! - What response patterns prove semantic completion
//! - What response patterns indicate hollow/silent completion (FAIL)

use super::types::{EvalWorkflowCategory, ObservableOutputContract, SemanticCompletionContract};

// ─── Contract builder helpers ─────────────────────────────────────────────────

fn output(
    description: &str,
    response_signals: &[&str],
    path_glob: Option<&str>,
    min_bytes: Option<u64>,
    content_contains: Option<&str>,
    required: bool,
) -> ObservableOutputContract {
    ObservableOutputContract {
        description: description.to_string(),
        response_must_contain: response_signals.iter().map(|s| s.to_string()).collect(),
        artifact_path_glob: path_glob.map(str::to_string),
        artifact_min_bytes: min_bytes,
        artifact_content_contains: content_contains.map(str::to_string),
        required,
    }
}

// ─── Coding ───────────────────────────────────────────────────────────────────

/// Contract for coding workflows (write code, run code, show output).
///
/// "Done" means:
/// 1. Code was written to a file (not just described)
/// 2. The IDE/editor opened the file
/// 3. Execution happened and the output is visible in the response
/// 4. The user can see the output — it must be surfaced explicitly
pub fn coding_contract() -> SemanticCompletionContract {
    SemanticCompletionContract {
        success_definition:
            "Code was written, executed, and its output surfaced visibly to the user".to_string(),
        category: EvalWorkflowCategory::Coding,
        required_observable_outputs: vec![
            output(
                "Source code file exists on disk",
                &["wrote", "created", "saved", "file"],
                Some("~/.kria/generated/*.{py,rs,js,ts,sh}"),
                Some(20),
                None,
                true,
            ),
            output(
                "Execution output shown in response",
                &["output", "result", "ran", "executed", "printed"],
                None,
                None,
                None,
                true,
            ),
        ],
        semantic_success_signals: vec![
            "output".to_string(),
            "result".to_string(),
            "printed".to_string(),
            "executed".to_string(),
            "ran successfully".to_string(),
        ],
        forbidden_silent_completion_patterns: vec![
            "i've opened".to_string(),
            "i opened the".to_string(),
            "task completed".to_string(),
            "done!".to_string(),
            "i have written the code".to_string(),
        ],
        required_stage_labels: vec!["open_application".to_string()],
        require_observable_before_success_claim: true,
    }
}

/// Tighter contract for "run and SHOW output" coding workflows.
///
/// These require that the actual output (e.g., Pascal triangle lines)
/// appears in the KRIA response — not just a claim that it ran.
pub fn coding_run_and_show_contract() -> SemanticCompletionContract {
    let mut contract = coding_contract();
    contract.success_definition =
        "Code executed and the actual output lines are surfaced in KRIA's response".to_string();
    contract.semantic_success_signals = vec![
        "1".to_string(), // Pascal triangle first row
        "output:".to_string(),
        "```".to_string(), // Code block wrapping the output
        "hello".to_string(),
    ];
    contract.forbidden_silent_completion_patterns.extend(vec![
        "code is ready".to_string(),
        "program has been written".to_string(),
        "you can run it".to_string(),
        "to run this".to_string(),
    ]);
    contract.require_observable_before_success_claim = true;
    contract
}

// ─── Browser ──────────────────────────────────────────────────────────────────

/// Contract for browser workflows (search, navigate, extract).
///
/// "Done" means:
/// 1. Browser navigated to the correct page
/// 2. Page content was extracted or interacted with
/// 3. A summary/result was surfaced to the user
pub fn browser_contract() -> SemanticCompletionContract {
    SemanticCompletionContract {
        success_definition:
            "Browser navigated to the target page and content was surfaced to the user".to_string(),
        category: EvalWorkflowCategory::Browser,
        required_observable_outputs: vec![output(
            "Page title or content summary shown in response",
            &[
                "page", "result", "found", "shows", "displays", "weather", "search",
            ],
            None,
            None,
            None,
            true,
        )],
        semantic_success_signals: vec![
            "result".to_string(),
            "found".to_string(),
            "page".to_string(),
            "summary".to_string(),
            "weather".to_string(),
            "temperature".to_string(),
        ],
        forbidden_silent_completion_patterns: vec![
            "i've opened the browser".to_string(),
            "browser is open".to_string(),
            "i opened chrome".to_string(),
        ],
        required_stage_labels: vec![],
        require_observable_before_success_claim: true,
    }
}

// ─── File Management ──────────────────────────────────────────────────────────

/// Contract for file management workflows (create, move, zip, rename).
pub fn file_management_contract() -> SemanticCompletionContract {
    SemanticCompletionContract {
        success_definition:
            "File operation completed and the resulting artifact is verifiable on disk".to_string(),
        category: EvalWorkflowCategory::FileManagement,
        required_observable_outputs: vec![output(
            "Target file or directory exists on disk",
            &["created", "moved", "renamed", "organized", "zipped"],
            None,
            None,
            None,
            true,
        )],
        semantic_success_signals: vec![
            "created".to_string(),
            "moved".to_string(),
            "renamed".to_string(),
            "organized".to_string(),
            "done".to_string(),
        ],
        forbidden_silent_completion_patterns: vec![
            "you can create".to_string(),
            "to create a folder".to_string(),
            "here are steps".to_string(),
            "i would recommend".to_string(),
        ],
        required_stage_labels: vec![],
        require_observable_before_success_claim: true,
    }
}

// ─── Interruption + Recovery ──────────────────────────────────────────────────

/// Contract for interruption recovery workflows.
///
/// "Done" means:
/// 1. KRIA detected the interruption
/// 2. Recovery behavior matched expectations (retry/pause/resume)
/// 3. The user was informed of the interruption and outcome
pub fn interruption_recovery_contract() -> SemanticCompletionContract {
    SemanticCompletionContract {
        success_definition: "Interruption detected, correct recovery taken, user informed clearly"
            .to_string(),
        category: EvalWorkflowCategory::Unknown,
        required_observable_outputs: vec![output(
            "Interruption acknowledgement in response",
            &[
                "interrupted",
                "retry",
                "attempting",
                "restarting",
                "paused",
                "recovered",
                "resumed",
            ],
            None,
            None,
            None,
            true,
        )],
        semantic_success_signals: vec![
            "retry".to_string(),
            "attempting".to_string(),
            "recovered".to_string(),
            "resumed".to_string(),
        ],
        forbidden_silent_completion_patterns: vec![
            "completed successfully".to_string(),
            "task is done".to_string(),
        ],
        required_stage_labels: vec![],
        require_observable_before_success_claim: false,
    }
}

// ─── Human Expectation ────────────────────────────────────────────────────────

/// Contract for human expectation alignment evals.
///
/// "Done" means the human-visible result is EXPLICITLY shown, not implied.
pub fn human_expectation_contract(visible_output_signal: &str) -> SemanticCompletionContract {
    SemanticCompletionContract {
        success_definition: format!(
            "The user can see '{}' in KRIA's response — not just be told it happened",
            visible_output_signal
        ),
        category: EvalWorkflowCategory::Unknown,
        required_observable_outputs: vec![output(
            "Explicit visible output in response",
            &[visible_output_signal],
            None,
            None,
            None,
            true,
        )],
        semantic_success_signals: vec![visible_output_signal.to_string()],
        forbidden_silent_completion_patterns: vec![
            "task completed".to_string(),
            "i've done it".to_string(),
            "check the terminal".to_string(),
            "see the output in".to_string(),
        ],
        required_stage_labels: vec![],
        require_observable_before_success_claim: true,
    }
}

// ─── Multi-App ────────────────────────────────────────────────────────────────

/// Contract for multi-application workflows (browser → IDE → terminal).
///
/// "Done" means all apps transitioned correctly and the final output
/// from the last stage is surfaced to the user.
pub fn multi_app_contract(final_app: &str, final_signal: &str) -> SemanticCompletionContract {
    SemanticCompletionContract {
        success_definition: format!(
            "All app transitions completed and final output from '{}' is visible",
            final_app
        ),
        category: EvalWorkflowCategory::MultiApp,
        required_observable_outputs: vec![output(
            &format!("Final output from {} visible in response", final_app),
            &[final_signal],
            None,
            None,
            None,
            true,
        )],
        semantic_success_signals: vec![final_signal.to_string()],
        forbidden_silent_completion_patterns: vec![
            "workflow complete".to_string(),
            "all steps done".to_string(),
            "task is complete".to_string(),
        ],
        required_stage_labels: vec![],
        require_observable_before_success_claim: true,
    }
}

// ─── Registry ─────────────────────────────────────────────────────────────────

/// Return the canonical semantic contract for a workflow category.
pub fn contract_for_category(cat: EvalWorkflowCategory) -> SemanticCompletionContract {
    match cat {
        EvalWorkflowCategory::Coding => coding_contract(),
        EvalWorkflowCategory::Browser => browser_contract(),
        EvalWorkflowCategory::FileManagement => file_management_contract(),
        EvalWorkflowCategory::MultiApp => multi_app_contract("terminal", "output"),
        _ => SemanticCompletionContract {
            success_definition: format!(
                "{} workflow completed and result surfaced to user",
                cat.as_str()
            ),
            category: cat,
            required_observable_outputs: vec![],
            semantic_success_signals: vec!["done".to_string(), "completed".to_string()],
            forbidden_silent_completion_patterns: vec![
                "task completed".to_string(),
                "i've done it".to_string(),
            ],
            required_stage_labels: vec![],
            require_observable_before_success_claim: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_contract_requires_observable() {
        let c = coding_contract();
        assert!(c.require_observable_before_success_claim);
        assert!(!c.required_observable_outputs.is_empty());
        assert!(!c.forbidden_silent_completion_patterns.is_empty());
    }

    #[test]
    fn coding_run_and_show_is_stricter_than_base() {
        let base = coding_contract();
        let show = coding_run_and_show_contract();
        assert!(
            show.forbidden_silent_completion_patterns.len()
                > base.forbidden_silent_completion_patterns.len()
        );
    }

    #[test]
    fn all_categories_have_contracts() {
        for cat in [
            EvalWorkflowCategory::Coding,
            EvalWorkflowCategory::Browser,
            EvalWorkflowCategory::FileManagement,
            EvalWorkflowCategory::MultiApp,
            EvalWorkflowCategory::Terminal,
        ] {
            let c = contract_for_category(cat);
            assert!(
                !c.success_definition.is_empty(),
                "{:?} has empty definition",
                cat
            );
        }
    }

    #[test]
    fn interruption_contract_does_not_require_observable_artifact() {
        let c = interruption_recovery_contract();
        assert!(!c.require_observable_before_success_claim);
    }
}
