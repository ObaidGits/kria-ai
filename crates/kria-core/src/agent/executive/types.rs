//! Core types for the Executive Controller.
//!
//! Design constraints:
//! - All types are `Send + 'static` (safe for跨-task transfer)
//! - TaskRequest carries its own CancellationToken for fine-grained preemption
//! - TaskPriority is `Ord` so BinaryHeap works naturally (lower number = higher priority)

use std::cmp::Ordering;
use std::fmt;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

// ─── Priority Tiers ──────────────────────────────────────────────────────────

/// Task priority. Lower numeric value = higher priority.
/// Derive `Ord` sorts ascending, so `Voice(0) < Maintenance(4)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum TaskPriority {
    /// Voice commands, barge-in, emergency stop. Always wins.
    Voice = 0,
    /// Interactive text chat from the user.
    Interactive = 1,
    /// HITL approval/rejection responses. Unblocks blocked tasks.
    HitlResponse = 2,
    /// Background: CuriosityLoop, proactive nudges, diagnostics.
    Background = 3,
    /// Maintenance: VRAM refresh GC, log rotation, model downloads.
    Maintenance = 4,
}

impl TaskPriority {
    /// Returns `true` if this priority tier is eligible for GPU lease.
    /// Background and Maintenance tasks NEVER get GPU lease directly.
    pub fn can_acquire_gpu(&self) -> bool {
        matches!(self, Self::Voice | Self::Interactive | Self::HitlResponse)
    }

    /// Returns `true` if this priority tier can preempt a running task.
    pub fn can_preempt(&self) -> bool {
        matches!(self, Self::Voice)
    }

    /// Returns `true` if this is a foreground (user-facing) task.
    pub fn is_foreground(&self) -> bool {
        matches!(self, Self::Voice | Self::Interactive)
    }
}

// Manual Ord: lower variant value = higher priority (comes first in max-heap via Reverse).
impl PartialOrd for TaskPriority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TaskPriority {
    fn cmp(&self, other: &Self) -> Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}

impl fmt::Display for TaskPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Voice => write!(f, "P0:Voice"),
            Self::Interactive => write!(f, "P1:Interactive"),
            Self::HitlResponse => write!(f, "P2:HITL"),
            Self::Background => write!(f, "P3:Background"),
            Self::Maintenance => write!(f, "P4:Maintenance"),
        }
    }
}

// ─── Task Source ─────────────────────────────────────────────────────────────

/// Identifies the origin subsystem of a task.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TaskSource {
    VoicePipeline,
    TextChat,
    HitlGateway,
    CuriosityLoop,
    ProactiveScheduler,
    SkillCompiler,
    Maintenance,
    /// Compiled skill executing directly (bypasses planner)
    CompiledSkill(String),
}

impl fmt::Display for TaskSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VoicePipeline => write!(f, "voice"),
            Self::TextChat => write!(f, "text"),
            Self::HitlGateway => write!(f, "hitl"),
            Self::CuriosityLoop => write!(f, "curiosity"),
            Self::ProactiveScheduler => write!(f, "proactive"),
            Self::SkillCompiler => write!(f, "skill_compiler"),
            Self::Maintenance => write!(f, "maintenance"),
            Self::CompiledSkill(name) => write!(f, "skill:{}", name),
        }
    }
}

// ─── Task Payload ────────────────────────────────────────────────────────────

/// What kind of work the task wants to do.
/// Each variant carries enough context for the worker to execute without
/// calling back into the ExecutiveController.
#[derive(Debug)]
pub enum TaskPayload {
    /// Process a user utterance (voice or text). Goes through the full
    /// routing → uncertainty → planning → execution pipeline.
    UserTurn {
        text: String,
        is_voice: bool,
        session_id: String,
    },

    /// Execute a single pre-planned command (from StructuredBranchingPlanner).
    ExecuteCommand {
        command: crate::tools::subprocess_executor::StructuredCommand,
    },

    /// Run background diagnostics (read-only, no GPU).
    BackgroundDiagnostics {
        commands: Vec<crate::tools::subprocess_executor::StructuredCommand>,
    },

    /// Gather evidence for the Uncertainty Engine (read-only).
    GatherEvidence {
        commands: Vec<crate::tools::subprocess_executor::StructuredCommand>,
    },

    /// Compile a skill from a successful plan.
    CompileSkill {
        plan_json: String,
    },

    /// VRAM maintenance: checkpoint → drop → reload the Planner model.
    VramMaintenanceRefresh {
        reason: String,
    },

    /// HITL response: unblock a blocked task.
    HitlResponse {
        request_id: String,
        approved: bool,
    },

    /// Generic maintenance task.
    Maintenance {
        description: String,
    },
}

// ─── Task Request ────────────────────────────────────────────────────────────

/// A unit of work submitted to the Executive Controller.
///
/// Created by subsystems (voice pipeline, text chat, curiosity loop, etc.)
/// and sent via the MPSC channel.
#[derive(Debug)]
pub struct TaskRequest {
    /// Unique ID for this task (for tracing and cancellation).
    pub id: uuid::Uuid,
    /// Priority tier.
    pub priority: TaskPriority,
    /// Origin subsystem.
    pub source: TaskSource,
    /// Whether this task needs GPU lease to execute.
    pub requires_gpu: bool,
    /// Estimated GPU duration (for lease TTL). None = use default.
    pub estimated_gpu_duration: Option<Duration>,
    /// The actual work to do.
    pub payload: TaskPayload,
    /// Cancellation token. The ExecutiveController or the task itself can cancel.
    pub cancel: CancellationToken,
    /// When the task was submitted (for latency tracking).
    pub submitted_at: Instant,
}

impl TaskRequest {
    /// Create a new task request with auto-generated ID and current timestamp.
    pub fn new(
        priority: TaskPriority,
        source: TaskSource,
        requires_gpu: bool,
        payload: TaskPayload,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            priority,
            source,
            requires_gpu,
            estimated_gpu_duration: None,
            payload,
            cancel: CancellationToken::new(),
            submitted_at: Instant::now(),
        }
    }

    /// Builder method: set estimated GPU duration.
    pub fn with_gpu_duration(mut self, d: Duration) -> Self {
        self.estimated_gpu_duration = Some(d);
        self
    }
}

// ─── Task Handle ─────────────────────────────────────────────────────────────

/// A handle to a running task, tracked by the ExecutiveController.
pub struct TaskHandle {
    pub id: uuid::Uuid,
    pub priority: TaskPriority,
    pub source: TaskSource,
    pub cancel: CancellationToken,
    pub join: JoinHandle<TaskResult>,
    pub started_at: Instant,
    pub requires_gpu: bool,
}

impl TaskHandle {
    /// Returns `true` if the task has finished (success, failure, or cancellation).
    pub fn is_finished(&self) -> bool {
        self.join.is_finished()
    }

    /// Abort the task forcefully (tokio::JoinHandle::abort).
    pub fn abort(&self) {
        self.join.abort();
    }

    /// How long the task has been running.
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }
}

// ─── Task Result ─────────────────────────────────────────────────────────────

/// Outcome of a task execution.
#[derive(Debug)]
pub enum TaskResult {
    /// Task completed successfully.
    Success {
        /// Duration from submission to completion.
        total_duration: Duration,
        /// Optional result payload (e.g., tool output).
        output: Option<String>,
    },
    /// Task failed.
    Failed {
        reason: String,
        /// Duration from submission to failure.
        total_duration: Duration,
    },
    /// Task was cancelled (preempted or user-cancelled).
    Cancelled {
        reason: String,
    },
    /// Task timed out.
    TimedOut {
        timeout: Duration,
    },
}

impl fmt::Display for TaskResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Success { total_duration, .. } => {
                write!(f, "success ({:.0}ms)", total_duration.as_millis())
            }
            Self::Failed { reason, total_duration } => {
                write!(f, "failed: {} ({:.0}ms)", reason, total_duration.as_millis())
            }
            Self::Cancelled { reason } => write!(f, "cancelled: {}", reason),
            Self::TimedOut { timeout } => write!(f, "timed out ({:.0}s)", timeout.as_secs_f64()),
        }
    }
}

// ─── Schedule Decision ───────────────────────────────────────────────────────

/// What the scheduler decided to do with an incoming task.
#[derive(Debug)]
pub enum ScheduleDecision {
    /// Execute immediately (foreground slot is free, or HITL response).
    Execute(TaskRequest),
    /// Enqueue in the priority queue.
    Enqueue(TaskRequest),
    /// Preempt the current foreground task and replace it.
    Preempt {
        victim_id: uuid::Uuid,
        replacement: TaskRequest,
    },
    /// Reject the task (e.g., duplicate voice task, background limit reached).
    Reject { task_id: uuid::Uuid, reason: String },
}

// ─── Controller Event ────────────────────────────────────────────────────────

/// Events emitted by the ExecutiveController for observability.
#[derive(Debug, Clone, serde::Serialize)]
pub enum ControllerEvent {
    TaskStarted {
        task_id: uuid::Uuid,
        priority: TaskPriority,
        source: TaskSource,
    },
    TaskCompleted {
        task_id: uuid::Uuid,
        result_summary: String,
        duration_ms: u64,
    },
    TaskPreempted {
        victim_id: uuid::Uuid,
        replacement_id: uuid::Uuid,
    },
    TaskRejected {
        task_id: uuid::Uuid,
        reason: String,
    },
    GpuLeaseAcquired {
        task_id: uuid::Uuid,
    },
    GpuLeaseReleased {
        task_id: uuid::Uuid,
    },
    VramMaintenanceStarted {
        reason: String,
    },
    VramMaintenanceCompleted {
        duration_ms: u64,
    },
}
