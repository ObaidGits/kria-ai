//! Work Management Layer (Phase 2).
//!
//! A unified, persistent task engine with deterministic prioritisation, durable
//! reminders that survive restart, and productivity analytics — all backed by
//! the shared `kria.db` SQLite database.

pub mod priority;
pub mod matching;
pub mod nl_time;
pub mod planner;
pub mod recurrence;
pub mod scheduler;
pub mod store;

pub use priority::PriorityBucket;
pub use recurrence::Recurrence;
pub use scheduler::spawn as spawn_reminder_scheduler;
pub use store::{NewTask, ProductivityStats, Reminder, Task, TaskFilter, TaskStore};
