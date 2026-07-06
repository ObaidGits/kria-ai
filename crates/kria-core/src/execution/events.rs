//! A7.9 Execution Events — ONE authoritative event stream for the engine.
//!
//! Every phase of planning, optimization, scheduling and execution emits through
//! this single stream. No duplicate event buses. Backend-agnostic: executors emit
//! `ExecutorStarted`/`ExecutorFinished` without naming a concrete backend.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// A single event in the execution engine lifecycle (A7.9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutionEvent {
    // Planning phase
    PlanningStarted {
        goal_id: String,
    },
    PlanningCompleted {
        goal_id: String,
        node_count: usize,
    },

    // Graph phase
    GraphCreated {
        graph_id: String,
        node_count: usize,
    },
    OptimizationStarted {
        graph_id: String,
    },
    OptimizationCompleted {
        graph_id: String,
        nodes_before: usize,
        nodes_after: usize,
    },

    // Execution phase
    ExecutionStarted {
        graph_id: String,
    },
    NodeStarted {
        graph_id: String,
        node_id: String,
        kind: String,
    },
    NodeCompleted {
        graph_id: String,
        node_id: String,
        latency_ms: u64,
    },
    NodeFailed {
        graph_id: String,
        node_id: String,
        reason: String,
    },

    // Recovery phase
    Retry {
        graph_id: String,
        node_id: String,
        attempt: u32,
    },
    Rollback {
        graph_id: String,
        node_id: String,
    },
    Cancelled {
        graph_id: String,
        node_id: Option<String>,
    },
    Recovered {
        graph_id: String,
        node_id: String,
    },

    // Terminal
    GraphCompleted {
        graph_id: String,
        latency_ms: u64,
    },
    GraphFailed {
        graph_id: String,
        reason: String,
    },

    // Executor lifecycle (backend-agnostic)
    ExecutorStarted {
        executor: String,
        node_id: String,
    },
    ExecutorFinished {
        executor: String,
        node_id: String,
        success: bool,
    },
}

/// The single broadcast channel for execution events.
#[derive(Clone)]
pub struct ExecutionEventStream {
    sender: broadcast::Sender<ExecutionEvent>,
}

impl Default for ExecutionEventStream {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionEventStream {
    pub fn new() -> Self {
        Self {
            sender: broadcast::channel(4096).0,
        }
    }

    /// Emit an event. Dropped if no subscribers (fire-and-forget).
    pub fn emit(&self, event: ExecutionEvent) {
        let _ = self.sender.send(event);
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<ExecutionEvent> {
        self.sender.subscribe()
    }
}
