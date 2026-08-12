//! Generic Execution Engine (A7).
//!
//! KRIA's native execution substrate. Turns a `Goal` into an `ExecutionGraph` and
//! drives it to a result through pluggable `Executor` backends. OpenClaw is the
//! FIRST executor (A7.4); GUI, Native, MCP, Browser, Memory, Cloud and Agent
//! executors plug into the same interface later (A7.13).
//!
//! # Single-authority invariants (self-audit targets)
//!
//! | Concern            | Single owner                          |
//! |--------------------|---------------------------------------|
//! | Planner            | `planner::ExecutionPlanner`           |
//! | Graph              | `graph::ExecutionGraph`               |
//! | Scheduler          | `scheduler::ExecutionScheduler`       |
//! | Execution context  | `context::ExecutionContext`           |
//! | Executor interface | `executor::Executor`                  |
//! | Event stream       | `events::ExecutionEventStream`        |
//! | Metrics            | `metrics::ExecutionMetrics`           |
//! | Recovery           | `recovery::RecoveryManager`           |
//! | Executor registry  | `executor::ExecutorRegistry`          |
//!
//! The planner contains ZERO executor-specific logic. OpenClaw is only an executor.

pub mod context;
pub mod dependency;
pub mod engine;
pub mod events;
pub mod executor;
pub mod executors;
pub mod graph;
pub mod metrics;
pub mod optimizer;
pub mod planner;
pub mod recovery;
pub mod scheduler;


// ── Re-exports: the public execution API ──
pub use context::{Artifact, ExecutionContext};
pub use dependency::{DependencyIssue, DependencyResolver};
pub use engine::{EngineError, ExecutionEngine};
pub use events::{ExecutionEvent, ExecutionEventStream};
pub use executor::{
    ExecutionRequest, Executor, ExecutorError, ExecutorHealth, ExecutorMetrics, ExecutorRegistry,
};
pub use executors::OpenClawExecutor;
pub use graph::{ExecutionGraph, GraphNode, NodeKind};
pub use metrics::{ExecutionMetrics, ExecutionMetricsSnapshot};
pub use optimizer::{GraphOptimizer, OptimizationReport};
pub use planner::{ExecutionPlanner, Goal, PlanError, PlanStep};
pub use recovery::{RecoveryAction, RecoveryManager, RecoveryOutcome, RecoveryPolicy};
pub use scheduler::{ExecutionScheduler, ScheduleResult, ScheduleStatus};
