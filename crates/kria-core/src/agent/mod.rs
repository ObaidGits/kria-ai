pub mod atspi_engine;
pub mod browser_cognition;
pub mod browser_media_governance;
pub mod collaborative_decision;
pub mod continuation_reentry;
pub mod curiosity;
pub mod execution_authority;
pub mod execution_gate;
pub mod execution_interpreter;
pub mod execution_mode_reasoner;
pub mod executive;
pub mod failure_analyzer;
pub mod gui_wiring;
pub mod htn_executor;
pub mod htn_integration;
pub mod ide_cognition;
pub mod intent_gate;
pub mod interaction;
pub mod loop_engine;
pub mod ml_orchestrator;
pub mod ocr_engine;
pub mod onnx_classifier;
pub mod perception;
pub mod planner;
pub mod planner_v2;
pub mod prompt_compiler;
pub mod prompt_optimizer;
pub mod prompts;
pub mod psdg;
pub mod resource_lease;
pub mod response_parser;
pub mod result_synthesizer;
pub mod resume_executor;
pub mod router;
pub mod self_model;
pub mod semantic_workflow;
pub mod skill_compiler;
pub mod synthesis_prompt;
pub mod tool_dependencies;
pub mod turn_context;
pub mod turn_gate;
pub mod turn_memory;
pub mod visual_reasoning;
pub mod workflow_intent_contract;
pub mod workflow_session;
pub mod working_set;
pub mod world_model;

// RFC v2: GUI cognition modules (always compiled, single authority).
pub mod environment_grounder;
pub mod execution_verifier;
pub mod execution_verifier_bounded;
pub mod execution_verifier_impl;
pub mod gui_lease;
pub mod gui_planner;
pub mod gui_production_readiness;
pub mod gui_services;
pub mod intent_compiler;
pub mod intent_compiler_llm;
pub mod intent_compiler_rule;
pub mod multi_intent;
pub mod opgraph;
pub mod opgraph_compiler;

// P3: GoalTree workflow cognition (multi-stage bounded workflows).
pub mod goal_tree;
pub mod stage_executor;
pub mod workflow_compiler;

// Substrate-aware GUI planner: picks the right execution path
// (file substrate vs keystroke vs browser) for a GuiTaskSpec.
pub mod gui_substrate_planner;
pub mod hybrid_synchronization;
pub mod uncertainty;
pub mod verifier_authority;
pub mod window_observer;

// ── Batch 2: Human-Aligned Workflow Cognition Runtime ──────────────────────

// Phase 1: Observable Completion Engine
pub mod observable_completion;

// Phase 2: Collaborative Autonomy Engine
pub mod collaborative_autonomy;

// Phase 3: Workflow Expectation Engine
pub mod workflow_expectation;

// Phase 4: Workflow Continuation Runtime
pub mod workflow_continuation;

// Phase 5: Execution Transparency Layer
pub mod execution_transparency;

// Phase 6: Workspace Operational Memory
pub mod workspace_memory;

// ── Batch 3: Persistent Operational Desktop Cognition Runtime ────────────────

// Phase 1: Cognition Event Bus — typed broadcast event bus
pub mod cognition_event_bus;

// Phase 2: Ambient Cognition Loop — low-frequency bounded background loop
pub mod ambient_cognition;

// Phase 3: Operational Context Tracker — bounded operational history chain
pub mod operational_context;

// Phase 4: Procedural Workflow Memory — workflow skill graph
pub mod procedural_memory;

// Phase 5: Persistent Goal Runtime — goals that survive restarts
pub mod goal_runtime;

// Phase 6: Operational Suggestions Engine — rate-limited proactive suggestions
pub mod operational_suggestions;

// Phase 7: Desktop Awareness Runtime — unified live operational state
pub mod desktop_awareness;

pub use interaction::Interaction;
pub use loop_engine::{AgentLoop, StreamEvent};
pub use router::IntentRouter;
pub use turn_context::{
    ActiveTurn, SessionId, TurnAdmission, TurnAdmissionError, TurnCancellationTree, TurnContext,
    TurnId,
};
pub use turn_gate::{IntentEnvelope, ResourcePlan, TurnGate, TurnGatePlan};

// Phase E: Perception + Curiosity
pub use curiosity::{CuriosityConfig, CuriosityLoop};
pub use perception::{
    EventDebouncer, PerceptionBus, PerceptionConfig, PerceptionEvent, PerceptionLoop,
};

// Phase F: Prompt Optimizer
pub use prompt_optimizer::{PromptOptimizer, PromptOptimizerConfig, TaskDomain, TaskOutcome};

// Batch 1: PSDG — Persistent Semantic Desktop Cognition
pub use psdg::{PsdgContextSnapshot, PsdgHandle};
