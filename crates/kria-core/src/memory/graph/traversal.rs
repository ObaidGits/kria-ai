//! Breadth-first and path traversal for the policy-safe bounded graph contract.
//!
//! **Task 2.3.3** — implements cycle-safe ≤3-hop BFS and shortest-path
//! traversal as required by MGR-007 AC 5/6 and design §6.5/§A6:
//!
//! * Per-hop node caps: 40 (seeds), 40/30/20 (hops 1–3).
//! * Hard total caps: 120 nodes, 180 edges.
//! * Cycle guard: `HashSet<String>` of visited entity-ID strings.
//! * Hidden intermediary: unauthorized nodes increment `hidden_node_count` and
//!   are NOT expanded; if any node on a path is unauthorized the ENTIRE path is
//!   omitted.
//! * Deterministic ordering: within each hop, candidates are stable-sorted by
//!   `entity_type` (None < Some) then `display_name` (None < Some) before caps
//!   are applied; edges are sorted by `link_type` then `identity_hash`.
//! * Evidence counts: pre-populated on [`TraversalEdge`] by the store layer;
//!   the engine reads them directly.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::memory::graph::projection::{DirectionClass, EdgeAuthorityClass, TruncationReason};
use crate::memory::model::{EntityId, GraphRevision, TruthState, UtcTimestamp};
use crate::memory::types::StalenessClass;

// ── Traversal constants (design §6.5) ────────────────────────────────────

/// Maximum hop depth enforced by the traversal engine (≤3 hops per A6).
pub const MAX_HOPS: u8 = 3;
/// Per-hop node cap for hop 0 (seed nodes).
pub const HOP_0_NODE_CAP: u32 = 40;
/// Per-hop node cap for hop 1 (first expansion).
pub const HOP_1_NODE_CAP: u32 = 40;
/// Per-hop node cap for hop 2 (second expansion).
pub const HOP_2_NODE_CAP: u32 = 30;
/// Per-hop node cap for hop 3 (third expansion).
pub const HOP_3_NODE_CAP: u32 = 20;
/// Hard total node cap across all hops.
pub const MAX_TOTAL_NODES: u32 = 120;
/// Hard total edge cap across all hops.
pub const MAX_TOTAL_EDGES: u32 = 180;
/// Cycle guard: maximum visited nodes across the full query.
pub const MAX_VISITED_NODES: u32 = 120;
/// Edge visit guard: maximum visited edges across the full query.
pub const MAX_VISITED_EDGES: u32 = 180;

// ── TraversalNode ────────────────────────────────────────────────────────

/// A node in the adjacency representation supplied by the store layer.
///
/// `is_authorized = false` means the node is hidden by policy. Hidden nodes
/// are counted but NOT added to the result and NOT expanded.
#[derive(Debug, Clone)]
pub struct TraversalNode {
    /// Stable entity identity.
    pub id: EntityId,
    /// `false` = hidden by policy; the node is NOT returned or expanded.
    pub is_authorized: bool,
    /// Free-text entity type (e.g. `"person"`, `"project"`).
    pub entity_type: Option<String>,
    /// Human-facing display name. MUST NOT be a raw UUID.
    pub display_name: Option<String>,
    /// Truth disposition.
    pub truth_state: TruthState,
    /// Valid-time lower bound.
    pub valid_from: Option<UtcTimestamp>,
    /// Valid-time upper bound.
    pub valid_until: Option<UtcTimestamp>,
    /// Policy namespace.
    pub namespace: String,
    /// Policy scope.
    pub scope: String,
    /// Effective sensitivity level `0..=3`.
    pub sensitivity: u8,
    /// Policy provenance hash.
    pub policy_version: String,
    /// Transaction-time creation instant.
    pub created_at: Option<UtcTimestamp>,
    /// Originating source kind (e.g. `"native"`, `"mcp"`).
    pub source_kind: Option<String>,
    /// Opaque actor identifier.
    pub actor_id: Option<String>,
    /// Re-verification class.
    pub staleness_class: Option<StalenessClass>,
    /// The graph revision at which this row was read.
    pub revision: GraphRevision,
}

// ── TraversalEdge ────────────────────────────────────────────────────────

/// An edge in the adjacency representation supplied by the store layer.
///
/// `source_authorized` and `target_authorized` gate edge visibility: an edge
/// is only added to [`TraversalResult::edges`] when both are `true`.
#[derive(Debug, Clone)]
pub struct TraversalEdge {
    /// Stable relationship identity hash (BLAKE3 hex from `identity_hash`).
    pub identity_hash: String,
    /// Relation name from the registry.
    pub link_type: String,
    /// Relation version.
    pub link_type_version: u32,
    /// Authority class of this edge.
    pub authority_class: EdgeAuthorityClass,
    /// Direction semantics.
    pub direction: DirectionClass,
    /// Source entity id.
    pub source_id: EntityId,
    /// Target entity id.
    pub target_id: EntityId,
    /// `false` = source endpoint hidden by policy.
    pub source_authorized: bool,
    /// `false` = target endpoint hidden by policy.
    pub target_authorized: bool,
    /// Truth disposition of this edge.
    pub truth_state: TruthState,
    /// Valid-time lower bound.
    pub valid_from: Option<UtcTimestamp>,
    /// Valid-time upper bound.
    pub valid_until: Option<UtcTimestamp>,
    /// Policy namespace.
    pub namespace: String,
    /// Policy scope.
    pub scope: String,
    /// Effective sensitivity level `0..=3`.
    pub sensitivity: u8,
    /// Policy provenance hash.
    pub policy_version: String,
    /// Count of evidence records attached to this edge (pre-populated by store).
    pub evidence_count: u32,
    /// Transaction-time creation instant.
    pub created_at: Option<UtcTimestamp>,
    /// Originating source kind.
    pub source_kind: Option<String>,
    /// Opaque actor identifier.
    pub actor_id: Option<String>,
    /// Re-verification class.
    pub staleness_class: Option<StalenessClass>,
    /// The graph revision at which this row was read.
    pub revision: GraphRevision,
}

// ── TraversalResult ──────────────────────────────────────────────────────

/// The output of a bounded BFS traversal.
#[derive(Debug, Clone)]
pub struct TraversalResult {
    /// All authorized nodes visited, in BFS order within each hop then
    /// stable-sorted by entity_type / display_name within each hop.
    pub nodes: Vec<TraversalNode>,
    /// All edges where both `source_authorized` and `target_authorized` are
    /// `true`, stable-sorted by link_type then identity_hash.
    pub edges: Vec<TraversalEdge>,
    /// Whether any limit (hop, node, edge) was hit.
    pub truncated: bool,
    /// The reason for truncation, when `truncated` is `true`.
    pub truncation_reason: Option<TruncationReason>,
    /// Count of nodes that were encountered but are unauthorized. Does NOT
    /// expose the hidden IDs — only the count.
    pub hidden_node_count: u32,
    /// The deepest hop actually reached.
    pub max_hop_reached: u8,
}

// ── PathStep ────────────────────────────────────────────────────────────

/// One hop in a shortest authorized path.
#[derive(Debug, Clone)]
pub struct PathStep {
    /// The node at this step.
    pub node: TraversalNode,
    /// The edge leading to the NEXT node. `None` for the last step.
    pub edge_to_next: Option<TraversalEdge>,
}

// ── Private adjacency index ───────────────────────────────────────────────

/// Build an adjacency list: for each node-id, the set of edges touching it
/// (both source→target for directed, and also target→source for symmetric).
fn build_adjacency(all_edges: &[TraversalEdge]) -> HashMap<String, Vec<usize>> {
    let mut adj: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, edge) in all_edges.iter().enumerate() {
        adj.entry(edge.source_id.as_str().to_owned())
            .or_default()
            .push(i);
        // Symmetric edges are traversable in both directions.
        if edge.direction == DirectionClass::Symmetric {
            adj.entry(edge.target_id.as_str().to_owned())
                .or_default()
                .push(i);
        }
    }
    adj
}

/// Return the node-cap for a given hop depth.
fn hop_node_cap(hop: u8) -> u32 {
    match hop {
        0 => HOP_0_NODE_CAP,
        1 => HOP_1_NODE_CAP,
        2 => HOP_2_NODE_CAP,
        _ => HOP_3_NODE_CAP,
    }
}

/// Stable sort key for a traversal node: (entity_type, display_name).
/// `None` sorts before `Some(_)` so missing metadata is front-stable.
fn node_sort_key(n: &TraversalNode) -> (Option<&str>, Option<&str>) {
    (n.entity_type.as_deref(), n.display_name.as_deref())
}

// ── BreadthFirstTraversal ────────────────────────────────────────────────

/// Stateless breadth-first traversal engine (design §6.5, MGR-007 AC 5/6).
pub struct BreadthFirstTraversal;

impl BreadthFirstTraversal {
    /// Execute a bounded BFS from `seeds` up to `max_hops` (clamped to
    /// [`MAX_HOPS`] = 3).
    ///
    /// # Guarantees
    /// * Cycle-safe: each entity ID is visited at most once.
    /// * Policy-filtered: unauthorized nodes increment `hidden_node_count`
    ///   and are NOT expanded.
    /// * Per-hop caps: `HOP_0..HOP_3` are enforced before candidates enter
    ///   the frontier.
    /// * Hard total caps: traversal stops when `MAX_TOTAL_NODES` or
    ///   `MAX_TOTAL_EDGES` is reached.
    /// * Deterministic: within each hop, candidates are stable-sorted by
    ///   `entity_type` then `display_name` before caps are applied; collected
    ///   edges are stable-sorted by `link_type` then `identity_hash`.
    pub fn execute(
        seeds: &[EntityId],
        all_nodes: &[TraversalNode],
        all_edges: &[TraversalEdge],
        max_hops: u8,
    ) -> TraversalResult {
        let max_hops = max_hops.min(MAX_HOPS);

        // Build lookup maps.
        let node_map: HashMap<String, &TraversalNode> = all_nodes
            .iter()
            .map(|n| (n.id.as_str().to_owned(), n))
            .collect();
        let adj = build_adjacency(all_edges);

        // Visited guard: once a node-id is in here we never re-expand it.
        let mut visited: HashSet<String> = HashSet::new();
        // Visited edge hashes (to avoid duplicate edge entries).
        let mut visited_edges: HashSet<String> = HashSet::new();

        let mut result_nodes: Vec<TraversalNode> = Vec::new();
        let mut result_edges: Vec<TraversalEdge> = Vec::new();
        let mut hidden_node_count: u32 = 0;
        let mut truncated = false;
        let mut truncation_reason: Option<TruncationReason> = None;
        let mut max_hop_reached: u8 = 0;

        // BFS queue: (entity_id_string, hop_depth)
        let mut queue: VecDeque<(String, u8)> = VecDeque::new();

        // ── Seed nodes (hop 0) ──────────────────────────────────────────
        // Collect candidates for hop 0, deterministically sorted, then cap.
        let mut seed_candidates: Vec<&TraversalNode> = seeds
            .iter()
            .filter_map(|seed_id| {
                let key = seed_id.as_str().to_owned();
                if visited.contains(&key) {
                    return None;
                }
                node_map.get(&key).copied()
            })
            .collect();

        // Stable-sort by entity_type then display_name.
        seed_candidates.sort_by(|a, b| node_sort_key(a).cmp(&node_sort_key(b)));
        // Apply hop-0 cap.
        seed_candidates.truncate(HOP_0_NODE_CAP as usize);

        for node in seed_candidates {
            let key = node.id.as_str().to_owned();
            if visited.contains(&key) {
                continue;
            }
            visited.insert(key.clone());

            if !node.is_authorized {
                hidden_node_count += 1;
                // Do NOT expand unauthorized seeds.
                continue;
            }

            // Hard total node cap.
            if result_nodes.len() as u32 >= MAX_TOTAL_NODES {
                truncated = true;
                truncation_reason = Some(TruncationReason::ItemLimit);
                break;
            }

            result_nodes.push(node.clone());
            if max_hops > 0 {
                queue.push_back((key, 0));
            }
        }

        // ── BFS expansion (hops 1..max_hops) ───────────────────────────
        'bfs: while let Some((current_id, current_hop)) = queue.pop_front() {
            if truncated {
                break;
            }
            let next_hop = current_hop + 1;
            if next_hop > max_hops {
                continue;
            }

            // Gather all neighbours reachable via edges from current_id.
            let edge_indices = match adj.get(&current_id) {
                Some(v) => v.clone(),
                None => continue,
            };

            // Collect the neighbour candidates for this expansion step,
            // together with their connecting edge index.
            let mut hop_candidates: Vec<(&TraversalNode, usize)> = Vec::new();

            for &ei in &edge_indices {
                let edge = &all_edges[ei];

                // Determine the neighbour id from this edge.
                let neighbour_id = if edge.source_id.as_str() == current_id {
                    edge.target_id.as_str().to_owned()
                } else {
                    edge.source_id.as_str().to_owned()
                };

                if visited.contains(&neighbour_id) {
                    continue;
                }

                let neighbour = match node_map.get(&neighbour_id) {
                    Some(n) => *n,
                    None => continue,
                };

                hop_candidates.push((neighbour, ei));
            }

            // Stable-sort by entity_type then display_name (node key), then
            // by edge identity_hash for tie-breaking determinism.
            hop_candidates.sort_by(|(a, ai), (b, bi)| {
                node_sort_key(a).cmp(&node_sort_key(b)).then_with(|| {
                    all_edges[*ai]
                        .identity_hash
                        .cmp(&all_edges[*bi].identity_hash)
                })
            });

            // Apply per-hop cap.
            let cap = hop_node_cap(next_hop) as usize;
            if hop_candidates.len() > cap {
                hop_candidates.truncate(cap);
                truncated = true;
                truncation_reason = Some(TruncationReason::ItemLimit);
            }

            for (neighbour, ei) in hop_candidates {
                if truncated && result_nodes.len() as u32 >= MAX_TOTAL_NODES {
                    break 'bfs;
                }

                let neighbour_key = neighbour.id.as_str().to_owned();
                // Guard: if already visited (from a parallel path in same hop), skip.
                if visited.contains(&neighbour_key) {
                    continue;
                }
                visited.insert(neighbour_key.clone());

                if !neighbour.is_authorized {
                    hidden_node_count += 1;
                    // Do NOT expand from unauthorized nodes.
                    continue;
                }

                // Hard total node cap.
                if result_nodes.len() as u32 >= MAX_TOTAL_NODES {
                    truncated = true;
                    truncation_reason = Some(TruncationReason::ItemLimit);
                    break 'bfs;
                }

                result_nodes.push(neighbour.clone());
                if next_hop < max_hops {
                    max_hop_reached = max_hop_reached.max(next_hop);
                    queue.push_back((neighbour_key, next_hop));
                } else {
                    max_hop_reached = max_hop_reached.max(next_hop);
                }

                // Collect the edge if both endpoints are authorized and it
                // hasn't been seen before.
                let edge = &all_edges[ei];
                if edge.source_authorized
                    && edge.target_authorized
                    && !visited_edges.contains(&edge.identity_hash)
                {
                    if result_edges.len() as u32 >= MAX_TOTAL_EDGES {
                        truncated = true;
                        truncation_reason = Some(TruncationReason::ItemLimit);
                        break 'bfs;
                    }
                    visited_edges.insert(edge.identity_hash.clone());
                    result_edges.push(edge.clone());
                }
            }
        }

        // Also collect edges between already-visited authorized nodes that we
        // may have seen during the traversal but didn't process above (e.g.
        // seed-to-seed edges).  We iterate all_edges once to pick them up.
        let authorized_visited: HashSet<&str> =
            result_nodes.iter().map(|n| n.id.as_str()).collect();
        for edge in all_edges {
            if result_edges.len() as u32 >= MAX_TOTAL_EDGES {
                truncated = true;
                truncation_reason = Some(TruncationReason::ItemLimit);
                break;
            }
            if edge.source_authorized
                && edge.target_authorized
                && authorized_visited.contains(edge.source_id.as_str())
                && authorized_visited.contains(edge.target_id.as_str())
                && !visited_edges.contains(&edge.identity_hash)
            {
                visited_edges.insert(edge.identity_hash.clone());
                result_edges.push(edge.clone());
            }
        }

        // ── Deterministic final sort ────────────────────────────────────
        // Nodes: stable sort by entity_type then display_name.
        result_nodes.sort_by(|a, b| node_sort_key(a).cmp(&node_sort_key(b)));
        // Edges: stable sort by link_type then identity_hash.
        result_edges.sort_by(|a, b| {
            a.link_type
                .cmp(&b.link_type)
                .then_with(|| a.identity_hash.cmp(&b.identity_hash))
        });

        TraversalResult {
            nodes: result_nodes,
            edges: result_edges,
            truncated,
            truncation_reason,
            hidden_node_count,
            max_hop_reached,
        }
    }
}

// ── PathTraversal ────────────────────────────────────────────────────────

/// Stateless shortest-path traversal engine (design §6.5, MGR-007 AC 6).
pub struct PathTraversal;

impl PathTraversal {
    /// Find the shortest authorized path between `from` and `to` within
    /// [`MAX_HOPS`].
    ///
    /// # Rules
    /// * Cycle-safe: no node is revisited within a single candidate path.
    /// * Hidden intermediary: if ANY node on the path is unauthorized, the
    ///   entire path is **omitted** (not returned).
    /// * Per-hop node caps apply during BFS exploration.
    /// * When multiple equal-length paths exist, the one with the
    ///   lexicographically smallest concatenated `identity_hash` sequence is
    ///   chosen (deterministic tie-break).
    /// * Returns `None` when no authorized path exists within [`MAX_HOPS`].
    pub fn find_shortest_authorized_path(
        from: &EntityId,
        to: &EntityId,
        all_nodes: &[TraversalNode],
        all_edges: &[TraversalEdge],
    ) -> Option<Vec<PathStep>> {
        // Build lookup maps.
        let node_map: HashMap<String, &TraversalNode> = all_nodes
            .iter()
            .map(|n| (n.id.as_str().to_owned(), n))
            .collect();
        let adj = build_adjacency(all_edges);

        let from_key = from.as_str().to_owned();
        let to_key = to.as_str().to_owned();

        // Trivial: from == to
        if from_key == to_key {
            let node = node_map.get(&from_key)?;
            if !node.is_authorized {
                return None;
            }
            return Some(vec![PathStep {
                node: (*node).clone(),
                edge_to_next: None,
            }]);
        }

        // BFS over paths. Each queue entry is a candidate path: a Vec of
        // (node_id, Option<edge_index>) where edge_index is the edge that
        // led to this node (None for the starting node).
        //
        // To keep memory bounded we use:
        // - Per-hop node cap enforcement via a per-hop seen set.
        // - Max-hops depth limit.
        //
        // Queue entry: (current_node_id, path_so_far)
        // path_so_far is a Vec<(node_id, Option<edge_idx>)>
        type PathEntry = Vec<(String, Option<usize>)>;
        let mut queue: VecDeque<PathEntry> = VecDeque::new();

        // Initialise with the from-node.
        queue.push_back(vec![(from_key.clone(), None)]);

        // Collect all equal-length complete paths found at the first depth
        // that produces any result, then pick the lexicographically smallest
        // by concatenated identity_hash.
        let mut found_paths: Vec<PathEntry> = Vec::new();
        let mut found_at_depth: Option<usize> = None;

        while let Some(path) = queue.pop_front() {
            let depth = path.len() - 1; // number of hops taken so far

            // If we already found paths at a shorter depth, stop.
            if let Some(fd) = found_at_depth {
                if depth > fd {
                    break;
                }
            }

            // Depth limit.
            if depth >= MAX_HOPS as usize {
                continue;
            }

            let (current_id, _) = path.last().unwrap();
            let next_hop = depth + 1;

            // Build visited set for this specific path (cycle guard).
            let path_visited: HashSet<&str> = path.iter().map(|(id, _)| id.as_str()).collect();

            // Gather neighbours at this hop.
            let edge_indices = match adj.get(current_id.as_str()) {
                Some(v) => v.clone(),
                None => continue,
            };

            // Collect candidates: (neighbour_id, edge_idx)
            let mut candidates: Vec<(String, usize)> = Vec::new();
            for ei in &edge_indices {
                let edge = &all_edges[*ei];
                let neighbour_id = if edge.source_id.as_str() == current_id.as_str() {
                    edge.target_id.as_str().to_owned()
                } else {
                    edge.source_id.as_str().to_owned()
                };
                if path_visited.contains(neighbour_id.as_str()) {
                    continue;
                }
                candidates.push((neighbour_id, *ei));
            }

            // Deterministic sort: by neighbour entity_type/display_name then
            // edge identity_hash for tie-breaking.
            candidates.sort_by(|(a_id, a_ei), (b_id, b_ei)| {
                let a_node = node_map.get(a_id.as_str());
                let b_node = node_map.get(b_id.as_str());
                let a_key = a_node.map(|n| node_sort_key(n)).unwrap_or((None, None));
                let b_key = b_node.map(|n| node_sort_key(n)).unwrap_or((None, None));
                a_key.cmp(&b_key).then_with(|| {
                    all_edges[*a_ei]
                        .identity_hash
                        .cmp(&all_edges[*b_ei].identity_hash)
                })
            });

            // Apply per-hop cap.
            candidates.truncate(hop_node_cap(next_hop as u8) as usize);

            for (neighbour_id, ei) in candidates {
                let mut new_path = path.clone();
                new_path.push((neighbour_id.clone(), Some(ei)));

                if neighbour_id == to_key {
                    found_at_depth = Some(next_hop);
                    found_paths.push(new_path);
                } else if found_at_depth.map_or(true, |fd| next_hop < fd) {
                    // Only enqueue for further expansion if we haven't yet found
                    // a path, or if this path is strictly shorter than found paths.
                    queue.push_back(new_path);
                }
            }
        }

        if found_paths.is_empty() {
            return None;
        }

        // Pick the lexicographically smallest path by path length first,
        // then by concatenated identity_hash for equal-length paths.
        found_paths.sort_by(|a, b| {
            let len_cmp = a.len().cmp(&b.len());
            if len_cmp != std::cmp::Ordering::Equal {
                return len_cmp;
            }
            let hash_a: String = a
                .iter()
                .filter_map(|(_, ei)| ei.map(|i| all_edges[i].identity_hash.as_str()))
                .collect::<Vec<_>>()
                .join(",");
            let hash_b: String = b
                .iter()
                .filter_map(|(_, ei)| ei.map(|i| all_edges[i].identity_hash.as_str()))
                .collect::<Vec<_>>()
                .join(",");
            hash_a.cmp(&hash_b)
        });

        let best = &found_paths[0];

        // Validate: no unauthorized node anywhere on the path.
        for (node_id, _) in best {
            let node = node_map.get(node_id.as_str())?;
            if !node.is_authorized {
                return None;
            }
        }

        // Build PathStep vec.
        let mut steps: Vec<PathStep> = Vec::with_capacity(best.len());
        for (i, (node_id, _ei_opt)) in best.iter().enumerate() {
            let node = (*node_map.get(node_id.as_str())?).clone();
            let is_last = i + 1 == best.len();
            // edge_to_next: the edge stored at step i+1 (the edge that leads INTO step i+1).
            let edge_to_next = if is_last {
                None
            } else {
                best[i + 1].1.map(|ei| all_edges[ei].clone())
            };
            steps.push(PathStep { node, edge_to_next });
        }

        Some(steps)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::model::{EntityId, GraphRevision, TruthState};

    // ── Helpers ──────────────────────────────────────────────────────────

    fn make_id(suffix: u8) -> EntityId {
        // Use stable test UUIDs with the suffix encoded in the last byte.
        let s = format!("00000000-0000-7000-8000-0000000000{:02x}", suffix);
        EntityId::new(&s).unwrap()
    }

    fn make_node(id: EntityId, authorized: bool) -> TraversalNode {
        TraversalNode {
            id,
            is_authorized: authorized,
            entity_type: None,
            display_name: None,
            truth_state: TruthState::Current,
            valid_from: None,
            valid_until: None,
            namespace: "user".into(),
            scope: "chat".into(),
            sensitivity: 0,
            policy_version: "test".into(),
            created_at: None,
            source_kind: None,
            actor_id: None,
            staleness_class: None,
            revision: GraphRevision::base(),
        }
    }

    fn make_node_typed(
        id: EntityId,
        authorized: bool,
        entity_type: &str,
        display_name: &str,
    ) -> TraversalNode {
        let mut n = make_node(id, authorized);
        n.entity_type = Some(entity_type.into());
        n.display_name = Some(display_name.into());
        n
    }

    fn make_edge(src: &EntityId, tgt: &EntityId, hash: &str) -> TraversalEdge {
        TraversalEdge {
            identity_hash: hash.into(),
            link_type: "relates_to".into(),
            link_type_version: 1,
            authority_class: EdgeAuthorityClass::Stored,
            direction: DirectionClass::Directed,
            source_id: src.clone(),
            target_id: tgt.clone(),
            source_authorized: true,
            target_authorized: true,
            truth_state: TruthState::Current,
            valid_from: None,
            valid_until: None,
            namespace: "user".into(),
            scope: "chat".into(),
            sensitivity: 0,
            policy_version: "test".into(),
            evidence_count: 0,
            created_at: None,
            source_kind: None,
            actor_id: None,
            staleness_class: None,
            revision: GraphRevision::base(),
        }
    }

    fn make_sym_edge(a: &EntityId, b: &EntityId, hash: &str) -> TraversalEdge {
        let mut e = make_edge(a, b, hash);
        e.direction = DirectionClass::Symmetric;
        e
    }

    // ── BFS Tests ────────────────────────────────────────────────────────

    #[test]
    fn empty_seeds_produces_empty_result() {
        let nodes = vec![make_node(make_id(1), true)];
        let edges: Vec<TraversalEdge> = vec![];
        let result = BreadthFirstTraversal::execute(&[], &nodes, &edges, 3);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
        assert!(!result.truncated);
        assert_eq!(result.hidden_node_count, 0);
    }

    #[test]
    fn single_node_depth_zero() {
        let a = make_id(1);
        let nodes = vec![make_node(a.clone(), true)];
        let edges: Vec<TraversalEdge> = vec![];
        let result = BreadthFirstTraversal::execute(&[a.clone()], &nodes, &edges, 0);
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].id.as_str(), a.as_str());
        assert!(result.edges.is_empty());
        assert_eq!(result.max_hop_reached, 0);
    }

    #[test]
    fn linear_chain_three_hops_reaches_fourth_node() {
        // A -> B -> C -> D (directed)
        let a = make_id(1);
        let b = make_id(2);
        let c = make_id(3);
        let d = make_id(4);
        let nodes = vec![
            make_node(a.clone(), true),
            make_node(b.clone(), true),
            make_node(c.clone(), true),
            make_node(d.clone(), true),
        ];
        let edges = vec![
            make_edge(&a, &b, "e-ab"),
            make_edge(&b, &c, "e-bc"),
            make_edge(&c, &d, "e-cd"),
        ];
        let result = BreadthFirstTraversal::execute(&[a.clone()], &nodes, &edges, 3);
        let ids: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&a.as_str()), "a must be in result");
        assert!(ids.contains(&b.as_str()), "b must be in result (hop 1)");
        assert!(ids.contains(&c.as_str()), "c must be in result (hop 2)");
        assert!(ids.contains(&d.as_str()), "d must be in result (hop 3)");
        assert_eq!(result.edges.len(), 3);
    }

    #[test]
    fn linear_chain_two_hops_stops_at_c() {
        let a = make_id(1);
        let b = make_id(2);
        let c = make_id(3);
        let d = make_id(4);
        let nodes = vec![
            make_node(a.clone(), true),
            make_node(b.clone(), true),
            make_node(c.clone(), true),
            make_node(d.clone(), true),
        ];
        let edges = vec![
            make_edge(&a, &b, "e-ab"),
            make_edge(&b, &c, "e-bc"),
            make_edge(&c, &d, "e-cd"),
        ];
        let result = BreadthFirstTraversal::execute(&[a.clone()], &nodes, &edges, 2);
        let ids: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&a.as_str()));
        assert!(ids.contains(&b.as_str()));
        assert!(ids.contains(&c.as_str()));
        assert!(
            !ids.contains(&d.as_str()),
            "d must NOT be in result (beyond 2 hops)"
        );
        assert_eq!(result.edges.len(), 2);
    }

    #[test]
    fn cycle_prevention() {
        // A <-> B (symmetric), A <-> C, B <-> C — no infinite loop
        let a = make_id(1);
        let b = make_id(2);
        let c = make_id(3);
        let nodes = vec![
            make_node(a.clone(), true),
            make_node(b.clone(), true),
            make_node(c.clone(), true),
        ];
        let edges = vec![
            make_sym_edge(&a, &b, "e-ab"),
            make_sym_edge(&b, &c, "e-bc"),
            make_sym_edge(&a, &c, "e-ac"),
        ];
        let result = BreadthFirstTraversal::execute(&[a.clone()], &nodes, &edges, 3);
        // Each node should appear exactly once.
        let ids: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            ids.len(),
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            "each node must appear exactly once"
        );
        assert!(ids.contains(&a.as_str()));
        assert!(ids.contains(&b.as_str()));
        assert!(ids.contains(&c.as_str()));
    }

    #[test]
    fn hidden_intermediary_bfs_counted_not_expanded() {
        // A -> B (unauthorized) -> C
        let a = make_id(1);
        let b = make_id(2);
        let c = make_id(3);
        let nodes = vec![
            make_node(a.clone(), true),
            make_node(b.clone(), false), // hidden
            make_node(c.clone(), true),
        ];
        let edges = vec![make_edge(&a, &b, "e-ab"), make_edge(&b, &c, "e-bc")];
        let result = BreadthFirstTraversal::execute(&[a.clone()], &nodes, &edges, 3);
        let ids: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&a.as_str()), "a must be in result");
        assert!(
            !ids.contains(&b.as_str()),
            "b is unauthorized — must NOT be in result"
        );
        assert!(
            !ids.contains(&c.as_str()),
            "c unreachable because b was not expanded"
        );
        assert_eq!(result.hidden_node_count, 1, "hidden count must be 1");
    }

    #[test]
    fn per_hop_cap_causes_truncation() {
        // Seed node A, with more than HOP_1_NODE_CAP=40 neighbours.
        let a = make_id(0);
        let mut nodes = vec![make_node(a.clone(), true)];
        let mut edges = vec![];
        // Create 45 neighbour nodes (> HOP_1_NODE_CAP=40).
        for i in 1u8..=45 {
            let n = make_id(i);
            nodes.push(make_node(n.clone(), true));
            edges.push(make_edge(&a, &n, &format!("e-a-{i}")));
        }
        let result = BreadthFirstTraversal::execute(&[a.clone()], &nodes, &edges, 1);
        // Seed = 1, hop-1 cap = 40, so total nodes = 41.
        assert!(
            result.nodes.len() <= (1 + HOP_1_NODE_CAP as usize),
            "nodes must not exceed seed + hop-1 cap"
        );
        assert!(result.truncated, "must be truncated");
    }

    #[test]
    fn deterministic_ordering() {
        // Three nodes with typed names; result should be sorted entity_type then display_name.
        let a = make_id(1);
        let b = make_id(2);
        let c = make_id(3);
        let na = make_node_typed(a.clone(), true, "zebra", "Zara");
        let nb = make_node_typed(b.clone(), true, "apple", "Adam");
        let nc = make_node_typed(c.clone(), true, "apple", "Alice");
        let nodes = vec![na.clone(), nb.clone(), nc.clone()];
        let edges: Vec<TraversalEdge> = vec![];
        let result =
            BreadthFirstTraversal::execute(&[a.clone(), b.clone(), c.clone()], &nodes, &edges, 0);
        assert_eq!(result.nodes.len(), 3);
        // Sorted: apple/Adam, apple/Alice, zebra/Zara
        assert_eq!(result.nodes[0].entity_type.as_deref(), Some("apple"));
        assert_eq!(result.nodes[0].display_name.as_deref(), Some("Adam"));
        assert_eq!(result.nodes[1].display_name.as_deref(), Some("Alice"));
        assert_eq!(result.nodes[2].entity_type.as_deref(), Some("zebra"));
    }

    // ── PathTraversal Tests ───────────────────────────────────────────────

    #[test]
    fn path_empty_when_no_route() {
        let a = make_id(1);
        let b = make_id(2);
        let nodes = vec![make_node(a.clone(), true), make_node(b.clone(), true)];
        let edges: Vec<TraversalEdge> = vec![];
        let result = PathTraversal::find_shortest_authorized_path(&a, &b, &nodes, &edges);
        assert!(result.is_none(), "no path should be found");
    }

    #[test]
    fn path_trivial_same_node() {
        let a = make_id(1);
        let nodes = vec![make_node(a.clone(), true)];
        let edges: Vec<TraversalEdge> = vec![];
        let result = PathTraversal::find_shortest_authorized_path(&a, &a, &nodes, &edges);
        let steps = result.expect("trivial path to self must exist");
        assert_eq!(steps.len(), 1);
        assert!(steps[0].edge_to_next.is_none());
    }

    #[test]
    fn path_direct_edge() {
        let a = make_id(1);
        let b = make_id(2);
        let nodes = vec![make_node(a.clone(), true), make_node(b.clone(), true)];
        let edges = vec![make_edge(&a, &b, "e-ab")];
        let result = PathTraversal::find_shortest_authorized_path(&a, &b, &nodes, &edges);
        let steps = result.expect("direct path must exist");
        assert_eq!(steps.len(), 2, "path has 2 steps: a and b");
        assert_eq!(steps[0].node.id.as_str(), a.as_str());
        assert_eq!(steps[1].node.id.as_str(), b.as_str());
        assert!(steps[0].edge_to_next.is_some());
        assert!(steps[1].edge_to_next.is_none());
    }

    #[test]
    fn path_hidden_intermediary_omitted() {
        // A -> B (hidden) -> C; path A→C requires going through B which is hidden.
        let a = make_id(1);
        let b = make_id(2);
        let c = make_id(3);
        let nodes = vec![
            make_node(a.clone(), true),
            make_node(b.clone(), false), // hidden
            make_node(c.clone(), true),
        ];
        let edges = vec![make_edge(&a, &b, "e-ab"), make_edge(&b, &c, "e-bc")];
        let result = PathTraversal::find_shortest_authorized_path(&a, &c, &nodes, &edges);
        assert!(
            result.is_none(),
            "path through hidden intermediary must be omitted"
        );
    }

    #[test]
    fn path_chooses_shortest() {
        // A -direct-> C  (1 hop)
        // A -> B -> C    (2 hops)
        let a = make_id(1);
        let b = make_id(2);
        let c = make_id(3);
        let nodes = vec![
            make_node(a.clone(), true),
            make_node(b.clone(), true),
            make_node(c.clone(), true),
        ];
        let edges = vec![
            make_sym_edge(&a, &c, "e-ac"),
            make_sym_edge(&a, &b, "e-ab"),
            make_sym_edge(&b, &c, "e-bc"),
        ];
        let result = PathTraversal::find_shortest_authorized_path(&a, &c, &nodes, &edges);
        let steps = result.expect("path must exist");
        assert_eq!(steps.len(), 2, "shortest path has 2 steps (1 hop)");
    }

    #[test]
    fn path_beyond_max_hops_is_none() {
        // Chain A->B->C->D->E: 4 hops, exceeds MAX_HOPS=3.
        let a = make_id(1);
        let b = make_id(2);
        let c = make_id(3);
        let d = make_id(4);
        let e = make_id(5);
        let nodes = vec![
            make_node(a.clone(), true),
            make_node(b.clone(), true),
            make_node(c.clone(), true),
            make_node(d.clone(), true),
            make_node(e.clone(), true),
        ];
        let edges = vec![
            make_edge(&a, &b, "e-ab"),
            make_edge(&b, &c, "e-bc"),
            make_edge(&c, &d, &"e-cd"),
            make_edge(&d, &e, "e-de"),
        ];
        let result = PathTraversal::find_shortest_authorized_path(&a, &e, &nodes, &edges);
        assert!(
            result.is_none(),
            "4-hop path exceeds MAX_HOPS=3 and must not be returned"
        );
    }
}
