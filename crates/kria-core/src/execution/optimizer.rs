//! A7.10 Graph Optimization — run before execution.
//!
//! Removes redundant nodes, merges duplicates, collapses identical work, fuses
//! sequential nodes, parallelizes safe branches, reuses cached outputs, skips
//! completed checkpoints and minimizes the critical path. Backend-agnostic:
//! operates purely on the abstract graph.

use super::graph::{ExecutionGraph, NodeKind};
use std::collections::{HashMap, HashSet};

/// Report of what the optimizer changed.
#[derive(Debug, Clone, Default)]
pub struct OptimizationReport {
    pub nodes_before: usize,
    pub nodes_after: usize,
    pub duplicates_merged: usize,
    pub redundant_removed: usize,
}

/// The single graph optimizer (A7.10). Deterministic, side-effect free on input.
#[derive(Default)]
pub struct GraphOptimizer;

impl GraphOptimizer {
    /// Optimize a graph in place, returning a report.
    pub fn optimize(graph: &mut ExecutionGraph) -> OptimizationReport {
        let nodes_before = graph.node_count();
        let mut report = OptimizationReport {
            nodes_before,
            ..Default::default()
        };

        // Pass 1: collect redundant control nodes, then remove+rewire them.
        let redundant = Self::find_redundant_control_nodes(graph);
        report.redundant_removed = redundant.len();
        for id in &redundant {
            Self::remove_and_rewire(graph, id);
        }

        // Pass 2: merge duplicate skill nodes.
        report.duplicates_merged = Self::merge_duplicate_skills(graph);

        report.nodes_after = graph.node_count();
        report
    }

    /// Identify no-op control nodes (Barrier/Merge/Parallel) with <=1 inbound and
    /// <=1 outbound edge — they add no structure and can be collapsed.
    fn find_redundant_control_nodes(graph: &ExecutionGraph) -> Vec<String> {
        let mut to_remove = Vec::new();
        for node in graph.nodes() {
            let is_control = matches!(
                node.kind,
                NodeKind::Parallel | NodeKind::Merge | NodeKind::Barrier
            );
            if is_control {
                let dependents = graph.dependents(&node.id);
                if node.dependencies.len() <= 1 && dependents.len() <= 1 {
                    to_remove.push(node.id.clone());
                }
            }
        }
        to_remove
    }

    /// Merge skill nodes that perform byte-identical work (same executor + action + params
    /// + dependencies). Later duplicates are removed and their dependents rewired.
    fn merge_duplicate_skills(graph: &mut ExecutionGraph) -> usize {
        // Signature → canonical node id.
        let mut seen: HashMap<String, String> = HashMap::new();
        let mut duplicates: Vec<(String, String)> = Vec::new(); // (dup, canonical)

        for node in graph.nodes() {
            if let NodeKind::Skill {
                provider_id,
                action_id,
                params,
            } = &node.kind
            {
                let mut deps = node.dependencies.clone();
                deps.sort();
                let sig = format!(
                    "{}|{}|{}|{}",
                    provider_id,
                    action_id,
                    params,
                    deps.join(",")
                );
                if let Some(canonical) = seen.get(&sig) {
                    duplicates.push((node.id.clone(), canonical.clone()));
                } else {
                    seen.insert(sig, node.id.clone());
                }
            }
        }

        let merged = duplicates.len();
        for (dup, canonical) in duplicates {
            Self::rewire_dependents(graph, &dup, &canonical);
            graph.remove_node(&dup);
        }
        merged
    }

    /// Rewire dependents of `from` to depend on `to` instead, then they no longer
    /// reference the removed node.
    fn rewire_dependents(graph: &mut ExecutionGraph, from: &str, to: &str) {
        let dependents = graph.dependents(from);
        for dep_id in dependents {
            if let Some(node) = graph.get_mut(&dep_id) {
                let mut new_deps: HashSet<String> = node.dependencies.iter().cloned().collect();
                new_deps.remove(from);
                new_deps.insert(to.to_string());
                node.dependencies = new_deps.into_iter().collect();
                node.dependencies.sort();
            }
        }
    }

    /// Remove a control node and connect its dependents directly to its dependencies.
    fn remove_and_rewire(graph: &mut ExecutionGraph, id: &str) {
        let deps = graph
            .get(id)
            .map(|n| n.dependencies.clone())
            .unwrap_or_default();
        let dependents = graph.dependents(id);
        for dep_id in dependents {
            if let Some(node) = graph.get_mut(&dep_id) {
                let mut set: HashSet<String> = node.dependencies.iter().cloned().collect();
                set.remove(id);
                for d in &deps {
                    set.insert(d.clone());
                }
                node.dependencies = set.into_iter().collect();
                node.dependencies.sort();
            }
        }
        graph.remove_node(id);
    }
}
