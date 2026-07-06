//! A9.12 Learning Events — the generation pipeline EMITS events only.
//!
//! OpenClaw must NOT own memory/learning. It broadcasts generation outcomes; a future
//! Memory subsystem consumes them. One event stream, no persistence here.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// A learning/telemetry event from the generation pipeline (A9.12).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GenerationEvent {
    /// A goal was received and requirement extraction began.
    Started { goal_id: String, prompt: String },
    /// An existing skill satisfied the goal — reused, not generated (A9.0).
    ReusedExisting {
        goal_id: String,
        skill_id: String,
        similarity: f64,
    },
    /// Requirement extraction completed.
    RequirementsExtracted { goal_id: String, intent: String },
    /// Skill design completed.
    Designed { goal_id: String, slug: String },
    /// Code generation completed.
    CodeGenerated { goal_id: String, slug: String },
    /// Validation outcome.
    Validated {
        goal_id: String,
        slug: String,
        passed: bool,
    },
    /// A repair attempt occurred.
    RepairAttempt {
        goal_id: String,
        slug: String,
        attempt: u32,
        reason: String,
    },
    /// Sandbox test outcome.
    SandboxTested {
        goal_id: String,
        slug: String,
        passed: bool,
    },
    /// Quality evaluation score.
    QualityScored {
        goal_id: String,
        slug: String,
        overall: f64,
    },
    /// Skill was packaged, signed and installed through the frozen lifecycle.
    Installed {
        goal_id: String,
        slug: String,
        version: String,
    },
    /// Installation is awaiting human approval (A9.0.3).
    AwaitingApproval {
        goal_id: String,
        slug: String,
        reasons: Vec<String>,
    },
    /// The generated skill executed successfully at least once.
    ExecutionSuccess {
        goal_id: String,
        slug: String,
        latency_ms: u64,
    },
    /// Generation failed terminally.
    Failed { goal_id: String, reason: String },
    /// Budget exhausted mid-generation (A9.0.4).
    BudgetExhausted { goal_id: String, dimension: String },
    /// User edited a generated skill after install.
    UserEdited { goal_id: String, slug: String },
}

/// The single generation event stream (A9.12).
#[derive(Clone)]
pub struct GenerationEventStream {
    sender: broadcast::Sender<GenerationEvent>,
}

impl Default for GenerationEventStream {
    fn default() -> Self {
        Self::new()
    }
}

impl GenerationEventStream {
    pub fn new() -> Self {
        Self {
            sender: broadcast::channel(1024).0,
        }
    }

    pub fn emit(&self, event: GenerationEvent) {
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<GenerationEvent> {
        self.sender.subscribe()
    }
}
