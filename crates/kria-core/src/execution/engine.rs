//! Execution Engine — the top-level orchestrator wiring every A7 subsystem.
//!
//! Goal → Planner → Graph → DependencyResolver → Optimizer → Scheduler → Result.
//! Owns the single instances of: executor registry, event stream, metrics,
//! recovery policy. Contains ZERO OpenClaw-specific logic — OpenClaw is just one
//! registered `Executor`.

use super::context::ExecutionContext;
use super::dependency::{DependencyIssue, DependencyResolver};
use super::events::{ExecutionEvent, ExecutionEventStream};
use super::executor::{Executor, ExecutorRegistry};
use super::graph::ExecutionGraph;
use super::metrics::{ExecutionMetrics, ExecutionMetricsSnapshot};
use super::optimizer::GraphOptimizer;
use super::planner::{ExecutionPlanner, Goal, PlanError};
use super::recovery::RecoveryPolicy;
use super::scheduler::{ExecutionScheduler, ScheduleResult};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::broadcast;

/// Errors from a full engine run.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("planning failed: {0}")]
    Plan(#[from] PlanError),
    #[error("dependency validation failed: {0:?}")]
    Dependencies(Vec<DependencyIssue>),
}

/// The generic Execution Engine (A7). Single owner of all engine subsystems.
pub struct ExecutionEngine {
    registry: ExecutorRegistry,
    events: ExecutionEventStream,
    metrics: ExecutionMetrics,
    recovery: RecoveryPolicy,
    optimize: bool,
}

impl Default for ExecutionEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionEngine {
    pub fn new() -> Self {
        Self {
            registry: ExecutorRegistry::new(),
            events: ExecutionEventStream::new(),
            metrics: ExecutionMetrics::new(),
            recovery: RecoveryPolicy::default(),
            optimize: true,
        }
    }

    /// Register an executor (A7.12). OpenClaw registers here at boot.
    pub fn register_executor(&mut self, executor: Arc<dyn Executor>) {
        self.registry.register(executor);
    }

    pub fn set_recovery(&mut self, policy: RecoveryPolicy) {
        self.recovery = policy;
    }

    pub fn set_optimization(&mut self, enabled: bool) {
        self.optimize = enabled;
    }

    /// Subscribe to the single engine event stream (A7.9).
    pub fn subscribe(&self) -> broadcast::Receiver<ExecutionEvent> {
        self.events.subscribe()
    }

    /// Metrics snapshot (A7.11).
    pub fn metrics(&self) -> ExecutionMetricsSnapshot {
        self.metrics.snapshot()
    }

    /// Registry accessor (read-only discovery).
    pub fn registry(&self) -> &ExecutorRegistry {
        &self.registry
    }

    /// Plan a goal into a validated, optimized graph without executing it.
    pub fn plan(&self, goal: &Goal) -> Result<ExecutionGraph, EngineError> {
        self.events.emit(ExecutionEvent::PlanningStarted {
            goal_id: goal.id.clone(),
        });
        let plan_start = Instant::now();

        let mut graph = ExecutionPlanner::plan(goal, &self.registry)?;
        self.metrics
            .set_planning_latency(plan_start.elapsed().as_millis() as u64);
        self.events.emit(ExecutionEvent::PlanningCompleted {
            goal_id: goal.id.clone(),
            node_count: graph.node_count(),
        });
        self.events.emit(ExecutionEvent::GraphCreated {
            graph_id: graph.id.clone(),
            node_count: graph.node_count(),
        });

        // Validate dependencies (A7.6).
        let issues = DependencyResolver::validate(&graph, &self.registry);
        if !issues.is_empty() {
            return Err(EngineError::Dependencies(issues));
        }

        // Optimize (A7.10).
        if self.optimize {
            let before = graph.node_count();
            self.events.emit(ExecutionEvent::OptimizationStarted {
                graph_id: graph.id.clone(),
            });
            let opt_start = Instant::now();
            GraphOptimizer::optimize(&mut graph);
            self.metrics
                .set_optimization_latency(opt_start.elapsed().as_millis() as u64);
            self.events.emit(ExecutionEvent::OptimizationCompleted {
                graph_id: graph.id.clone(),
                nodes_before: before,
                nodes_after: graph.node_count(),
            });
        }

        // Record graph shape.
        self.metrics
            .set_graph_shape(graph.depth(), graph.roots().len(), graph.node_count());

        Ok(graph)
    }

    /// Full run: plan → validate → optimize → schedule → result.
    pub async fn execute(&self, goal: &Goal) -> Result<ScheduleResult, EngineError> {
        let graph = self.plan(goal)?;
        let ctx = ExecutionContext::new(goal.id.clone(), format!("corr-{}", goal.id));
        Ok(self.execute_graph(&graph, &ctx).await)
    }

    /// Execute a pre-built graph with a caller-provided context.
    pub async fn execute_graph(
        &self,
        graph: &ExecutionGraph,
        ctx: &ExecutionContext,
    ) -> ScheduleResult {
        let scheduler = ExecutionScheduler::new(
            self.registry.clone(),
            self.events.clone(),
            self.metrics.clone(),
        )
        .with_recovery(self.recovery.clone());
        scheduler.run(graph, ctx).await
    }
}
