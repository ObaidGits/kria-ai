//! RFC 007 Phase 4 - AgentLoop to GuiExecutor Wiring
//!
//! This module provides the integration between the AgentLoop and the HTN GuiExecutor.
//! It detects GUI workflows from TurnGate and routes them appropriately.

use crate::agent::htn_executor::{
    GuiExecutor, GuiWorkflow, SafeAbortExecutor, ToolExecutor, WorkflowResult,
};
use crate::agent::htn_integration::{generate_gui_workflow, requires_gui_automation};
use crate::agent::turn_gate::{IntentEnvelope, Operation, TurnGatePlan};
use crate::tools::gui_automation::KillSwitchInterceptor;
use crate::tools::registry::ToolRegistry;
use crate::infra::ToolResult;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

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
}

impl GuiExecutionCoordinator {
    /// Create new coordinator with all required components.
    pub fn new(
        tool_registry: Arc<ToolRegistry>,
        kill_switch: Arc<KillSwitchInterceptor>,
    ) -> Self {
        let gui_backend = kill_switch.get_backend();
        
        // Create tool executor wrapper
        let tool_executor: Arc<dyn ToolExecutor> = 
            Arc::new(RegistryToolExecutor { registry: Arc::clone(&tool_registry) });
        
        // Create safe abort executor
        let abort_executor = SafeAbortExecutor::new(Arc::clone(&tool_executor));
        
        // Create GUI executor
        let executor = GuiExecutor::new(
            kill_switch.clone(),
            tool_executor,
            abort_executor,
        );
        
        Self {
            executor,
            tool_registry,
            gui_backend,
            kill_switch,
        }
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
    
    /// Generate or retrieve GUI workflow for the intent.
    ///
    /// Returns `Some(workflow)` only when the rule-based planner can produce a
    /// concrete, actionable plan. When the prompt looks GUI-bound but no
    /// concrete plan can be built, this returns `None` so the caller can route
    /// to the LLM HTN planner / ReAct loop instead of executing a trivial
    /// "discovery" stub that would falsely report success.
    pub fn generate_workflow(
        &self,
        task_id: &str,
        _intent: &IntentEnvelope,
        user_text: &str,
    ) -> Option<GuiWorkflow> {
        if let Some(workflow) = generate_gui_workflow(task_id, user_text) {
            return Some(workflow);
        }

        if requires_gui_automation(user_text) {
            tracing::warn!(
                target: "gui_wiring",
                task_id = %task_id,
                "GUI intent detected but rule-based planner produced no workflow; \
                 deferring to LLM HTN planner / ReAct loop instead of trivial discovery stub"
            );
        }

        None
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
        self.executor.execute_workflow(
            workflow,
            cancellation,
        ).await
    }
    
    /// Build a generic discovery workflow for unknown GUI tasks.
    fn build_discovery_workflow(&self, task_id: &str) -> GuiWorkflow {
        use crate::agent::htn_executor::{GuiWorkflowBuilder, VerificationType};
        
        GuiWorkflowBuilder::new(task_id)
            .max_duration(60)
            // Step 1: Get screen elements to understand current UI
            .add_step(
                1,
                "get_screen_elements",
                serde_json::json!({}),
                VerificationType::ElementsFound {
                    element_ids: vec![],
                    min_count: 0,
                },
            )
            .add_abort_step(
                "press_shortcut",
                serde_json::json!({"keys": ["Escape"]}),
            )
            .build()
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
    use crate::agent::turn_gate::{HazardHint, ComputeClass, IntentSource, Modality};
    
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
        
        assert!(GuiExecutionCoordinator::should_route_to_gui_executor(&gui_plan));
        
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
                residency: crate::agent::turn_gate::L1ResidencyRequirement::Auto 
            },
            direct_tool_hint: None,
            fallback_tool_hints: vec![],
        };
        
        assert!(!GuiExecutionCoordinator::should_route_to_gui_executor(&chat_plan));
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
