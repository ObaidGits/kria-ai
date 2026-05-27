//! Long-Horizon Multi-Application Workflow Eval Suite.
//!
//! Tests workflows that span multiple applications, involve delayed continuation,
//! and require PSDG context to persist across app transitions.
//!
//! Examples:
//! - Browser → IDE → Terminal (research → code → run)
//! - Email → Editor → Terminal (read requirements → write code → deploy)
//! - Browser → File Manager → Editor (download → organize → edit)
//!
//! Validates:
//! - Operational continuity across app transitions
//! - Workflow identity preservation (same session ID throughout)
//! - Checkpoint recovery after mid-workflow interruption
//! - PSDG context correct at each stage (right app, right file)
//! - Ambient continuation suggestions at appropriate moments

use crate::workflow_eval::contracts::multi_app_contract;
use crate::workflow_eval::types::{
    EvalWorkflowCategory, ObservableOutputContract, SafetyClass, SemanticCompletionContract,
    WorkflowEvalCase,
};
use std::time::Duration;

fn multi_app_case(
    id: &str,
    description: &str,
    prompt: &str,
    contract: SemanticCompletionContract,
    tags: &[&str],
) -> WorkflowEvalCase {
    WorkflowEvalCase {
        id: id.to_string(),
        description: description.to_string(),
        prompt: prompt.to_string(),
        category: EvalWorkflowCategory::MultiApp,
        contract,
        safety_class: SafetyClass::Reversible,
        interruption: None,
        timeout: Duration::from_secs(240),
        requires_daemon: true,
        requires_display: true,
        tags: {
            let mut t = tags.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            t.push("long-horizon".into());
            t.push("multi-app".into());
            t
        },
        eval_notes: format!(
            "Long-horizon multi-app eval. Case: {}. \
             Validates operational continuity, PSDG context persistence, \
             and workflow identity across app boundaries.",
            id
        ),
    }
}

pub fn long_horizon_suite() -> Vec<WorkflowEvalCase> {
    vec![
        // ── Browser → IDE → Terminal ──────────────────────────────────────────
        multi_app_case(
            "wf-lh-001-research-code-run",
            "Research Python sorting algorithms online, implement the best one, run it",
            "search for the most efficient python sorting algorithm, implement it in vscode, and run it to verify it works",
            {
                let mut c = multi_app_contract("terminal", "output");
                c.required_observable_outputs = vec![
                    ObservableOutputContract {
                        description: "Code file written to disk".into(),
                        response_must_contain: vec!["wrote".into(), "created".into()],
                        artifact_path_glob: Some("~/.kria/generated/*.py".into()),
                        artifact_min_bytes: Some(50),
                        artifact_content_contains: Some("def ".into()),
                        required: true,
                    },
                    ObservableOutputContract {
                        description: "Execution output shown in response".into(),
                        response_must_contain: vec!["output".into(), "result".into(), "sorted".into()],
                        artifact_path_glob: None,
                        artifact_min_bytes: None,
                        artifact_content_contains: None,
                        required: true,
                    },
                ];
                c
            },
            &["browser-ide-terminal", "research-implement-run"],
        ),
        // ── IDE → Terminal → Browser ──────────────────────────────────────────
        multi_app_case(
            "wf-lh-002-write-test-document",
            "Write a Python function, run its unit tests, then search for documentation",
            "write a python function that reverses a string, write a unit test for it, run the tests, and then search for the python unittest documentation",
            {
                let mut c = multi_app_contract("browser", "unittest");
                c.semantic_success_signals = vec![
                    "test".into(), "passed".into(), "unittest".into(),
                ];
                c
            },
            &["ide-terminal-browser", "test-doc"],
        ),
        // ── File Manager → IDE → Terminal ─────────────────────────────────────
        multi_app_case(
            "wf-lh-003-organize-code-run",
            "Create project structure, write code in it, run from terminal",
            "create a src folder with a main.py file, write a calculator class in it, then run it from the terminal",
            multi_app_contract("terminal", "calculator"),
            &["file-manager-ide-terminal", "project-structure"],
        ),
        // ── Delayed continuation ──────────────────────────────────────────────
        multi_app_case(
            "wf-lh-004-delayed-continuation",
            "Start a workflow, user pauses, KRIA resumes after re-prompt",
            "start writing a python web scraper in vscode — I'll continue later",
            {
                let mut c = multi_app_contract("editor", "scraper");
                c.success_definition =
                    "Workflow started, checkpoint saved, continuation context preserved".into();
                c.semantic_success_signals = vec![
                    "started".into(), "saved".into(), "checkpoint".into(), "continue".into(),
                ];
                c.forbidden_silent_completion_patterns = vec![
                    "completed".into(), "done".into(), "finished".into(),
                ];
                c
            },
            &["delayed-continuation", "checkpoint", "ambient-suggestion"],
        ),
        // ── Cross-app context preservation ────────────────────────────────────
        multi_app_case(
            "wf-lh-005-context-preservation",
            "PSDG context must follow the workflow across app switches",
            "open vscode and write a flask app, then switch to terminal and install flask, then run the app",
            {
                let mut c = multi_app_contract("terminal", "running");
                c.semantic_success_signals = vec![
                    "flask".into(), "running".into(), "app".into(), "http".into(),
                ];
                c
            },
            &["context-preservation", "psdg", "flask", "multi-step"],
        ),
        // ── Ambient continuation suggestion ───────────────────────────────────
        multi_app_case(
            "wf-lh-006-ambient-suggestion",
            "After completing a coding task, KRIA should suggest the next logical step",
            "write a python data analysis script and run it",
            {
                let mut c = multi_app_contract("editor", "analysis");
                c.semantic_success_signals = vec![
                    "data".into(), "analysis".into(), "output".into(),
                ];
                c
            },
            &["ambient-suggestion", "next-step", "proactive"],
        ),
    ]
}
