//! A7.2 Execution Planner — deterministic Goal → ExecutionGraph.
//!
//! The planner chooses WHAT to run; the executor decides HOW. It contains ZERO
//! executor-specific logic: it only assigns an open-vocabulary `provider_id` to
//! each step based on the step's declared backend, then emits an abstract graph.
//!
//! Input:  Goal + available executors + available skills + capabilities +
//!         dependencies + resources.
//! Output: an `ExecutionGraph` (backend-agnostic).

use super::executor::ExecutorRegistry;
use super::graph::{ExecutionGraph, GraphNode, NodeKind};
use serde::{Deserialize, Serialize};

/// A single planned step in a goal. Backend selection is data — the planner never
/// branches on a concrete backend's behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// Stable step id (becomes the node id).
    pub id: String,
    /// Which provider (open-vocabulary id) should run this step.
    pub provider_id: String,
    /// Logical action/skill id.
    pub action_id: String,
    /// Parameters for the action.
    pub params: serde_json::Value,
    /// Step ids this step depends on.
    pub dependencies: Vec<String>,
    /// Optional: run this step in parallel with siblings sharing the same parallel group.
    pub parallel_group: Option<String>,
}

impl PlanStep {
    pub fn new(
        id: impl Into<String>,
        provider_id: impl Into<String>,
        action_id: impl Into<String>,
        params: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            provider_id: provider_id.into(),
            action_id: action_id.into(),
            params,
            dependencies: Vec::new(),
            parallel_group: None,
        }
    }

    pub fn depends_on(mut self, dep: impl Into<String>) -> Self {
        self.dependencies.push(dep.into());
        self
    }
}

/// A goal handed to the planner: an ordered set of steps + metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub description: String,
    pub steps: Vec<PlanStep>,
}

impl Goal {
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            steps: Vec::new(),
        }
    }

    pub fn with_step(mut self, step: PlanStep) -> Self {
        self.steps.push(step);
        self
    }
}

/// Errors from planning.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("goal has no steps")]
    EmptyGoal,
    #[error("step '{step}' requires provider '{provider_id}' which is not registered")]
    ExecutorUnavailable { step: String, provider_id: String },
    #[error("duplicate step id: {0}")]
    DuplicateStep(String),
}

/// The single execution planner (A7.2). Deterministic — same input → same graph.
#[derive(Default)]
pub struct ExecutionPlanner;

impl ExecutionPlanner {
    /// Build an execution graph from a goal.
    ///
    /// Validates every required executor is registered (A7.2 uses the registry only
    /// to check availability — it never calls into a backend).
    pub fn plan(goal: &Goal, registry: &ExecutorRegistry) -> Result<ExecutionGraph, PlanError> {
        if goal.steps.is_empty() {
            return Err(PlanError::EmptyGoal);
        }

        let graph_id = format!("graph-{}", goal.id);
        let mut graph = ExecutionGraph::new(graph_id, goal.id.clone());
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        for step in &goal.steps {
            if !seen.insert(step.id.clone()) {
                return Err(PlanError::DuplicateStep(step.id.clone()));
            }
            if !registry.has(&step.provider_id) {
                return Err(PlanError::ExecutorUnavailable {
                    step: step.id.clone(),
                    provider_id: step.provider_id.clone(),
                });
            }

            let mut node = GraphNode::new(
                step.id.clone(),
                NodeKind::Skill {
                    provider_id: step.provider_id.clone(),
                    action_id: step.action_id.clone(),
                    params: step.params.clone(),
                },
            );
            node.dependencies = step.dependencies.clone();
            node = node.with_label(format!("{}::{}", step.provider_id, step.action_id));
            graph.add_node(node);
        }

        Ok(graph)
    }
}
