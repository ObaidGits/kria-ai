//! Stress + Operational Testing Suite.
//!
//! Tests operational continuity under load and adverse conditions:
//! - Daemon failure and restart loops
//! - Event storm detection and flood protection
//! - Bounded memory growth under extended workflows
//! - Stable PSDG cognition under sustained operation
//!
//! Note: These tests are classified as `LiveOptIn` and require
//! `KRIA_EVAL_LIVE=1` to execute. They are never run in CI.

use crate::workflow_eval::contracts::contract_for_category;
use crate::workflow_eval::types::{
    EvalWorkflowCategory, SafetyClass, SemanticCompletionContract, WorkflowEvalCase,
};
use std::time::Duration;

fn stress_case(
    id: &str,
    description: &str,
    prompt: &str,
    contract: SemanticCompletionContract,
    timeout: Duration,
    tags: &[&str],
) -> WorkflowEvalCase {
    WorkflowEvalCase {
        id: id.to_string(),
        description: description.to_string(),
        prompt: prompt.to_string(),
        category: EvalWorkflowCategory::Unknown,
        contract,
        safety_class: SafetyClass::LiveOptIn,
        interruption: None,
        timeout,
        requires_daemon: true,
        requires_display: true,
        tags: {
            let mut t = tags.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            t.push("stress".into());
            t.push("live-opt-in".into());
            t
        },
        eval_notes: format!(
            "Stress eval — requires KRIA_EVAL_LIVE=1. Case: {}. \
             Validates operational continuity and bounded resource growth.",
            id
        ),
    }
}

pub fn stress_suite() -> Vec<WorkflowEvalCase> {
    vec![
        // ── Daemon resilience ─────────────────────────────────────────────────
        stress_case(
            "wf-stress-001-daemon-restart-loop",
            "Daemon restart: KRIA must auto-restart daemon up to 3 times with backoff",
            "open vscode and write a python program",
            {
                let mut c = contract_for_category(EvalWorkflowCategory::Coding);
                c.success_definition =
                    "Daemon restarted and workflow resumed within retry budget".into();
                c.semantic_success_signals = vec![
                    "restarted".into(),
                    "retry".into(),
                    "attempting".into(),
                    "wrote".into(),
                ];
                c
            },
            Duration::from_secs(180),
            &["daemon", "restart", "resilience", "backoff"],
        ),
        stress_case(
            "wf-stress-002-daemon-exhaustion",
            "All 3 restart attempts exhausted; KRIA must fail closed with diagnostics",
            "type hello world in the editor",
            {
                let mut c = contract_for_category(EvalWorkflowCategory::Unknown);
                c.success_definition =
                    "Daemon exhaustion correctly fails closed with actionable sudoers guidance"
                        .into();
                c.semantic_success_signals =
                    vec!["unavailable".into(), "sudoers".into(), "nopasswd".into()];
                c.forbidden_silent_completion_patterns = vec!["completed".into(), "done".into()];
                c
            },
            Duration::from_secs(300),
            &["daemon", "exhaustion", "fail-closed", "diagnostics"],
        ),
        // ── Event storm ───────────────────────────────────────────────────────
        stress_case(
            "wf-stress-003-event-storm-protection",
            "PSDG must drop non-focus events during storm (>2000/s) without crashing",
            "watch my screen while vs code is building",
            {
                let mut c = contract_for_category(EvalWorkflowCategory::Unknown);
                c.success_definition =
                    "PSDG coordinator survives event storm without memory growth or crash".into();
                c.semantic_success_signals = vec!["watching".into(), "monitoring".into()];
                c
            },
            Duration::from_secs(60),
            &["event-storm", "psdg", "flood-protection"],
        ),
        // ── Extended workflow ─────────────────────────────────────────────────
        stress_case(
            "wf-stress-004-long-workflow-memory",
            "Execute 20 sequential tool calls; validate PSDG memory stays bounded",
            "create 10 python files each with a simple function and run them all",
            {
                let mut c = contract_for_category(EvalWorkflowCategory::Coding);
                c.success_definition =
                    "10 files created and run; PSDG memory bounded; no event amplification".into();
                c
            },
            Duration::from_secs(600),
            &["long-workflow", "bounded-memory", "psdg-growth"],
        ),
        // ── Workflow continuation under load ──────────────────────────────────
        stress_case(
            "wf-stress-005-continuation-loop",
            "Pause and resume 5 times; validate checkpoint consistency across cycles",
            "write a complex python program with multiple functions",
            {
                let mut c = contract_for_category(EvalWorkflowCategory::Coding);
                c.success_definition =
                    "Workflow paused and resumed correctly across 5 cycles with checkpoint integrity".into();
                c.semantic_success_signals =
                    vec!["resumed".into(), "checkpoint".into(), "continuation".into()];
                c
            },
            Duration::from_secs(300),
            &["continuation", "checkpoint", "resume-loop"],
        ),
    ]
}
