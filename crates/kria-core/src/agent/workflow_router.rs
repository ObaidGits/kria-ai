//! Workflow Runtime Router — Canonical Dispatch Authority.
//!
//! This module is the SINGLE dispatch authority for all workflow execution.
//! It receives a classified intent and routes it to the appropriate runtime:
//!
//! - Canonical workflow runtime (HybridWorkflowExecutor)
//! - Legacy compatibility runtime (existing loop_engine GUI path)
//! - ReAct fallback (non-GUI intents)
//!
//! # Migration Strategy
//!
//! The router supports progressive migration via `RuntimeMode`:
//! - `Canonical`: New runtime handles execution (target state)
//! - `Legacy`: Old runtime handles execution (current default)
//! - `Shadow`: Both run; compare results for parity testing
//!
//! # Current GUI Execution Paths (Migration Map)
//!
//! The existing loop_engine has THREE GUI execution branches:
//!
//! 1. **OpGraph/GoalTree path** (line ~4476):
//!    `coordinator.generate_opgraph_workflow()` → `coordinator.execute_goal_tree()`
//!    Used for: multi-intent decomposed workflows
//!    Status: Complex, rarely triggered, keep on legacy for now
//!
//! 2. **Substrate/HTN path** (line ~4640):
//!    `coordinator.generate_workflow()` → `coordinator.execute_workflow()`
//!    Used for: most GUI workflows (IDE, browser, file, terminal)
//!    Status: PRIMARY migration target — this is what the new runtime replaces
//!
//! 3. **LLM HTN fallback** (line ~4644):
//!    `plan_gui_workflow_via_llm()` → same executor
//!    Used for: when substrate planner returns Unknown
//!    Status: Keep as fallback, route through new runtime when stable
//!
//! # Completion Synthesis Points (Deduplication Map)
//!
//! Currently, workflow completion is synthesized in FOUR places:
//! - `format_gui_workflow_success_for_user()` (line 268)
//! - `format_gui_workflow_partial_for_user()` (line 302)
//! - `format_gui_workflow_failure_for_user()` (line 328)
//! - `verdict_computation` (line 4784) — our new canonical path
//!
//! After convergence: ONLY `verdict_computation` should produce verdicts.
//! The format_* functions become legacy adapters that render verdicts as strings.

use crate::agent::intent_compiler::GuiTaskSpec;
use crate::agent::workflow_planner::{CapabilityAwarePlanner, PlanningResult};
use crate::agent::workflow_types::*;

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Runtime Mode (Migration Control)
// ═══════════════════════════════════════════════════════════════════════════════

/// Controls which runtime handles GUI workflow execution.
/// Used for progressive migration without hard cutovers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    /// New canonical runtime handles execution (target state)
    Canonical,
    /// Legacy runtime handles execution (current default during migration)
    Legacy,
    /// Both runtimes execute; results compared for parity testing
    Shadow,
}

impl Default for RuntimeMode {
    fn default() -> Self {
        // Default to Legacy during migration — switch to Canonical when proven
        Self::Legacy
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Routing Decision
// ═══════════════════════════════════════════════════════════════════════════════

/// The routing decision made by the WorkflowRuntimeRouter.
#[derive(Debug, Clone)]
pub enum RoutingDecision {
    /// Route to canonical HybridWorkflowExecutor
    CanonicalWorkflow {
        planning_result: PlanningResultSummary,
    },
    /// Route to legacy GUI execution path (existing loop_engine branch)
    LegacyGuiExecution {
        reason: &'static str,
    },
    /// Route to ReAct loop (non-GUI intent)
    ReactLoop {
        reason: &'static str,
    },
    /// Workflow needs HITL before routing can proceed
    HitlBeforeRouting {
        reason: HitlReason,
        options: Vec<HitlOption>,
        context: String,
    },
    /// Cannot route — explain to user
    Unroutable {
        reason: String,
    },
}

/// Summary of planning result for routing decision (avoids cloning full plan).
#[derive(Debug, Clone)]
pub struct PlanningResultSummary {
    pub substrate: String,
    pub execution_mode: ExecutionMode,
    pub step_count: u32,
    pub has_outcome_contract: bool,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Workflow Runtime Router
// ═══════════════════════════════════════════════════════════════════════════════

/// The canonical workflow dispatch authority.
///
/// This is the SINGLE point where runtime routing decisions are made.
/// No other module should independently decide which runtime to use.
pub struct WorkflowRuntimeRouter {
    mode: RuntimeMode,
}

impl WorkflowRuntimeRouter {
    pub fn new(mode: RuntimeMode) -> Self {
        Self { mode }
    }

    /// Route a classified intent to the appropriate runtime.
    ///
    /// This is the canonical routing entry point. It:
    /// 1. Checks if the intent is a GUI workflow
    /// 2. Runs capability-aware planning
    /// 3. Decides which runtime should handle execution
    /// 4. Returns a structured routing decision
    pub fn route(
        &self,
        spec: &GuiTaskSpec,
        raw_user_text: &str,
        capabilities: &CapabilitySet,
        app_registry: &crate::platform::app_registry::InstalledAppRegistry,
        is_gui_intent: bool,
    ) -> RoutingDecision {
        // Non-GUI intents always go to ReAct
        if !is_gui_intent {
            return RoutingDecision::ReactLoop {
                reason: "Non-GUI intent classified by TurnGate",
            };
        }

        // Run capability-aware planning
        let planning_result = CapabilityAwarePlanner::plan(
            spec,
            raw_user_text,
            capabilities,
            app_registry,
        );

        self.route_from_planning_result(planning_result, capabilities)
    }

    /// Route without app registry (used when registry is not directly accessible).
    /// Skips app-availability pre-flight checks but still does capability routing.
    pub fn route_without_registry(
        &self,
        spec: &GuiTaskSpec,
        raw_user_text: &str,
        _capabilities: &CapabilitySet,
        is_gui_intent: bool,
    ) -> RoutingDecision {
        if !is_gui_intent {
            return RoutingDecision::ReactLoop {
                reason: "Non-GUI intent classified by TurnGate",
            };
        }

        // Skip app-availability pre-flight (no registry available in async context).
        // The substrate planner will still check app availability at execution time.
        // Route based on substrate planning without registry-dependent checks.
        let planner = crate::agent::gui_substrate_planner::SubstratePlanner;
        let substrate_plan = planner.plan(spec, raw_user_text);

        if substrate_plan.substrate == crate::agent::gui_substrate_planner::ExecutionSubstrate::Unknown {
            return RoutingDecision::ReactLoop {
                reason: "Substrate planner could not generate a plan",
            };
        }

        let summary = PlanningResultSummary {
            substrate: format!("{:?}", substrate_plan.substrate),
            execution_mode: crate::agent::workflow_telemetry::execution_mode_from_previews(
                &substrate_plan.workflow.as_ref()
                    .map(|w| crate::agent::workflow_telemetry::step_previews_from_workflow(w))
                    .unwrap_or_default()
            ),
            step_count: substrate_plan.workflow.as_ref().map(|w| w.sub_goals.len() as u32).unwrap_or(0),
            has_outcome_contract: true,
        };

        match self.mode {
            RuntimeMode::Canonical => {
                tracing::info!(
                    target: "workflow_router",
                    substrate = %summary.substrate,
                    steps = summary.step_count,
                    "Routing to CANONICAL workflow runtime (no registry)"
                );
                RoutingDecision::CanonicalWorkflow { planning_result: summary }
            }
            RuntimeMode::Legacy | RuntimeMode::Shadow => {
                RoutingDecision::LegacyGuiExecution {
                    reason: "RuntimeMode::Legacy — canonical runtime not yet default",
                }
            }
        }
    }

    fn route_from_planning_result(
        &self,
        planning_result: PlanningResult,
        _capabilities: &CapabilitySet,
    ) -> RoutingDecision {

        match planning_result {
            PlanningResult::Planned {
                substrate_plan,
                outcome_contract,
                execution_mode,
                adaptations,
            } => {
                let summary = PlanningResultSummary {
                    substrate: format!("{:?}", substrate_plan.substrate),
                    execution_mode: execution_mode.clone(),
                    step_count: substrate_plan
                        .workflow
                        .as_ref()
                        .map(|w| w.sub_goals.len() as u32)
                        .unwrap_or(0),
                    has_outcome_contract: !outcome_contract.required.is_empty()
                        || !outcome_contract.desired.is_empty(),
                };

                match self.mode {
                    RuntimeMode::Canonical => {
                        tracing::info!(
                            target: "workflow_router",
                            substrate = %summary.substrate,
                            steps = summary.step_count,
                            mode = ?execution_mode,
                            adaptations = adaptations.len(),
                            "Routing to CANONICAL workflow runtime"
                        );
                        RoutingDecision::CanonicalWorkflow {
                            planning_result: summary,
                        }
                    }
                    RuntimeMode::Legacy => {
                        tracing::info!(
                            target: "workflow_router",
                            substrate = %summary.substrate,
                            steps = summary.step_count,
                            "Routing to LEGACY runtime (migration mode)"
                        );
                        RoutingDecision::LegacyGuiExecution {
                            reason: "RuntimeMode::Legacy — canonical runtime not yet default",
                        }
                    }
                    RuntimeMode::Shadow => {
                        tracing::info!(
                            target: "workflow_router",
                            substrate = %summary.substrate,
                            steps = summary.step_count,
                            "SHADOW mode: legacy executes, canonical traces for comparison"
                        );
                        // In shadow mode, legacy executes but we log what canonical would do
                        RoutingDecision::LegacyGuiExecution {
                            reason: "RuntimeMode::Shadow — legacy executes, canonical shadows",
                        }
                    }
                }
            }
            PlanningResult::NeedsHitl {
                reason,
                options,
                context,
            } => {
                tracing::info!(
                    target: "workflow_router",
                    reason = ?reason,
                    "Routing to HITL — workflow needs user input before execution"
                );
                RoutingDecision::HitlBeforeRouting {
                    reason,
                    options,
                    context,
                }
            }
            PlanningResult::Unplannable { reason } => {
                tracing::info!(
                    target: "workflow_router",
                    reason = %reason,
                    "Unplannable — falling back to ReAct loop"
                );
                RoutingDecision::ReactLoop {
                    reason: "Substrate planner could not generate a plan",
                }
            }
        }
    }

    /// Get the current runtime mode.
    pub fn mode(&self) -> RuntimeMode {
        self.mode
    }

    /// Switch runtime mode (for progressive migration).
    pub fn set_mode(&mut self, mode: RuntimeMode) {
        tracing::info!(
            target: "workflow_router",
            old_mode = ?self.mode,
            new_mode = ?mode,
            "Runtime mode changed"
        );
        self.mode = mode;
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — Workflow Execution Trace (Persistence-Ready)
// ═══════════════════════════════════════════════════════════════════════════════

/// A complete execution trace for a workflow session.
/// Designed for persistence, replay, and eval analysis.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkflowExecutionTrace {
    /// Unique workflow identifier
    pub workflow_id: String,
    /// User's original text
    pub user_text: String,
    /// Resolved capabilities at plan time
    pub capabilities_summary: CapabilitySummary,
    /// Routing decision made
    pub routing_decision: String,
    /// Execution mode selected
    pub execution_mode: ExecutionMode,
    /// Runtime mode (canonical/legacy/shadow)
    pub runtime_mode: String,
    /// Telemetry events in order
    pub telemetry_events: Vec<TelemetryEnvelope>,
    /// Final verdict
    pub verdict: Option<WorkflowVerdict>,
    /// Total duration (ms)
    pub duration_ms: u64,
    /// Timestamp (ISO 8601)
    pub timestamp: String,
}

/// Compact capability summary for trace persistence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CapabilitySummary {
    pub session_type: SessionType,
    pub atspi_level: String,
    pub uinput_available: bool,
    pub window_max_confidence: f32,
    pub cdp_available: bool,
}

impl CapabilitySummary {
    pub fn from_capabilities(caps: &CapabilitySet) -> Self {
        Self {
            session_type: caps.environment.session_type,
            atspi_level: format!("{:?}", caps.environment.atspi_level),
            uinput_available: caps.environment.uinput_available,
            window_max_confidence: caps.verifier.window_state_max_confidence,
            cdp_available: caps.verifier.cdp_available,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §5 — Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::intent_compiler::{GuiTaskSpec, TargetRef, Verb, ContentClass};

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
                available_methods: vec![VerificationMethod::FileSystem, VerificationMethod::ProcessTable],
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

    fn make_spec(verb: Verb, targets: Vec<TargetRef>) -> GuiTaskSpec {
        GuiTaskSpec {
            primary_verb: verb,
            targets,
            content: None,
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        }
    }

    #[test]
    fn non_gui_intent_routes_to_react() {
        let router = WorkflowRuntimeRouter::new(RuntimeMode::Canonical);
        let spec = make_spec(Verb::Other("chat".into()), vec![]);
        let caps = make_capabilities();
        let registry = crate::platform::app_registry::InstalledAppRegistry::build_sync();

        let decision = router.route(&spec, "what is the weather", &caps, &registry, false);
        assert!(matches!(decision, RoutingDecision::ReactLoop { .. }));
    }

    #[test]
    fn gui_intent_in_legacy_mode_routes_to_legacy() {
        let router = WorkflowRuntimeRouter::new(RuntimeMode::Legacy);
        let spec = make_spec(Verb::Open, vec![TargetRef::App("firefox".into())]);
        let caps = make_capabilities();
        let registry = crate::platform::app_registry::InstalledAppRegistry::build_sync();

        let decision = router.route(&spec, "open firefox", &caps, &registry, true);
        // In legacy mode, even plannable workflows go to legacy runtime
        assert!(
            matches!(decision, RoutingDecision::LegacyGuiExecution { .. }
                | RoutingDecision::HitlBeforeRouting { .. }),
            "Expected Legacy or HITL routing, got {:?}",
            decision
        );
    }

    #[test]
    fn gui_intent_in_canonical_mode_routes_to_canonical() {
        let router = WorkflowRuntimeRouter::new(RuntimeMode::Canonical);
        let spec = make_spec(Verb::Open, vec![TargetRef::App("firefox".into())]);
        let caps = make_capabilities();
        let registry = crate::platform::app_registry::InstalledAppRegistry::build_sync();

        let decision = router.route(&spec, "open firefox", &caps, &registry, true);
        // Should route to canonical (if firefox is installed) or HITL (if not)
        assert!(
            matches!(
                decision,
                RoutingDecision::CanonicalWorkflow { .. }
                    | RoutingDecision::HitlBeforeRouting { .. }
            ),
            "Expected Canonical or HITL routing, got {:?}",
            decision
        );
    }

    #[test]
    fn missing_app_triggers_hitl_before_routing() {
        let router = WorkflowRuntimeRouter::new(RuntimeMode::Canonical);
        let spec = make_spec(
            Verb::Open,
            vec![TargetRef::App("kria_nonexistent_app_xyz".into())],
        );
        let caps = make_capabilities();
        let registry = crate::platform::app_registry::InstalledAppRegistry::build_sync();

        let decision = router.route(&spec, "open nonexistent app", &caps, &registry, true);
        assert!(
            matches!(decision, RoutingDecision::HitlBeforeRouting { .. }),
            "Missing app should trigger HITL, got {:?}",
            decision
        );
    }

    #[test]
    fn interaction_without_capability_triggers_hitl() {
        let router = WorkflowRuntimeRouter::new(RuntimeMode::Canonical);
        let spec = make_spec(Verb::Type, vec![TargetRef::Element("search".into())]);
        // No uinput = no interaction
        let mut caps = make_capabilities();
        caps.interaction.keyboard_injection = InputInjectionLevel::None;
        caps.interaction.mouse_injection = InputInjectionLevel::None;
        let registry = crate::platform::app_registry::InstalledAppRegistry::build_sync();

        let decision = router.route(&spec, "type hello", &caps, &registry, true);
        assert!(
            matches!(decision, RoutingDecision::HitlBeforeRouting { .. }),
            "No interaction capability should trigger HITL, got {:?}",
            decision
        );
    }

    #[test]
    fn runtime_mode_can_be_changed() {
        let mut router = WorkflowRuntimeRouter::new(RuntimeMode::Legacy);
        assert_eq!(router.mode(), RuntimeMode::Legacy);

        router.set_mode(RuntimeMode::Canonical);
        assert_eq!(router.mode(), RuntimeMode::Canonical);

        router.set_mode(RuntimeMode::Shadow);
        assert_eq!(router.mode(), RuntimeMode::Shadow);
    }

    #[test]
    fn capability_summary_captures_key_fields() {
        let caps = make_capabilities();
        let summary = CapabilitySummary::from_capabilities(&caps);
        assert_eq!(summary.session_type, SessionType::X11);
        assert!(summary.uinput_available);
        assert_eq!(summary.window_max_confidence, 0.90);
    }
}
