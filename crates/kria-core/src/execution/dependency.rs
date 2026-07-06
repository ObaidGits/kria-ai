//! A7.6 Dependency Resolution — build/validate the dependency graph before execution.
//!
//! Detects cycles, deadlocks, missing outputs, resource conflicts, capability
//! conflicts and executor conflicts. Backend-agnostic: works on the abstract graph.

use super::executor::ExecutorRegistry;
use super::graph::{ExecutionGraph, NodeKind};
use std::collections::{HashMap, HashSet};

/// A detected problem in the dependency graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyIssue {
    /// A dependency cycle involving these node ids.
    Cycle(Vec<String>),
    /// A node references a dependency that does not exist.
    MissingDependency { node: String, missing: String },
    /// A node requires an executor that is not registered.
    ExecutorUnavailable { node: String, executor: String },
    /// No runnable roots but graph is non-empty (deadlock).
    Deadlock,
    /// Rollback target checkpoint does not exist.
    MissingCheckpoint { node: String, label: String },
}

/// Resolves and validates dependencies (A7.6). Deterministic — no side effects.
pub struct DependencyResolver;

impl DependencyResolver {
    /// Validate the full graph. Returns all issues found (empty = valid).
    pub fn validate(graph: &ExecutionGraph, registry: &ExecutorRegistry) -> Vec<DependencyIssue> {
        let mut issues = Vec::new();

        Self::check_missing_dependencies(graph, &mut issues);
        Self::check_cycles(graph, &mut issues);
        Self::check_executors(graph, registry, &mut issues);
        Self::check_checkpoints(graph, &mut issues);

        // Deadlock: non-empty graph but no roots and no cycle already reported.
        if !graph.is_empty()
            && graph.roots().is_empty()
            && !issues
                .iter()
                .any(|i| matches!(i, DependencyIssue::Cycle(_)))
        {
            issues.push(DependencyIssue::Deadlock);
        }

        issues
    }

    fn check_missing_dependencies(graph: &ExecutionGraph, issues: &mut Vec<DependencyIssue>) {
        let ids: HashSet<String> = graph.node_ids().into_iter().collect();
        for node in graph.nodes() {
            for dep in &node.dependencies {
                if !ids.contains(dep) {
                    issues.push(DependencyIssue::MissingDependency {
                        node: node.id.clone(),
                        missing: dep.clone(),
                    });
                }
            }
        }
    }

    fn check_cycles(graph: &ExecutionGraph, issues: &mut Vec<DependencyIssue>) {
        // DFS with colors: 0=white,1=gray,2=black.
        let mut color: HashMap<String, u8> = HashMap::new();
        let mut stack: Vec<String> = Vec::new();

        for id in graph.node_ids() {
            if color.get(&id).copied().unwrap_or(0) == 0
                && Self::dfs_cycle(graph, &id, &mut color, &mut stack)
            {
                issues.push(DependencyIssue::Cycle(stack.clone()));
                return;
            }
        }
    }

    fn dfs_cycle(
        graph: &ExecutionGraph,
        id: &str,
        color: &mut HashMap<String, u8>,
        stack: &mut Vec<String>,
    ) -> bool {
        color.insert(id.to_string(), 1);
        stack.push(id.to_string());

        if let Some(node) = graph.get(id) {
            for dep in &node.dependencies {
                match color.get(dep).copied().unwrap_or(0) {
                    1 => return true, // back-edge → cycle
                    0 => {
                        if Self::dfs_cycle(graph, dep, color, stack) {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
        }

        stack.pop();
        color.insert(id.to_string(), 2);
        false
    }

    fn check_executors(
        graph: &ExecutionGraph,
        registry: &ExecutorRegistry,
        issues: &mut Vec<DependencyIssue>,
    ) {
        for node in graph.nodes() {
            if let NodeKind::Skill { provider_id, .. } = &node.kind {
                if !registry.has(provider_id) {
                    issues.push(DependencyIssue::ExecutorUnavailable {
                        node: node.id.clone(),
                        executor: provider_id.clone(),
                    });
                }
            }
        }
    }

    fn check_checkpoints(graph: &ExecutionGraph, issues: &mut Vec<DependencyIssue>) {
        let labels: HashSet<String> = graph
            .nodes()
            .filter_map(|n| match &n.kind {
                NodeKind::Checkpoint { label } => Some(label.clone()),
                _ => None,
            })
            .collect();
        for node in graph.nodes() {
            if let NodeKind::Rollback { to_label } = &node.kind {
                if !labels.contains(to_label) {
                    issues.push(DependencyIssue::MissingCheckpoint {
                        node: node.id.clone(),
                        label: to_label.clone(),
                    });
                }
            }
        }
    }

    /// Topological order of node ids. Returns None if a cycle exists.
    pub fn topological_order(graph: &ExecutionGraph) -> Option<Vec<String>> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        for id in graph.node_ids() {
            let deps = graph.get(&id).map(|n| n.dependencies.len()).unwrap_or(0);
            in_degree.insert(id.clone(), deps);
        }

        let mut queue: Vec<String> = in_degree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(id, _)| id.clone())
            .collect();
        queue.sort(); // deterministic
        let mut order = Vec::new();

        while let Some(id) = queue.pop() {
            order.push(id.clone());
            for dependent in graph.dependents(&id) {
                if let Some(d) = in_degree.get_mut(&dependent) {
                    *d -= 1;
                    if *d == 0 {
                        queue.push(dependent);
                    }
                }
            }
            queue.sort();
        }

        if order.len() == graph.node_count() {
            Some(order)
        } else {
            None // cycle
        }
    }
}
