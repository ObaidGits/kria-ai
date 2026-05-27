//! WorkflowEvalRunner — populates `WorkflowEvalObservation` from real execution.
//!
//! # Design
//!
//! This is the missing piece that makes `workflow_eval` go from 0% → real execution.
//! The runner bridges `WorkflowEvalCase` (contracts) → `WorkflowEvalObservation` (evidence).
//!
//! ## Execution strategy
//!
//! | Case class                        | Strategy                                          |
//! |-----------------------------------|---------------------------------------------------|
//! | `requires_daemon=false`, NoDisplay| GoalTree path via StageExecutor + DirectExecutor  |
//! | `requires_daemon=true`, no daemon | `daemon_alive_at_start=false` → judge Skips       |
//! | `KRIA_EVAL_VM=1` set              | Full daemon path allowed; judge runs live         |
//!
//! ## Observation population
//!
//! Fields mapped from `GoalTreeResult`:
//! - `tools_called` ← stage action names from all stages that passed
//! - `completed_stage_labels` ← `stage_results[i].label` where outcome is Passed/Skipped
//! - `reported_success` ← `result.success`
//! - `artifacts_found` ← filesystem scan after execution
//! - `stage_errors` ← error from Failed stage outcomes
//! - `terminal_output` → fed into `final_response` for observable scoring

use std::sync::Arc;
use std::time::Instant;

use kria_core::agent::environment_grounder::{GroundingCapabilities, OperationalFacts};
use kria_core::agent::execution_verifier::{
    ExecutionVerifier, Verifiability, VerificationConfidenceTier, VerifyOutcome,
};
use kria_core::agent::htn_executor::ToolExecutor;
use kria_core::agent::stage_executor::{StageExecutor, StageOutcome};
use kria_core::agent::workflow_compiler::{
    MultiVerbSpec, RuleBasedWorkflowCompiler, WorkflowCompiler,
};
use kria_core::infra::ToolResult;
use kria_core::tools::registry::{build_default_registry, ToolRegistry};
use tokio_util::sync::CancellationToken;

use super::judge::WorkflowCognitionJudge;
use super::types::{ArtifactFound, WorkflowEvalCase, WorkflowEvalObservation, WorkflowEvalVerdict};

// ============================================================================
// DirectToolExecutor — no policy layer, for CI eval
// ============================================================================

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
// RecordingToolExecutor — wraps DirectToolExecutor, records actual calls
// ============================================================================

struct RecordingToolExecutor {
    inner: DirectToolExecutor,
    calls: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl ToolExecutor for RecordingToolExecutor {
    async fn execute(&self, action: &str, params: &serde_json::Value) -> ToolResult {
        self.calls.lock().unwrap().push(action.to_string());
        self.inner.execute(action, params).await
    }
}

impl RecordingToolExecutor {
    fn new(registry: Arc<ToolRegistry>) -> Self {
        Self {
            inner: DirectToolExecutor { registry },
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn drain_calls(&self) -> Vec<String> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }
}

// ============================================================================
// AlwaysPassVerifier
// ============================================================================

struct AlwaysPassVerifier;

#[async_trait::async_trait]
impl ExecutionVerifier for AlwaysPassVerifier {
    async fn verify(&self, _: &Verifiability) -> VerifyOutcome {
        VerifyOutcome {
            verified: true,
            confidence: 1.0,
            confidence_tier: VerificationConfidenceTier::FullSemantic,
            evidence: "eval: always pass".to_string(),
            latency_ms: 0,
        }
    }
}

// ============================================================================
// WorkflowEvalRunner
// ============================================================================

/// Executes `WorkflowEvalCase` instances and produces `WorkflowEvalObservation`
/// by running the GoalTree → StageExecutor → real ToolRegistry path.
pub struct WorkflowEvalRunner {
    pub verbose: bool,
}

impl Default for WorkflowEvalRunner {
    fn default() -> Self {
        Self { verbose: false }
    }
}

impl WorkflowEvalRunner {
    /// Check whether the uinput daemon socket is alive.
    fn daemon_alive() -> bool {
        kria_core::agent::gui_services::default_uinput_socket_path().exists()
    }

    /// Returns true if VM-destructive tests are opted in.
    pub fn vm_mode() -> bool {
        std::env::var("KRIA_EVAL_VM").as_deref() == Ok("1")
    }

    /// Execute a single eval case.
    pub async fn run(
        &self,
        case: &WorkflowEvalCase,
        spec: Option<MultiVerbSpec>,
    ) -> (WorkflowEvalObservation, WorkflowEvalVerdict) {
        let daemon_alive = Self::daemon_alive();
        let start = Instant::now();

        // ── Gating: skip daemon-required cases when daemon is down ────────────
        if case.requires_daemon && !daemon_alive && !Self::vm_mode() {
            let obs = WorkflowEvalObservation {
                case_id: case.id.clone(),
                final_response: "SKIP: daemon not running".to_string(),
                tools_called: vec![],
                completed_stage_labels: vec![],
                reported_success: false,
                interruption_handled: None,
                artifacts_found: vec![],
                stage_errors: vec![],
                duration_ms: 0,
                daemon_alive_at_start: false,
                daemon_alive_at_end: false,
            };
            let (verdict, _diag) = WorkflowCognitionJudge::evaluate(case, &obs);
            return (obs, verdict);
        }

        // ── Compile MultiVerbSpec → GoalTree ──────────────────────────────────
        let spec = match spec {
            Some(s) => s,
            None => {
                let obs = WorkflowEvalObservation {
                    case_id: case.id.clone(),
                    final_response: "SKIP: no MultiVerbSpec provided for this case".to_string(),
                    tools_called: vec![],
                    completed_stage_labels: vec![],
                    reported_success: false,
                    interruption_handled: None,
                    artifacts_found: vec![],
                    stage_errors: vec![],
                    duration_ms: start.elapsed().as_millis() as u64,
                    daemon_alive_at_start: daemon_alive,
                    daemon_alive_at_end: Self::daemon_alive(),
                };
                let (verdict, _) = WorkflowCognitionJudge::evaluate(case, &obs);
                return (obs, verdict);
            }
        };

        let facts = OperationalFacts::empty(GroundingCapabilities::none());
        let tree = match RuleBasedWorkflowCompiler.compile(&spec, &facts) {
            Ok(t) => t,
            Err(e) => {
                let obs = WorkflowEvalObservation {
                    case_id: case.id.clone(),
                    final_response: format!("compile error: {e}"),
                    tools_called: vec![],
                    completed_stage_labels: vec![],
                    reported_success: false,
                    interruption_handled: None,
                    artifacts_found: vec![],
                    stage_errors: vec![format!("compile: {e}")],
                    duration_ms: start.elapsed().as_millis() as u64,
                    daemon_alive_at_start: daemon_alive,
                    daemon_alive_at_end: Self::daemon_alive(),
                };
                let (verdict, _) = WorkflowCognitionJudge::evaluate(case, &obs);
                return (obs, verdict);
            }
        };

        // ── Execute GoalTree ──────────────────────────────────────────────────
        let registry = Arc::new(build_default_registry());
        let recorder = Arc::new(RecordingToolExecutor::new(registry));
        let recorder_ref = Arc::clone(&recorder);
        let verifier: Arc<dyn ExecutionVerifier> = Arc::new(AlwaysPassVerifier);
        let executor = StageExecutor::new(recorder_ref, verifier);

        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;

        let tools_called = recorder.drain_calls();
        let duration_ms = start.elapsed().as_millis() as u64;

        // ── Collect observation ───────────────────────────────────────────────
        let mut completed_stage_labels = Vec::new();
        let mut stage_errors = Vec::new();

        for sr in &result.stage_results {
            match &sr.outcome {
                StageOutcome::Passed
                | StageOutcome::PassedAfterRecovery
                | StageOutcome::Skipped => {
                    completed_stage_labels.push(sr.label.clone());
                }
                StageOutcome::Failed { reason } => {
                    stage_errors.push(format!("stage {}: {}", sr.stage_index, reason));
                }
                _ => {}
            }
        }

        // Scan contract artifact globs for files that now exist
        let artifacts_found = scan_artifacts(&case.contract);

        // Build final_response: include terminal_output when present
        let final_response = if let Some(ref output) = result.terminal_output {
            format!(
                "Completed: {} stages in {}ms.\n\nProgram output:\n```\n{}\n```",
                result.stage_results.len(),
                result.duration_ms,
                output.trim()
            )
        } else if result.success {
            format!(
                "Completed: {} stages in {}ms.",
                result.stage_results.len(),
                result.duration_ms
            )
        } else {
            format!(
                "Failed: {}",
                result.error.as_deref().unwrap_or("unknown error")
            )
        };

        let obs = WorkflowEvalObservation {
            case_id: case.id.clone(),
            final_response,
            tools_called,
            completed_stage_labels,
            reported_success: result.success,
            interruption_handled: None,
            artifacts_found,
            stage_errors,
            duration_ms,
            daemon_alive_at_start: daemon_alive,
            daemon_alive_at_end: Self::daemon_alive(),
        };

        if self.verbose {
            eprintln!(
                "[workflow_eval] {} → success={} stages={} tools={:?}",
                case.id,
                obs.reported_success,
                obs.completed_stage_labels.len(),
                obs.tools_called
            );
        }

        let (verdict, _diag) = WorkflowCognitionJudge::evaluate(case, &obs);
        (obs, verdict)
    }

    /// Execute a pre-built `GoalTree` directly (skips MultiVerbSpec compilation).
    /// Use this for NoDisplay eval cases where you want execute_bash for all stages.
    pub async fn run_with_tree(
        &self,
        case: &WorkflowEvalCase,
        tree: kria_core::agent::goal_tree::GoalTree,
    ) -> (WorkflowEvalObservation, WorkflowEvalVerdict) {
        let daemon_alive = Self::daemon_alive();
        let start = Instant::now();
        let registry = Arc::new(build_default_registry());
        let recorder = Arc::new(RecordingToolExecutor::new(registry));
        let recorder_ref = Arc::clone(&recorder);
        let verifier: Arc<dyn ExecutionVerifier> = Arc::new(AlwaysPassVerifier);
        let executor = StageExecutor::new(recorder_ref, verifier);
        let result = executor
            .execute_goal_tree(&tree, CancellationToken::new())
            .await;

        let tools_called = recorder.drain_calls();
        let duration_ms = start.elapsed().as_millis() as u64;

        let mut completed_stage_labels = Vec::new();
        let mut stage_errors = Vec::new();
        for sr in &result.stage_results {
            match &sr.outcome {
                StageOutcome::Passed
                | StageOutcome::PassedAfterRecovery
                | StageOutcome::Skipped => {
                    completed_stage_labels.push(sr.label.clone());
                }
                StageOutcome::Failed { reason } => {
                    stage_errors.push(format!("stage {}: {}", sr.stage_index, reason));
                }
                _ => {}
            }
        }

        let artifacts_found = scan_artifacts(&case.contract);

        let final_response = if let Some(ref output) = result.terminal_output {
            format!(
                "Completed: {} stages in {}ms.\n\nProgram output:\n```\n{}\n```",
                result.stage_results.len(),
                result.duration_ms,
                output.trim()
            )
        } else if result.success {
            format!(
                "Completed: {} stages in {}ms.",
                result.stage_results.len(),
                result.duration_ms
            )
        } else {
            format!(
                "Failed: {}",
                result.error.as_deref().unwrap_or("unknown error")
            )
        };

        let obs = WorkflowEvalObservation {
            case_id: case.id.clone(),
            final_response,
            tools_called,
            completed_stage_labels,
            reported_success: result.success,
            interruption_handled: None,
            artifacts_found,
            stage_errors,
            duration_ms,
            daemon_alive_at_start: daemon_alive,
            daemon_alive_at_end: Self::daemon_alive(),
        };

        let (verdict, _) = WorkflowCognitionJudge::evaluate(case, &obs);
        (obs, verdict)
    }

    /// Run all cases in a suite, returning per-case (observation, verdict) pairs.
    pub async fn run_suite(
        &self,
        cases: &[(WorkflowEvalCase, Option<MultiVerbSpec>)],
    ) -> Vec<(WorkflowEvalObservation, WorkflowEvalVerdict)> {
        let mut results = Vec::new();
        for (case, spec) in cases {
            results.push(self.run(case, spec.clone()).await);
        }
        results
    }
}

// ============================================================================
// Artifact scanning
// ============================================================================

fn scan_artifacts(contract: &super::types::SemanticCompletionContract) -> Vec<ArtifactFound> {
    let mut found = Vec::new();
    for output in &contract.required_observable_outputs {
        if let Some(ref glob) = output.artifact_path_glob {
            // Expand ~ and simple globs
            let expanded = glob.replace(
                "~",
                &std::env::var("HOME").unwrap_or_else(|_| "/root".to_string()),
            );
            // Use glob crate if available, otherwise simple prefix check
            if let Ok(paths) = glob::glob(&expanded) {
                for entry in paths.flatten() {
                    if let Ok(meta) = std::fs::metadata(&entry) {
                        let preview = std::fs::read_to_string(&entry)
                            .unwrap_or_default()
                            .chars()
                            .take(200)
                            .collect();
                        found.push(ArtifactFound {
                            path: entry.to_string_lossy().to_string(),
                            size_bytes: meta.len(),
                            content_preview: preview,
                        });
                    }
                }
            }
        }
    }
    found
}

// ============================================================================
// MultiVerbSpec builders for NoDisplay eval cases
// ============================================================================

/// Build a NoDisplay MultiVerbSpec that writes a file then executes it with
/// execute_bash. Use for coding eval cases that don't need a GUI.
pub fn nodisplay_write_run_spec(
    original_prompt: &str,
    write_cmd: &str,
    run_cmd: &str,
) -> MultiVerbSpec {
    use kria_core::agent::intent_compiler::{ContentClass, TargetRef, Verb};
    use kria_core::agent::workflow_compiler::VerbClause;

    MultiVerbSpec {
        original_text: original_prompt.to_string(),
        clauses: vec![
            VerbClause {
                verb: Verb::Run,
                targets: vec![TargetRef::App("bash".to_string())],
                content: Some(ContentClass::Literal(write_cmd.to_string())),
            },
            VerbClause {
                verb: Verb::Run,
                targets: vec![TargetRef::App("bash".to_string())],
                content: Some(ContentClass::Literal(run_cmd.to_string())),
            },
        ],
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_eval::contracts::coding_run_and_show_contract;
    use crate::workflow_eval::types::{
        EvalWorkflowCategory, SafetyClass, WorkflowEvalCase, WorkflowVerdictKind,
    };

    fn make_nodisplay_coding_case(id: &str, prompt: &str) -> WorkflowEvalCase {
        WorkflowEvalCase {
            id: id.to_string(),
            description: format!("NoDisplay: {}", id),
            prompt: prompt.to_string(),
            category: EvalWorkflowCategory::Coding,
            contract: coding_run_and_show_contract(),
            safety_class: SafetyClass::Safe,
            interruption: None,
            timeout: std::time::Duration::from_secs(60),
            requires_daemon: false,
            requires_display: false,
            tags: vec!["nodisplay".to_string()],
            eval_notes: "NoDisplay GoalTree path eval".to_string(),
        }
    }

    /// A NoDisplay coding eval that writes + runs a Python file via execute_bash.
    /// Uses run_with_tree to bypass the MultiVerbSpec → type_text compilation and
    /// directly build a GoalTree with execute_bash stages (no daemon needed).
    #[tokio::test]
    async fn nodisplay_coding_eval_fibonacci() {
        use kria_core::agent::goal_tree::{
            ActionGroup, CompletionContract, GoalTree, StageAction, StageContextHints,
            VerificationCheckpoint, WorkflowStage, MAX_STAGE_DURATION_SEC,
        };
        use kria_core::agent::htn_executor::VerificationType;

        let case = make_nodisplay_coding_case(
            "wf-nodisplay-fibonacci",
            "write a python fibonacci program and run it and show me the output",
        );

        fn bash_eval_stage(index: u32, cmd: &str, is_last: bool) -> WorkflowStage {
            WorkflowStage {
                index,
                label: format!("bash:{}", &cmd[..cmd.len().min(20)]),
                action_group: ActionGroup {
                    actions: vec![StageAction {
                        action: "execute_bash".to_string(),
                        params: serde_json::json!({ "command": cmd }),
                        verify: VerificationType::None,
                        timeout_ms: Some(15_000),
                    }],
                },
                checkpoint: VerificationCheckpoint::None,
                recovery: if is_last {
                    None
                } else {
                    Some(kria_core::agent::goal_tree::RecoveryPath {
                        max_attempts: 1,
                        recovery_action: kria_core::agent::goal_tree::RecoveryAction::SkipStage,
                    })
                },
                context_hints: StageContextHints::default(),
                timeout_sec: MAX_STAGE_DURATION_SEC,
                skippable: !is_last,
            }
        }

        let tree = GoalTree {
            workflow_id: "wf-nodisplay-fibonacci".to_string(),
            description: "NoDisplay: write + run fibonacci python".to_string(),
            stages: vec![
                bash_eval_stage(
                    0,
                    // Write fibonacci python file — use /var/tmp (world-writable even when /tmp is 0755)
                    r#"printf 'def f(n):\n  a,b=0,1\n  for _ in range(n): print(a,end=" ");a,b=b,a+b\nf(10)\n' > /var/tmp/kria_eval_fib.py"#,
                    false,
                ),
                bash_eval_stage(
                    1,
                    "python3 /var/tmp/kria_eval_fib.py && rm -f /var/tmp/kria_eval_fib.py",
                    true,
                ),
            ],
            completion: CompletionContract::AllStagesPassed,
            global_abort: vec![],
            preconditions: vec![],
            max_total_duration_sec: 60,
        };

        let runner = WorkflowEvalRunner { verbose: false };
        let (obs, verdict) = runner.run_with_tree(&case, tree).await;

        assert!(
            !obs.tools_called.is_empty(),
            "expected tools to be called, got none"
        );
        assert!(
            obs.reported_success,
            "expected success, stage_errors: {:?}",
            obs.stage_errors
        );
        assert!(
            obs.final_response.contains("Program output")
                || obs.final_response.contains("Completed"),
            "response was: {}",
            obs.final_response
        );

        eprintln!(
            "[test] verdict={:?} response={}",
            verdict.kind, obs.final_response
        );
    }

    /// Verify that daemon-required cases get Skipped when daemon is not running.
    #[tokio::test]
    async fn daemon_required_case_skips_without_daemon() {
        if WorkflowEvalRunner::daemon_alive() {
            eprintln!("[SKIP] daemon is alive — test only valid when daemon is down");
            return;
        }

        use crate::workflow_eval::contracts::coding_contract;
        let case = WorkflowEvalCase {
            id: "wf-daemon-skip-test".to_string(),
            description: "Daemon required case".to_string(),
            prompt: "open vscode and write fibonacci".to_string(),
            category: EvalWorkflowCategory::Coding,
            contract: coding_contract(),
            safety_class: SafetyClass::Safe,
            interruption: None,
            timeout: std::time::Duration::from_secs(60),
            requires_daemon: true,
            requires_display: true,
            tags: vec![],
            eval_notes: String::new(),
        };

        let runner = WorkflowEvalRunner::default();
        let (obs, verdict) = runner.run(&case, None).await;

        assert!(!obs.daemon_alive_at_start);
        assert!(
            matches!(verdict.kind, WorkflowVerdictKind::Skip),
            "expected Skip verdict, got {:?}",
            verdict.kind
        );
    }

    /// VM-only: full daemon coding workflow (requires KRIA_EVAL_VM=1).
    #[tokio::test]
    async fn vm_only_daemon_coding_workflow() {
        if !WorkflowEvalRunner::vm_mode() {
            eprintln!("[SKIP] vm_only_daemon_coding_workflow: set KRIA_EVAL_VM=1 to run on VM");
            return;
        }
        // On VM with daemon: run through the full path.
        // This test verifies that live daemon cases actually execute via GoalTree.
        let case = make_nodisplay_coding_case(
            "wf-vm-coding-run-show",
            "run python3 -c 'print(42)' and show me the output",
        );
        let spec = nodisplay_write_run_spec(
            &case.prompt,
            "true", // no-op write
            "python3 -c 'print(42)'",
        );
        let runner = WorkflowEvalRunner { verbose: true };
        let (obs, verdict) = runner.run(&case, Some(spec)).await;
        assert!(
            obs.reported_success,
            "expected success on VM, errors: {:?}",
            obs.stage_errors
        );
        eprintln!(
            "[VM test] verdict={:?} response={}",
            verdict.kind, obs.final_response
        );
    }
}
