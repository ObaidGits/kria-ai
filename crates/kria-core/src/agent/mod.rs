pub mod curiosity;
pub mod executive;
pub mod failure_analyzer;
pub mod interaction;
pub mod loop_engine;
pub mod ml_orchestrator;
pub mod onnx_classifier;
pub mod perception;
pub mod planner;
pub mod planner_v2;
pub mod prompt_optimizer;
pub mod prompts;
pub mod response_parser;
pub mod router;
pub mod self_model;
pub mod skill_compiler;
pub mod turn_context;
pub mod turn_gate;
pub mod uncertainty;
pub mod working_set;
pub mod world_model;

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
