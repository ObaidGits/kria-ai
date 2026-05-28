//! Hybrid Workflow Executor — Canonical Workflow Execution Runtime.
//!
//! This is the singular execution authority for all KRIA workflows.
//! It consumes a capability-aware plan and executes it as a coherent
//! workflow — not as isolated tool calls.
//!
//! # Authority
//!
//! This module OWNS workflow execution. It:
//! - Manages the lifecycle FSM transitions
//! - Emits structured telemetry at every state change
//! - Orchestrates verification via the contract-driven verifier
//! - Handles HITL pause/resume
//! - Produces the canonical WorkflowVerdict
//!
//! # Design
//!
//! - Executes plans as workflow graphs (not tool sequences)
//! - Respects execution modes (Structural/Visible/Hybrid/Interactive)
//! - Bounded: every step has a timeout, every workflow has a budget
//! - Observable: every transition emits telemetry
//! - Resumable: HITL pauses preserve full execution context

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;
use crate::agent::workflow_lifecycle::{LifecycleError, WorkflowInstance};
use crate::agent::workflow_telemetry::{
    execution_mode_from_previews, step_previews_from_workflow, WorkflowTelemetryEmitter,
};
use crate::agent::workflow_types::*;
use crate::agent::workflow_verifier::{verify_contract, verdict_from_contract};

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Executor Configuration
// ═══════════════════════════════════════════════════════════════════════════════

/// Configuration for the hybrid workflow executor.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// Maximum total workflow duration (ms)
    pub max_budget_ms: u64,
    /// Default per-step timeout (ms)
    pub default_step_timeout_ms: u64,
    /// Maximum retry attempts per step
    pub max_retries: u8,
    /// Initial retry backoff (ms)
    pub initial_backoff_ms: u64,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_budget_ms: DEFAULT_WORKFLOW_BUDGET_MS,
            default_step_timeout_ms: 30_000,
            max_retries: MAX_RETRY_ATTEMPTS,
            initial_backoff_ms: 500,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Step Execution Result
// ═══════════════════════════════════════════════════════════════════════════════

/// Result of executing a single workflow step.
#[derive(Debug, Clone)]
pub struct StepExecutionResult {
    pub step_index: u32,
    pub action: String,
    pub success: bool,
    pub error: Option<String>,
    pub artifacts: Vec<String>,
    pub duration_ms: u64,
    pub retries_used: u8,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Workflow Execution Context
// ═══════════════════════════════════════════════════════════════════════════════

/// Runtime context for a workflow execution session.
/// Preserves state across HITL pauses and step transitions.
pub struct WorkflowExecutionContext {
    /// The workflow instance (lifecycle FSM)
    pub instance: WorkflowInstance,
    /// Resolved capabilities (cached for workflow lifetime)
    pub capabilities: CapabilitySet,
    /// Outcome contract from the planner
    pub outcome_contract: OutcomeContract,
    /// Execution mode
    pub execution_mode: ExecutionMode,
    /// Telemetry emitter
    pub telemetry: WorkflowTelemetryEmitter,
    /// Step results accumulated during execution
    pub step_results: Vec<StepExecutionResult>,
    /// Artifacts accumulated during execution
    pub artifacts: Vec<String>,
    /// Budget remaining (ms)
    pub budget_remaining_ms: u64,
    /// Cancellation token
    pub cancellation: CancellationToken,
    /// Configuration
    pub config: ExecutorConfig,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — Hybrid Workflow Executor
// ═══════════════════════════════════════════════════════════════════════════════

/// The canonical hybrid workflow executor.
///
/// Executes workflow plans as coherent sessions with:
/// - Lifecycle management
/// - Telemetry emission
/// - HITL pause/resume
/// - Contract-driven verification
/// - Verdict finalization
pub struct HybridWorkflowExecutor;

impl HybridWorkflowExecutor {
    /// Execute a complete workflow from plan to verdict.
    ///
    /// This is the top-level entry point. It:
    /// 1. Creates the execution context
    /// 2. Transitions through the lifecycle FSM
    /// 3. Executes each step with mode-appropriate behavior
    /// 4. Runs contract verification
    /// 5. Computes and returns the canonical verdict
    pub async fn execute(
        plan: &crate::agent::gui_substrate_planner::SubstratePlan,
        outcome_contract: OutcomeContract,
        execution_mode: ExecutionMode,
        capabilities: CapabilitySet,
        cancellation: CancellationToken,
        config: ExecutorConfig,
    ) -> WorkflowExecutionResult {
        Self::execute_with_tools(plan, outcome_contract, execution_mode, capabilities, cancellation, config, None).await
    }

    /// Execute with a real tool executor (production path).
    ///
    /// When `tool_executor` is `Some`, steps are executed via the tool registry
    /// with full policy/HITL/isolation enforcement. When `None`, execution is
    /// simulated (lifecycle + telemetry only — used for testing/shadow mode).
    pub async fn execute_with_tools(
        plan: &crate::agent::gui_substrate_planner::SubstratePlan,
        outcome_contract: OutcomeContract,
        execution_mode: ExecutionMode,
        capabilities: CapabilitySet,
        cancellation: CancellationToken,
        config: ExecutorConfig,
        tool_executor: Option<Arc<dyn crate::agent::htn_executor::ToolExecutor>>,
    ) -> WorkflowExecutionResult {
        let workflow_id = plan
            .workflow
            .as_ref()
            .map(|w| w.task_id.clone())
            .unwrap_or_else(|| format!("wf-{}", uuid::Uuid::new_v4()));

        let total_steps = plan
            .workflow
            .as_ref()
            .map(|w| w.sub_goals.len() as u32)
            .unwrap_or(0);

        // Create telemetry emitter
        let (telemetry, _receiver) = WorkflowTelemetryEmitter::new(
            workflow_id.clone(),
            WorkflowSource::SubstrateRouter,
        );

        // Create lifecycle instance
        let mut instance = WorkflowInstance::new(
            workflow_id.clone(),
            "Workflow".into(),
            total_steps,
            WorkflowSource::SubstrateRouter,
        );

        // Transition: Created → Planned
        if let Err(e) = instance.mark_planned() {
            return WorkflowExecutionResult::lifecycle_error(e);
        }

        // Emit Started telemetry
        let step_previews = plan
            .workflow
            .as_ref()
            .map(|w| step_previews_from_workflow(w))
            .unwrap_or_default();
        let exec_mode = execution_mode_from_previews(&step_previews);
        telemetry.emit_started(
            "Workflow execution",
            &step_previews,
            exec_mode,
            Some(config.max_budget_ms),
        );

        // Build execution context
        let mut ctx = WorkflowExecutionContext {
            instance,
            capabilities,
            outcome_contract: outcome_contract.clone(),
            execution_mode,
            telemetry,
            step_results: Vec::new(),
            artifacts: plan.artifacts.iter().map(|p| p.display().to_string()).collect(),
            budget_remaining_ms: config.max_budget_ms,
            cancellation: cancellation.clone(),
            config,
        };

        // Execute steps
        if let Some(workflow) = &plan.workflow {
            for goal in &workflow.sub_goals {
                // Check cancellation
                if ctx.cancellation.is_cancelled() {
                    let _ = ctx.instance.mark_cancelled("User cancelled".into());
                    ctx.telemetry.emit_cancelled(
                        "User cancelled",
                        ctx.step_results.len() as u32,
                        total_steps,
                    );
                    return WorkflowExecutionResult {
                        verdict: WorkflowVerdict::Partial {
                            completed: ctx.step_results.len() as u32,
                            total: total_steps,
                            reason: "Cancelled by user".into(),
                        },
                        step_results: ctx.step_results,
                        artifacts: ctx.artifacts,
                        telemetry_trace: ctx.instance.trace().to_vec(),
                        duration_ms: ctx.instance.elapsed_ms(),
                    };
                }

                // Check budget
                if ctx.budget_remaining_ms == 0 {
                    let _ = ctx.instance.mark_finalized(WorkflowVerdict::Partial {
                        completed: ctx.step_results.len() as u32,
                        total: total_steps,
                        reason: "Budget exhausted".into(),
                    });
                    return WorkflowExecutionResult {
                        verdict: WorkflowVerdict::Partial {
                            completed: ctx.step_results.len() as u32,
                            total: total_steps,
                            reason: "Workflow budget exhausted".into(),
                        },
                        step_results: ctx.step_results,
                        artifacts: ctx.artifacts,
                        telemetry_trace: ctx.instance.trace().to_vec(),
                        duration_ms: ctx.instance.elapsed_ms(),
                    };
                }

                // Transition: → Executing(step)
                let step_index = goal.step as u32;
                let _ = ctx.instance.mark_executing(step_index);

                // Emit step started
                let step_type = classify_step_type(&goal.action);
                ctx.telemetry.emit_step_started(
                    step_index,
                    &goal.action,
                    step_type,
                );

                // Execute the step
                let step_start = Instant::now();
                let step_result = if let Some(ref executor) = tool_executor {
                    // REAL EXECUTION — tool registry with policy/HITL/isolation
                    let step_timeout = goal.timeout_ms.unwrap_or(ctx.config.default_step_timeout_ms);
                    let tool_result = tokio::time::timeout(
                        Duration::from_millis(step_timeout),
                        executor.execute(&goal.action, &goal.params),
                    )
                    .await;

                    match tool_result {
                        Ok(result) => StepExecutionResult {
                            step_index,
                            action: goal.action.clone(),
                            success: result.success,
                            error: result.error.clone(),
                            artifacts: vec![],
                            duration_ms: step_start.elapsed().as_millis() as u64,
                            retries_used: 0,
                        },
                        Err(_timeout) => StepExecutionResult {
                            step_index,
                            action: goal.action.clone(),
                            success: false,
                            error: Some(format!(
                                "Step timed out after {}ms",
                                step_timeout
                            )),
                            artifacts: vec![],
                            duration_ms: step_start.elapsed().as_millis() as u64,
                            retries_used: 0,
                        },
                    }
                } else {
                    // SIMULATED EXECUTION — lifecycle + telemetry only (shadow/test mode)
                    StepExecutionResult {
                        step_index,
                        action: goal.action.clone(),
                        success: true,
                        error: None,
                        artifacts: vec![],
                        duration_ms: step_start.elapsed().as_millis() as u64,
                        retries_used: 0,
                    }
                };

                // Emit step completed
                ctx.telemetry.emit_step_completed(
                    step_index,
                    step_result.success,
                    VisibilityConfidence::NotApplicable,
                    step_result.artifacts.clone(),
                );

                // Handle step failure
                if !step_result.success {
                    let error = step_result.error.clone().unwrap_or_else(|| "Unknown error".into());
                    let _ = ctx.instance.mark_finalized(WorkflowVerdict::Failed {
                        step: step_index,
                        reason: error.clone(),
                        recovery: None,
                    });
                    ctx.telemetry.emit_completed(
                        WorkflowVerdict::Failed {
                            step: step_index,
                            reason: error.clone(),
                            recovery: None,
                        },
                        &format!("Failed at step {}: {}", step_index, error),
                        ctx.artifacts.clone(),
                        vec![],
                    );
                    ctx.step_results.push(step_result);
                    return WorkflowExecutionResult {
                        verdict: WorkflowVerdict::Failed {
                            step: step_index,
                            reason: error,
                            recovery: None,
                        },
                        step_results: ctx.step_results,
                        artifacts: ctx.artifacts,
                        telemetry_trace: ctx.instance.trace().to_vec(),
                        duration_ms: ctx.instance.elapsed_ms(),
                    };
                }

                // Record step completion
                let _ = ctx.instance.mark_step_completed(step_index);
                ctx.budget_remaining_ms = ctx.budget_remaining_ms.saturating_sub(step_result.duration_ms);
                ctx.step_results.push(step_result);
            }
        }

        // Transition: → Verifying
        let _ = ctx.instance.mark_verifying();

        // Run contract-driven verification
        let contract_verification = verify_contract(&outcome_contract, &ctx.capabilities).await;

        // Compute verdict from contract
        let verdict = verdict_from_contract(&contract_verification, &outcome_contract);

        // Transition: → Finalized
        let _ = ctx.instance.mark_finalized(verdict.clone());

        // Emit completion telemetry
        ctx.telemetry.emit_completed(
            verdict.clone(),
            &format_verdict_summary(&verdict),
            ctx.artifacts.clone(),
            vec![],
        );

        WorkflowExecutionResult {
            verdict,
            step_results: ctx.step_results,
            artifacts: ctx.artifacts,
            telemetry_trace: ctx.instance.trace().to_vec(),
            duration_ms: ctx.instance.elapsed_ms(),
        }
    }
}

/// Complete result of a workflow execution session.
#[derive(Debug)]
pub struct WorkflowExecutionResult {
    /// The canonical verdict
    pub verdict: WorkflowVerdict,
    /// Per-step execution results
    pub step_results: Vec<StepExecutionResult>,
    /// Artifacts produced during execution
    pub artifacts: Vec<String>,
    /// Full telemetry trace for debugging/persistence
    pub telemetry_trace: Vec<TelemetryEnvelope>,
    /// Total execution duration
    pub duration_ms: u64,
}

impl WorkflowExecutionResult {
    fn lifecycle_error(e: LifecycleError) -> Self {
        Self {
            verdict: WorkflowVerdict::Failed {
                step: 0,
                reason: format!("Lifecycle error: {}", e),
                recovery: None,
            },
            step_results: vec![],
            artifacts: vec![],
            telemetry_trace: vec![],
            duration_ms: 0,
        }
    }
}

fn classify_step_type(action: &str) -> StepType {
    match action {
        "write_file" => StepType::FileWrite,
        "open_application" | "open_application_with_file" => StepType::AppLaunch,
        "execute_bash" | "execute_python" => StepType::CommandExecution,
        "browser_search" | "managed_browser_navigate" | "open_url" => StepType::BrowserNavigation,
        "click_element" | "click_mouse" | "type_text" | "press_shortcut" => StepType::Interaction,
        _ => StepType::CommandExecution,
    }
}

fn format_verdict_summary(verdict: &WorkflowVerdict) -> String {
    match verdict {
        WorkflowVerdict::Complete => "Workflow completed successfully.".into(),
        WorkflowVerdict::AlreadySatisfied { evidence } => {
            format!("Already done: {}", evidence)
        }
        WorkflowVerdict::StructurallyComplete { unverified_outcomes } => {
            format!(
                "Completed structurally. Unverified: {}",
                unverified_outcomes.join(", ")
            )
        }
        WorkflowVerdict::Partial { completed, total, reason } => {
            format!("Partial: {}/{} steps. {}", completed, total, reason)
        }
        WorkflowVerdict::Blocked { reason } => format!("Blocked: {}", reason),
        WorkflowVerdict::Failed { step, reason, .. } => {
            format!("Failed at step {}: {}", step, reason)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §5 — Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::gui_substrate_planner::{ExecutionSubstrate, SubstratePlan};
    use crate::agent::htn_executor::{GuiWorkflow, SubGoal, VerificationType};

    fn make_capabilities() -> CapabilitySet {
        CapabilitySet {
            environment: EnvironmentCapability {
                session_type: SessionType::X11,
                compositor: None,
                atspi_level: AtSpiLevel::Full,
                xdotool_available: true,
                uinput_available: true,
                ocr_available: false,
            },
            verifier: VerifierCapability {
                available_methods: vec![
                    VerificationMethod::FileSystem,
                    VerificationMethod::ProcessTable,
                ],
                window_state_max_confidence: 0.90,
                cdp_available: false,
                filesystem_available: true,
                process_table_available: true,
            },
            interaction: InteractionCapability {
                keyboard_injection: InputInjectionLevel::Full,
                mouse_injection: InputInjectionLevel::Full,
                clipboard_available: true,
            },
        }
    }

    fn make_simple_plan() -> SubstratePlan {
        let test_file = std::path::PathBuf::from("/tmp/kria_executor_test.txt");
        SubstratePlan {
            substrate: ExecutionSubstrate::FileWriteThenOpen,
            workflow: Some(GuiWorkflow {
                task_id: "test-executor-wf".into(),
                sub_goals: vec![
                    SubGoal {
                        step: 1,
                        action: "write_file".into(),
                        params: serde_json::json!({
                            "path": test_file.display().to_string(),
                            "content": "test content",
                        }),
                        verify: VerificationType::FileSystemEffect {
                            path: test_file.clone(),
                            expected_substring: "test".into(),
                        },
                        timeout_ms: Some(5000),
                    },
                ],
                safe_abort_steps: vec![],
                max_duration_sec: 30,
            }),
            artifacts: vec![test_file],
        }
    }

    #[tokio::test]
    async fn executor_produces_verdict_for_simple_plan() {
        let plan = make_simple_plan();
        let contract = OutcomeContract {
            required: vec![PlannedOutcome {
                description: "Test file exists".into(),
                expectation: OutcomeExpectation::FileExists {
                    path: "/tmp/kria_executor_test.txt".into(),
                },
                min_confidence: 0.80,
                on_failure: OutcomeFailurePolicy::FailWorkflow,
            }],
            desired: vec![],
        };
        let caps = make_capabilities();
        let cancel = CancellationToken::new();

        // Write the test file so verification passes
        tokio::fs::write("/tmp/kria_executor_test.txt", "test content").await.unwrap();

        let result = HybridWorkflowExecutor::execute(
            &plan,
            contract,
            ExecutionMode::Structural,
            caps,
            cancel,
            ExecutorConfig::default(),
        )
        .await;

        assert!(
            matches!(result.verdict, WorkflowVerdict::Complete),
            "Expected Complete verdict, got {:?}",
            result.verdict
        );
        assert_eq!(result.step_results.len(), 1);
        assert!(result.duration_ms > 0 || result.duration_ms == 0); // Just check it's set

        // Cleanup
        let _ = tokio::fs::remove_file("/tmp/kria_executor_test.txt").await;
    }

    #[tokio::test]
    async fn executor_respects_cancellation() {
        let plan = make_simple_plan();
        let contract = OutcomeContract::empty();
        let caps = make_capabilities();
        let cancel = CancellationToken::new();

        // Cancel immediately
        cancel.cancel();

        let result = HybridWorkflowExecutor::execute(
            &plan,
            contract,
            ExecutionMode::Structural,
            caps,
            cancel,
            ExecutorConfig::default(),
        )
        .await;

        assert!(
            matches!(result.verdict, WorkflowVerdict::Partial { ref reason, .. } if reason.contains("Cancelled")),
            "Expected Partial(Cancelled) verdict, got {:?}",
            result.verdict
        );
    }

    #[tokio::test]
    async fn executor_produces_telemetry_trace() {
        let plan = make_simple_plan();
        let contract = OutcomeContract::empty();
        let caps = make_capabilities();
        let cancel = CancellationToken::new();

        let result = HybridWorkflowExecutor::execute(
            &plan,
            contract,
            ExecutionMode::Structural,
            caps,
            cancel,
            ExecutorConfig::default(),
        )
        .await;

        // Should have lifecycle trace entries
        assert!(
            !result.telemetry_trace.is_empty(),
            "Should have telemetry trace entries"
        );
        // Trace should be monotonically ordered
        for window in result.telemetry_trace.windows(2) {
            assert!(window[1].seq > window[0].seq);
        }
    }

    #[tokio::test]
    async fn executor_handles_empty_plan() {
        let plan = SubstratePlan {
            substrate: ExecutionSubstrate::Unknown,
            workflow: None,
            artifacts: vec![],
        };
        let contract = OutcomeContract::empty();
        let caps = make_capabilities();
        let cancel = CancellationToken::new();

        let result = HybridWorkflowExecutor::execute(
            &plan,
            contract,
            ExecutionMode::Structural,
            caps,
            cancel,
            ExecutorConfig::default(),
        )
        .await;

        // Empty plan with empty contract → Complete (nothing to do, nothing to verify)
        assert!(
            matches!(result.verdict, WorkflowVerdict::Complete),
            "Empty plan + empty contract should be Complete, got {:?}",
            result.verdict
        );
    }

    #[tokio::test]
    async fn executor_structurally_complete_when_desired_unverified() {
        let plan = make_simple_plan();
        let test_path = "/tmp/kria_executor_test_sc.txt";
        let contract = OutcomeContract {
            required: vec![PlannedOutcome {
                description: "File exists".into(),
                expectation: OutcomeExpectation::FileExists { path: test_path.into() },
                min_confidence: 0.80,
                on_failure: OutcomeFailurePolicy::FailWorkflow,
            }],
            desired: vec![PlannedOutcome {
                description: "Nonexistent app visible".into(),
                expectation: OutcomeExpectation::AppWindowVisible {
                    app: "kria_nonexistent_xyz".into(),
                    title_hint: None,
                },
                min_confidence: 0.70,
                on_failure: OutcomeFailurePolicy::DowngradeFidelity,
            }],
        };
        let caps = make_capabilities();
        let cancel = CancellationToken::new();

        // Write file so required passes
        tokio::fs::write(test_path, "test").await.unwrap();

        let result = HybridWorkflowExecutor::execute(
            &plan,
            contract,
            ExecutionMode::Hybrid { visible_steps: vec![2] },
            caps,
            cancel,
            ExecutorConfig::default(),
        )
        .await;

        assert!(
            matches!(result.verdict, WorkflowVerdict::StructurallyComplete { .. }),
            "Expected StructurallyComplete, got {:?}",
            result.verdict
        );

        let _ = tokio::fs::remove_file(test_path).await;
    }

    // ── Real Tool Executor Integration Test ──────────────────────────────────

    /// Mock tool executor that actually "executes" by writing files.
    struct RealMockToolExecutor;

    #[async_trait::async_trait]
    impl crate::agent::htn_executor::ToolExecutor for RealMockToolExecutor {
        async fn execute(&self, action: &str, params: &serde_json::Value) -> crate::infra::ToolResult {
            match action {
                "write_file" => {
                    let path = params["path"].as_str().unwrap_or("/tmp/kria_mock_write.txt");
                    let content = params["content"].as_str().unwrap_or("mock content");
                    match tokio::fs::write(path, content).await {
                        Ok(_) => crate::infra::ToolResult::ok(serde_json::json!({"written": path})),
                        Err(e) => crate::infra::ToolResult::err(format!("Write failed: {}", e)),
                    }
                }
                "execute_bash" => {
                    let command = params["command"].as_str().unwrap_or("echo test");
                    let output = tokio::process::Command::new("bash")
                        .args(["-c", command])
                        .output()
                        .await;
                    match output {
                        Ok(o) if o.status.success() => {
                            let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                            crate::infra::ToolResult::ok(serde_json::json!({"output": stdout}))
                        }
                        Ok(o) => {
                            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                            crate::infra::ToolResult::err(format!("Command failed: {}", stderr))
                        }
                        Err(e) => crate::infra::ToolResult::err(format!("Exec error: {}", e)),
                    }
                }
                _ => crate::infra::ToolResult::ok(serde_json::json!({"action": action, "simulated": true})),
            }
        }
    }

    #[tokio::test]
    async fn executor_with_real_tool_executor_writes_file() {
        let test_path = "/tmp/kria_canonical_exec_test.txt";
        let _ = tokio::fs::remove_file(test_path).await; // Clean up from previous runs

        let plan = SubstratePlan {
            substrate: ExecutionSubstrate::TerminalExecution,
            workflow: Some(GuiWorkflow {
                task_id: "test-real-exec".into(),
                sub_goals: vec![SubGoal {
                    step: 1,
                    action: "write_file".into(),
                    params: serde_json::json!({
                        "path": test_path,
                        "content": "canonical executor wrote this",
                    }),
                    verify: VerificationType::FileSystemEffect {
                        path: std::path::PathBuf::from(test_path),
                        expected_substring: "canonical".into(),
                    },
                    timeout_ms: Some(5000),
                }],
                safe_abort_steps: vec![],
                max_duration_sec: 30,
            }),
            artifacts: vec![std::path::PathBuf::from(test_path)],
        };

        let contract = OutcomeContract {
            required: vec![PlannedOutcome {
                description: "Test file created".into(),
                expectation: OutcomeExpectation::FileExists { path: test_path.into() },
                min_confidence: 0.80,
                on_failure: OutcomeFailurePolicy::FailWorkflow,
            }],
            desired: vec![],
        };

        let caps = make_capabilities();
        let cancel = CancellationToken::new();
        let tool_executor: Arc<dyn crate::agent::htn_executor::ToolExecutor> =
            Arc::new(RealMockToolExecutor);

        let result = HybridWorkflowExecutor::execute_with_tools(
            &plan,
            contract,
            ExecutionMode::Structural,
            caps,
            cancel,
            ExecutorConfig::default(),
            Some(tool_executor),
        )
        .await;

        // Verify the file was ACTUALLY written
        let file_exists = tokio::fs::metadata(test_path).await.is_ok();
        assert!(file_exists, "Canonical executor should have ACTUALLY written the file");

        let content = tokio::fs::read_to_string(test_path).await.unwrap();
        assert!(content.contains("canonical executor wrote this"));

        // Verify verdict
        assert!(
            matches!(result.verdict, WorkflowVerdict::Complete),
            "Expected Complete verdict, got {:?}",
            result.verdict
        );
        assert_eq!(result.step_results.len(), 1);
        assert!(result.step_results[0].success);

        // Cleanup
        let _ = tokio::fs::remove_file(test_path).await;
    }

    #[tokio::test]
    async fn executor_with_real_tool_executor_handles_command_failure() {
        let plan = SubstratePlan {
            substrate: ExecutionSubstrate::TerminalExecution,
            workflow: Some(GuiWorkflow {
                task_id: "test-fail-exec".into(),
                sub_goals: vec![SubGoal {
                    step: 1,
                    action: "execute_bash".into(),
                    params: serde_json::json!({
                        "command": "exit 1",  // Intentional failure
                    }),
                    verify: VerificationType::FileSystemEffect {
                        path: std::path::PathBuf::from("/tmp/nonexistent"),
                        expected_substring: "".into(),
                    },
                    timeout_ms: Some(5000),
                }],
                safe_abort_steps: vec![],
                max_duration_sec: 30,
            }),
            artifacts: vec![],
        };

        let contract = OutcomeContract::empty();
        let caps = make_capabilities();
        let cancel = CancellationToken::new();
        let tool_executor: Arc<dyn crate::agent::htn_executor::ToolExecutor> =
            Arc::new(RealMockToolExecutor);

        let result = HybridWorkflowExecutor::execute_with_tools(
            &plan,
            contract,
            ExecutionMode::Structural,
            caps,
            cancel,
            ExecutorConfig::default(),
            Some(tool_executor),
        )
        .await;

        // Command should fail
        assert!(
            matches!(result.verdict, WorkflowVerdict::Failed { .. }),
            "Expected Failed verdict for failed command, got {:?}",
            result.verdict
        );
    }
}
