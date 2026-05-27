//! Coding Workflow Eval Suite.
//!
//! Validates that KRIA correctly fulfils coding workflow intents:
//! - Opens IDE
//! - Writes syntactically appropriate code
//! - Executes the code
//! - **SURFACES THE OUTPUT** to the user — not just runs it silently
//!
//! FAILS if:
//! - Output is hidden from the user
//! - Execution happened silently
//! - Wrong file extension used
//! - Workflow semantically incomplete
//! - "Done!" claimed with no visible output

use crate::workflow_eval::contracts::{coding_contract, coding_run_and_show_contract};
use crate::workflow_eval::types::{EvalWorkflowCategory, SafetyClass, WorkflowEvalCase};
use std::time::Duration;

fn coding_case(
    id: &str,
    description: &str,
    prompt: &str,
    tags: &[&str],
    run_and_show: bool,
) -> WorkflowEvalCase {
    WorkflowEvalCase {
        id: id.to_string(),
        description: description.to_string(),
        prompt: prompt.to_string(),
        category: EvalWorkflowCategory::Coding,
        contract: if run_and_show {
            coding_run_and_show_contract()
        } else {
            coding_contract()
        },
        safety_class: SafetyClass::Safe,
        interruption: None,
        timeout: Duration::from_secs(120),
        requires_daemon: true,
        requires_display: true,
        tags: {
            let mut t = tags.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            t.push("coding".into());
            t
        },
        eval_notes: format!(
            "Validates end-to-end coding cognition. FAIL if: output hidden, execution silent, \
             wrong extension, workflow incomplete. Case: {}",
            id
        ),
    }
}

pub fn coding_suite() -> Vec<WorkflowEvalCase> {
    vec![
        // ── Core coding workflows ─────────────────────────────────────────────
        coding_case(
            "wf-coding-001-pascal-triangle",
            "Open VS Code, write Pascal triangle in Python, run it, SHOW output",
            "open code and write a python program to print pascal triangle and run it and show me the output",
            &["run-and-show", "python", "vscode"],
            true,
        ),
        coding_case(
            "wf-coding-002-fibonacci-python",
            "Write Python Fibonacci program, run it, surface output",
            "open a text editor, write a python fibonacci program, save it as fibonacci.py and run it",
            &["run-and-show", "python", "fibonacci"],
            true,
        ),
        coding_case(
            "wf-coding-003-hello-world-rust",
            "Create Rust hello-world project, compile it, show result",
            "create a new rust project, write hello world in main.rs and compile it",
            &["rust", "compile", "terminal"],
            false,
        ),
        coding_case(
            "wf-coding-004-javascript-todo",
            "Write JavaScript TODO app, save as todo.js",
            "open vscode and write a simple javascript todo app with add and remove functions",
            &["javascript", "vscode", "todo"],
            false,
        ),
        coding_case(
            "wf-coding-005-fix-compile-error",
            "Edit existing Python file, fix syntax error, rerun",
            "open the python file in vscode, fix the syntax error on line 3 and run it again",
            &["python", "fix", "debug", "rerun"],
            true,
        ),
        coding_case(
            "wf-coding-006-bash-script",
            "Write and execute a bash script that lists files",
            "write a bash script that lists all files in the current directory and run it in terminal",
            &["bash", "terminal", "script"],
            true,
        ),
        // ── Semantic completeness checks ──────────────────────────────────────
        coding_case(
            "wf-coding-007-output-must-be-shown",
            "CRITICAL: KRIA must show the actual printed output, not just claim it ran",
            "write a python program that prints the numbers 1 to 10 and run it, show me the output",
            &["run-and-show", "observable-output", "critical"],
            true,
        ),
        coding_case(
            "wf-coding-008-wrong-extension-fail",
            "Ensure correct .py extension for Python code (not .txt)",
            "write a python hello world program and save it",
            &["file-extension", "python", "artifact"],
            false,
        ),
        coding_case(
            "wf-coding-009-ide-focus-verified",
            "IDE window must be open and focused before writing code",
            "open vscode and write a sorting algorithm in python",
            &["ide-focus", "vscode", "python", "sorting"],
            false,
        ),
        coding_case(
            "wf-coding-010-multi-file-project",
            "Create multiple files for a Python project",
            "create a python project with main.py and utils.py where utils has a helper function",
            &["multi-file", "python", "project"],
            false,
        ),
    ]
}

/// Minimal safe subset that can run without a live daemon.
pub fn coding_suite_safe_subset() -> Vec<WorkflowEvalCase> {
    coding_suite()
        .into_iter()
        .filter(|c| !c.requires_daemon)
        .collect()
}
