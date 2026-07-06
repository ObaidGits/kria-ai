//! Concrete executors that plug into the generic engine.
//!
//! A7 ships ONLY the OpenClaw executor. Future backends (GUI, Native, MCP, Browser,
//! Memory, Cloud, Agent) live here and implement the same `Executor` trait — no
//! planner/scheduler/engine changes required (A7.13).

pub mod openclaw;

pub use openclaw::{openclaw_executor_from_pool, OpenClawExecutor};
