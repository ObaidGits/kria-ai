//! Batch 3 — Operational Workflow Graph (OpGraph)
//!
//! OpGraph is a bounded, immutable planning abstraction that compiles into
//! GoalTree. It does NOT execute actions. It is a typed representation of
//! operational intent, dependencies, and execution policy.
//!
//! Core invariants:
//! - Bounded node/edge counts
//! - No hidden runtime authority
//! - Deterministic ordering via explicit edges
//! - Immutable once frozen (planner-only)

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

use crate::agent::execution_verifier::Verifiability;
use crate::agent::goal_tree::{
    RecoveryPath, StageAction, StageContextHints, VerificationCheckpoint,
};
use crate::safety::RiskLevel;

/// Maximum nodes allowed in a single OpGraph.
pub const MAX_OPGRAPH_NODES: usize = 24;
/// Maximum edges allowed in a single OpGraph.
pub const MAX_OPGRAPH_EDGES: usize = 64;

/// High-level operational domain for a node or workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowDomain {
    Coding,
    Debugging,
    Browser,
    Deployment,
    Filesystem,
    JiraDevops,
    VmContainer,
    Communication,
    Research,
    Recovery,
    SystemOperations,
    Unknown,
}

/// Dependency semantics between intents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyType {
    Hard,
    Soft,
    Recoverable,
    Optional,
}

/// Confirmation expectation for a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfirmationPolicy {
    None,
    Notice,
    Clarify,
    Confirm,
}

/// Rollback ownership boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RollbackOwnership {
    None,
    Stage,
    Workflow,
}

/// Retry policy for a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u8,
    pub delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 0,
            delay_ms: 0,
        }
    }
}

/// Timeout policy for a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeoutPolicy {
    pub stage_timeout_sec: u64,
    pub action_timeout_ms: Option<u64>,
}

impl Default for TimeoutPolicy {
    fn default() -> Self {
        Self {
            stage_timeout_sec: crate::agent::goal_tree::MAX_STAGE_DURATION_SEC,
            action_timeout_ms: None,
        }
    }
}

/// Policy and verification metadata attached to an OpGraph node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpNodeMetadata {
    pub risk: RiskLevel,
    pub confirmation: ConfirmationPolicy,
    pub expected_evidence: Option<String>,
    pub verifiability: Option<Verifiability>,
    pub workflow_domain: WorkflowDomain,
    pub rollback: RollbackOwnership,
    pub retry_policy: RetryPolicy,
    pub timeout_policy: TimeoutPolicy,
}

impl Default for OpNodeMetadata {
    fn default() -> Self {
        Self {
            risk: RiskLevel::Green,
            confirmation: ConfirmationPolicy::None,
            expected_evidence: None,
            verifiability: None,
            workflow_domain: WorkflowDomain::Unknown,
            rollback: RollbackOwnership::None,
            retry_policy: RetryPolicy::default(),
            timeout_policy: TimeoutPolicy::default(),
        }
    }
}

/// An intent-level node (planning-only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentNode {
    pub summary: String,
    pub dependency: DependencyType,
    /// Optional structured GUI intent (used for GoalTree compilation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gui_intent: Option<crate::agent::multi_intent::GuiIntent>,
}

/// A subgoal node (planning-only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgoalNode {
    pub summary: String,
    pub dependency: DependencyType,
}

/// An executable action-stage node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionStageNode {
    pub actions: Vec<StageAction>,
    pub checkpoint: VerificationCheckpoint,
    pub recovery: Option<RecoveryPath>,
    pub context_hints: StageContextHints,
    pub skippable: bool,
    pub timeout_sec: u64,
}

/// A verification-only node (non-executable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationNode {
    pub checkpoint: VerificationCheckpoint,
}

/// A checkpoint boundary node (non-executable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointNode {
    pub label: String,
}

/// A rollback boundary node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryBoundaryNode {
    pub label: String,
    pub rollback_to: Option<String>,
}

/// Node variants for OpGraph.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpNodeKind {
    Intent(IntentNode),
    Subgoal(SubgoalNode),
    ActionStage(ActionStageNode),
    Verification(VerificationNode),
    Checkpoint(CheckpointNode),
    RecoveryBoundary(RecoveryBoundaryNode),
}

/// A single node in the OpGraph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpNode {
    pub id: String,
    pub label: String,
    pub kind: OpNodeKind,
    #[serde(default)]
    pub metadata: OpNodeMetadata,
}

/// Edge types in OpGraph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpEdgeKind {
    DependsOn,
    Blocks,
    Requires,
    Fallback,
    RetryAfter,
    RollbackTo,
}

/// A directed edge between two nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpEdge {
    pub from: String,
    pub to: String,
    pub kind: OpEdgeKind,
    pub dependency: DependencyType,
}

/// Operational workflow graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpGraph {
    pub graph_id: String,
    pub description: String,
    pub nodes: Vec<OpNode>,
    pub edges: Vec<OpEdge>,
    pub created_at: u64,
    pub frozen: bool,
}

/// OpGraph validation errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpGraphValidationError {
    TooManyNodes { count: usize, max: usize },
    TooManyEdges { count: usize, max: usize },
    DuplicateNodeId { id: String },
    MissingNode { id: String },
    CycleDetected { path: Vec<String> },
}

impl OpGraph {
    pub fn new(graph_id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            graph_id: graph_id.into(),
            description: description.into(),
            nodes: Vec::new(),
            edges: Vec::new(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            frozen: false,
        }
    }

    pub fn validate(&self) -> Vec<OpGraphValidationError> {
        let mut errors = Vec::new();
        if self.nodes.len() > MAX_OPGRAPH_NODES {
            errors.push(OpGraphValidationError::TooManyNodes {
                count: self.nodes.len(),
                max: MAX_OPGRAPH_NODES,
            });
        }
        if self.edges.len() > MAX_OPGRAPH_EDGES {
            errors.push(OpGraphValidationError::TooManyEdges {
                count: self.edges.len(),
                max: MAX_OPGRAPH_EDGES,
            });
        }

        let mut seen = HashSet::new();
        for node in &self.nodes {
            if !seen.insert(node.id.clone()) {
                errors.push(OpGraphValidationError::DuplicateNodeId {
                    id: node.id.clone(),
                });
            }
        }

        let node_ids: HashSet<String> = self.nodes.iter().map(|n| n.id.clone()).collect();
        for edge in &self.edges {
            if !node_ids.contains(&edge.from) {
                errors.push(OpGraphValidationError::MissingNode {
                    id: edge.from.clone(),
                });
            }
            if !node_ids.contains(&edge.to) {
                errors.push(OpGraphValidationError::MissingNode {
                    id: edge.to.clone(),
                });
            }
        }

        if let Err(err) = self.detect_cycles() {
            errors.push(err);
        }

        errors
    }

    /// Return a deterministic topological order based on dependency edges.
    pub fn topo_order(&self) -> Result<Vec<String>, OpGraphValidationError> {
        self.detect_cycles()?;

        let mut indegree: HashMap<String, usize> =
            self.nodes.iter().map(|n| (n.id.clone(), 0)).collect();
        let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();

        for edge in &self.edges {
            if matches!(
                edge.kind,
                OpEdgeKind::DependsOn | OpEdgeKind::Requires | OpEdgeKind::Blocks
            ) {
                let entry = outgoing.entry(edge.from.clone()).or_default();
                entry.push(edge.to.clone());
                if let Some(value) = indegree.get_mut(&edge.to) {
                    *value += 1;
                }
            }
        }

        let mut queue: VecDeque<String> = indegree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(k, _)| k.clone())
            .collect();
        let mut ordered = Vec::with_capacity(indegree.len());

        while let Some(node) = queue.pop_front() {
            ordered.push(node.clone());
            if let Some(children) = outgoing.get(&node) {
                for child in children {
                    if let Some(d) = indegree.get_mut(child) {
                        *d = d.saturating_sub(1);
                        if *d == 0 {
                            queue.push_back(child.clone());
                        }
                    }
                }
            }
        }

        if ordered.len() != indegree.len() {
            return Err(OpGraphValidationError::CycleDetected { path: Vec::new() });
        }

        Ok(ordered)
    }

    fn detect_cycles(&self) -> Result<(), OpGraphValidationError> {
        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();

        for edge in &self.edges {
            if matches!(
                edge.kind,
                OpEdgeKind::DependsOn | OpEdgeKind::Requires | OpEdgeKind::Blocks
            ) {
                adjacency
                    .entry(edge.from.clone())
                    .or_default()
                    .push(edge.to.clone());
            }
        }

        for node in &self.nodes {
            if !visited.contains(&node.id) {
                if let Some(path) = visit_cycle(
                    &node.id,
                    &adjacency,
                    &mut visiting,
                    &mut visited,
                    &mut Vec::new(),
                ) {
                    return Err(OpGraphValidationError::CycleDetected { path });
                }
            }
        }
        Ok(())
    }
}

fn visit_cycle(
    node: &str,
    adjacency: &HashMap<String, Vec<String>>,
    visiting: &mut HashSet<String>,
    visited: &mut HashSet<String>,
    stack: &mut Vec<String>,
) -> Option<Vec<String>> {
    if visiting.contains(node) {
        stack.push(node.to_string());
        return Some(stack.clone());
    }
    if visited.contains(node) {
        return None;
    }

    visiting.insert(node.to_string());
    stack.push(node.to_string());

    if let Some(children) = adjacency.get(node) {
        for child in children {
            if let Some(path) = visit_cycle(child, adjacency, visiting, visited, stack) {
                return Some(path);
            }
        }
    }

    visiting.remove(node);
    visited.insert(node.to_string());
    stack.pop();
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topo_order_linear() {
        let mut graph = OpGraph::new("g1", "test");
        graph.nodes.push(OpNode {
            id: "n1".into(),
            label: "n1".into(),
            kind: OpNodeKind::Checkpoint(CheckpointNode { label: "c1".into() }),
            metadata: OpNodeMetadata::default(),
        });
        graph.nodes.push(OpNode {
            id: "n2".into(),
            label: "n2".into(),
            kind: OpNodeKind::Checkpoint(CheckpointNode { label: "c2".into() }),
            metadata: OpNodeMetadata::default(),
        });
        graph.edges.push(OpEdge {
            from: "n1".into(),
            to: "n2".into(),
            kind: OpEdgeKind::DependsOn,
            dependency: DependencyType::Hard,
        });

        let order = graph.topo_order().expect("topo order");
        assert_eq!(order, vec!["n1".to_string(), "n2".to_string()]);
    }

    #[test]
    fn detects_cycle() {
        let mut graph = OpGraph::new("g2", "cycle");
        graph.nodes.push(OpNode {
            id: "a".into(),
            label: "a".into(),
            kind: OpNodeKind::Checkpoint(CheckpointNode { label: "a".into() }),
            metadata: OpNodeMetadata::default(),
        });
        graph.nodes.push(OpNode {
            id: "b".into(),
            label: "b".into(),
            kind: OpNodeKind::Checkpoint(CheckpointNode { label: "b".into() }),
            metadata: OpNodeMetadata::default(),
        });
        graph.edges.push(OpEdge {
            from: "a".into(),
            to: "b".into(),
            kind: OpEdgeKind::DependsOn,
            dependency: DependencyType::Hard,
        });
        graph.edges.push(OpEdge {
            from: "b".into(),
            to: "a".into(),
            kind: OpEdgeKind::DependsOn,
            dependency: DependencyType::Hard,
        });

        let errors = graph.validate();
        assert!(errors
            .iter()
            .any(|e| matches!(e, OpGraphValidationError::CycleDetected { .. })));
    }
}
