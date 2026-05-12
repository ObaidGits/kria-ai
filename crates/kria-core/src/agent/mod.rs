pub mod curiosity;
pub mod executive;
pub mod failure_analyzer;
pub mod gui_wiring;
pub mod htn_executor;
pub mod htn_integration;
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
pub mod tool_dependencies;
pub mod turn_context;
pub mod turn_gate;
pub mod uncertainty;
pub mod visual_reasoning;
pub mod working_set;
pub mod world_model;

// RFC v2: GUI cognition skeletons (trait + type sketches, behind feature flag).
// Implementations land in phases P1–P5 per docs/GUI_INTELLIGENCE_REVIEW.md.
#[cfg(feature = "gui_cognition_v2")]
pub mod intent_compiler;
#[cfg(feature = "gui_cognition_v2")]
pub mod environment_grounder;
#[cfg(feature = "gui_cognition_v2")]
pub mod execution_verifier;

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
