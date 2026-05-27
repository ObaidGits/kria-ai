//! GoalTree end-to-end eval — exercises the exact production path that
//! Bug 6 and Bug 7 live in:
//!
//!   RuleBasedWorkflowCompiler → GoalTree → StageExecutor → real ToolRegistry
//!
//! These tests are the first evals to actually exercise the StageExecutor path.
//! The existing `gui_eval` uses `execute_workflow` (HTN path) which bypasses
//! StageExecutor entirely and therefore cannot catch Bug 6/7 regressions.
//!
//! # NoDisplay guarantee
//! Every test here uses only `execute_bash` or `write_file` actions — no
//! `open_application`, `type_text`, or `press_shortcut`. Safe for CI.
//!
//! # VM-required tests
//! Tests gated with `KRIA_EVAL_VM=1` test destructive/critical paths:
//! - File deletion, process killing, privileged commands.
//! - They are compiled always but skip at runtime unless the env var is set.

use std::sync::Arc;

use kria_core::agent::execution_verifier::{ExecutionVerifier, Verifiability, VerifyOutcome};
use kria_core::agent::goal_tree::{
    ActionGroup, CompletionContract, GoalTree, RecoveryAction, RecoveryPath, SafeAbortStep,
    StageAction, StageContextHints, VerificationCheckpoint, WorkflowStage, MAX_STAGE_DURATION_SEC,
};
use kria_core::agent::htn_executor::{ToolExecutor, VerificationType};
use kria_core::agent::stage_executor::StageExecutor;
#[allow(unused_imports)]
use kria_core::agent::stage_executor::StageOutcome;
use kria_core::infra::ToolResult;
use kria_core::tools::registry::{build_default_registry, ToolRegistry};
use tokio_util::sync::CancellationToken;

// ============================================================================
// DirectToolExecutor — wraps ToolRegistry with no policy layer.
// Used in eval so bash commands run without HITL / policy blocking.
// ============================================================================

#[allow(dead_code)]
struct DirectToolExecutor {
    registry: Arc<ToolRegistry>,
}

#[async_trait::async_trait]
impl ToolExecutor for DirectToolExecutor {
    async fn execute(&self, action: &str, params: &serde_json::Value) -> ToolResult {
        match self.registry.get_handler(action) {
            Some(handler) => {
                let ctx = self.registry.make_tool_context(CancellationToken::new());
                handler.execute_with_context(params.clone(), ctx).await
            }
            None => ToolResult::err(format!("no handler for tool '{action}'")),
        }
    }
}

// ============================================================================
// AlwaysPassVerifier — NoDisplay verifier for CI tests.
// All checkpoint assertions pass without querying the window manager.
// ============================================================================

#[allow(dead_code)]
struct AlwaysPassVerifier;

#[async_trait::async_trait]
impl ExecutionVerifier for AlwaysPassVerifier {
    async fn verify(&self, _leaf: &Verifiability) -> VerifyOutcome {
        VerifyOutcome {
            verified: true,
            confidence: 1.0,
            confidence_tier:
                kria_core::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
            evidence: "eval: always pass".to_string(),
            latency_ms: 0,
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

#[allow(dead_code)]
fn build_executor() -> StageExecutor {
    let registry = Arc::new(build_default_registry());
    let tool_exec = Arc::new(DirectToolExecutor { registry });
    let verifier: Arc<dyn ExecutionVerifier> = Arc::new(AlwaysPassVerifier);
    StageExecutor::new(tool_exec, verifier)
}

#[allow(dead_code)]
fn single_bash_stage(index: u32, command: &str, is_last: bool, label: &str) -> WorkflowStage {
    WorkflowStage {
        index,
        label: label.to_string(),
        action_group: ActionGroup {
            actions: vec![StageAction {
                action: "execute_bash".to_string(),
                params: serde_json::json!({ "command": command }),
                verify: VerificationType::None,
                timeout_ms: Some(15_000),
            }],
        },
        checkpoint: VerificationCheckpoint::None,
        recovery: if is_last {
            None
        } else {
            Some(RecoveryPath {
                max_attempts: 1,
                recovery_action: RecoveryAction::SkipStage,
            })
        },
        context_hints: StageContextHints::default(),
        timeout_sec: MAX_STAGE_DURATION_SEC,
        skippable: false,
    }
}

#[allow(dead_code)]
fn empty_abort() -> Vec<SafeAbortStep> {
    vec![]
}

#[allow(dead_code)]
fn two_stage_tree(cmd0: &str, cmd1: &str) -> GoalTree {
    GoalTree {
        workflow_id: "eval-test".to_string(),
        description: "integration eval goal tree".to_string(),
        stages: vec![
            single_bash_stage(
                0,
                cmd0,
                false,
                &format!("Stage 0: {}", &cmd0[..cmd0.len().min(20)]),
            ),
            single_bash_stage(
                1,
                cmd1,
                true,
                &format!("Stage 1: {}", &cmd1[..cmd1.len().min(20)]),
            ),
        ],
        completion: CompletionContract::AllStagesPassed,
        global_abort: empty_abort(),
        preconditions: Vec::new(),
        max_total_duration_sec: 60,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Gate helper: skip unless KRIA_EVAL_VM=1 is set.
    fn requires_vm() -> bool {
        std::env::var("KRIA_EVAL_VM").as_deref() == Ok("1")
    }

    // ── Core path: StageExecutor + real execute_bash ──────────────────────────

    #[tokio::test]
    async fn goal_tree_single_bash_succeeds() {
        let executor = build_executor();
        let tree = GoalTree {
            workflow_id: "single-bash".to_string(),
            description: "single execute_bash".to_string(),
            stages: vec![single_bash_stage(0, "echo hello_kria", true, "Echo")],
            completion: CompletionContract::AllStagesPassed,
            global_abort: vec![],
            preconditions: Vec::new(),
            max_total_duration_sec: 30,
        };
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;
        assert!(result.success, "expected success, got: {:?}", result.error);
        assert_eq!(result.stage_results.len(), 1);
        assert!(matches!(
            result.stage_results[0].outcome,
            StageOutcome::Passed
        ));
    }

    #[tokio::test]
    async fn goal_tree_terminal_output_captured() {
        let executor = build_executor();
        let tree = GoalTree {
            workflow_id: "terminal-output".to_string(),
            description: "verify terminal_output field".to_string(),
            stages: vec![single_bash_stage(
                0,
                "echo kria_output_marker",
                true,
                "Echo marker",
            )],
            completion: CompletionContract::AllStagesPassed,
            global_abort: vec![],
            preconditions: Vec::new(),
            max_total_duration_sec: 30,
        };
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;
        assert!(result.success);
        let output = result
            .terminal_output
            .expect("terminal_output must be Some");
        assert!(
            output.contains("kria_output_marker"),
            "expected 'kria_output_marker' in terminal_output, got: {:?}",
            output
        );
    }

    #[tokio::test]
    async fn goal_tree_two_stage_sequential_execution() {
        let executor = build_executor();
        let tree = two_stage_tree("echo stage_zero", "echo stage_one");
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;
        assert!(result.success);
        assert_eq!(result.stage_results.len(), 2);
        assert!(matches!(
            result.stage_results[0].outcome,
            StageOutcome::Passed
        ));
        assert!(matches!(
            result.stage_results[1].outcome,
            StageOutcome::Passed
        ));
        // terminal_output should contain last stage's output
        let output = result.terminal_output.unwrap_or_default();
        assert!(
            output.contains("stage_one"),
            "last stage stdout not captured: {:?}",
            output
        );
    }

    #[tokio::test]
    async fn goal_tree_failed_command_returns_failure() {
        let executor = build_executor();
        // `false` is a POSIX command that always exits with code 1.
        let tree = GoalTree {
            workflow_id: "fail-bash".to_string(),
            description: "execute_bash with failing command".to_string(),
            stages: vec![single_bash_stage(0, "false", true, "Always fail")],
            completion: CompletionContract::AllStagesPassed,
            global_abort: vec![],
            preconditions: Vec::new(),
            max_total_duration_sec: 30,
        };
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;
        assert!(
            !result.success,
            "expected failure for 'false' command but got success"
        );
        assert!(matches!(
            result.stage_results[0].outcome,
            StageOutcome::Failed { .. }
        ));
    }

    #[tokio::test]
    async fn goal_tree_cancellation_mid_execution() {
        let executor = build_executor();
        let tree = two_stage_tree("sleep 0.05", "echo after_sleep");
        let cancel = CancellationToken::new();
        // Cancel immediately after creating the token — stage 0 may still run
        // (sleep 50ms) before cancellation is processed. The important assertion
        // is that the workflow reports cancelled == true.
        cancel.cancel();
        let result = executor.execute_goal_tree(&tree, cancel).await;
        assert!(result.cancelled, "expected cancelled=true");
        assert!(!result.success);
    }

    // ── Bug 6: WindowFocused + skippable → Skipped (not abort) ───────────────

    #[tokio::test]
    async fn bug6_window_focused_skippable_skips_not_aborts() {
        use kria_core::agent::execution_verifier::{
            Verifiability, VerificationConfidenceTier, VerifyOutcome,
        };

        // Verifier that always fails WindowFocused checks
        struct WindowFocusedFailVerifier;
        #[async_trait::async_trait]
        impl ExecutionVerifier for WindowFocusedFailVerifier {
            async fn verify(&self, _: &Verifiability) -> VerifyOutcome {
                VerifyOutcome {
                    verified: false,
                    confidence: 0.0,
                    confidence_tier: VerificationConfidenceTier::Unobservable,
                    evidence: "focus check failed (Wayland sim)".to_string(),
                    latency_ms: 0,
                }
            }
        }

        let registry = Arc::new(build_default_registry());
        let tool_exec = Arc::new(DirectToolExecutor { registry });
        let verifier: Arc<dyn ExecutionVerifier> = Arc::new(WindowFocusedFailVerifier);
        let executor = StageExecutor::new(tool_exec, verifier);

        // Stage 0: skippable Open-equivalent with WindowFocused checkpoint
        let open_stage = WorkflowStage {
            index: 0,
            label: "Open app (skippable)".to_string(),
            action_group: ActionGroup {
                actions: vec![StageAction {
                    action: "execute_bash".to_string(),
                    params: serde_json::json!({ "command": "true" }),
                    verify: VerificationType::None,
                    timeout_ms: Some(5_000),
                }],
            },
            checkpoint: VerificationCheckpoint::WindowFocused {
                title_contains: Some("nonexistent-window".to_string()),
                class: None,
                pid: None,
            },
            recovery: Some(RecoveryPath {
                max_attempts: 2,
                recovery_action: RecoveryAction::RetryFromAction {
                    restart_from_index: 0,
                },
            }),
            context_hints: StageContextHints::default(),
            timeout_sec: MAX_STAGE_DURATION_SEC,
            skippable: true, // Bug 6: must skip, not abort
        };

        // Stage 1: subsequent bash stage that should still run
        let run_stage = single_bash_stage(1, "echo downstream_ran", true, "Run downstream");

        let tree = GoalTree {
            workflow_id: "bug6-skip-test".to_string(),
            description: "Bug 6 regression: skippable WindowFocused exhaustion".to_string(),
            stages: vec![open_stage, run_stage],
            completion: CompletionContract::AllStagesPassed,
            global_abort: vec![],
            preconditions: Vec::new(),
            max_total_duration_sec: 60,
        };

        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;

        // Stage 0 should be Skipped (not Failed/Aborted)
        assert!(
            matches!(result.stage_results[0].outcome, StageOutcome::Skipped),
            "expected Skipped for exhausted WindowFocused skippable stage, got {:?}",
            result.stage_results[0].outcome
        );

        // Stage 1 should still execute and pass
        assert_eq!(result.stage_results.len(), 2, "stage 1 must have executed");
        assert!(
            matches!(result.stage_results[1].outcome, StageOutcome::Passed),
            "expected stage 1 to pass after stage 0 skip, got {:?}",
            result.stage_results[1].outcome
        );

        // The workflow should succeed overall (one skip + one pass)
        assert!(
            result.success,
            "workflow should succeed after skip: {:?}",
            result.error
        );
    }

    // ── Bug 7: execute_bash output surfaces in terminal_output ───────────────

    #[tokio::test]
    async fn bug7_execute_bash_stdout_in_terminal_output() {
        let executor = build_executor();
        let expected = "kria_bug7_marker_42";
        let tree = GoalTree {
            workflow_id: "bug7-output-test".to_string(),
            description: "Bug 7 regression: terminal_output populated from execute_bash"
                .to_string(),
            stages: vec![single_bash_stage(
                0,
                &format!("echo {}", expected),
                true,
                "Echo marker",
            )],
            completion: CompletionContract::AllStagesPassed,
            global_abort: vec![],
            preconditions: Vec::new(),
            max_total_duration_sec: 30,
        };
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;
        assert!(result.success);
        let output = result.terminal_output.unwrap_or_default();
        assert!(
            output.contains(expected),
            "Bug 7 regression: '{}' not found in terminal_output. Got: {:?}",
            expected,
            output
        );
    }

    // ── Destructive / VM-only evals ───────────────────────────────────────────

    #[tokio::test]
    async fn vm_only_write_and_delete_file() {
        if !requires_vm() {
            eprintln!("[SKIP] vm_only_write_and_delete_file: set KRIA_EVAL_VM=1 to run on VM");
            return;
        }
        let executor = build_executor();
        let tree = two_stage_tree(
            "echo vm_test_content > /tmp/kria_eval_vm_test.txt",
            "rm /tmp/kria_eval_vm_test.txt && echo deleted_ok",
        );
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;
        assert!(result.success);
        let output = result.terminal_output.unwrap_or_default();
        assert!(
            output.contains("deleted_ok"),
            "expected 'deleted_ok', got: {:?}",
            output
        );
        assert!(
            !std::path::Path::new("/tmp/kria_eval_vm_test.txt").exists(),
            "file should have been deleted"
        );
    }

    #[tokio::test]
    async fn vm_only_python_program_execution() {
        if !requires_vm() {
            eprintln!("[SKIP] vm_only_python_program_execution: set KRIA_EVAL_VM=1 to run on VM");
            return;
        }
        let executor = build_executor();
        // Write + run a Python program via two bash stages
        let write_cmd = r#"python3 -c "print('fibonacci: 0 1 1 2 3 5 8 13')""#;
        let tree = GoalTree {
            workflow_id: "vm-python-exec".to_string(),
            description: "VM: write python, execute, verify output".to_string(),
            stages: vec![single_bash_stage(0, write_cmd, true, "Python exec")],
            completion: CompletionContract::AllStagesPassed,
            global_abort: vec![],
            preconditions: Vec::new(),
            max_total_duration_sec: 30,
        };
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;
        assert!(result.success);
        let output = result.terminal_output.unwrap_or_default();
        assert!(
            output.contains("fibonacci"),
            "expected fibonacci in output, got: {:?}",
            output
        );
    }

    #[tokio::test]
    async fn vm_only_regression_003_goal_tree_path() {
        if !requires_vm() {
            eprintln!("[SKIP] vm_only_regression_003_goal_tree_path: set KRIA_EVAL_VM=1");
            return;
        }
        // Regression 003 via GoalTree path (not HTN).
        // Simulates: compile "Open code and write pascal triangle, run it"
        // → the GoalTree path with execute_bash should capture pascal output.
        let executor = build_executor();
        let pascal_cmd = concat!(
            r#"python3 -c ""#,
            r#"def p(n):
"#,
            r#"  r=[1]
"#,
            r#"  for _ in range(n):
"#,
            r#"    print(r)
"#,
            r#"    r=[1]+[r[i]+r[i+1] for i in range(len(r)-1)]+[1]
"#,
            r#"p(5)""#
        );
        let tree = GoalTree {
            workflow_id: "vm-regression-003".to_string(),
            description: "VM: regression-003 via GoalTree/StageExecutor path".to_string(),
            stages: vec![single_bash_stage(0, pascal_cmd, true, "Pascal triangle")],
            completion: CompletionContract::AllStagesPassed,
            global_abort: vec![],
            preconditions: Vec::new(),
            max_total_duration_sec: 30,
        };
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;
        assert!(result.success);
        let output = result.terminal_output.unwrap_or_default();
        assert!(
            output.contains("[1, 1]"),
            "expected pascal triangle rows in output, got: {:?}",
            output
        );
    }
}
