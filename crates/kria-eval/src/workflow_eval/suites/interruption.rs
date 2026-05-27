//! Interruption + Recovery Eval Suite.
//!
//! Simulates real-world interruptions and validates that KRIA:
//! 1. Detects the interruption correctly
//! 2. Applies the correct recovery strategy
//! 3. Resumes or fails gracefully with informative messaging
//! 4. Does NOT silently continue past an unhandled interruption
//!
//! Interruptions tested:
//! - Modal popup (password dialog mid-workflow)
//! - uinput daemon crash mid-workflow
//! - Window focus theft by another application
//! - User-initiated workflow pause
//! - Browser reload during scraping
//! - IDE freeze during code writing

use crate::workflow_eval::contracts::interruption_recovery_contract;
use crate::workflow_eval::types::{
    EvalWorkflowCategory, ExpectedRecovery, InterruptionKind, InterruptionScenario, SafetyClass,
    SemanticCompletionContract, WorkflowEvalCase,
};
use std::time::Duration;

fn interruption_case(
    id: &str,
    description: &str,
    prompt: &str,
    interruption: InterruptionScenario,
    contract_override: Option<SemanticCompletionContract>,
    tags: &[&str],
) -> WorkflowEvalCase {
    WorkflowEvalCase {
        id: id.to_string(),
        description: description.to_string(),
        prompt: prompt.to_string(),
        category: EvalWorkflowCategory::Unknown,
        contract: contract_override.unwrap_or_else(interruption_recovery_contract),
        safety_class: SafetyClass::Safe,
        interruption: Some(interruption),
        timeout: Duration::from_secs(60),
        requires_daemon: true,
        requires_display: true,
        tags: {
            let mut t = tags.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            t.push("interruption".into());
            t.push("recovery".into());
            t
        },
        eval_notes: format!(
            "Interruption recovery eval. Case: {}. \
             FAIL if: KRIA continues past interruption silently, no recovery message given, \
             or wrong recovery strategy used.",
            id
        ),
    }
}

pub fn interruption_suite() -> Vec<WorkflowEvalCase> {
    vec![
        // ── Popup interruptions ───────────────────────────────────────────────
        interruption_case(
            "wf-int-001-password-popup",
            "Modal password popup appears mid-workflow; KRIA must pause and ask user",
            "open vscode and run the tests",
            InterruptionScenario {
                kind: InterruptionKind::ModalPopup {
                    description: "sudo password dialog appeared".into(),
                },
                inject_after_stage: 0,
                expected_recovery: ExpectedRecovery::PauseAndAsk,
            },
            None,
            &["popup", "auth", "pause"],
        ),
        interruption_case(
            "wf-int-002-update-dialog",
            "Software update dialog appears; KRIA must handle gracefully",
            "open the application",
            InterruptionScenario {
                kind: InterruptionKind::ModalPopup {
                    description: "Update available dialog".into(),
                },
                inject_after_stage: 0,
                expected_recovery: ExpectedRecovery::FailGracefully,
            },
            None,
            &["popup", "update-dialog"],
        ),
        // ── Daemon crash ──────────────────────────────────────────────────────
        interruption_case(
            "wf-int-003-daemon-crash-recovery",
            "uinput daemon crashes mid-coding workflow; KRIA must restart and resume",
            "open vscode and write a hello world program",
            InterruptionScenario {
                kind: InterruptionKind::DaemonCrash,
                inject_after_stage: 0,
                expected_recovery: ExpectedRecovery::AutoRetry { max_attempts: 3 },
            },
            {
                let mut c = interruption_recovery_contract();
                c.semantic_success_signals = vec![
                    "restarting".into(),
                    "attempting".into(),
                    "retry".into(),
                    "daemon".into(),
                ];
                Some(c)
            },
            &["daemon-crash", "auto-restart", "resilience"],
        ),
        interruption_case(
            "wf-int-004-daemon-crash-fail-closed",
            "Daemon crashes and cannot restart; KRIA must fail closed with clear message",
            "type the following text in the editor: hello world",
            InterruptionScenario {
                kind: InterruptionKind::DaemonCrash,
                inject_after_stage: 0,
                expected_recovery: ExpectedRecovery::FailGracefully,
            },
            {
                let mut c = interruption_recovery_contract();
                c.semantic_success_signals = vec![
                    "unavailable".into(),
                    "daemon".into(),
                    "service".into(),
                    "sudoers".into(),
                ];
                Some(c)
            },
            &["daemon-crash", "fail-closed", "diagnostics"],
        ),
        // ── Window focus theft ────────────────────────────────────────────────
        interruption_case(
            "wf-int-005-focus-theft",
            "Another app steals focus during typing; KRIA must detect and recover",
            "open vscode and write some code",
            InterruptionScenario {
                kind: InterruptionKind::WindowFocusTheft {
                    stealer_app: "gnome-calendar".into(),
                },
                inject_after_stage: 0,
                expected_recovery: ExpectedRecovery::AutoRetry { max_attempts: 2 },
            },
            None,
            &["focus-theft", "window-management", "auto-retry"],
        ),
        // ── User pause ────────────────────────────────────────────────────────
        interruption_case(
            "wf-int-006-user-pause-and-resume",
            "User pauses workflow mid-execution; KRIA must save checkpoint and resume",
            "open vscode, write a sorting algorithm, and run it",
            InterruptionScenario {
                kind: InterruptionKind::UserPause,
                inject_after_stage: 1,
                expected_recovery: ExpectedRecovery::ResumeFromCheckpoint,
            },
            {
                let mut c = interruption_recovery_contract();
                c.semantic_success_signals =
                    vec!["paused".into(), "checkpoint".into(), "resume".into()];
                Some(c)
            },
            &["user-pause", "checkpoint", "resume", "continuation"],
        ),
        // ── Browser reload ────────────────────────────────────────────────────
        interruption_case(
            "wf-int-007-browser-reload",
            "Browser reloads during content extraction; KRIA must detect and retry",
            "open the browser and search for python documentation",
            InterruptionScenario {
                kind: InterruptionKind::NetworkTimeout,
                inject_after_stage: 0,
                expected_recovery: ExpectedRecovery::AutoRetry { max_attempts: 2 },
            },
            None,
            &["browser", "network", "reload", "retry"],
        ),
    ]
}
