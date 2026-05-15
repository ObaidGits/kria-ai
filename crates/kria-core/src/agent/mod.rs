pub mod curiosity;
pub mod executive;
pub mod failure_analyzer;
pub mod gui_wiring;
pub mod htn_executor;
pub mod htn_integration;
pub mod intent_gate;
pub mod interaction;
pub mod loop_engine;
pub mod turn_memory;
pub mod ml_orchestrator;
pub mod onnx_classifier;
pub mod perception;
pub mod planner;
pub mod planner_v2;
pub mod prompt_compiler;
pub mod prompt_optimizer;
pub mod prompts;
pub mod response_parser;
pub mod router;
pub mod self_model;
pub mod skill_compiler;
pub mod tool_dependencies;
pub mod turn_context;
pub mod turn_gate;
pub mod visual_reasoning;
pub mod working_set;

// RFC v2: GUI cognition modules (always compiled, single authority).
pub mod environment_grounder;
pub mod execution_verifier;
pub mod execution_verifier_impl;
pub mod gui_planner;
pub mod intent_compiler;
pub mod intent_compiler_llm;

// P3: GoalTree workflow cognition (multi-stage bounded workflows).
pub mod goal_tree;
pub mod stage_executor;
pub mod workflow_compiler;

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
