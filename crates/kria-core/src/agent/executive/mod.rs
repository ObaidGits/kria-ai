//! Executive Controller — Central brain for KRIA.
//!
//! Architecture: Thin Brain pattern.
//! - Main loop: pure dispatcher (<2ms per scheduling decision)
//! - Worker pools: command execution, audit logging, HITL coordination
//! - No I/O in the scheduling path
//!
//! Priority tiers:
//! - P0 (Voice): wake word, barge-in, emergency stop
//! - P1 (Interactive): text chat commands
//! - P2 (HitlResponse): approval/rejection responses (unblock blocked tasks)
//! - P3 (Background): CuriosityLoop, proactive nudges
//! - P4 (Maintenance): VRAM refresh, log rotation, model downloads

pub mod controller;
pub mod preemption;
pub mod types;

pub use controller::{ExecutiveConfig, ExecutiveController, ExecutiveSender};
pub use types::{
    ControllerEvent, ExecutiveSnapshot, ExecutiveTaskSnapshot, ScheduleDecision, TaskHandle,
    TaskPayload, TaskPriority, TaskRequest, TaskSource, TaskState,
};
