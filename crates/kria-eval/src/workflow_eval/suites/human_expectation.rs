//! Human Expectation Alignment Eval Suite.
//!
//! MOST IMPORTANT SUITE.
//!
//! Validates that KRIA fulfils human expectations — not just tool correctness.
//! These evals test the critical gap between "KRIA did something" and
//! "KRIA did what the human needed, visibly."
//!
//! Every test here FAILS if KRIA:
//! - Ran code but didn't SHOW the output
//! - Opened the browser but didn't VISIBLY show the result
//! - Fixed an error but didn't EXPLAIN what changed
//! - Completed a task but provided only a hollow confirmation
//!
//! These test observable success + expectation alignment — the hardest
//! class of eval and the one most important for production cognition.

use crate::workflow_eval::contracts::human_expectation_contract;
use crate::workflow_eval::types::{
    EvalWorkflowCategory, ObservableOutputContract, SafetyClass, WorkflowEvalCase,
};
use std::time::Duration;

fn expectation_case(
    id: &str,
    description: &str,
    prompt: &str,
    visible_signal: &str,
    extra_forbidden: &[&str],
    category: EvalWorkflowCategory,
    tags: &[&str],
) -> WorkflowEvalCase {
    let mut contract = human_expectation_contract(visible_signal);
    contract.category = category;
    contract
        .forbidden_silent_completion_patterns
        .extend(extra_forbidden.iter().map(|s| s.to_string()));

    WorkflowEvalCase {
        id: id.to_string(),
        description: description.to_string(),
        prompt: prompt.to_string(),
        category,
        contract,
        safety_class: SafetyClass::Safe,
        interruption: None,
        timeout: Duration::from_secs(90),
        requires_daemon: true,
        requires_display: true,
        tags: {
            let mut t = tags.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            t.push("human-expectation".into());
            t.push("observable".into());
            t
        },
        eval_notes: format!(
            "Human expectation eval. Case: {}. \
             CRITICAL: FAIL if result is not visibly shown in KRIA's response. \
             Tool execution alone is NOT sufficient.",
            id
        ),
    }
}

pub fn human_expectation_suite() -> Vec<WorkflowEvalCase> {
    vec![
        // ── "Run and SHOW output" ─────────────────────────────────────────────
        expectation_case(
            "wf-he-001-run-and-show-code",
            "CRITICAL: Run code and SHOW the actual output in response",
            "run the hello_world.py file and show me the output",
            "hello",
            &["ran the code", "code has been run", "executed successfully"],
            EvalWorkflowCategory::Coding,
            &["run-show-output", "critical"],
        ),
        expectation_case(
            "wf-he-002-run-and-show-numbers",
            "Print 1-10 and SHOW the numbers — not just claim it printed them",
            "write a python script that prints 1 to 10 and run it and show me what it prints",
            "10",
            &["numbers are printed", "printed 1 to 10", "script ran"],
            EvalWorkflowCategory::Coding,
            &["numbers", "print-show", "critical"],
        ),
        // ── "Open browser and VISIBLY search" ────────────────────────────────
        expectation_case(
            "wf-he-003-browser-visible-search",
            "Open browser and VISIBLY show search results in response",
            "open the browser and search for python tutorials and show me what comes up",
            "python",
            &[
                "browser is open",
                "browser has been opened",
                "i opened chrome",
            ],
            EvalWorkflowCategory::Browser,
            &["browser-visible", "search-results", "critical"],
        ),
        expectation_case(
            "wf-he-004-weather-visible-result",
            "Search weather and SHOW the temperature in the response",
            "search for the weather today and tell me the temperature",
            "°",
            &[
                "checked the weather",
                "weather has been searched",
                "i looked up",
            ],
            EvalWorkflowCategory::Browser,
            &["weather-temperature", "visible-result"],
        ),
        // ── "Fix error and EXPLAIN what changed" ────────────────────────────
        expectation_case(
            "wf-he-005-fix-and-explain",
            "Fix compile error and EXPLAIN the fix — not just say 'fixed'",
            "fix the syntax error in the python file and explain what you changed",
            "changed",
            &["fixed the error", "error has been fixed", "issue resolved"],
            EvalWorkflowCategory::Coding,
            &["fix-explain", "transparency"],
        ),
        // ── Observable completion: file must EXIST ────────────────────────────
        {
            let mut c = human_expectation_contract("created");
            c.category = EvalWorkflowCategory::FileManagement;
            c.required_observable_outputs = vec![ObservableOutputContract {
                description: "report.md exists on disk".to_string(),
                response_must_contain: vec!["created".to_string(), "report".to_string()],
                artifact_path_glob: Some("~/report.md".to_string()),
                artifact_min_bytes: Some(10),
                artifact_content_contains: Some("#".to_string()),
                required: true,
            }];
            c.forbidden_silent_completion_patterns.extend(vec![
                "task complete".to_string(),
                "i've done it".to_string(),
            ]);
            WorkflowEvalCase {
                id: "wf-he-006-artifact-must-exist".to_string(),
                description: "Create report.md — artifact must EXIST and be non-empty".to_string(),
                prompt: "create a markdown file called report.md with a header and a bullet point"
                    .to_string(),
                category: EvalWorkflowCategory::FileManagement,
                contract: c,
                safety_class: SafetyClass::Reversible,
                interruption: None,
                timeout: Duration::from_secs(60),
                requires_daemon: false,
                requires_display: false,
                tags: vec![
                    "artifact-must-exist".into(),
                    "human-expectation".into(),
                    "file".into(),
                ],
                eval_notes: "Validates that file creation is verifiable, not just claimed.".into(),
            }
        },
        // ── No hollow completions ─────────────────────────────────────────────
        expectation_case(
            "wf-he-007-no-hollow-done",
            "Task completed — but hollow 'Done!' response must FAIL",
            "write a python fibonacci function",
            "def fibonacci",
            &[
                "done!",
                "task completed",
                "i've done",
                "completed successfully",
            ],
            EvalWorkflowCategory::Coding,
            &["hollow-completion", "false-success-detection"],
        ),
        expectation_case(
            "wf-he-008-terminal-output-surfaced",
            "Run terminal command and SURFACE the actual terminal output",
            "run the command ls -la in the terminal and show me what it outputs",
            "total",
            &[
                "command has been run",
                "terminal is open",
                "ran the command",
            ],
            EvalWorkflowCategory::Terminal,
            &["terminal-output", "ls", "surface-output"],
        ),
        // ── Expectation alignment: IDE must be focused ────────────────────────
        expectation_case(
            "wf-he-009-ide-focus-confirmed",
            "Open IDE — KRIA must confirm the IDE is actually open, not just claim it",
            "open vs code and tell me when it is open",
            "open",
            &["i've opened code", "vscode has been opened"],
            EvalWorkflowCategory::Coding,
            &["ide-focus", "app-lifecycle"],
        ),
    ]
}
