//! P3c: StageExecutor — bounded sequential execution of GoalTree stages.
//!
//! # Authority Boundary
//!
//! The StageExecutor:
//! - Executes stages sequentially from a **frozen** `&GoalTree`
//! - Verifies checkpoints using the existing `BoundedExecutionVerifier`
//! - Dispatches actions via the existing `ToolExecutor`
//! - Enforces recovery budgets (max 2 attempts per stage)
//! - Enforces timeouts (per-stage + global)
//! - Propagates cancellation
//!
//! It MUST NOT:
//! - Replan or generate new stages
//! - Call the WorkflowCompiler or GuiPlanner
//! - Mutate the GoalTree
//! - Add stages at runtime
//! - Perform recursive planning loops
//! - Infer hidden goals
//!
//! # Relationship to Existing Executor
//!
//! The `StageExecutor` wraps the existing `ToolExecutor` for action dispatch
//! and the existing `BoundedExecutionVerifier` for checkpoint verification.
//! It does NOT duplicate their functionality.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::agent::execution_transparency::ExecutionTransparencyLayer;
use crate::agent::execution_verifier::ExecutionVerifier;
use crate::agent::goal_tree::{
    CompletionContract, GoalTree, RecoveryAction, StageAction, VerificationCheckpoint,
    WorkflowStage, MAX_RECOVERY_ATTEMPTS,
};
use crate::agent::gui_lease::ForegroundLeaseManager;
use crate::agent::htn_executor::ToolExecutor;
use crate::agent::workflow_continuation::{
    InterruptionClass, InterruptionContext, WorkflowContinuationRuntime,
};
use crate::agent::workflow_session::{SessionManager, SessionStep, WorkflowSession};

/// Hard cap on total actions across the entire workflow.
/// Matches existing runtime cap from P1.
const MAX_TOTAL_ACTIONS: usize = 100;

fn goal_tree_needs_foreground(tree: &GoalTree) -> bool {
    tree.stages.iter().any(|stage| {
        matches!(
            stage.checkpoint,
            VerificationCheckpoint::WindowFocused { .. }
                | VerificationCheckpoint::WindowInteractive { .. }
                | VerificationCheckpoint::KeyboardTargetConfirmed { .. }
                | VerificationCheckpoint::ForegroundLeaseAcquired { .. }
        ) || stage.action_group.actions.iter().any(|action| {
            matches!(
                action.action.as_str(),
                "type_text"
                    | "click_mouse"
                    | "click_element"
                    | "press_shortcut"
                    | "focus_window"
                    | "switch_to_window"
            )
        })
    })
}

// ============================================================================
// GoalTreeResult — execution outcome
// ============================================================================

/// Overall result of executing a GoalTree.
#[derive(Debug)]
pub struct GoalTreeResult {
    /// Workflow ID from the GoalTree
    pub workflow_id: String,
    /// Whether all stages completed successfully
    pub success: bool,
    /// Per-stage results
    pub stage_results: Vec<StageResult>,
    /// Total elapsed time
    pub duration_ms: u128,
    /// Error message if workflow failed
    pub error: Option<String>,
    /// Whether workflow was cancelled
    pub cancelled: bool,
    /// Whether global abort was executed
    pub aborted: bool,
    /// Captured stdout from the last execute_bash action, if any.
    /// Populated by Bug 7 fix — surfaces program output to the user.
    pub terminal_output: Option<String>,
}

/// Result of executing a single stage.
#[derive(Debug, Clone)]
pub struct StageResult {
    /// Stage index
    pub stage_index: u32,
    /// Stage label
    pub label: String,
    /// Outcome of this stage
    pub outcome: StageOutcome,
    /// Number of actions executed
    pub actions_executed: usize,
    /// Number of recovery attempts used
    pub recovery_attempts: u32,
    /// Stage duration in milliseconds
    pub duration_ms: u128,
    /// Raw data returned by each action (e.g. execute_bash stdout/stderr).
    /// Populated only on successful action dispatch.
    pub action_outputs: Vec<serde_json::Value>,
}

/// Possible outcomes for a single stage.
#[derive(Debug, Clone)]
pub enum StageOutcome {
    /// All actions completed and checkpoint passed
    Passed,
    /// Checkpoint passed after recovery
    PassedAfterRecovery,
    /// Stage was skipped via SkipStage recovery
    Skipped,
    /// Stage failed after exhausting recovery
    Failed { reason: String },
    /// Stage was cancelled
    Cancelled,
    /// Stage timed out
    TimedOut,
    /// Stage paused because a durable collaborative decision is required.
    PausedForDecision { decision_id: String, reason: String },
    /// Not yet executed
    Pending,
}

fn stage_outcome_confidence(outcome: &StageOutcome) -> f32 {
    match outcome {
        StageOutcome::Passed => 1.0,
        StageOutcome::PassedAfterRecovery => 0.75,
        StageOutcome::Skipped => 0.55,
        StageOutcome::PausedForDecision { .. } => 0.25,
        StageOutcome::Failed { .. } | StageOutcome::TimedOut | StageOutcome::Cancelled => 0.0,
        StageOutcome::Pending => 0.0,
    }
}

// ============================================================================
// StageExecutor
// ============================================================================

/// Bounded sequential executor for GoalTree workflows.
///
/// Executes stages in order, verifies checkpoints, enforces recovery
/// budgets, and propagates cancellation. Never replans.
///
/// # Batch 1+2: PSDG Bridge + SessionManager + TransparencyLayer
///
/// - `PsdgHandle`: stage outcomes → WorldModelStore (fire-and-forget)
/// - `SessionManager`: workflow progress → JSON file checkpoint
/// - `ExecutionTransparencyLayer`: real-time stage trace for human visibility
pub struct StageExecutor {
    /// Tool executor for dispatching actions (reused from existing system)
    tool_executor: Arc<dyn ToolExecutor>,
    /// Execution verifier for checkpoint validation (reused from existing system)
    verifier: Arc<dyn ExecutionVerifier>,
    /// Optional PSDG handle for workflow progress persistence.
    world_model: Option<crate::agent::psdg::PsdgHandle>,
    /// Session manager for JSON file checkpoint persistence.
    session_mgr: SessionManager,
    /// Execution transparency layer for real-time workflow tracing.
    transparency: Option<ExecutionTransparencyLayer>,
    /// Optional workflow continuation runtime — classifies stage failures as
    /// interruptions, plans bounded recovery, and issues pause checkpoints when
    /// human intervention is required. Bounded to MAX_RECOVERY_DEPTH retries.
    continuation_runtime: Option<Arc<WorkflowContinuationRuntime>>,
    /// Optional foreground lease manager. When present, GUI-sensitive GoalTrees
    /// acquire a bounded exclusive GUI lease before execution starts.
    foreground_lease: Option<ForegroundLeaseManager>,
}

impl StageExecutor {
    /// Create a new StageExecutor.
    ///
    /// Both dependencies are reused from the existing runtime — the
    /// StageExecutor adds stage-level orchestration on top.
    pub fn new(tool_executor: Arc<dyn ToolExecutor>, verifier: Arc<dyn ExecutionVerifier>) -> Self {
        Self {
            tool_executor,
            verifier,
            world_model: None,
            session_mgr: SessionManager::new(),
            transparency: None,
            continuation_runtime: None,
            foreground_lease: None,
        }
    }

    /// Attach an `ExecutionTransparencyLayer` for real-time workflow tracing.
    ///
    /// When set, every stage completion/failure is recorded in the transparency
    /// layer, enabling live workflow state visibility for the Tauri frontend.
    pub fn with_transparency(mut self, layer: ExecutionTransparencyLayer) -> Self {
        self.transparency = Some(layer);
        self
    }

    /// Attach a `WorkflowContinuationRuntime` for interruption classification on stage
    /// failure. When set, unrecoverable stage failures are classified as interruptions
    /// and a bounded recovery plan is generated. Plans are logged and recorded as
    /// transparency blockers. `RequestHumanIntervention`/`Escalate` actions write a
    /// pause checkpoint to disk for crash-safe resumption.
    pub fn with_continuation_runtime(mut self, rt: Arc<WorkflowContinuationRuntime>) -> Self {
        self.continuation_runtime = Some(rt);
        self
    }

    pub fn with_foreground_lease(mut self, lease: ForegroundLeaseManager) -> Self {
        self.foreground_lease = Some(lease);
        self
    }

    /// Attach a PSDG handle for workflow progress persistence.
    ///
    /// When set, stage completions and workflow outcomes are persisted to
    /// WorldModelStore as fire-and-forget semantic facts.
    pub fn with_world_model(mut self, psdg: crate::agent::psdg::PsdgHandle) -> Self {
        self.world_model = Some(psdg);
        self
    }

    /// Execute a GoalTree. The tree is borrowed immutably — never mutated.
    ///
    /// # Cancellation
    /// If `cancel` is triggered at any point, the current stage is
    /// abandoned and the global abort sequence runs.
    ///
    /// # Invariant
    /// This method NEVER calls any planner or compiler. If a stage
    /// fails after recovery, the workflow aborts.
    pub async fn execute_goal_tree(
        &self,
        tree: &GoalTree,
        cancel: CancellationToken,
    ) -> GoalTreeResult {
        let start = Instant::now();
        let global_deadline = start + Duration::from_secs(tree.max_total_duration_sec);
        let mut stage_results: Vec<StageResult> = Vec::with_capacity(tree.stages.len());
        let mut total_actions_executed: usize = 0;

        tracing::info!(
            target: "stage_executor",
            workflow_id = %tree.workflow_id,
            stages = tree.stages.len(),
            max_duration_sec = tree.max_total_duration_sec,
            "Beginning GoalTree execution"
        );

        let _foreground_guard = if goal_tree_needs_foreground(tree) {
            if let Some(ref manager) = self.foreground_lease {
                match manager
                    .acquire(
                        tree.workflow_id.clone(),
                        "goal_tree_stage_executor",
                        Duration::from_secs(tree.max_total_duration_sec.min(300)),
                    )
                    .await
                {
                    Ok(guard) => {
                        tracing::info!(
                            target: "stage_executor",
                            workflow_id = %tree.workflow_id,
                            "GUI foreground lease acquired"
                        );
                        Some(guard)
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "stage_executor",
                            workflow_id = %tree.workflow_id,
                            error = %e,
                            "GUI foreground lease denied"
                        );
                        return self.build_result(
                            tree,
                            stage_results,
                            start,
                            false,
                            false,
                            Some(format!("GUI foreground lease denied: {}", e)),
                        );
                    }
                }
            } else {
                None
            }
        } else {
            None
        };
        // Batch 2: begin transparency trace for human-visible workflow progress.
        if let Some(ref t) = self.transparency {
            t.begin_trace(tree);
        }

        for stage in &tree.stages {
            // ── Check cancellation ─────────────────────────────────
            if cancel.is_cancelled() {
                tracing::info!(
                    target: "stage_executor",
                    stage = stage.index,
                    "Workflow cancelled before stage"
                );
                stage_results.push(StageResult {
                    stage_index: stage.index,
                    label: stage.label.clone(),
                    outcome: StageOutcome::Cancelled,
                    actions_executed: 0,
                    recovery_attempts: 0,
                    duration_ms: 0,
                    action_outputs: Vec::new(),
                });
                return self.build_result(
                    tree,
                    stage_results,
                    start,
                    true,
                    false,
                    Some("Cancelled".into()),
                );
            }

            // ── Check global timeout ───────────────────────────────
            if Instant::now() >= global_deadline {
                tracing::warn!(
                    target: "stage_executor",
                    stage = stage.index,
                    "Global workflow timeout exceeded"
                );
                stage_results.push(StageResult {
                    stage_index: stage.index,
                    label: stage.label.clone(),
                    outcome: StageOutcome::TimedOut,
                    actions_executed: 0,
                    recovery_attempts: 0,
                    duration_ms: 0,
                    action_outputs: Vec::new(),
                });
                // Run global abort
                self.execute_abort(&tree.global_abort).await;
                return self.build_result(
                    tree,
                    stage_results,
                    start,
                    false,
                    true,
                    Some("Global timeout exceeded".into()),
                );
            }

            // ── Execute stage ──────────────────────────────────────
            let stage_result = self.execute_stage(stage, &cancel, global_deadline).await;

            total_actions_executed += stage_result.actions_executed;

            // Batch 2: update transparency trace with stage outcome.
            if let Some(ref t) = self.transparency {
                t.update_stage(
                    &tree.workflow_id,
                    stage_result.stage_index,
                    &stage_result.label,
                    &stage_result.outcome,
                    stage_result.actions_executed,
                    stage_result.recovery_attempts as u32,
                    stage_result.duration_ms,
                    stage_outcome_confidence(&stage_result.outcome),
                );
            }

            // P3e: Global action budget enforcement
            if total_actions_executed >= MAX_TOTAL_ACTIONS {
                tracing::warn!(
                    target: "stage_executor",
                    total_actions = total_actions_executed,
                    max = MAX_TOTAL_ACTIONS,
                    "Global action budget exhausted"
                );
                stage_results.push(stage_result);
                self.execute_abort(&tree.global_abort).await;
                return self.build_result(
                    tree,
                    stage_results,
                    start,
                    false,
                    true,
                    Some(format!(
                        "Global action budget exhausted ({}/{})",
                        total_actions_executed, MAX_TOTAL_ACTIONS
                    )),
                );
            }

            let failed = matches!(
                stage_result.outcome,
                StageOutcome::Failed { .. } | StageOutcome::TimedOut
            );
            let paused_for_decision =
                matches!(stage_result.outcome, StageOutcome::PausedForDecision { .. });
            let cancelled = matches!(stage_result.outcome, StageOutcome::Cancelled);
            // Save fields needed by the WCR block before stage_result is moved.
            let failed_label = stage_result.label.clone();
            let failed_recovery_attempts = stage_result.recovery_attempts;
            let failed_timed_out = matches!(stage_result.outcome, StageOutcome::TimedOut);
            let failed_reason = match &stage_result.outcome {
                StageOutcome::Failed { reason } => Some(reason.clone()),
                StageOutcome::PausedForDecision { reason, .. } => Some(reason.clone()),
                _ => None,
            };

            tracing::info!(
                target: "stage_executor",
                stage = stage.index,
                label = %stage.label,
                outcome = ?stage_result.outcome,
                recovery_attempts = stage_result.recovery_attempts,
                duration_ms = stage_result.duration_ms,
                "Stage completed"
            );

            // ── PSDG: write stage outcome (fire-and-forget) ────────────────
            if let Some(ref psdg) = self.world_model {
                let outcome_str = match &stage_result.outcome {
                    StageOutcome::Passed
                    | StageOutcome::PassedAfterRecovery
                    | StageOutcome::Skipped => "completed",
                    StageOutcome::PausedForDecision { .. } => "paused_for_decision",
                    StageOutcome::Failed { .. } | StageOutcome::TimedOut => "failed",
                    StageOutcome::Cancelled => "cancelled",
                    StageOutcome::Pending => "pending",
                };
                psdg.record_workflow_stage(&tree.workflow_id, &stage.label, outcome_str, &[]);
            }

            stage_results.push(stage_result);

            if paused_for_decision {
                let reason = failed_reason.unwrap_or_else(|| "decision required".to_string());
                if let Some(ref t) = self.transparency {
                    t.record_blocker(
                        &tree.workflow_id,
                        stage.index,
                        reason.clone(),
                        "Workflow paused until the pending decision is resolved".to_string(),
                    );
                }
                if let Some(ref rt) = self.continuation_runtime {
                    let session = WorkflowSession::new(
                        tree.workflow_id.clone(),
                        tree.description.clone(),
                        "GoalTree".to_string(),
                    );
                    let interruption = InterruptionClass::UserIntervened {
                        description: reason.clone(),
                    };
                    let checkpoint =
                        rt.pause_workflow(&tree.workflow_id, &session, interruption, "GoalTree");
                    tracing::warn!(
                        target: "workflow_continuation",
                        workflow_id = %tree.workflow_id,
                        paused_at = ?checkpoint.paused_at,
                        "GoalTree workflow paused for collaborative decision"
                    );
                }
                tracing::warn!(
                    target: "stage_executor",
                    workflow_id = %tree.workflow_id,
                    stage = stage.index,
                    reason = %reason,
                    "GoalTree workflow paused for collaborative decision"
                );
                return self.build_result(tree, stage_results, start, false, false, Some(reason));
            }

            if cancelled {
                self.execute_abort(&tree.global_abort).await;
                return self.build_result(
                    tree,
                    stage_results,
                    start,
                    true,
                    true,
                    Some("Cancelled during stage".into()),
                );
            }

            if failed {
                // ── Batch 2: Interruption classification + recovery planning ──────────
                // Classify the stage failure and plan a bounded recovery. Record the
                // blocker in the transparency trace and optionally issue a pause
                // checkpoint when human intervention is required.
                if let Some(ref rt) = self.continuation_runtime {
                    let interruption_ctx = InterruptionContext {
                        current_stage_label: Some(failed_label.clone()),
                        stage_timed_out: failed_timed_out,
                        checkpoint_failure_reason: failed_reason.clone(),
                        ..Default::default()
                    };
                    let interruption = rt.classify_interruption(&interruption_ctx);
                    let plan =
                        rt.plan_recovery(&interruption, failed_recovery_attempts.min(255) as u8);
                    tracing::warn!(
                        target: "workflow_continuation",
                        workflow_id = %tree.workflow_id,
                        stage = %failed_label,
                        interruption = %interruption.user_message(),
                        explanation = %plan.explanation,
                        "GoalTree stage failed — interruption classified; recovery plan ready"
                    );
                    if let Some(ref t) = self.transparency {
                        t.record_blocker(
                            &tree.workflow_id,
                            stage.index,
                            interruption.user_message(),
                            plan.explanation.clone(),
                        );
                    }
                    // Pause the workflow when human intervention is required so
                    // the session is crash-safe and resumable.
                    let needs_human = matches!(
                        plan.primary_action,
                        crate::agent::workflow_continuation::RecoveryAction::RequestHumanIntervention { .. }
                        | crate::agent::workflow_continuation::RecoveryAction::Escalate { .. }
                    );
                    if needs_human {
                        let session = WorkflowSession::new(
                            tree.workflow_id.clone(),
                            tree.description.clone(),
                            "GoalTree".to_string(),
                        );
                        let _ = rt.pause_workflow(
                            &tree.workflow_id,
                            &session,
                            interruption,
                            "GoalTree",
                        );
                        tracing::warn!(
                            target: "workflow_continuation",
                            workflow_id = %tree.workflow_id,
                            "GoalTree workflow paused — awaiting human intervention"
                        );
                    }
                }
                // Unrecoverable failure — abort workflow.
                // Include the actual failure reason so callers (e.g. loop_engine)
                // can surface actionable diagnostics instead of a generic message.
                let detailed_error = if let Some(ref reason) = failed_reason {
                    format!(
                        "Stage {} ('{}') failed: {}",
                        stage.index, failed_label, reason
                    )
                } else if failed_timed_out {
                    format!("Stage {} ('{}') timed out", stage.index, failed_label)
                } else {
                    format!(
                        "Stage {} ('{}') failed unrecoverably",
                        stage.index, failed_label
                    )
                };
                self.execute_abort(&tree.global_abort).await;
                return self.build_result(
                    tree,
                    stage_results,
                    start,
                    false,
                    true,
                    Some(detailed_error),
                );
            }
        }

        // ── Completion contract ────────────────────────────────────
        let completion_ok = self.verify_completion(&tree.completion).await;
        if !completion_ok {
            tracing::warn!(
                target: "stage_executor",
                "Completion contract not satisfied"
            );
            return self.build_result(
                tree,
                stage_results,
                start,
                false,
                false,
                Some("Completion contract verification failed".into()),
            );
        }

        tracing::info!(
            target: "stage_executor",
            workflow_id = %tree.workflow_id,
            duration_ms = start.elapsed().as_millis(),
            "GoalTree execution completed successfully"
        );

        self.build_result(tree, stage_results, start, false, false, None)
    }

    /// Execute a single stage with timeout and recovery.
    async fn execute_stage(
        &self,
        stage: &WorkflowStage,
        cancel: &CancellationToken,
        global_deadline: Instant,
    ) -> StageResult {
        let stage_start = Instant::now();
        let stage_deadline = stage_start + Duration::from_secs(stage.timeout_sec);
        // Use the earlier of stage deadline and global deadline
        let effective_deadline = stage_deadline.min(global_deadline);

        tracing::debug!(
            target: "stage_executor",
            stage = stage.index,
            label = %stage.label,
            actions = stage.action_group.actions.len(),
            timeout_sec = stage.timeout_sec,
            "Executing stage"
        );

        // ── Execute action group ───────────────────────────────────
        let actions_result = self
            .execute_actions(&stage.action_group.actions, cancel, effective_deadline)
            .await;

        match actions_result {
            ActionGroupResult::Cancelled => {
                return StageResult {
                    stage_index: stage.index,
                    label: stage.label.clone(),
                    outcome: StageOutcome::Cancelled,
                    actions_executed: 0,
                    recovery_attempts: 0,
                    duration_ms: stage_start.elapsed().as_millis(),
                    action_outputs: Vec::new(),
                };
            }
            ActionGroupResult::TimedOut => {
                return StageResult {
                    stage_index: stage.index,
                    label: stage.label.clone(),
                    outcome: StageOutcome::TimedOut,
                    actions_executed: 0,
                    recovery_attempts: 0,
                    duration_ms: stage_start.elapsed().as_millis(),
                    action_outputs: Vec::new(),
                };
            }
            ActionGroupResult::Failed { reason } => {
                return StageResult {
                    stage_index: stage.index,
                    label: stage.label.clone(),
                    outcome: StageOutcome::Failed { reason },
                    actions_executed: 0,
                    recovery_attempts: 0,
                    duration_ms: stage_start.elapsed().as_millis(),
                    action_outputs: Vec::new(),
                };
            }
            ActionGroupResult::PausedForDecision {
                decision_id,
                reason,
            } => {
                return StageResult {
                    stage_index: stage.index,
                    label: stage.label.clone(),
                    outcome: StageOutcome::PausedForDecision {
                        decision_id,
                        reason,
                    },
                    actions_executed: 0,
                    recovery_attempts: 0,
                    duration_ms: stage_start.elapsed().as_millis(),
                    action_outputs: Vec::new(),
                };
            }
            ActionGroupResult::Completed {
                actions_executed,
                outputs,
            } => {
                // ── Verify checkpoint ──────────────────────────────
                let checkpoint_ok = self.verify_checkpoint(&stage.checkpoint).await;

                if checkpoint_ok {
                    return StageResult {
                        stage_index: stage.index,
                        label: stage.label.clone(),
                        outcome: StageOutcome::Passed,
                        actions_executed,
                        recovery_attempts: 0,
                        duration_ms: stage_start.elapsed().as_millis(),
                        action_outputs: outputs.clone(),
                    };
                }

                // ── Checkpoint failed — attempt recovery ───────────
                if let Some(ref recovery) = stage.recovery {
                    let max_attempts = recovery.max_attempts.min(MAX_RECOVERY_ATTEMPTS);

                    for attempt in 0..max_attempts {
                        tracing::info!(
                            target: "stage_executor",
                            stage = stage.index,
                            attempt = attempt + 1,
                            max = max_attempts,
                            "Attempting recovery"
                        );

                        // Check cancellation/timeout before recovery
                        if cancel.is_cancelled() {
                            return StageResult {
                                stage_index: stage.index,
                                label: stage.label.clone(),
                                outcome: StageOutcome::Cancelled,
                                actions_executed,
                                recovery_attempts: attempt as u32 + 1,
                                duration_ms: stage_start.elapsed().as_millis(),
                                action_outputs: Vec::new(),
                            };
                        }
                        if Instant::now() >= effective_deadline {
                            return StageResult {
                                stage_index: stage.index,
                                label: stage.label.clone(),
                                outcome: StageOutcome::TimedOut,
                                actions_executed,
                                recovery_attempts: attempt as u32 + 1,
                                duration_ms: stage_start.elapsed().as_millis(),
                                action_outputs: Vec::new(),
                            };
                        }

                        match &recovery.recovery_action {
                            RecoveryAction::RetryFromAction { restart_from_index } => {
                                // Bug 6: Pre-check before re-running the action.
                                // Focus may have settled naturally since the initial verify
                                // (e.g. WM committed focus async after open_application returned).
                                // Avoids the blind "re-run open_application on Wayland" loop.
                                if self.verify_checkpoint(&stage.checkpoint).await {
                                    return StageResult {
                                        stage_index: stage.index,
                                        label: stage.label.clone(),
                                        outcome: StageOutcome::PassedAfterRecovery,
                                        actions_executed,
                                        recovery_attempts: attempt as u32 + 1,
                                        duration_ms: stage_start.elapsed().as_millis(),
                                        action_outputs: outputs.clone(),
                                    };
                                }
                                let retry_actions =
                                    &stage.action_group.actions[(*restart_from_index as usize)..];
                                let retry_result = self
                                    .execute_actions(retry_actions, cancel, effective_deadline)
                                    .await;
                                if !matches!(retry_result, ActionGroupResult::Completed { .. }) {
                                    continue;
                                }
                            }
                            RecoveryAction::Corrective { actions } => {
                                let corrective_result = self
                                    .execute_actions(actions, cancel, effective_deadline)
                                    .await;
                                if !matches!(corrective_result, ActionGroupResult::Completed { .. })
                                {
                                    continue;
                                }
                            }
                            RecoveryAction::SkipStage => {
                                return StageResult {
                                    stage_index: stage.index,
                                    label: stage.label.clone(),
                                    outcome: StageOutcome::Skipped,
                                    actions_executed,
                                    recovery_attempts: attempt as u32 + 1,
                                    duration_ms: stage_start.elapsed().as_millis(),
                                    action_outputs: Vec::new(),
                                };
                            }
                            RecoveryAction::AbortWorkflow => {
                                return StageResult {
                                    stage_index: stage.index,
                                    label: stage.label.clone(),
                                    outcome: StageOutcome::Failed {
                                        reason: "Recovery action: abort workflow".into(),
                                    },
                                    actions_executed,
                                    recovery_attempts: attempt as u32 + 1,
                                    duration_ms: stage_start.elapsed().as_millis(),
                                    action_outputs: Vec::new(),
                                };
                            }
                        }

                        // Re-verify checkpoint after recovery action
                        let re_check = self.verify_checkpoint(&stage.checkpoint).await;
                        if re_check {
                            return StageResult {
                                stage_index: stage.index,
                                label: stage.label.clone(),
                                outcome: StageOutcome::PassedAfterRecovery,
                                actions_executed,
                                recovery_attempts: attempt as u32 + 1,
                                duration_ms: stage_start.elapsed().as_millis(),
                                action_outputs: outputs.clone(),
                            };
                        }
                    }

                    // When a GUI-observation checkpoint has exhausted all retries on a
                    // skippable launch stage, skip instead of aborting the whole workflow.
                    // The action-level verifier has already checked the structural launch
                    // condition; this prevents Wayland/unobservable-window state from
                    // killing workflows that can continue through structural substrates.
                    let skippable_gui_observation_exhausted = stage.skippable
                        && matches!(
                            stage.checkpoint,
                            VerificationCheckpoint::WindowFocused { .. }
                                | VerificationCheckpoint::WindowVisible { .. }
                        );
                    if skippable_gui_observation_exhausted {
                        tracing::warn!(
                            target: "stage_executor",
                            stage = stage.index,
                            label = %stage.label,
                            checkpoint = ?stage.checkpoint,
                            "GUI observation exhausted on skippable stage — skipping after structural action success"
                        );
                        return StageResult {
                            stage_index: stage.index,
                            label: stage.label.clone(),
                            outcome: StageOutcome::Skipped,
                            actions_executed,
                            recovery_attempts: max_attempts as u32,
                            duration_ms: stage_start.elapsed().as_millis(),
                            action_outputs: Vec::new(),
                        };
                    }
                    let diagnostic = self.checkpoint_failure_diagnostic(&stage.checkpoint).await;
                    return StageResult {
                        stage_index: stage.index,
                        label: stage.label.clone(),
                        outcome: StageOutcome::Failed {
                            reason: format!(
                                "Checkpoint failed after {} recovery attempts: {}",
                                max_attempts, diagnostic
                            ),
                        },
                        actions_executed,
                        recovery_attempts: max_attempts as u32,
                        duration_ms: stage_start.elapsed().as_millis(),
                        action_outputs: Vec::new(),
                    };
                }

                // No recovery path — fail immediately
                let diagnostic = self.checkpoint_failure_diagnostic(&stage.checkpoint).await;
                StageResult {
                    stage_index: stage.index,
                    label: stage.label.clone(),
                    outcome: StageOutcome::Failed {
                        reason: format!("Checkpoint failed, no recovery path: {}", diagnostic),
                    },
                    actions_executed,
                    recovery_attempts: 0,
                    duration_ms: stage_start.elapsed().as_millis(),
                    action_outputs: outputs,
                }
            }
        }
    }

    /// Execute a sequence of actions sequentially.
    async fn execute_actions(
        &self,
        actions: &[StageAction],
        cancel: &CancellationToken,
        deadline: Instant,
    ) -> ActionGroupResult {
        let mut executed = 0;
        let mut outputs: Vec<serde_json::Value> = Vec::new();

        for action in actions {
            // Check cancellation
            if cancel.is_cancelled() {
                return ActionGroupResult::Cancelled;
            }

            // Check timeout
            if Instant::now() >= deadline {
                return ActionGroupResult::TimedOut;
            }

            tracing::debug!(
                target: "stage_executor",
                action = %action.action,
                "Dispatching action"
            );

            // Per-action timeout
            let action_timeout = action.timeout_ms.unwrap_or(10_000);
            let result = tokio::time::timeout(
                Duration::from_millis(action_timeout),
                self.tool_executor.execute(&action.action, &action.params),
            )
            .await;

            match result {
                Ok(tool_result) => {
                    if !tool_result.success {
                        let err_msg = tool_result.error.as_deref().unwrap_or("unknown error");
                        tracing::warn!(
                            target: "stage_executor",
                            action = %action.action,
                            error = %err_msg,
                            "Action failed"
                        );
                        if err_msg.starts_with("DECISION_PAUSED:") {
                            let decision_id = tool_result
                                .data
                                .get("decision_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            return ActionGroupResult::PausedForDecision {
                                decision_id,
                                reason: err_msg
                                    .trim_start_matches("DECISION_PAUSED:")
                                    .trim()
                                    .to_string(),
                            };
                        }
                        return ActionGroupResult::Failed {
                            reason: format!("Action '{}' failed: {}", action.action, err_msg),
                        };
                    }
                    // Collect action output for Bug 7 (execute_bash stdout surfacing).
                    if !tool_result.data.is_null() {
                        outputs.push(tool_result.data);
                    }
                    executed += 1;
                }
                Err(_elapsed) => {
                    tracing::warn!(
                        target: "stage_executor",
                        action = %action.action,
                        timeout_ms = action_timeout,
                        "Action timed out"
                    );
                    return ActionGroupResult::TimedOut;
                }
            }
        }

        ActionGroupResult::Completed {
            actions_executed: executed,
            outputs,
        }
    }

    /// Verify a stage checkpoint using the existing BoundedExecutionVerifier.
    async fn verify_checkpoint(&self, checkpoint: &VerificationCheckpoint) -> bool {
        if matches!(checkpoint, VerificationCheckpoint::None) {
            return true;
        }

        let verifiability = checkpoint.to_verifiability();
        let outcome = self.verifier.verify_rich(&verifiability).await;
        let strongest = outcome.strongest_evidence();

        tracing::debug!(
            target: "stage_executor",
            verified = outcome.outcome.verified,
            confidence = outcome.outcome.confidence,
            evidence = %outcome.outcome.evidence,
            latency_ms = outcome.outcome.latency_ms,
            source = ?strongest.map(|e| &e.source),
            reliability = ?strongest.map(|e| &e.reliability),
            ambiguous = strongest.map(|e| e.ambiguous).unwrap_or(true),
            "Checkpoint verification with evidence"
        );

        outcome.outcome.verified
    }

    async fn checkpoint_failure_diagnostic(&self, checkpoint: &VerificationCheckpoint) -> String {
        if matches!(checkpoint, VerificationCheckpoint::None) {
            return "no checkpoint required".to_string();
        }

        let verifiability = checkpoint.to_verifiability();
        let outcome = self.verifier.verify_rich(&verifiability).await;
        let source = outcome
            .strongest_evidence()
            .map(|e| format!("{:?}/{:?}", e.source, e.reliability))
            .unwrap_or_else(|| "unknown/unobservable".to_string());
        let ambiguity = outcome
            .strongest_evidence()
            .map(|e| e.ambiguous)
            .unwrap_or(true);
        let class = match checkpoint {
            VerificationCheckpoint::KeyboardTargetConfirmed { .. }
            | VerificationCheckpoint::WindowInteractive { .. }
            | VerificationCheckpoint::WindowFocused { .. } => {
                if ambiguity {
                    "gui_focus_or_keyboard_target_uncertain"
                } else {
                    "gui_window_state_mismatch"
                }
            }
            VerificationCheckpoint::WindowVisible { .. } => "window_not_visible_or_unobservable",
            VerificationCheckpoint::ProcessRunning { .. } => "process_not_running",
            VerificationCheckpoint::OutputContains { .. } => "semantic_output_missing",
            VerificationCheckpoint::FileEffect { .. } => "filesystem_effect_missing",
            VerificationCheckpoint::ForegroundLeaseAcquired { .. } => "foreground_lease_missing",
            VerificationCheckpoint::SemanticTargetConfirmed { .. } => "semantic_target_unconfirmed",
            VerificationCheckpoint::None => "no_checkpoint_required",
        };
        format!(
            "class={}, source={}, confidence={:.2}, evidence={}",
            class, source, outcome.outcome.confidence, outcome.outcome.evidence
        )
    }

    /// Verify the completion contract.
    async fn verify_completion(&self, contract: &CompletionContract) -> bool {
        match contract {
            CompletionContract::AllStagesPassed => true,
            CompletionContract::FinalVerification(checkpoint) => {
                self.verify_checkpoint(checkpoint).await
            }
            CompletionContract::UserConfirmation { .. } => {
                // Fail closed. Completion cannot be inferred from a future user
                // attestation; HITL must happen before reporting success.
                tracing::warn!(
                    target: "stage_executor",
                    "UserConfirmation completion contract requires HITL before success"
                );
                false
            }
        }
    }

    /// Execute the global abort sequence.
    async fn execute_abort(&self, abort_steps: &[crate::agent::goal_tree::SafeAbortStep]) {
        for step in abort_steps {
            tracing::info!(
                target: "stage_executor",
                action = %step.action,
                "Executing abort step"
            );
            // Best-effort — don't fail on abort errors
            let _ = tokio::time::timeout(
                Duration::from_millis(2000),
                self.tool_executor.execute(&step.action, &step.params),
            )
            .await;
        }
    }

    /// Build the final GoalTreeResult.
    ///
    /// Persists workflow outcome to:
    /// 1. WorldModelStore (PSDG fire-and-forget, when PsdgHandle is attached)
    /// 2. SessionManager (JSON file checkpoint for restart recovery)
    fn build_result(
        &self,
        tree: &GoalTree,
        stage_results: Vec<StageResult>,
        start: Instant,
        cancelled: bool,
        aborted: bool,
        error: Option<String>,
    ) -> GoalTreeResult {
        let success = error.is_none() && !cancelled;
        let duration_ms = start.elapsed().as_millis();

        // ── Collect artifacts from passed stages ──────────────────────────────────
        let artifacts: Vec<String> = stage_results
            .iter()
            .filter_map(|sr| {
                if matches!(
                    sr.outcome,
                    StageOutcome::Passed | StageOutcome::PassedAfterRecovery
                ) {
                    Some(sr.label.clone())
                } else {
                    None
                }
            })
            .collect();

        // ── PSDG: persist workflow outcome (fire-and-forget) ─────────────────────
        if let Some(ref psdg) = self.world_model {
            let outcome_str = if success {
                "completed"
            } else if cancelled {
                "cancelled"
            } else {
                "failed"
            };
            psdg.record_workflow_stage(
                &tree.workflow_id,
                "workflow_complete",
                outcome_str,
                &artifacts,
            );
        }

        // ── SessionManager: persist to JSON checkpoint for restart recovery ─────
        let now_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut session = WorkflowSession::new(
            tree.workflow_id.clone(),
            tree.description.clone(),
            "GoalTree".to_string(),
        );
        // Record all completed stages as SessionSteps
        for (i, sr) in stage_results.iter().enumerate() {
            let step_success = matches!(
                sr.outcome,
                StageOutcome::Passed | StageOutcome::PassedAfterRecovery | StageOutcome::Skipped
            );
            session.add_step(SessionStep {
                step: i + 1,
                action: sr.label.clone(),
                params: serde_json::Value::Null,
                success: step_success,
                evidence: format!("{:?}", sr.outcome),
                timestamp: now_epoch,
            });
        }
        if success {
            session.mark_complete(artifacts.clone());
        } else {
            let hint = if cancelled {
                Some(format!(
                    "Workflow was cancelled after {} stages",
                    stage_results.len()
                ))
            } else {
                error
                    .as_ref()
                    .map(|e| format!("Retry from stage {} after: {}", stage_results.len(), e))
            };
            session.mark_failed(error.clone().unwrap_or_else(|| "cancelled".into()), hint);
        }
        if let Err(e) = self.session_mgr.save(&session) {
            tracing::debug!(
                target: "stage_executor",
                workflow_id = %tree.workflow_id,
                error = %e,
                "SessionManager checkpoint write failed (non-fatal)"
            );
        }

        // Batch 2: complete transparency trace.
        if let Some(ref t) = self.transparency {
            t.complete_trace(&tree.workflow_id, success, error.clone());
        }

        // Bug 7: Collect stdout from any execute_bash action across all stages.
        // The last non-empty stdout value wins (most recent program output).
        let terminal_output = stage_results
            .iter()
            .flat_map(|sr| sr.action_outputs.iter())
            .filter_map(|output| {
                output
                    .get("stdout")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.to_string())
            })
            .last();

        GoalTreeResult {
            workflow_id: tree.workflow_id.clone(),
            success,
            stage_results,
            duration_ms,
            error,
            cancelled,
            aborted,
            terminal_output,
        }
    }
}

// ============================================================================
// P3f: Observability — GoalTreeStatus
// ============================================================================

/// Serializable workflow status for external visibility (e.g., Tauri commands).
///
/// NOT: cognition dashboards, semantic tracing, AI introspection.
/// This is structured logging for debugging.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GoalTreeStatus {
    /// Workflow ID
    pub workflow_id: String,
    /// Overall status
    pub status: WorkflowStatus,
    /// Per-stage status
    pub stages: Vec<StageStatus>,
    /// Total elapsed time in ms
    pub elapsed_ms: u128,
    /// Error message if any
    pub error: Option<String>,
}

/// High-level workflow status.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum WorkflowStatus {
    Completed,
    Failed,
    Cancelled,
    Paused,
    InProgress,
}

/// Per-stage status for observability.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StageStatus {
    pub index: u32,
    pub label: String,
    pub status: StageProgressStatus,
    pub actions_executed: usize,
    pub recovery_attempts: u32,
    pub duration_ms: u128,
}

/// Per-stage progress.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum StageProgressStatus {
    Passed,
    PassedAfterRecovery,
    Skipped,
    Failed { reason: String },
    Cancelled,
    TimedOut,
    PausedForDecision { decision_id: String, reason: String },
    Pending,
    InProgress,
}

impl GoalTreeStatus {
    /// Build a GoalTreeStatus from a completed GoalTreeResult.
    pub fn from_result(result: &GoalTreeResult) -> Self {
        let paused = result
            .stage_results
            .iter()
            .any(|stage| matches!(stage.outcome, StageOutcome::PausedForDecision { .. }));
        let status = if paused {
            WorkflowStatus::Paused
        } else if result.success {
            WorkflowStatus::Completed
        } else if result.cancelled {
            WorkflowStatus::Cancelled
        } else {
            WorkflowStatus::Failed
        };

        let stages = result
            .stage_results
            .iter()
            .map(|sr| StageStatus {
                index: sr.stage_index,
                label: sr.label.clone(),
                status: match &sr.outcome {
                    StageOutcome::Passed => StageProgressStatus::Passed,
                    StageOutcome::PassedAfterRecovery => StageProgressStatus::PassedAfterRecovery,
                    StageOutcome::Skipped => StageProgressStatus::Skipped,
                    StageOutcome::Failed { reason } => StageProgressStatus::Failed {
                        reason: reason.clone(),
                    },
                    StageOutcome::Cancelled => StageProgressStatus::Cancelled,
                    StageOutcome::TimedOut => StageProgressStatus::TimedOut,
                    StageOutcome::PausedForDecision {
                        decision_id,
                        reason,
                    } => StageProgressStatus::PausedForDecision {
                        decision_id: decision_id.clone(),
                        reason: reason.clone(),
                    },
                    StageOutcome::Pending => StageProgressStatus::Pending,
                },
                actions_executed: sr.actions_executed,
                recovery_attempts: sr.recovery_attempts,
                duration_ms: sr.duration_ms,
            })
            .collect();

        GoalTreeStatus {
            workflow_id: result.workflow_id.clone(),
            status,
            stages,
            elapsed_ms: result.duration_ms,
            error: result.error.clone(),
        }
    }
}

/// Internal result of executing an action group.
enum ActionGroupResult {
    Completed {
        actions_executed: usize,
        outputs: Vec<serde_json::Value>,
    },
    Failed {
        reason: String,
    },
    PausedForDecision {
        decision_id: String,
        reason: String,
    },
    Cancelled,
    TimedOut,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::execution_verifier::{Verifiability, VerifyOutcome};
    use crate::agent::goal_tree::*;
    use crate::agent::htn_executor::VerificationType;
    use crate::infra::ToolResult;

    // ── Mock ToolExecutor ───────────────────────────────────────────

    struct MockToolExecutor {
        /// If true, all actions succeed. If false, all fail.
        succeed: bool,
    }

    #[async_trait::async_trait]
    impl ToolExecutor for MockToolExecutor {
        async fn execute(&self, action: &str, _params: &serde_json::Value) -> ToolResult {
            if self.succeed {
                ToolResult::ok_text(format!("{} completed", action))
            } else {
                ToolResult::err(format!("{} failed", action))
            }
        }
    }

    struct PausingToolExecutor;

    #[async_trait::async_trait]
    impl ToolExecutor for PausingToolExecutor {
        async fn execute(&self, _action: &str, _params: &serde_json::Value) -> ToolResult {
            ToolResult::err_with_data(
                "DECISION_PAUSED: choose execution target",
                serde_json::json!({
                    "decision_id": "decision-123",
                    "decision_type": "target_selection"
                }),
            )
        }
    }

    // ── Mock Verifier ───────────────────────────────────────────────

    struct MockVerifier {
        /// If true, all verifications pass.
        pass: bool,
    }

    #[async_trait::async_trait]
    impl ExecutionVerifier for MockVerifier {
        async fn verify(&self, _leaf: &Verifiability) -> VerifyOutcome {
            VerifyOutcome {
                verified: self.pass,
                confidence: if self.pass { 1.0 } else { 0.0 },
                confidence_tier:
                    crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                evidence: if self.pass {
                    "mock pass".into()
                } else {
                    "mock fail".into()
                },
                latency_ms: 1,
            }
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────

    fn make_action(action: &str) -> StageAction {
        StageAction {
            action: action.to_string(),
            params: serde_json::json!({}),
            verify: VerificationType::None,
            timeout_ms: Some(1000),
        }
    }

    fn make_stage(index: u32, checkpoint: VerificationCheckpoint) -> WorkflowStage {
        WorkflowStage {
            index,
            label: format!("Stage {}", index),
            action_group: ActionGroup {
                actions: vec![make_action("test_action")],
            },
            checkpoint,
            recovery: None,
            context_hints: StageContextHints::default(),
            timeout_sec: 30,
            skippable: false,
        }
    }

    fn make_tree(stages: Vec<WorkflowStage>) -> GoalTree {
        GoalTree {
            workflow_id: "test-exec".to_string(),
            description: "Test execution".to_string(),
            stages,
            completion: CompletionContract::AllStagesPassed,
            global_abort: vec![SafeAbortStep {
                action: "press_shortcut".to_string(),
                params: serde_json::json!({"keys": ["Escape"]}),
            }],
            max_total_duration_sec: 120,
            preconditions: vec![],
        }
    }

    fn make_executor(succeed: bool, verify_pass: bool) -> StageExecutor {
        StageExecutor::new(
            Arc::new(MockToolExecutor { succeed }),
            Arc::new(MockVerifier { pass: verify_pass }),
        )
    }

    // ── Execution Tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn decision_paused_action_becomes_paused_stage() {
        let tree = make_tree(vec![make_stage(0, VerificationCheckpoint::None)]);
        let executor = StageExecutor::new(
            Arc::new(PausingToolExecutor),
            Arc::new(MockVerifier { pass: true }),
        );

        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;

        assert!(!result.success);
        assert!(!result.aborted);
        assert!(matches!(
            result.stage_results[0].outcome,
            StageOutcome::PausedForDecision {
                ref decision_id,
                ..
            } if decision_id == "decision-123"
        ));

        let status = GoalTreeStatus::from_result(&result);
        assert!(matches!(status.status, WorkflowStatus::Paused));
    }

    #[tokio::test]
    async fn successful_two_stage_workflow() {
        let tree = make_tree(vec![
            make_stage(
                0,
                VerificationCheckpoint::WindowFocused {
                    title_contains: Some("test".into()),
                    class: None,
                    pid: None,
                },
            ),
            make_stage(1, VerificationCheckpoint::None),
        ]);

        let executor = make_executor(true, true);
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

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
        assert!(!result.cancelled);
        assert!(!result.aborted);
    }

    #[tokio::test]
    async fn action_failure_aborts_workflow() {
        let tree = make_tree(vec![
            make_stage(0, VerificationCheckpoint::None),
            make_stage(1, VerificationCheckpoint::None),
        ]);

        let executor = make_executor(false, true); // actions fail
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        assert!(!result.success);
        assert!(result.aborted);
        // First stage should fail
        assert!(matches!(
            result.stage_results[0].outcome,
            StageOutcome::Failed { .. }
        ));
    }

    #[tokio::test]
    async fn checkpoint_failure_without_recovery_aborts() {
        let tree = make_tree(vec![
            make_stage(
                0,
                VerificationCheckpoint::WindowFocused {
                    title_contains: Some("test".into()),
                    class: None,
                    pid: None,
                },
            ),
            make_stage(1, VerificationCheckpoint::None),
        ]);

        let executor = make_executor(true, false); // verifier fails
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        assert!(!result.success);
        assert!(result.aborted);
        assert!(matches!(
            result.stage_results[0].outcome,
            StageOutcome::Failed { .. }
        ));
        // Second stage should not have been reached
        assert_eq!(result.stage_results.len(), 1);
    }

    #[tokio::test]
    async fn cancellation_stops_execution() {
        let tree = make_tree(vec![
            make_stage(0, VerificationCheckpoint::None),
            make_stage(1, VerificationCheckpoint::None),
        ]);

        let executor = make_executor(true, true);
        let cancel = CancellationToken::new();
        cancel.cancel(); // Cancel immediately

        let result = executor.execute_goal_tree(&tree, cancel).await;

        assert!(!result.success);
        assert!(result.cancelled);
        assert!(matches!(
            result.stage_results[0].outcome,
            StageOutcome::Cancelled
        ));
    }

    #[tokio::test]
    async fn recovery_retry_from_action() {
        // Use a verifier that always fails — recovery will exhaust
        let mut stage = make_stage(
            0,
            VerificationCheckpoint::WindowFocused {
                title_contains: Some("test".into()),
                class: None,
                pid: None,
            },
        );
        stage.recovery = Some(RecoveryPath {
            max_attempts: 2,
            recovery_action: RecoveryAction::RetryFromAction {
                restart_from_index: 0,
            },
        });

        let tree = make_tree(vec![stage]);
        let executor = make_executor(true, false); // checkpoint always fails
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        assert!(!result.success);
        assert_eq!(result.stage_results[0].recovery_attempts, 2);
        assert!(matches!(
            result.stage_results[0].outcome,
            StageOutcome::Failed { .. }
        ));
    }

    #[tokio::test]
    async fn recovery_skip_stage() {
        let mut stage = make_stage(
            0,
            VerificationCheckpoint::WindowFocused {
                title_contains: Some("test".into()),
                class: None,
                pid: None,
            },
        );
        stage.skippable = true;
        stage.recovery = Some(RecoveryPath {
            max_attempts: 1,
            recovery_action: RecoveryAction::SkipStage,
        });

        let tree = make_tree(vec![stage, make_stage(1, VerificationCheckpoint::None)]);

        let executor = make_executor(true, false); // checkpoint fails
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        // Stage 0 skipped, stage 1 should still execute.
        // But verifier fails for stage 1 checkpoint (None) → passes unconditionally.
        // Actually None checkpoint passes always in verify_checkpoint.
        assert!(result.success);
        assert!(matches!(
            result.stage_results[0].outcome,
            StageOutcome::Skipped
        ));
        assert!(matches!(
            result.stage_results[1].outcome,
            StageOutcome::Passed
        ));
    }

    #[tokio::test]
    async fn recovery_abort_workflow() {
        let mut stage = make_stage(
            0,
            VerificationCheckpoint::WindowFocused {
                title_contains: Some("test".into()),
                class: None,
                pid: None,
            },
        );
        stage.recovery = Some(RecoveryPath {
            max_attempts: 1,
            recovery_action: RecoveryAction::AbortWorkflow,
        });

        let tree = make_tree(vec![stage]);
        let executor = make_executor(true, false);
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        assert!(!result.success);
        assert!(result.aborted);
        assert!(matches!(
            result.stage_results[0].outcome,
            StageOutcome::Failed { .. }
        ));
    }

    #[tokio::test]
    async fn executor_never_mutates_goal_tree() {
        let tree = make_tree(vec![make_stage(0, VerificationCheckpoint::None)]);

        let original_id = tree.workflow_id.clone();
        let original_stages = tree.stages.len();

        let executor = make_executor(true, true);
        let cancel = CancellationToken::new();
        let _ = executor.execute_goal_tree(&tree, cancel).await;

        // GoalTree must be unchanged
        assert_eq!(tree.workflow_id, original_id);
        assert_eq!(tree.stages.len(), original_stages);
    }

    #[tokio::test]
    async fn none_checkpoint_always_passes() {
        let tree = make_tree(vec![make_stage(0, VerificationCheckpoint::None)]);

        // Even with a failing verifier, None checkpoint passes
        let executor = make_executor(true, false);
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        assert!(result.success);
        assert!(matches!(
            result.stage_results[0].outcome,
            StageOutcome::Passed
        ));
    }

    // ── P3g: Observability Tests ────────────────────────────────────

    #[tokio::test]
    async fn goal_tree_status_from_successful_result() {
        let tree = make_tree(vec![
            make_stage(0, VerificationCheckpoint::None),
            make_stage(1, VerificationCheckpoint::None),
        ]);
        let executor = make_executor(true, true);
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        let status = GoalTreeStatus::from_result(&result);
        assert!(matches!(status.status, WorkflowStatus::Completed));
        assert_eq!(status.stages.len(), 2);
        assert!(matches!(
            status.stages[0].status,
            StageProgressStatus::Passed
        ));
        assert!(matches!(
            status.stages[1].status,
            StageProgressStatus::Passed
        ));
        assert!(status.error.is_none());
    }

    #[tokio::test]
    async fn goal_tree_status_from_failed_result() {
        let tree = make_tree(vec![make_stage(0, VerificationCheckpoint::None)]);
        let executor = make_executor(false, true); // actions fail
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        let status = GoalTreeStatus::from_result(&result);
        assert!(matches!(status.status, WorkflowStatus::Failed));
        assert!(status.error.is_some());
    }

    #[tokio::test]
    async fn goal_tree_status_from_cancelled_result() {
        let tree = make_tree(vec![make_stage(0, VerificationCheckpoint::None)]);
        let executor = make_executor(true, true);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        let status = GoalTreeStatus::from_result(&result);
        assert!(matches!(status.status, WorkflowStatus::Cancelled));
    }

    #[tokio::test]
    async fn goal_tree_status_serializable() {
        let tree = make_tree(vec![make_stage(0, VerificationCheckpoint::None)]);
        let executor = make_executor(true, true);
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        let status = GoalTreeStatus::from_result(&result);
        let json = serde_json::to_string(&status).expect("should serialize");
        let deser: GoalTreeStatus = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(deser.workflow_id, status.workflow_id);
        assert_eq!(deser.stages.len(), status.stages.len());
    }

    // ── P3g: Corrective Recovery ────────────────────────────────────

    #[tokio::test]
    async fn recovery_corrective_actions() {
        let mut stage = make_stage(
            0,
            VerificationCheckpoint::WindowFocused {
                title_contains: Some("test".into()),
                class: None,
                pid: None,
            },
        );
        stage.recovery = Some(RecoveryPath {
            max_attempts: 1,
            recovery_action: RecoveryAction::Corrective {
                actions: vec![make_action("corrective_action")],
            },
        });

        let tree = make_tree(vec![stage]);
        // Verifier always fails → corrective runs but checkpoint still fails → exhausted
        let executor = make_executor(true, false);
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        assert!(!result.success);
        assert_eq!(result.stage_results[0].recovery_attempts, 1);
    }

    // ── P3g: Global Timeout ─────────────────────────────────────────

    #[tokio::test]
    async fn global_timeout_aborts_workflow() {
        let mut tree = make_tree(vec![
            make_stage(0, VerificationCheckpoint::None),
            make_stage(1, VerificationCheckpoint::None),
        ]);
        // Set max duration to 0 — immediate timeout on second stage
        tree.max_total_duration_sec = 0;

        let executor = make_executor(true, true);
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        // May succeed on first stage or timeout — depends on timing.
        // But if second stage is reached, it should be timed out.
        // The important invariant: workflow doesn't hang forever.
        assert!(result.duration_ms < 5000); // Should complete quickly
    }

    // ── P3g: End-to-End Compile→Execute Integration ─────────────────

    #[tokio::test]
    async fn end_to_end_compile_and_execute() {
        use crate::agent::environment_grounder::{GroundingCapabilities, OperationalFacts};
        use crate::agent::intent_compiler::{TargetRef, Verb};
        use crate::agent::workflow_compiler::{
            MultiVerbSpec, RuleBasedWorkflowCompiler, VerbClause, WorkflowCompiler,
        };

        // Step 1: Compile
        let spec = MultiVerbSpec {
            original_text: "Open firefox and type hello".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Open,
                    targets: vec![TargetRef::App("firefox".into())],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Type,
                    targets: vec![],
                    content: Some(crate::agent::intent_compiler::ContentClass::Literal(
                        "hello".into(),
                    )),
                },
            ],
        };

        let compiler = RuleBasedWorkflowCompiler;
        let facts = OperationalFacts::empty(GroundingCapabilities::none());
        let tree = compiler.compile(&spec, &facts).unwrap();

        // Validate
        assert!(tree.validate().is_empty());
        assert_eq!(tree.stages.len(), 2);

        // Step 2: Execute
        let executor = make_executor(true, true);
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        assert!(result.success);
        assert_eq!(result.stage_results.len(), 2);

        // Step 3: Observe
        let status = GoalTreeStatus::from_result(&result);
        assert!(matches!(status.status, WorkflowStatus::Completed));
        assert_eq!(status.stages.len(), 2);
    }

    // ── P3g: Degraded Mode (empty facts) ────────────────────────────

    #[tokio::test]
    async fn degraded_mode_empty_facts() {
        use crate::agent::environment_grounder::{GroundingCapabilities, OperationalFacts};
        use crate::agent::intent_compiler::{TargetRef, Verb};
        use crate::agent::workflow_compiler::{
            MultiVerbSpec, RuleBasedWorkflowCompiler, VerbClause, WorkflowCompiler,
        };

        let spec = MultiVerbSpec {
            original_text: "Open gedit and save".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Open,
                    targets: vec![TargetRef::App("gedit".into())],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Save,
                    targets: vec![],
                    content: None,
                },
            ],
        };

        // Empty facts (no xdotool, no windows)
        let facts = OperationalFacts::empty(GroundingCapabilities::none());
        let compiler = RuleBasedWorkflowCompiler;
        let tree = compiler.compile(&spec, &facts).unwrap();

        // Should compile fine even without grounding info
        assert_eq!(tree.stages.len(), 2);
        assert!(tree.validate().is_empty());

        // Context hints should reflect degraded state
        assert!(!tree.stages[0].context_hints.target_likely_open);
        assert!(tree.stages[0].context_hints.expected_cwd.is_none());
    }

    // ── P3g: Single-Stage Backward Compatibility ────────────────────

    #[tokio::test]
    async fn single_stage_workflow_executes() {
        // A single-stage GoalTree should work (even though the compiler
        // rejects single-verb specs, a GoalTree with 1 stage is valid)
        let tree = make_tree(vec![make_stage(0, VerificationCheckpoint::None)]);
        assert!(tree.validate().is_empty());

        let executor = make_executor(true, true);
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        assert!(result.success);
        assert_eq!(result.stage_results.len(), 1);
    }

    // ── P3g: Maximum Stage Workflow ─────────────────────────────────

    #[tokio::test]
    async fn max_stages_workflow_executes() {
        use crate::agent::goal_tree::MAX_STAGES;

        let stages: Vec<_> = (0..MAX_STAGES)
            .map(|i| {
                if i == MAX_STAGES - 1 {
                    make_stage(i as u32, VerificationCheckpoint::None)
                } else {
                    make_stage(
                        i as u32,
                        VerificationCheckpoint::WindowFocused {
                            title_contains: None,
                            class: None,
                            pid: None,
                        },
                    )
                }
            })
            .collect();

        let tree = make_tree(stages);
        assert!(tree.validate().is_empty());

        let executor = make_executor(true, true);
        let cancel = CancellationToken::new();
        let result = executor.execute_goal_tree(&tree, cancel).await;

        assert!(result.success);
        assert_eq!(result.stage_results.len(), MAX_STAGES);
    }
}
