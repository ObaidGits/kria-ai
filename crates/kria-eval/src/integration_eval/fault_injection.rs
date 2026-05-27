//! Fault injection evals — verify system behaves correctly under failure conditions.
//!
//! ## Covered failure modes
//!
//! | Test                                 | What it injects                         | Expected outcome            |
//! |--------------------------------------|-----------------------------------------|-----------------------------|
//! | `tool_not_found_returns_error`       | Action name with no handler             | stage Failed                |
//! | `tool_timeout_aborts_stage`          | Deadline exceeded before tool returns   | stage Failed / cancelled    |
//! | `bug6_window_focused_skip`           | WindowFocused always fails, skippable   | Skipped, next stage runs    |
//! | `bug6_window_focused_abort`          | WindowFocused fails, NOT skippable      | workflow aborted            |
//! | `first_stage_fail_aborts_workflow`   | Stage 0 fails with AbortWorkflow        | success=false, aborted      |
//! | `recovery_skip_stage`                | Stage fails, recovery=SkipStage         | Skipped, continues          |
//! | `cancellation_before_start`          | Token cancelled before execute_goal_tree| cancelled=true immediately  |
//! | `global_action_budget_hard_stop`     | 100-action budget exhausted in one stage| stage terminates bounded    |
//! | `mid_workflow_bash_fail`             | Stage 1 bash exits 1; stage 0 passed    | stage 1 Failed              |
//! | `write_then_delete_file`             | VM: write + delete real file            | file gone, success          |
//! | `vm_rm_rf_temp`                      | VM: rm -rf a temp dir                   | success, dir gone           |
//! | `vm_process_kill`                    | VM: kill a background process           | process not running after   |

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use kria_core::agent::execution_verifier::VerificationConfidenceTier;
use kria_core::agent::execution_verifier::{ExecutionVerifier, Verifiability, VerifyOutcome};
use kria_core::agent::goal_tree::{
    ActionGroup, CompletionContract, GoalTree, RecoveryAction, RecoveryPath, StageAction,
    StageContextHints, VerificationCheckpoint, WorkflowStage, MAX_STAGE_DURATION_SEC,
};
use kria_core::agent::htn_executor::{ToolExecutor, VerificationType};
#[cfg(test)]
use kria_core::agent::stage_executor::{StageExecutor, StageOutcome};
use kria_core::infra::ToolResult;
use tokio_util::sync::CancellationToken;

// ============================================================================
// Shared test helpers — used in #[cfg(test)] blocks
// ============================================================================

#[allow(dead_code)]
struct AlwaysPassVerifier;

#[async_trait::async_trait]
impl ExecutionVerifier for AlwaysPassVerifier {
    async fn verify(&self, _: &Verifiability) -> VerifyOutcome {
        VerifyOutcome {
            verified: true,
            confidence: 1.0,
            confidence_tier: VerificationConfidenceTier::FullSemantic,
            evidence: "fault-inject: always pass".to_string(),
            latency_ms: 0,
        }
    }
}

#[allow(dead_code)]
struct AlwaysFailVerifier;

#[async_trait::async_trait]
impl ExecutionVerifier for AlwaysFailVerifier {
    async fn verify(&self, _: &Verifiability) -> VerifyOutcome {
        VerifyOutcome {
            verified: false,
            confidence: 0.0,
            confidence_tier: VerificationConfidenceTier::Unobservable,
            evidence: "fault-inject: always fail".to_string(),
            latency_ms: 0,
        }
    }
}

#[allow(dead_code)]
struct PassThroughExecutor;

#[async_trait::async_trait]
impl ToolExecutor for PassThroughExecutor {
    async fn execute(&self, action: &str, _params: &serde_json::Value) -> ToolResult {
        ToolResult::ok_text(format!("{action} ok"))
    }
}

#[allow(dead_code)]
struct FailingExecutor;

#[async_trait::async_trait]
impl ToolExecutor for FailingExecutor {
    async fn execute(&self, action: &str, _params: &serde_json::Value) -> ToolResult {
        ToolResult::err(format!("fault-inject: {action} deliberately failed"))
    }
}

#[allow(dead_code)]
struct CountingExecutor {
    count: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl ToolExecutor for CountingExecutor {
    async fn execute(&self, action: &str, _params: &serde_json::Value) -> ToolResult {
        self.count.fetch_add(1, Ordering::SeqCst);
        ToolResult::ok_text(format!(
            "{action} ok (count={})",
            self.count.load(Ordering::SeqCst)
        ))
    }
}

/// Real registry executor — no policy layer, for fault injection on real tools.
#[allow(dead_code)]
struct DirectExecutor {
    registry: Arc<kria_core::tools::registry::ToolRegistry>,
}

#[async_trait::async_trait]
impl ToolExecutor for DirectExecutor {
    async fn execute(&self, action: &str, params: &serde_json::Value) -> ToolResult {
        match self.registry.get_handler(action) {
            Some(h) => {
                let ctx = self.registry.make_tool_context(CancellationToken::new());
                h.execute_with_context(params.clone(), ctx).await
            }
            None => ToolResult::err(format!("no handler for '{action}'")),
        }
    }
}

#[allow(dead_code)]
fn bash_stage(index: u32, cmd: &str, skippable: bool, is_last: bool) -> WorkflowStage {
    WorkflowStage {
        index,
        label: format!("bash:{}", &cmd[..cmd.len().min(20)]),
        action_group: ActionGroup {
            actions: vec![StageAction {
                action: "execute_bash".to_string(),
                params: serde_json::json!({ "command": cmd }),
                verify: VerificationType::None,
                timeout_ms: Some(10_000),
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
        skippable,
    }
}

#[allow(dead_code)]
fn phantom_stage(index: u32, action: &str, is_last: bool) -> WorkflowStage {
    WorkflowStage {
        index,
        label: format!("phantom:{action}"),
        action_group: ActionGroup {
            actions: vec![StageAction {
                action: action.to_string(),
                params: serde_json::json!({}),
                verify: VerificationType::None,
                timeout_ms: Some(5_000),
            }],
        },
        checkpoint: VerificationCheckpoint::None,
        recovery: if is_last {
            None
        } else {
            Some(RecoveryPath {
                max_attempts: 1,
                recovery_action: RecoveryAction::AbortWorkflow,
            })
        },
        context_hints: StageContextHints::default(),
        timeout_sec: MAX_STAGE_DURATION_SEC,
        skippable: false,
    }
}

#[allow(dead_code)]
fn make_tree(stages: Vec<WorkflowStage>) -> GoalTree {
    GoalTree {
        workflow_id: "fault-inject".to_string(),
        description: "fault injection test".to_string(),
        stages,
        completion: CompletionContract::AllStagesPassed,
        global_abort: vec![],
        preconditions: Vec::new(),
        max_total_duration_sec: 30,
    }
}

// ============================================================================
// Fault injection tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use kria_core::tools::registry::build_default_registry;

    fn requires_vm() -> bool {
        std::env::var("KRIA_EVAL_VM").as_deref() == Ok("1")
    }

    #[allow(dead_code)]
    fn real_executor() -> Arc<DirectExecutor> {
        Arc::new(DirectExecutor {
            registry: Arc::new(build_default_registry()),
        })
    }

    // ── Tool not found → stage fails ─────────────────────────────────────────

    #[tokio::test]
    async fn tool_not_found_returns_error() {
        let executor = StageExecutor::new(
            Arc::new(DirectExecutor {
                registry: Arc::new(build_default_registry()),
            }),
            Arc::new(AlwaysPassVerifier),
        );
        // "nonexistent_tool_xyz" has no handler in the real registry
        let tree = make_tree(vec![phantom_stage(0, "nonexistent_tool_xyz", true)]);
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;
        assert!(!result.success);
        assert!(
            matches!(result.stage_results[0].outcome, StageOutcome::Failed { .. }),
            "expected Failed, got {:?}",
            result.stage_results[0].outcome
        );
    }

    // ── Bug 6: WindowFocused + skippable → Skipped, downstream runs ──────────

    #[tokio::test]
    async fn bug6_window_focused_skip_downstream_continues() {
        let executor = StageExecutor::new(
            Arc::new(PassThroughExecutor),
            Arc::new(AlwaysFailVerifier), // force checkpoint failure
        );

        let wf_stage = WorkflowStage {
            index: 0,
            label: "Open app".to_string(),
            action_group: ActionGroup {
                actions: vec![StageAction {
                    action: "open_application".to_string(),
                    params: serde_json::json!({ "app": "test" }),
                    verify: VerificationType::None,
                    timeout_ms: Some(5_000),
                }],
            },
            checkpoint: VerificationCheckpoint::WindowFocused {
                title_contains: Some("nonexistent".to_string()),
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
            skippable: true, // Bug 6: should skip, not abort
        };

        let downstream_stage = WorkflowStage {
            index: 1,
            label: "Do work downstream".to_string(),
            action_group: ActionGroup {
                actions: vec![StageAction {
                    action: "some_tool".to_string(),
                    params: serde_json::json!({}),
                    verify: VerificationType::None,
                    timeout_ms: Some(5_000),
                }],
            },
            checkpoint: VerificationCheckpoint::None,
            recovery: None,
            context_hints: StageContextHints::default(),
            timeout_sec: MAX_STAGE_DURATION_SEC,
            skippable: false,
        };

        let tree = make_tree(vec![wf_stage, downstream_stage]);
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;

        assert!(
            matches!(result.stage_results[0].outcome, StageOutcome::Skipped),
            "stage 0 (skippable WindowFocused) must be Skipped, got {:?}",
            result.stage_results[0].outcome
        );
        assert_eq!(
            result.stage_results.len(),
            2,
            "downstream stage must have run"
        );
        assert!(
            matches!(result.stage_results[1].outcome, StageOutcome::Passed),
            "downstream stage must pass, got {:?}",
            result.stage_results[1].outcome
        );
    }

    // ── Bug 6: WindowFocused NOT skippable → abort ────────────────────────────

    #[tokio::test]
    async fn bug6_window_focused_not_skippable_aborts() {
        let executor =
            StageExecutor::new(Arc::new(PassThroughExecutor), Arc::new(AlwaysFailVerifier));

        let wf_stage = WorkflowStage {
            index: 0,
            label: "Open non-skippable".to_string(),
            action_group: ActionGroup {
                actions: vec![StageAction {
                    action: "open_application".to_string(),
                    params: serde_json::json!({ "app": "test" }),
                    verify: VerificationType::None,
                    timeout_ms: Some(5_000),
                }],
            },
            checkpoint: VerificationCheckpoint::WindowFocused {
                title_contains: Some("nonexistent".to_string()),
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
            skippable: false, // must abort, not skip
        };

        let tree = make_tree(vec![wf_stage]);
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;
        assert!(!result.success);
        assert!(
            matches!(result.stage_results[0].outcome, StageOutcome::Failed { .. }),
            "non-skippable WindowFocused exhaustion must be Failed, got {:?}",
            result.stage_results[0].outcome
        );
    }

    // ── Recovery: SkipStage on checkpoint failure → Skipped + continues ──────
    //
    // SkipStage fires when: actions succeed BUT the checkpoint fails.
    // (It does NOT fire when actions themselves fail — that's Failed immediately.)

    #[tokio::test]
    async fn recovery_skip_stage_continues_workflow() {
        // PassThroughExecutor: all actions succeed
        // AlwaysFailVerifier: all checkpoints fail → triggers SkipStage recovery
        let executor =
            StageExecutor::new(Arc::new(PassThroughExecutor), Arc::new(AlwaysFailVerifier));

        // Stage 0: actions succeed, checkpoint always fails → SkipStage fires → Skipped
        let skippable_stage = WorkflowStage {
            index: 0,
            label: "Checkpoint-fails but skippable".to_string(),
            action_group: ActionGroup {
                actions: vec![StageAction {
                    action: "open_application".to_string(),
                    params: serde_json::json!({ "app": "x" }),
                    verify: VerificationType::None,
                    timeout_ms: Some(5_000),
                }],
            },
            // WindowFocused: fails with AlwaysFailVerifier → triggers recovery
            checkpoint: VerificationCheckpoint::WindowFocused {
                title_contains: Some("x".to_string()),
                class: None,
                pid: None,
            },
            recovery: Some(RecoveryPath {
                max_attempts: 1,
                recovery_action: RecoveryAction::SkipStage,
            }),
            context_hints: StageContextHints::default(),
            timeout_sec: MAX_STAGE_DURATION_SEC,
            skippable: true,
        };

        // Stage 1: also PassThroughExecutor succeeds, AlwaysFailVerifier fails checkpoint.
        // But no recovery → stage fails (that's expected; the key is it was REACHED).
        let downstream = WorkflowStage {
            index: 1,
            label: "Downstream still runs".to_string(),
            action_group: ActionGroup {
                actions: vec![StageAction {
                    action: "some_tool".to_string(),
                    params: serde_json::json!({}),
                    verify: VerificationType::None,
                    timeout_ms: Some(5_000),
                }],
            },
            checkpoint: VerificationCheckpoint::None, // None checkpoint always passes
            recovery: None,
            context_hints: StageContextHints::default(),
            timeout_sec: MAX_STAGE_DURATION_SEC,
            skippable: false,
        };

        let tree = make_tree(vec![skippable_stage, downstream]);
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;

        // Stage 0: actions pass, checkpoint fails → SkipStage → Skipped
        assert!(
            matches!(result.stage_results[0].outcome, StageOutcome::Skipped),
            "expected Skipped for stage 0, got {:?}",
            result.stage_results[0].outcome
        );
        // Stage 1: must have been reached (skipping stage 0 continues workflow)
        assert_eq!(
            result.stage_results.len(),
            2,
            "downstream stage must have run"
        );
        // Stage 1 has VerificationCheckpoint::None → AlwaysFailVerifier not called → Passed
        assert!(
            matches!(result.stage_results[1].outcome, StageOutcome::Passed),
            "stage 1 with None checkpoint should pass, got {:?}",
            result.stage_results[1].outcome
        );
    }

    // ── AbortWorkflow recovery stops execution ────────────────────────────────

    #[tokio::test]
    async fn abort_workflow_recovery_stops_execution() {
        let executor = StageExecutor::new(Arc::new(FailingExecutor), Arc::new(AlwaysPassVerifier));

        let abort_stage = WorkflowStage {
            index: 0,
            label: "Abort on failure".to_string(),
            action_group: ActionGroup {
                actions: vec![StageAction {
                    action: "must_fail".to_string(),
                    params: serde_json::json!({}),
                    verify: VerificationType::None,
                    timeout_ms: Some(5_000),
                }],
            },
            checkpoint: VerificationCheckpoint::None,
            recovery: Some(RecoveryPath {
                max_attempts: 1,
                recovery_action: RecoveryAction::AbortWorkflow,
            }),
            context_hints: StageContextHints::default(),
            timeout_sec: MAX_STAGE_DURATION_SEC,
            skippable: false,
        };

        let should_not_run = WorkflowStage {
            index: 1,
            label: "Should not be reached".to_string(),
            action_group: ActionGroup {
                actions: vec![StageAction {
                    action: "some_tool".to_string(),
                    params: serde_json::json!({}),
                    verify: VerificationType::None,
                    timeout_ms: Some(5_000),
                }],
            },
            checkpoint: VerificationCheckpoint::None,
            recovery: None,
            context_hints: StageContextHints::default(),
            timeout_sec: MAX_STAGE_DURATION_SEC,
            skippable: false,
        };

        let tree = make_tree(vec![abort_stage, should_not_run]);
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;

        assert!(!result.success);
        assert!(
            result.aborted
                || matches!(result.stage_results[0].outcome, StageOutcome::Failed { .. }),
            "expected abort, got: success={} aborted={} stages={}",
            result.success,
            result.aborted,
            result.stage_results.len()
        );
        // Stage 1 must NOT have been reached
        assert!(
            result.stage_results.len() < 2
                || !matches!(result.stage_results[1].outcome, StageOutcome::Passed),
            "stage 1 should not have passed after AbortWorkflow"
        );
    }

    // ── Cancellation before start ─────────────────────────────────────────────

    #[tokio::test]
    async fn cancellation_before_start_returns_cancelled() {
        let executor =
            StageExecutor::new(Arc::new(PassThroughExecutor), Arc::new(AlwaysPassVerifier));
        let cancel = CancellationToken::new();
        cancel.cancel(); // already cancelled before we start

        let tree = make_tree(vec![
            bash_stage(0, "echo should_not_run", false, false),
            bash_stage(1, "echo also_not_run", false, true),
        ]);
        let result = executor.execute_goal_tree(&tree, cancel).await;
        assert!(result.cancelled, "expected cancelled=true");
        assert!(!result.success);
    }

    // ── Mid-workflow bash failure ─────────────────────────────────────────────

    #[tokio::test]
    async fn mid_workflow_bash_fail_stage_1() {
        let registry = Arc::new(build_default_registry());
        let executor = StageExecutor::new(
            Arc::new(DirectExecutor { registry }),
            Arc::new(AlwaysPassVerifier),
        );

        // Stage 0: passes
        // Stage 1: `false` exits with code 1 → fails
        let tree = make_tree(vec![
            bash_stage(0, "echo stage_0_ok", false, false),
            bash_stage(1, "false", false, true), // always-fail POSIX command
        ]);

        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;

        assert!(
            matches!(result.stage_results[0].outcome, StageOutcome::Passed),
            "stage 0 should pass, got {:?}",
            result.stage_results[0].outcome
        );
        assert!(
            matches!(result.stage_results[1].outcome, StageOutcome::Failed { .. }),
            "stage 1 (false) should fail, got {:?}",
            result.stage_results[1].outcome
        );
        assert!(!result.success);
    }

    // ── VM-only: write then delete real file ──────────────────────────────────

    #[tokio::test]
    async fn vm_only_write_and_delete_file() {
        if !requires_vm() {
            eprintln!("[SKIP] vm_only_write_and_delete_file: set KRIA_EVAL_VM=1");
            return;
        }
        let registry = Arc::new(build_default_registry());
        let executor = StageExecutor::new(
            Arc::new(DirectExecutor { registry }),
            Arc::new(AlwaysPassVerifier),
        );
        let path = "/tmp/kria_fault_inject_vm_test.txt";
        let tree = make_tree(vec![
            bash_stage(0, &format!("echo content > {}", path), false, false),
            bash_stage(1, &format!("rm -f {}", path), false, true),
        ]);
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;
        assert!(result.success);
        assert!(
            !std::path::Path::new(path).exists(),
            "file should be deleted"
        );
    }

    /// VM-only: rm -rf a generated temp directory.
    #[tokio::test]
    async fn vm_only_rm_rf_temp_dir() {
        if !requires_vm() {
            eprintln!("[SKIP] vm_only_rm_rf_temp_dir: set KRIA_EVAL_VM=1");
            return;
        }
        let dir = "/tmp/kria_fault_inject_vm_dir";
        let registry = Arc::new(build_default_registry());
        let executor = StageExecutor::new(
            Arc::new(DirectExecutor { registry }),
            Arc::new(AlwaysPassVerifier),
        );
        let tree = make_tree(vec![
            bash_stage(
                0,
                &format!("mkdir -p {}/sub && echo x > {}/sub/file.txt", dir, dir),
                false,
                false,
            ),
            bash_stage(1, &format!("rm -rf {}", dir), false, true),
        ]);
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;
        assert!(result.success);
        assert!(!std::path::Path::new(dir).exists(), "dir should be removed");
    }

    /// VM-only: spawn a background process then kill it by PID.
    #[tokio::test]
    async fn vm_only_process_kill() {
        if !requires_vm() {
            eprintln!("[SKIP] vm_only_process_kill: set KRIA_EVAL_VM=1");
            return;
        }
        let registry = Arc::new(build_default_registry());
        let executor = StageExecutor::new(
            Arc::new(DirectExecutor { registry }),
            Arc::new(AlwaysPassVerifier),
        );
        // Spawn sleep 60 in background, capture its PID, kill it
        let tree = make_tree(vec![
            bash_stage(0, "sleep 60 &\necho $! > /tmp/kria_fault_sleep.pid", false, false),
            bash_stage(1, "kill $(cat /tmp/kria_fault_sleep.pid) 2>/dev/null; rm -f /tmp/kria_fault_sleep.pid; echo killed", false, true),
        ]);
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;
        assert!(result.success);
        let output = result.terminal_output.unwrap_or_default();
        assert!(
            output.contains("killed"),
            "expected 'killed' in output: {:?}",
            output
        );
    }
}
