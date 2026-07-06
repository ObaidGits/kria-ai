//! A7.1 Execution Graph — ONE authoritative graph. Backend-agnostic.
//!
//! Goal → Execution Graph → Execution → Result. Nodes execute through an
//! `Executor` selected by an open-vocabulary `provider_id`, never through a
//! concrete backend. No duplicate graph systems exist.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The kind of work a node represents (A7.1). Control-flow kinds are backend-agnostic;
/// only `Skill` carries a provider selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeKind {
    /// A unit of work dispatched to a provider's executor, addressed by the
    /// open-vocabulary `provider_id` (replaces the former closed `ExecutorKindTag`).
    Skill {
        provider_id: String,
        action_id: String,
        params: serde_json::Value,
    },
    /// Branch on a condition evaluated against context.
    Condition { expression: String },
    /// Fan-out: run children in parallel.
    Parallel,
    /// Fan-in: wait for all inbound branches, merge outputs.
    Merge,
    /// Retry wrapper around a child node.
    Retry { max_attempts: u32 },
    /// Timeout wrapper around a child node.
    Timeout { millis: u64 },
    /// Loop over a child node while condition holds.
    Loop {
        expression: String,
        max_iterations: u32,
    },
    /// Wait for a duration or external signal.
    Wait { millis: u64 },
    /// Synchronization barrier across branches.
    Barrier,
    /// Nested subgraph.
    Subgraph { graph_id: String },
    /// Persist a checkpoint of context state.
    Checkpoint { label: String },
    /// Roll back to a prior checkpoint.
    Rollback { to_label: String },
}

/// A single node in the execution graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub kind: NodeKind,
    /// Node ids this node depends on (must complete first).
    pub dependencies: Vec<String>,
    /// Optional human-readable label.
    pub label: Option<String>,
}

impl GraphNode {
    pub fn new(id: impl Into<String>, kind: NodeKind) -> Self {
        Self {
            id: id.into(),
            kind,
            dependencies: Vec::new(),
            label: None,
        }
    }

    pub fn depends_on(mut self, dep: impl Into<String>) -> Self {
        self.dependencies.push(dep.into());
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Short kind name for events/metrics.
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            NodeKind::Skill { .. } => "skill",
            NodeKind::Condition { .. } => "condition",
            NodeKind::Parallel => "parallel",
            NodeKind::Merge => "merge",
            NodeKind::Retry { .. } => "retry",
            NodeKind::Timeout { .. } => "timeout",
            NodeKind::Loop { .. } => "loop",
            NodeKind::Wait { .. } => "wait",
            NodeKind::Barrier => "barrier",
            NodeKind::Subgraph { .. } => "subgraph",
            NodeKind::Checkpoint { .. } => "checkpoint",
            NodeKind::Rollback { .. } => "rollback",
        }
    }
}

/// The authoritative execution graph (A7.1). A DAG of `GraphNode`s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionGraph {
    pub id: String,
    pub goal_id: String,
    nodes: HashMap<String, GraphNode>,
    /// Insertion order preserved for deterministic traversal.
    order: Vec<String>,
}

impl ExecutionGraph {
    pub fn new(id: impl Into<String>, goal_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            goal_id: goal_id.into(),
            nodes: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// Add a node. Replaces any node with the same id.
    pub fn add_node(&mut self, node: GraphNode) {
        if !self.nodes.contains_key(&node.id) {
            self.order.push(node.id.clone());
        }
        self.nodes.insert(node.id.clone(), node);
    }

    /// Remove a node by id (used by the optimizer).
    pub fn remove_node(&mut self, id: &str) {
        self.nodes.remove(id);
        self.order.retain(|n| n != id);
    }

    pub fn get(&self, id: &str) -> Option<&GraphNode> {
        self.nodes.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut GraphNode> {
        self.nodes.get_mut(id)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Nodes in insertion order.
    pub fn nodes(&self) -> impl Iterator<Item = &GraphNode> {
        self.order.iter().filter_map(move |id| self.nodes.get(id))
    }

    /// All node ids in insertion order.
    pub fn node_ids(&self) -> Vec<String> {
        self.order.clone()
    }

    /// Nodes with no dependencies (roots).
    pub fn roots(&self) -> Vec<String> {
        self.nodes()
            .filter(|n| n.dependencies.is_empty())
            .map(|n| n.id.clone())
            .collect()
    }

    /// Direct dependents of a node (reverse edges).
    pub fn dependents(&self, id: &str) -> Vec<String> {
        self.nodes()
            .filter(|n| n.dependencies.iter().any(|d| d == id))
            .map(|n| n.id.clone())
            .collect()
    }

    /// Maximum depth of the graph (critical-path length in nodes).
    pub fn depth(&self) -> usize {
        let mut memo: HashMap<String, usize> = HashMap::new();
        self.node_ids()
            .iter()
            .map(|id| self.depth_of(id, &mut memo))
            .max()
            .unwrap_or(0)
    }

    fn depth_of(&self, id: &str, memo: &mut HashMap<String, usize>) -> usize {
        if let Some(d) = memo.get(id) {
            return *d;
        }
        let node = match self.nodes.get(id) {
            Some(n) => n,
            None => return 0,
        };
        let d = if node.dependencies.is_empty() {
            1
        } else {
            1 + node
                .dependencies
                .iter()
                .map(|dep| self.depth_of(dep, memo))
                .max()
                .unwrap_or(0)
        };
        memo.insert(id.to_string(), d);
        d
    }
}
