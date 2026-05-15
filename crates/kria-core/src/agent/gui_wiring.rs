//! RFC 007 Phase 4 - AgentLoop to GuiExecutor Wiring
//!
//! This module provides the integration between the AgentLoop and the HTN GuiExecutor.
//! It detects GUI workflows from TurnGate and routes them appropriately.

use crate::agent::environment_grounder::{EnvironmentGrounder, NoopEnvironmentGrounder};
use crate::agent::gui_planner::{GuiPlanner, RuleBasedPlanner};
use crate::agent::htn_executor::{
    GuiExecutor, GuiWorkflow, SafeAbortExecutor, ToolExecutor, WorkflowResult,
};
use crate::agent::intent_compiler::GuiTaskSpec;
use crate::agent::turn_gate::{IntentEnvelope, Operation, TurnGatePlan};
use crate::infra::ToolResult;
use crate::tools::gui_automation::KillSwitchInterceptor;
use crate::tools::registry::ToolRegistry;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

// P3: GoalTree multi-stage workflow types
use crate::agent::goal_tree::GoalTree;
use crate::agent::stage_executor::{GoalTreeResult, StageExecutor};
use crate::agent::workflow_compiler::{MultiVerbSpec, RuleBasedWorkflowCompiler, WorkflowCompiler};

/// GUI execution coordinator that wires AgentLoop to GuiExecutor.
#[allow(dead_code)] // Fields reserved for future tool-based actions
pub struct GuiExecutionCoordinator {
    /// The HTN executor for GUI workflows
    executor: GuiExecutor,
    /// Tool registry for executing actions
    tool_registry: Arc<ToolRegistry>,
    /// GUI backend for input injection
    gui_backend: Arc<dyn crate::tools::gui_automation::GuiBackend>,
    /// Kill switch interceptor for safety
    kill_switch: Arc<KillSwitchInterceptor>,
    /// P2: Environment grounder for operational facts
    grounder: Arc<dyn EnvironmentGrounder>,
}

impl GuiExecutionCoordinator {
    /// Create new coordinator with all required components.
    pub fn new(tool_registry: Arc<ToolRegistry>, kill_switch: Arc<KillSwitchInterceptor>) -> Self {
        let gui_backend = kill_switch.get_backend();

        // Create tool executor wrapper
        let tool_executor: Arc<dyn ToolExecutor> = Arc::new(RegistryToolExecutor {
            registry: Arc::clone(&tool_registry),
        });

        // Create safe abort executor
        let abort_executor = SafeAbortExecutor::new(Arc::clone(&tool_executor));

        // Create GUI executor
        let executor = GuiExecutor::new(kill_switch.clone(), tool_executor, abort_executor);

        Self {
            executor,
            tool_registry,
            gui_backend,
            kill_switch,
            grounder: Arc::new(NoopEnvironmentGrounder),
        }
    }

    /// Set a custom grounder (e.g. LiveEnvironmentGrounder at app startup).
    pub fn set_grounder(&mut self, grounder: Arc<dyn EnvironmentGrounder>) {
        self.grounder = grounder;
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
    /// Returns `Some(workflow)` only when the rule-based planner can produce a
    /// concrete, actionable plan. On failure the caller falls back to the
    /// LLM HTN planner.
    pub async fn generate_workflow(
        &self,
        _task_id: &str,
        _intent: &IntentEnvelope,
        spec: &GuiTaskSpec,
    ) -> Option<GuiWorkflow> {
        // P2: Ground operational facts before planning
        let facts = self.grounder.ground(&spec.targets).await;
        tracing::debug!(
            target: "gui_wiring",
            focused_app = ?facts.focused_app,
            visible_windows = facts.visible_windows.len(),
            capabilities = ?facts.capabilities,
            "grounding complete"
        );

        match RuleBasedPlanner.plan(spec, &facts).await {
            Ok(workflow) => {
                tracing::debug!(
                    target: "gui_wiring",
                    verb = ?spec.primary_verb,
                    "Rule-based planner produced workflow"
                );
                Some(workflow)
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
    pub async fn execute_workflow(
        &mut self,
        workflow: &GuiWorkflow,
        cancellation: CancellationToken,
    ) -> WorkflowResult {
        // RFC 008: Start heartbeat task to keep uinput daemon alive
        let backend = Arc::clone(&self.gui_backend);
        let heartbeat_cancel = cancellation.clone();
        let _heartbeat_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
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
        });

        // Execute through HTN executor (fetches window context dynamically)
        self.executor.execute_workflow(workflow, cancellation).await
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
    ) -> GoalTreeResult {
        // RFC 008: Start heartbeat task (same pattern as existing execute_workflow)
        let backend = Arc::clone(&self.gui_backend);
        let heartbeat_cancel = cancellation.clone();
        let _heartbeat_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
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
        });

        // Create StageExecutor using the same ToolExecutor as the existing executor
        let tool_executor: Arc<dyn ToolExecutor> = Arc::new(RegistryToolExecutor {
            registry: Arc::clone(&self.tool_registry),
        });
        let verifier: Arc<dyn crate::agent::execution_verifier::ExecutionVerifier> =
            Arc::new(crate::agent::execution_verifier_impl::BoundedExecutionVerifier::new());

        let stage_executor = StageExecutor::new(tool_executor, verifier);

        stage_executor.execute_goal_tree(tree, cancellation).await
    }
}

/// Tool executor wrapper for registry.
struct RegistryToolExecutor {
    registry: Arc<ToolRegistry>,
}

#[async_trait::async_trait]
impl ToolExecutor for RegistryToolExecutor {
    async fn execute(&self, action: &str, params: &serde_json::Value) -> ToolResult {
        if let Some(handler) = self.registry.get_handler(action) {
            handler.execute(params.clone()).await
        } else {
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
}
