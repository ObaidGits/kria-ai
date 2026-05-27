//! RFC 007 Phase 4 - AgentLoop to GuiExecutor Wiring
//!
//! This module provides the integration between the AgentLoop and the HTN GuiExecutor.
//! It detects GUI workflows from TurnGate and routes them appropriately.

use crate::agent::collaborative_decision::{ActionProposal, DecisionStore};
use crate::agent::environment_grounder::EnvironmentGrounder;
use crate::agent::execution_gate::{ExecutionGate, ExecutionGateInput, ExecutionGateOutcome};
use crate::agent::gui_planner::{GuiPlanner, RuleBasedPlanner};
use crate::agent::htn_executor::{
    GuiExecutor, GuiWorkflow, SafeAbortExecutor, ToolExecutor, WorkflowResult,
};
use crate::agent::intent_compiler::GuiTaskSpec;
use crate::agent::psdg::env_tracker::EnvironmentStateTracker;
use crate::agent::turn_gate::{IntentEnvelope, Operation, TurnGatePlan};
use crate::infra::ToolResult;
use crate::routing::verbs;
use crate::safety::audit::{DecidedBy, Decision};
use crate::safety::hitl::{ApprovalResponse, HitlGateway};
use crate::safety::{AuditLogger, PolicyEngine};
use crate::tools::gui_automation::KillSwitchInterceptor;
use crate::tools::registry::ToolRegistry;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

// P3: GoalTree multi-stage workflow types
use crate::agent::environment_grounder::{GroundingCapabilities, OperationalFacts};
use crate::agent::execution_transparency::ExecutionTransparencyLayer;
use crate::agent::goal_tree::GoalTree;
use crate::agent::gui_lease::ForegroundLeaseManager;
use crate::agent::multi_intent::{
    DecompositionQuality, MultiIntentDecomposer, RuleBasedMultiIntentDecomposer,
};
use crate::agent::opgraph_compiler::GoalTreeOpGraphCompiler;
use crate::agent::psdg::PsdgHandle;
#[cfg(test)]
use crate::agent::resource_lease::ResourceLeaseRequest;
use crate::agent::resource_lease::{ResourceLeaseGuard, ResourceLeaseManager, ResourceRequirement};
use crate::agent::stage_executor::{GoalTreeResult, StageExecutor};
use crate::agent::workflow_compiler::{MultiVerbSpec, RuleBasedWorkflowCompiler, WorkflowCompiler};
use crate::agent::workflow_continuation::WorkflowContinuationRuntime;

fn workflow_needs_input_daemon(workflow: &GuiWorkflow) -> bool {
    workflow.sub_goals.iter().any(|step| {
        matches!(
            step.action.as_str(),
            "type_text" | "click_mouse" | "click_element" | "press_shortcut" | "focus_window"
        )
    })
}

fn goal_tree_needs_input_daemon(tree: &GoalTree) -> bool {
    tree.stages.iter().any(|stage| {
        stage.action_group.actions.iter().any(|action| {
            matches!(
                action.action.as_str(),
                "type_text" | "click_mouse" | "click_element" | "press_shortcut" | "focus_window"
            )
        })
    })
}

/// Validate that the tool registry has all critical GUI automation tools registered
/// with working `execute_with_context` implementations.
///
/// This is a startup self-test that catches the production/eval divergence where
/// a stale binary has tools registered with only `execute` (which returns
/// "tool does not implement execute") instead of `execute_with_context`.
///
/// Returns a list of tool names that failed validation. Empty = all good.
pub async fn validate_gui_tool_registry(registry: &ToolRegistry) -> Vec<String> {
    let critical_tools = [
        (
            "write_file",
            serde_json::json!({
                "path": "/tmp/kria_registry_selftest.txt",
                "content": "selftest"
            }),
        ),
        (
            "execute_bash",
            serde_json::json!({
                "command": "echo selftest",
                "timeout": 5
            }),
        ),
        (
            "open_application",
            serde_json::json!({
                "name": "__kria_selftest_nonexistent__"
            }),
        ),
    ];

    let mut failed = Vec::new();

    for (tool_name, params) in &critical_tools {
        if let Some(handler) = registry.get_handler(tool_name) {
            let ctx = registry.make_tool_context(tokio_util::sync::CancellationToken::new());
            let result = handler.execute_with_context(params.clone(), ctx).await;
            // The tool should NOT return "tool does not implement execute"
            // It may return an error (e.g., file permission, app not found) but
            // that's fine — we're testing the dispatch path, not the outcome.
            if result.error.as_deref() == Some("tool does not implement execute") {
                tracing::error!(
                    target: "gui_wiring",
                    tool = %tool_name,
                    "STARTUP VALIDATION FAILED: tool returns 'does not implement execute' — \
                     this indicates a stale binary or incorrect tool registration. \
                     Rebuild and redeploy the production binary."
                );
                failed.push(tool_name.to_string());
            } else {
                tracing::debug!(
                    target: "gui_wiring",
                    tool = %tool_name,
                    "Startup validation OK"
                );
            }
        } else {
            tracing::error!(
                target: "gui_wiring",
                tool = %tool_name,
                "STARTUP VALIDATION FAILED: tool not found in registry"
            );
            failed.push(tool_name.to_string());
        }
    }

    // Clean up selftest file
    let _ = std::fs::remove_file("/tmp/kria_registry_selftest.txt");

    if !failed.is_empty() {
        tracing::error!(
            target: "gui_wiring",
            failed_tools = ?failed,
            "GUI tool registry validation FAILED — GUI automation will not work correctly. \
             This is a production/eval divergence. Rebuild the binary."
        );
    } else {
        tracing::info!(
            target: "gui_wiring",
            "GUI tool registry validation passed — all critical tools registered correctly"
        );
    }

    failed
}

/// GUI execution coordinator that wires AgentLoop to GuiExecutor.
#[allow(dead_code)] // Fields reserved for future tool-based actions
pub struct GuiExecutionCoordinator {
    /// Tool registry for executing actions
    tool_registry: Arc<ToolRegistry>,
    /// GUI backend for input injection
    gui_backend: Arc<dyn crate::tools::gui_automation::GuiBackend>,
    /// Kill switch interceptor for safety
    kill_switch: Arc<KillSwitchInterceptor>,
    /// Policy engine for safety gating
    policy_engine: Arc<PolicyEngine>,
    /// HITL gateway for approvals
    hitl_gateway: Arc<HitlGateway>,
    /// Audit logger for execution decisions
    audit_logger: Arc<AuditLogger>,
    /// P2: Environment grounder for operational facts
    grounder: Arc<dyn EnvironmentGrounder>,
    /// PSDG: Observer that persists grounding facts to WorldModelStore.
    /// `None` when no PSDG handle is available (tests / degraded mode).
    env_tracker: Option<EnvironmentStateTracker>,
    /// Batch 2: WorkflowContinuationRuntime for interruption classification on
    /// GoalTree stage failure. Passed through to StageExecutor.
    continuation_runtime: Option<Arc<WorkflowContinuationRuntime>>,
    /// Batch 2: Transparency layer for GoalTree stage tracing.
    transparency: Option<ExecutionTransparencyLayer>,
    /// Batch 2: Raw PSDG handle for StageExecutor world-model persistence.
    psdg: Option<PsdgHandle>,
    /// GUI foreground lease. This is intentionally separate from the input
    /// daemon so KRIA can arbitrate GUI ownership without depending on ydotool.
    foreground_lease: ForegroundLeaseManager,
    /// Minimal generic resource leases for tool-bound side effects.
    resource_lease: ResourceLeaseManager,
    /// Durable collaborative decisions for ambiguity, approval, and recovery
    /// pauses. This is additive to `HitlGateway`; it gives recoverable waits a
    /// workflow-bound identity.
    decision_store: Arc<DecisionStore>,
}

impl GuiExecutionCoordinator {
    /// Create new coordinator with all required components.
    ///
    /// Defaults to a [`LiveEnvironmentGrounder`] so X11 window queries work
    /// out of the box; callers may override with [`Self::set_grounder`].
    pub fn new(
        tool_registry: Arc<ToolRegistry>,
        kill_switch: Arc<KillSwitchInterceptor>,
        policy_engine: Arc<PolicyEngine>,
        hitl_gateway: Arc<HitlGateway>,
        audit_logger: Arc<AuditLogger>,
    ) -> Self {
        let gui_backend = kill_switch.get_backend();

        // Default to a Live grounder so window/focus facts are available on
        // first call. The Live grounder probes capabilities lazily and falls
        // back gracefully if xdotool/wmctrl are missing.
        let grounder: Arc<dyn EnvironmentGrounder> =
            Arc::new(crate::agent::environment_grounder::LiveEnvironmentGrounder::new());

        Self {
            tool_registry,
            gui_backend,
            kill_switch,
            policy_engine,
            hitl_gateway,
            audit_logger,
            grounder,
            env_tracker: None,
            continuation_runtime: None,
            transparency: None,
            psdg: None,
            foreground_lease: ForegroundLeaseManager::new(),
            resource_lease: ResourceLeaseManager::global(),
            decision_store: Arc::new(DecisionStore::default_persistent()),
        }
    }

    /// Attach a custom collaborative decision store.
    pub fn with_decision_store(mut self, store: Arc<DecisionStore>) -> Self {
        self.decision_store = store;
        self
    }

    /// Attach a `WorkflowContinuationRuntime` for GoalTree stage interruption handling.
    ///
    /// When set, the `StageExecutor` will classify stage failures as interruptions,
    /// plan bounded recovery, and write pause checkpoints when human intervention
    /// is required.
    pub fn with_continuation_runtime(mut self, rt: Arc<WorkflowContinuationRuntime>) -> Self {
        self.continuation_runtime = Some(rt);
        self
    }

    /// Attach a transparency layer for real-time GoalTree stage tracing.
    pub fn with_transparency(mut self, layer: ExecutionTransparencyLayer) -> Self {
        self.transparency = Some(layer);
        self
    }

    /// Attach a raw PSDG handle for GoalTree stage outcome persistence.
    pub fn with_psdg(mut self, psdg: PsdgHandle) -> Self {
        self.psdg = Some(psdg);
        self
    }

    /// Attach a PSDG handle so grounding results are persisted to WorldModelStore.
    ///
    /// Once set, every `EnvironmentGrounder::ground()` call will feed its
    /// `OperationalFacts` into `EnvironmentStateTracker`, which computes a
    /// delta and issues fire-and-forget PSDG writes for changed values only.
    pub fn with_env_tracker(mut self, psdg: crate::agent::psdg::PsdgHandle) -> Self {
        self.env_tracker = Some(EnvironmentStateTracker::new(psdg));
        self
    }

    /// Set a custom grounder (e.g. LiveEnvironmentGrounder at app startup).
    pub fn set_grounder(&mut self, grounder: Arc<dyn EnvironmentGrounder>) {
        self.grounder = grounder;
    }

    fn build_tool_executor(
        &self,
        cancellation: CancellationToken,
        session_id: &str,
        user_text: &str,
    ) -> Arc<dyn ToolExecutor> {
        let modality = verbs::classify_modality(user_text);
        Arc::new(PolicyToolExecutor {
            registry: Arc::clone(&self.tool_registry),
            cancellation,
            policy_engine: Arc::clone(&self.policy_engine),
            hitl_gateway: Arc::clone(&self.hitl_gateway),
            audit_logger: Arc::clone(&self.audit_logger),
            session_id: session_id.to_string(),
            user_text: user_text.to_string(),
            destructive_hint: modality.destructive,
            decision_store: Arc::clone(&self.decision_store),
            resource_lease: self.resource_lease.clone(),
        })
    }

    fn build_gui_executor(&self, tool_executor: Arc<dyn ToolExecutor>) -> GuiExecutor {
        let abort_executor = SafeAbortExecutor::new(Arc::clone(&tool_executor));
        // Build the canonical verifier; inject GuiBackend when available so that
        // WindowState checks use the live window manager query.  This replaces the
        // previous #[allow(deprecated)] path and consolidates both execution paths
        // into a single BoundedExecutionVerifier implementation.
        let verifier = crate::agent::execution_verifier_bounded::BoundedExecutionVerifier::new()
            .with_gui_backend(Arc::clone(&self.gui_backend));
        let mut executor = GuiExecutor::with_verifier(
            self.kill_switch.clone(),
            tool_executor,
            abort_executor,
            Arc::new(verifier),
        );
        // Batch 2: wire continuation_runtime into HTN executor for interruption
        // classification on sub-goal verification failure.
        if let Some(ref rt) = self.continuation_runtime {
            executor = executor.with_continuation_runtime(Arc::clone(rt));
        }
        executor
    }

    /// Check if TurnGate plan should trigger GUI workflow execution.
    /// Per RFC 007: Route to GuiExecutor when intent requires GUI automation.
    ///
    /// RFC v2 (F12): GUI routing is now gated on intent confidence. A low-
    /// confidence `Automate` classification (`< MIN_GUI_INTENT_CONFIDENCE`)
    /// falls through to the regular ReAct path unless an explicit GUI tool
    /// hint is present. This avoids hijacking ambiguous prompts into a
    /// rigid HTN workflow that may not match user intent.
    pub fn should_route_to_gui_executor(plan: &TurnGatePlan) -> bool {
        /// Below this confidence we do NOT auto-route ambiguous prompts to GUI.
        const MIN_GUI_INTENT_CONFIDENCE: f32 = 0.6;

        // An explicit GUI tool hint always wins (user opted in deliberately).
        // This covers:
        //   - "open_application" → Automate
        //   - "browser_search"   → Search (but IS a GUI action — open browser)
        //   - "open_url"         → may be Search or Automate
        //   - "open_application_with_file" → Automate
        //   - click/type/shortcut → Automate
        let has_gui_tool_hint = plan
            .direct_tool_hint
            .as_ref()
            .map(|hint| {
                matches!(
                    hint.as_str(),
                    "click_mouse"
                        | "type_text"
                        | "press_shortcut"
                        | "get_screen_elements"
                        | "click_element"
                        | "open_application"
                        | "open_application_with_file"
                        | "browser_search"
                        | "open_url"
                )
            })
            .unwrap_or(false);

        if has_gui_tool_hint {
            return true;
        }

        // Operation-based routing now requires sufficient confidence.
        let is_gui_operation = matches!(
            plan.intent.operation,
            Operation::Automate | Operation::ConfigureSystem
        );

        if !is_gui_operation {
            return false;
        }

        if plan.intent.confidence >= MIN_GUI_INTENT_CONFIDENCE {
            true
        } else {
            tracing::info!(
                target: "gui_wiring",
                confidence = plan.intent.confidence,
                threshold = MIN_GUI_INTENT_CONFIDENCE,
                operation = ?plan.intent.operation,
                "Low-confidence GUI intent — deferring to ReAct loop instead of HTN executor"
            );
            false
        }
    }

    /// Generate GUI workflow from a compiled `GuiTaskSpec`.
    ///
    /// Returns `Some((workflow, artifacts))` only when a planner can produce a
    /// concrete, actionable plan. The artifacts list contains paths that the
    /// workflow will create — callers must pass these into `execute_workflow`
    /// so `WorkflowResult.created_artifacts` is populated correctly.
    ///
    /// The selection is **substrate-aware**:
    ///
    /// 1. The [`SubstratePlanner`] is consulted first. If it picks a concrete
    ///    substrate (file-write, app-open, browser-navigate, keystroke), the
    ///    workflow it emits is preferred — these substrates are verifiable on
    ///    both X11 and Wayland.
    /// 2. Otherwise we fall back to the existing rule-based planner.
    /// 3. If both decline, the caller should invoke the LLM HTN planner.
    pub async fn generate_workflow(
        &self,
        _task_id: &str,
        _intent: &IntentEnvelope,
        spec: &GuiTaskSpec,
        raw_user_text: &str,
    ) -> Option<(GuiWorkflow, Vec<std::path::PathBuf>)> {
        let semantic_analysis =
            crate::agent::semantic_workflow::analyze_semantic_workflow(spec, raw_user_text);
        let mode_decision = crate::agent::execution_mode_reasoner::ExecutionModeReasoner.decide(
            spec,
            &semantic_analysis,
            &crate::agent::execution_mode_reasoner::EnvironmentCapabilities::unchecked_default(),
            &crate::agent::execution_mode_reasoner::PolicyContext::default(),
        );
        let contract_check = crate::agent::workflow_intent_contract::WorkflowIntentContractRegistry
            .evaluate(&mode_decision, &semantic_analysis);
        let verifier_authority = crate::agent::verifier_authority::VerifierAuthorityEvaluator
            .assess(
                &contract_check,
                &mode_decision,
                &semantic_analysis,
                "live-gui-planning",
            );
        tracing::info!(
            target: "gui_workflow_intelligence",
            task_family = ?semantic_analysis.frame.task_family,
            requested_fidelity = ?semantic_analysis.fidelity.requested_fidelity,
            mode = ?mode_decision.mode,
            contract = ?mode_decision.workflow_contract_id,
            missing_contract_requirements = contract_check.missing_requirements.len(),
            verifier_requirements = verifier_authority.requirements.len(),
            "Semantic GUI contract resolved before substrate planning"
        );

        // Substrate-aware fast path: pick the right execution substrate before
        // touching any GUI heuristics. For "Open gedit and type a fibonacci
        // program in python" this returns a workflow that writes the file and
        // opens it directly — no keystroke automation, works on Wayland.
        let substrate_plan =
            crate::agent::gui_substrate_planner::SubstratePlanner.plan(spec, raw_user_text);
        if let Some(workflow) = substrate_plan.workflow {
            tracing::info!(
                target: "gui_wiring",
                substrate = ?substrate_plan.substrate,
                steps = workflow.sub_goals.len(),
                artifacts = substrate_plan.artifacts.len(),
                "Substrate-aware planner produced workflow"
            );
            // FIX #2: Return artifacts alongside the workflow so the caller
            // can populate WorkflowResult.created_artifacts correctly.
            return Some((workflow, substrate_plan.artifacts));
        }

        // P2: Ground operational facts before planning
        let facts = self.grounder.ground(&spec.targets).await;
        tracing::debug!(
            target: "gui_wiring",
            focused_app = ?facts.focused_app,
            visible_windows = facts.visible_windows.len(),
            capabilities = ?facts.capabilities,
            "grounding complete"
        );
        // PSDG: Persist grounding results to WorldModelStore as semantic deltas.
        // Fire-and-forget via PsdgHandle inside the tracker — non-blocking.
        if let Some(ref tracker) = self.env_tracker {
            tracker.track(&facts);
        }

        match RuleBasedPlanner.plan(spec, &facts).await {
            Ok(workflow) => {
                tracing::debug!(
                    target: "gui_wiring",
                    verb = ?spec.primary_verb,
                    "Rule-based planner produced workflow"
                );
                // Rule-based planner doesn't track artifacts — return empty list.
                Some((workflow, Vec::new()))
            }
            Err(e) => {
                tracing::info!(
                    target: "gui_wiring",
                    error = %e,
                    "Rule-based planner declined intent; caller should fall back to LLM"
                );
                None
            }
        }
    }

    /// Execute GUI workflow with full RFC 007 safety pipeline.
    ///
    /// `planned_artifacts`: paths that the workflow is expected to create,
    /// from `SubstratePlan.artifacts`. After execution, any of these that
    /// actually exist on disk are reported in `WorkflowResult.created_artifacts`.
    pub async fn execute_workflow(
        &mut self,
        workflow: &GuiWorkflow,
        cancellation: CancellationToken,
        planned_artifacts: Vec<std::path::PathBuf>,
        session_id: &str,
        user_text: &str,
    ) -> WorkflowResult {
        let needs_input_daemon = workflow_needs_input_daemon(workflow);
        let foreground_lease = if needs_input_daemon {
            match self
                .foreground_lease
                .acquire(
                    workflow.task_id.clone(),
                    "legacy-htn-gui-workflow",
                    Duration::from_secs(120),
                )
                .await
            {
                Ok(guard) => Some(guard),
                Err(err) => {
                    let reason = format!("GUI_FOREGROUND_LEASE_DENIED: {err}");
                    tracing::warn!(
                        target: "gui_wiring",
                        workflow_id = %workflow.task_id,
                        reason = %reason,
                        "Legacy GUI workflow blocked because foreground lease is unavailable"
                    );
                    return WorkflowResult {
                        task_id: workflow.task_id.clone(),
                        success: false,
                        completed_steps: 0,
                        total_steps: workflow.sub_goals.len(),
                        error: Some(reason),
                        aborted: true,
                        duration_ms: 0,
                        created_artifacts: Vec::new(),
                    };
                }
            }
        } else {
            None
        };

        // RFC 008: Start heartbeat task to keep uinput daemon alive.
        // Store the handle and abort it when the workflow completes to prevent
        // the task from leaking indefinitely (dropping a JoinHandle detaches
        // the task in Tokio — it does NOT cancel it).
        let heartbeat_task = if needs_input_daemon {
            let backend = Arc::clone(&self.gui_backend);
            let heartbeat_cancel = cancellation.clone();
            Some(tokio::spawn(async move {
                let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
                // reset() ensures the first tick fires after the full 2s interval rather
                // than immediately. Without this, tokio fires t=0 and we get a spurious
                // heartbeat-failed log before any stage has started executing.
                interval.reset();
                loop {
                    interval.tick().await;
                    if heartbeat_cancel.is_cancelled() {
                        break;
                    }
                    if let Err(e) = backend.send_heartbeat().await {
                        tracing::error!("RFC 008: Uinput daemon heartbeat failed: {}", e);
                        break;
                    }
                    tracing::debug!("RFC 008: Uinput daemon heartbeat sent");
                }
            }))
        } else {
            tracing::debug!(
                target: "gui_wiring",
                "Skipping uinput heartbeat for structural workflow"
            );
            None
        };

        // Execute through HTN executor (fetches window context dynamically)
        let tool_executor = self.build_tool_executor(cancellation.clone(), session_id, user_text);
        let mut executor = self.build_gui_executor(tool_executor);
        let mut result = executor
            .execute_workflow(workflow, cancellation.clone())
            .await;

        // Popup-aware recovery: if the workflow failed and a dialog is visible,
        // attempt to dismiss it and report the dialog as the root cause.
        // This prevents silent blocking when a "Save?", "Permission?", or
        // "Update?" dialog appears mid-workflow.
        if !result.success {
            if let Some(dialog_info) = Self::check_for_blocking_dialog().await {
                tracing::warn!(
                    target: "gui_wiring",
                    dialog_role = %dialog_info.0,
                    dialog_name = %dialog_info.1,
                    "Workflow failed — blocking dialog detected. Attempting dismissal."
                );
                // Attempt to dismiss the dialog
                let engine = crate::agent::atspi_engine::AtSpiEngine::new();
                let dismiss_result = engine.dismiss_dialog().await;
                if dismiss_result.success {
                    tracing::info!(
                        target: "gui_wiring",
                        "Dialog dismissed — workflow failure was caused by dialog interruption"
                    );
                    // Annotate the error with dialog context
                    if let Some(ref mut err) = result.error {
                        *err = format!(
                            "{} [Dialog interrupted workflow: '{}' ({}) — dismissed successfully]",
                            err, dialog_info.1, dialog_info.0
                        );
                    }
                } else {
                    tracing::warn!(
                        target: "gui_wiring",
                        dismiss_evidence = %dismiss_result.evidence,
                        "Dialog dismissal failed"
                    );
                    if let Some(ref mut err) = result.error {
                        *err = format!(
                            "{} [Dialog interrupted workflow: '{}' ({}) — could not dismiss: {}]",
                            err, dialog_info.1, dialog_info.0, dismiss_result.evidence
                        );
                    }
                }
            }
        }

        // Abort the heartbeat task now that the workflow has completed.
        if let Some(task) = heartbeat_task {
            task.abort();
        }
        if let Some(lease) = foreground_lease {
            lease.release().await;
        }

        // FIX #2: Populate created_artifacts from planned paths that exist on disk.
        if result.created_artifacts.is_empty() && !planned_artifacts.is_empty() {
            result.created_artifacts = planned_artifacts
                .into_iter()
                .filter(|p| p.exists())
                .collect();
            if !result.created_artifacts.is_empty() {
                tracing::info!(
                    target: "gui_wiring",
                    count = result.created_artifacts.len(),
                    "Populated created_artifacts from planned paths"
                );
            }
        }

        // Session persistence: save a checkpoint for long-horizon workflow support.
        // Pass None here — the loop engine will overwrite with the real user intent.
        // We save here as a safety net in case the loop engine path is not taken
        // (e.g., direct coordinator calls from tests or other callers).
        // The loop engine's save_session_checkpoint call with last_user_text takes precedence.
        Self::save_session_checkpoint(workflow, &result, None).await;

        result
    }

    /// Check if a blocking dialog is currently visible via AT-SPI.
    /// Only runs when AT-SPI is available (avoids overhead on every failure
    /// when AT-SPI is not running).
    async fn check_for_blocking_dialog() -> Option<(String, String)> {
        // Quick pre-check: is the AT-SPI bus socket available?
        // Use nix::unistd::getuid() for reliable UID, falling back to env var.
        let uid = unsafe { libc::getuid() };
        let atspi_socket = std::path::PathBuf::from(format!("/run/user/{}/at-spi/bus", uid));

        if !atspi_socket.exists() {
            // AT-SPI not running — skip dialog check entirely (0ms overhead)
            return None;
        }

        let engine = crate::agent::atspi_engine::AtSpiEngine::new();
        // Use a short 300ms timeout — dialog detection should be fast
        let result = tokio::time::timeout(
            tokio::time::Duration::from_millis(300),
            engine.detect_dialog(),
        )
        .await;
        match result {
            Ok(Some(el)) => Some((el.role, el.name)),
            _ => None,
        }
    }

    /// Save a session checkpoint for long-horizon workflow support.
    ///
    /// `user_intent`: the original user text, if available. Falls back to
    /// the workflow task_id if not provided.
    pub async fn save_session_checkpoint(
        workflow: &crate::agent::htn_executor::GuiWorkflow,
        result: &crate::agent::htn_executor::WorkflowResult,
        user_intent: Option<&str>,
    ) {
        use crate::agent::workflow_session::{SessionManager, SessionStep, WorkflowSession};

        let manager = SessionManager::new();

        // Clean up old sessions periodically (every ~50 saves, probabilistically)
        // to prevent unbounded accumulation.
        if rand::random::<u8>() < 5 {
            manager.cleanup_old_sessions(24); // Keep sessions for 24 hours
        }

        let intent = user_intent.unwrap_or(&workflow.task_id).to_string();

        let mut session =
            WorkflowSession::new(workflow.task_id.clone(), intent, "unknown".to_string());

        // Add completed steps
        for (_i, sub_goal) in workflow.sub_goals.iter().enumerate() {
            let step_success = sub_goal.step <= result.completed_steps;
            session.add_step(SessionStep {
                step: sub_goal.step,
                action: sub_goal.action.clone(),
                params: sub_goal.params.clone(),
                success: step_success,
                evidence: if step_success {
                    "completed".to_string()
                } else {
                    result.error.clone().unwrap_or_else(|| "failed".to_string())
                },
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            });
        }

        if result.success {
            let artifacts: Vec<String> = result
                .created_artifacts
                .iter()
                .map(|p| p.display().to_string())
                .collect();
            session.mark_complete(artifacts);
        } else {
            let continuation_hint = if result.completed_steps > 0 {
                Some(format!(
                    "Workflow partially completed ({}/{} steps). Retry from step {}.",
                    result.completed_steps,
                    result.total_steps,
                    result.completed_steps + 1
                ))
            } else {
                None
            };
            session.mark_failed(
                result
                    .error
                    .clone()
                    .unwrap_or_else(|| "unknown error".to_string()),
                continuation_hint,
            );
        }

        if let Err(e) = manager.save(&session) {
            tracing::debug!(
                target: "gui_wiring",
                error = %e,
                "Session checkpoint save failed (non-critical)"
            );
        }
    }

    /// Generate an OpGraph-based GoalTree for multi-intent workflows.
    ///
    /// Uses the MultiIntentDecomposer to produce an OpGraph, then compiles
    /// into GoalTree via GoalTreeOpGraphCompiler. This is planning-only; no
    /// execution occurs here.
    pub async fn generate_opgraph_workflow(
        &self,
        user_text: &str,
        intent: &IntentEnvelope,
    ) -> Option<GoalTree> {
        let decomposer = RuleBasedMultiIntentDecomposer::new();
        let decomposition = decomposer.decompose(user_text, intent).await;

        if matches!(decomposition.quality, DecompositionQuality::SingleIntent) {
            return None;
        }

        if decomposition.opgraph.nodes.len() < 2 {
            return None;
        }

        // P1: Substrate-first routing guard. For multi-intent prompts that a reliable
        // file/terminal substrate can handle (e.g. "open code + write + run"), return None
        // so the caller falls through to generate_workflow(), which invokes SubstratePlanner.
        // This avoids the fragile WindowFocused GoalTree path on Wayland or with daemon down.
        if let Some(first_clause) = decomposition.clauses.first() {
            if let Some(ref gui_intent) = first_clause.gui_intent {
                let all_targets: Vec<_> = decomposition
                    .clauses
                    .iter()
                    .filter_map(|c| c.gui_intent.as_ref())
                    .flat_map(|gi| gi.targets.iter().cloned())
                    .collect();
                let content = decomposition
                    .clauses
                    .iter()
                    .filter_map(|c| c.gui_intent.as_ref())
                    .find_map(|gi| gi.content.clone());
                let synth_spec = GuiTaskSpec {
                    primary_verb: gui_intent.verb.clone(),
                    targets: all_targets,
                    content,
                    declared_preconditions: Vec::new(),
                    declared_success_criteria: Vec::new(),
                    ambiguities: Vec::new(),
                };
                let substrate_plan = crate::agent::gui_substrate_planner::SubstratePlanner
                    .plan(&synth_spec, user_text);
                if substrate_plan.substrate
                    != crate::agent::gui_substrate_planner::ExecutionSubstrate::Unknown
                {
                    tracing::info!(
                        target: "gui_wiring",
                        substrate = ?substrate_plan.substrate,
                        "P1: SubstratePlanner handles multi-intent prompt — deferring to generate_workflow()"
                    );
                    return None;
                }
            }
        }

        let mut targets = Vec::new();
        for clause in &decomposition.clauses {
            if let Some(gui_intent) = &clause.gui_intent {
                targets.extend(gui_intent.targets.clone());
            }
        }

        let facts = if targets.is_empty() {
            OperationalFacts::empty(GroundingCapabilities::none())
        } else {
            self.grounder.ground(&targets).await
        };

        let compiler = GoalTreeOpGraphCompiler;
        match compiler.compile(&decomposition.opgraph, Some(&facts)) {
            Ok(tree) => Some(tree),
            Err(e) => {
                tracing::info!(
                    target: "gui_wiring",
                    error = %e,
                    "OpGraph compilation declined; falling back to existing planners"
                );
                None
            }
        }
    }

    // ================================================================
    // P3: Multi-stage GoalTree workflow path
    // ================================================================

    /// Compile a multi-verb specification into a GoalTree.
    ///
    /// This is the P3 entry point for multi-stage workflows. The coordinator:
    /// 1. Grounds operational facts via the existing grounder
    /// 2. Compiles the MultiVerbSpec into a GoalTree via RuleBasedWorkflowCompiler
    /// 3. Returns the compiled GoalTree for execution
    ///
    /// If compilation fails (single verb, unsupported pattern, etc.), returns None.
    /// The caller should fall back to the existing single-verb GuiPlanner path.
    ///
    /// **Invariant**: This method does NOT execute. It only compiles.
    pub async fn generate_multi_stage_workflow(&self, spec: &MultiVerbSpec) -> Option<GoalTree> {
        // Ground operational facts for advisory context hints
        // Extract targets from all clauses for grounder relevance filtering
        let all_targets: Vec<_> = spec
            .clauses
            .iter()
            .flat_map(|c| c.targets.iter().cloned())
            .collect();
        let facts = self.grounder.ground(&all_targets).await;

        tracing::debug!(
            target: "gui_wiring",
            clauses = spec.clauses.len(),
            focused_app = ?facts.focused_app,
            "P3: Grounding complete for multi-verb workflow"
        );

        // Compile via deterministic rule-based compiler
        let compiler = RuleBasedWorkflowCompiler;
        match compiler.compile(spec, &facts) {
            Ok(tree) => {
                tracing::info!(
                    target: "gui_wiring",
                    workflow_id = %tree.workflow_id,
                    stages = tree.stages.len(),
                    "P3: GoalTree compiled successfully"
                );
                Some(tree)
            }
            Err(e) => {
                tracing::info!(
                    target: "gui_wiring",
                    error = %e,
                    "P3: WorkflowCompiler declined multi-verb spec; fall back to single-verb path"
                );
                None
            }
        }
    }

    /// Execute a compiled GoalTree via the StageExecutor.
    ///
    /// This wraps the StageExecutor with:
    /// - Heartbeat task for uinput daemon (same as existing execute_workflow)
    /// - Cancellation propagation
    ///
    /// **Invariant**: The GoalTree is borrowed immutably. The StageExecutor
    /// never calls planners, never mutates the tree, never invents stages.
    pub async fn execute_goal_tree(
        &self,
        tree: &GoalTree,
        cancellation: CancellationToken,
        session_id: &str,
        user_text: &str,
    ) -> GoalTreeResult {
        // RFC 008: Start heartbeat task (same pattern as existing execute_workflow).
        // AUDIT FIX #7: Store the handle and abort it after execution completes.
        // Previously used `let _heartbeat_task = ...` which drops the handle
        // immediately, detaching the task so it runs forever.
        let heartbeat_task = if goal_tree_needs_input_daemon(tree) {
            let backend = Arc::clone(&self.gui_backend);
            let heartbeat_cancel = cancellation.clone();
            Some(tokio::spawn(async move {
                let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
                // reset() delays the first tick by the full interval so we do not attempt
                // a heartbeat before the first workflow stage has started executing.
                interval.reset();
                loop {
                    interval.tick().await;
                    if heartbeat_cancel.is_cancelled() {
                        break;
                    }
                    if let Err(e) = backend.send_heartbeat().await {
                        tracing::error!("P3: Uinput daemon heartbeat failed: {}", e);
                        break;
                    }
                    tracing::debug!("P3: Uinput daemon heartbeat sent");
                }
            }))
        } else {
            tracing::debug!(
                target: "gui_wiring",
                workflow_id = %tree.workflow_id,
                "Skipping uinput heartbeat for structural GoalTree"
            );
            None
        };

        // Create StageExecutor using the same ToolExecutor as the existing executor,
        // but with the workflow-level cancellation token so tools can be cancelled.
        let tool_executor = self.build_tool_executor(cancellation.clone(), session_id, user_text);
        // Use canonical verifier for GoalTree execution path.
        // Inject gui_backend so WindowFocused checkpoints use the live window
        // manager query instead of falling back to AT-SPI / xdotool.
        let verifier: Arc<dyn crate::agent::execution_verifier::ExecutionVerifier> = Arc::new(
            crate::agent::execution_verifier_bounded::BoundedExecutionVerifier::new()
                .with_gui_backend(Arc::clone(&self.gui_backend)),
        );

        // Batch 2: wire continuation_runtime, transparency, and psdg into
        // StageExecutor so GoalTree stage failures get interruption classification,
        // transparency traces, and PSDG outcome persistence.
        let mut stage_executor = StageExecutor::new(tool_executor, verifier);
        stage_executor = stage_executor.with_foreground_lease(self.foreground_lease.clone());
        if let Some(ref rt) = self.continuation_runtime {
            stage_executor = stage_executor.with_continuation_runtime(Arc::clone(rt));
        }
        if let Some(ref t) = self.transparency {
            stage_executor = stage_executor.with_transparency(t.clone());
        }
        if let Some(ref p) = self.psdg {
            stage_executor = stage_executor.with_world_model(p.clone());
        }

        let result = stage_executor.execute_goal_tree(tree, cancellation).await;

        // Abort the heartbeat task now that execution has completed.
        // This mirrors the pattern in execute_workflow and prevents the task
        // from running indefinitely after the GoalTree finishes.
        if let Some(task) = heartbeat_task {
            task.abort();
        }

        result
    }
}

/// Tool executor wrapper that enforces policy + HITL + execution authority.
struct PolicyToolExecutor {
    registry: Arc<ToolRegistry>,
    /// Workflow-level cancellation token threaded through to tool contexts.
    /// This allows running tools to be cancelled when the workflow is cancelled.
    cancellation: tokio_util::sync::CancellationToken,
    policy_engine: Arc<PolicyEngine>,
    hitl_gateway: Arc<HitlGateway>,
    audit_logger: Arc<AuditLogger>,
    session_id: String,
    user_text: String,
    destructive_hint: bool,
    decision_store: Arc<DecisionStore>,
    resource_lease: ResourceLeaseManager,
}

async fn acquire_required_resources(
    manager: &ResourceLeaseManager,
    action: &str,
    proposal: &ActionProposal,
    requirements: &[ResourceRequirement],
) -> Result<Vec<ResourceLeaseGuard>, String> {
    manager
        .acquire_requirements(action, proposal, requirements)
        .await
        .map_err(|error| format!("RESOURCE_LEASE_DENIED: {error}"))
}

#[async_trait::async_trait]
impl ToolExecutor for PolicyToolExecutor {
    async fn execute(&self, action: &str, params: &serde_json::Value) -> ToolResult {
        let gate = ExecutionGate::new(
            Arc::clone(&self.policy_engine),
            Arc::clone(&self.decision_store),
        );
        let gate_evaluation = gate.evaluate(ExecutionGateInput {
            session_id: &self.session_id,
            user_text: &self.user_text,
            action,
            params,
            destructive_hint: self.destructive_hint,
        });

        let decision = gate_evaluation.policy_decision.clone();
        let action_proposal = gate_evaluation.action_proposal.clone();
        let resource_requirements = gate_evaluation.resource_requirements.clone();
        match gate_evaluation.outcome {
            ExecutionGateOutcome::Block { reason } => {
                if let Some(policy_decision) = &decision {
                    if policy_decision.blocked {
                        self.audit_logger.log(
                            &self.session_id,
                            action,
                            params,
                            policy_decision.risk_level,
                            Decision::Blocked,
                            DecidedBy::Hardcoded,
                        );
                    }
                }
                tracing::warn!(
                    target: "authority_trace",
                    action = action,
                    reason = %reason,
                    "execution gate blocked tool"
                );
                return ToolResult::err(reason);
            }
            ExecutionGateOutcome::PauseForDecision {
                decision_id,
                decision_type,
                reason,
            } => {
                tracing::info!(
                    target: "authority_trace",
                    action = action,
                    reason = %reason,
                    "execution gate paused for decision"
                );
                return ToolResult::err_with_data(
                    format!("DECISION_PAUSED: {reason}"),
                    serde_json::json!({
                        "decision_id": decision_id,
                        "decision_type": decision_type,
                        "reason": reason,
                    }),
                );
            }
            ExecutionGateOutcome::RequiresApproval {
                decision: durable_decision,
            } => {
                let Some(decision) = decision else {
                    return ToolResult::err(
                        "EXECUTION_GATE_ERROR: approval requested without policy decision",
                    );
                };
                let request_id = HitlGateway::generate_request_id();
                let approval = self
                    .hitl_gateway
                    .request_approval_with_id(
                        &request_id,
                        action,
                        params.clone(),
                        decision.risk_level,
                        &format!("Execute {} with params: {}", action, params),
                        true,
                    )
                    .await;

                let (audit_decision, decided_by, approved, denial_reason) = match approval {
                    ApprovalResponse::Approved => {
                        (Decision::Approved, DecidedBy::UserGui, true, "")
                    }
                    ApprovalResponse::Denied => (
                        Decision::Denied,
                        DecidedBy::UserGui,
                        false,
                        "denied by user",
                    ),
                    ApprovalResponse::Timeout => (
                        Decision::Timeout,
                        DecidedBy::Timeout,
                        false,
                        "approval timed out",
                    ),
                };
                let store_result = if approved {
                    self.decision_store.resolve_with_version(
                        &durable_decision.id,
                        durable_decision.version,
                        "approve",
                        "user_gui",
                    )
                } else if matches!(approval, ApprovalResponse::Timeout) {
                    self.decision_store.expire(&durable_decision.id, "timeout")
                } else {
                    self.decision_store.resolve_with_version(
                        &durable_decision.id,
                        durable_decision.version,
                        "deny",
                        "user_gui",
                    )
                };
                if let Err(error) = store_result {
                    tracing::warn!(
                        target: "authority_trace",
                        action = action,
                        decision_id = %durable_decision.id,
                        error = %error,
                        "failed to update durable HITL decision"
                    );
                }
                self.audit_logger.log(
                    &self.session_id,
                    action,
                    params,
                    decision.risk_level,
                    audit_decision,
                    decided_by,
                );

                if !approved {
                    tracing::warn!(
                        target: "authority_trace",
                        action = action,
                        reason = denial_reason,
                        "HITL denied tool execution"
                    );
                    return ToolResult::err(format!("HITL_DENIED: {denial_reason}"));
                }
            }
            ExecutionGateOutcome::Proceed => {
                let Some(decision) = decision else {
                    return ToolResult::err(
                        "EXECUTION_GATE_ERROR: proceed returned without policy decision",
                    );
                };
                self.audit_logger.log(
                    &self.session_id,
                    action,
                    params,
                    decision.risk_level,
                    Decision::AutoExecuted,
                    DecidedBy::Policy,
                );
            }
        }

        let lease_guards = if resource_requirements.is_empty() {
            Vec::new()
        } else {
            let Some(action_proposal) = action_proposal.as_ref() else {
                return ToolResult::err(
                    "EXECUTION_GATE_ERROR: resource requirements missing action proposal",
                );
            };
            match acquire_required_resources(
                &self.resource_lease,
                action,
                action_proposal,
                &resource_requirements,
            )
            .await
            {
                Ok(guards) => guards,
                Err(reason) => {
                    tracing::warn!(
                        target: "authority_trace",
                        action = action,
                        reason = %reason,
                        "resource lease blocked tool execution"
                    );
                    return ToolResult::err(reason);
                }
            }
        };

        if let Some(handler) = self.registry.get_handler(action) {
            // AUDIT FIX #10/#24: Use the workflow-level cancellation token instead
            // of a fresh unconnected token. This allows running tools to be
            // cancelled when the workflow is cancelled.
            let ctx = self.registry.make_tool_context(self.cancellation.clone());
            let result = handler.execute_with_context(params.clone(), ctx).await;
            for guard in lease_guards {
                guard.release().await;
            }
            result
        } else {
            for guard in lease_guards {
                guard.release().await;
            }
            ToolResult::err(format!("Tool '{}' not found in registry", action))
        }
    }
}

/// Extension trait for KillSwitchInterceptor to access backend.
pub trait KillSwitchBackendExt {
    fn get_backend(&self) -> Arc<dyn crate::tools::gui_automation::GuiBackend>;
}

// ============================================================================
// Integration Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::resource_lease::{AccessMode, ResourceKind};
    use crate::agent::turn_gate::{ComputeClass, HazardHint, IntentSource, Modality};

    #[test]
    fn test_should_route_to_gui_executor() {
        // GUI operation should route
        let gui_plan = TurnGatePlan {
            intent: IntentEnvelope {
                modality: Modality::Text,
                operation: Operation::Automate,
                hazard_hint: HazardHint::Red,
                compute: ComputeClass::ReflexRust,
                confidence: 0.9,
                source: IntentSource::DeterministicGuard,
            },
            resource_plan: crate::agent::turn_gate::ResourcePlan::ToolOnly,
            direct_tool_hint: None,
            fallback_tool_hints: vec![],
        };

        assert!(GuiExecutionCoordinator::should_route_to_gui_executor(
            &gui_plan
        ));

        // Non-GUI operation should not route
        let chat_plan = TurnGatePlan {
            intent: IntentEnvelope {
                modality: Modality::Text,
                operation: Operation::Converse,
                hazard_hint: HazardHint::Green,
                compute: ComputeClass::L1Text,
                confidence: 0.9,
                source: IntentSource::DeterministicGuard,
            },
            resource_plan: crate::agent::turn_gate::ResourcePlan::L1Text {
                residency: crate::agent::turn_gate::L1ResidencyRequirement::Auto,
            },
            direct_tool_hint: None,
            fallback_tool_hints: vec![],
        };

        assert!(!GuiExecutionCoordinator::should_route_to_gui_executor(
            &chat_plan
        ));
    }

    #[test]
    fn test_gui_tool_hint_routing() {
        let plan = TurnGatePlan {
            intent: IntentEnvelope {
                modality: Modality::Text,
                operation: Operation::Write,
                hazard_hint: HazardHint::Yellow,
                compute: ComputeClass::ToolOnly,
                confidence: 0.8,
                source: IntentSource::FastEmbedSemanticRouter,
            },
            resource_plan: crate::agent::turn_gate::ResourcePlan::ToolOnly,
            direct_tool_hint: Some("click_mouse".to_string()),
            fallback_tool_hints: vec![],
        };

        assert!(GuiExecutionCoordinator::should_route_to_gui_executor(&plan));
    }

    /// End-to-end GUI Automation eval: verifies that the registry-backed tool
    /// executor can actually execute the substrate planner's `write_file` step.
    ///
    /// This is the regression test for the "tool does not implement execute"
    /// failure: tools like `write_file` only implement `execute_with_context`,
    /// so the executor must dispatch via that path.
    #[tokio::test]
    async fn eval_registry_executor_runs_write_file() {
        use crate::tools::registry::ToolRegistry;
        let reg = Arc::new(ToolRegistry::new());
        crate::tools::file_ops::register(&reg);

        let policy_engine = Arc::new(PolicyEngine::new());
        let hitl_gateway = Arc::new(HitlGateway::new(0));
        let audit_logger = Arc::new(AuditLogger::new(
            rusqlite::Connection::open_in_memory().expect("open in-memory audit db"),
        ));
        let executor = PolicyToolExecutor {
            registry: Arc::clone(&reg),
            cancellation: tokio_util::sync::CancellationToken::new(),
            policy_engine,
            hitl_gateway,
            audit_logger,
            session_id: "gui_wiring_test".to_string(),
            user_text: "write a file".to_string(),
            destructive_hint: false,
            decision_store: Arc::new(DecisionStore::in_memory()),
            resource_lease: ResourceLeaseManager::new(),
        };

        // Write to a path under ~/.kria/generated (the same location the
        // substrate planner uses) so the test is hermetic and respects any
        // sandbox filesystem restrictions.
        let dir = crate::agent::gui_substrate_planner::generated_files_dir();
        let path = dir.join(format!("kria-eval-write-{}.txt", uuid::Uuid::new_v4()));
        let path_str = path.to_string_lossy().to_string();

        let result = executor
            .execute(
                "write_file",
                &serde_json::json!({
                    "path": path_str.clone(),
                    "content": "fibonacci eval payload",
                }),
            )
            .await;

        assert!(
            result.success,
            "write_file via PolicyToolExecutor failed: {}",
            result.error.as_deref().unwrap_or("")
        );

        let bytes = std::fs::read(&path).expect("file should exist after write_file");
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("fibonacci eval payload"));

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn policy_tool_executor_blocks_when_required_resource_is_leased() {
        use crate::tools::registry::ToolRegistry;
        let reg = Arc::new(ToolRegistry::new());
        crate::tools::file_ops::register(&reg);

        let lease_manager = ResourceLeaseManager::new();
        let path = crate::agent::gui_substrate_planner::generated_files_dir()
            .join(format!("kria-lease-conflict-{}.txt", uuid::Uuid::new_v4()));
        let path_str = path.to_string_lossy().to_string();
        let _held = lease_manager
            .acquire(ResourceLeaseRequest {
                workflow_id: "other-workflow".to_string(),
                stage_id: Some("stage-1".to_string()),
                action_hash: "other-action".to_string(),
                kind: ResourceKind::FilesystemPath,
                scope: path_str.clone(),
                access_mode: AccessMode::Write,
                owner: "test".to_string(),
                ttl: Duration::from_secs(30),
                preemptible: false,
            })
            .await
            .expect("held lease");

        let executor = PolicyToolExecutor {
            registry: Arc::clone(&reg),
            cancellation: tokio_util::sync::CancellationToken::new(),
            policy_engine: Arc::new(PolicyEngine::new()),
            hitl_gateway: Arc::new(HitlGateway::new(0)),
            audit_logger: Arc::new(AuditLogger::new(
                rusqlite::Connection::open_in_memory().expect("open in-memory audit db"),
            )),
            session_id: "gui_wiring_test".to_string(),
            user_text: "write a file".to_string(),
            destructive_hint: false,
            decision_store: Arc::new(DecisionStore::in_memory()),
            resource_lease: lease_manager,
        };

        let result = executor
            .execute(
                "write_file",
                &serde_json::json!({
                    "path": path_str,
                    "content": "should not be written",
                }),
            )
            .await;

        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("RESOURCE_LEASE_DENIED"));
        assert!(!path.exists());
    }

    /// End-to-end GUI Automation eval: simulates "Open gedit and type a
    /// fibonacci program in python" and verifies the substrate planner emits a
    /// 2-step workflow whose first step (`write_file`) actually executes
    /// successfully through the real registry.
    #[tokio::test]
    async fn eval_substrate_workflow_writes_fibonacci_file() {
        use crate::agent::gui_substrate_planner::{ExecutionSubstrate, SubstratePlanner};
        use crate::agent::intent_compiler::{ContentClass, GuiTaskSpec, TargetRef, Verb};
        use crate::tools::registry::ToolRegistry;

        let reg = Arc::new(ToolRegistry::new());
        crate::tools::file_ops::register(&reg);

        let spec = GuiTaskSpec {
            primary_verb: Verb::Open,
            targets: vec![TargetRef::App("gedit".into())],
            content: Some(ContentClass::Generated {
                hint: "program to print fibonacci series".into(),
                language: Some("python".into()),
            }),
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        };

        let plan = SubstratePlanner.plan(
            &spec,
            "Open gedit and type a program to print fibonacci series in python",
        );
        assert_eq!(plan.substrate, ExecutionSubstrate::FileWriteThenOpen);
        let workflow = plan.workflow.expect("substrate must produce workflow");
        assert_eq!(workflow.sub_goals.len(), 2);

        // Step 1 must be write_file
        let step1 = &workflow.sub_goals[0];
        assert_eq!(step1.action, "write_file");

        // Execute step 1 via the registry executor (the path that was failing)
        let policy_engine = Arc::new(PolicyEngine::new());
        let hitl_gateway = Arc::new(HitlGateway::new(0));
        let audit_logger = Arc::new(AuditLogger::new(
            rusqlite::Connection::open_in_memory().expect("open in-memory audit db"),
        ));
        let executor = PolicyToolExecutor {
            registry: Arc::clone(&reg),
            cancellation: tokio_util::sync::CancellationToken::new(),
            policy_engine,
            hitl_gateway,
            audit_logger,
            session_id: "gui_wiring_test".to_string(),
            user_text: "write a file".to_string(),
            destructive_hint: false,
            decision_store: Arc::new(DecisionStore::in_memory()),
            resource_lease: ResourceLeaseManager::new(),
        };
        let result = executor.execute(&step1.action, &step1.params).await;
        assert!(
            result.success,
            "write_file step must succeed but got: {}",
            result.error.as_deref().unwrap_or("")
        );

        // Verify the artifact actually exists and contains fibonacci code
        let artifact = plan
            .artifacts
            .first()
            .expect("substrate plan should track the generated file path");
        let bytes = std::fs::read(artifact).expect("artifact must exist after step 1");
        let content = String::from_utf8_lossy(&bytes);
        assert!(
            content.contains("def fibonacci"),
            "generated file should contain fibonacci code, got: {}",
            &content[..content.len().min(200)]
        );

        let _ = std::fs::remove_file(artifact);
    }
}
