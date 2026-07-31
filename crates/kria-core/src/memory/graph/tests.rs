//! Comprehensive property test cases for the graph layer (task 2.3.6).
//!
//! Covers: cyclic graphs, parallel edges, depth 0–4, hidden intermediaries,
//! mixed endpoint authorization, deadline/cancellation structural invariants,
//! and combined property invariants.
//!
//! All tests are deterministic golden fixtures — no proptest or randomness.

use std::collections::HashMap;

use crate::memory::graph::frontier::{EdgeAssembler, FrontierTokenBuilder};
use crate::memory::graph::policy_filter::{GraphPolicyFilter, PolicyContext};
use crate::memory::graph::projection::{
    DirectionClass, EdgeAuthorityClass, EffectivePolicySummary, ProjectedNode, ProjectedNodeKind,
};
use crate::memory::graph::query::{
    GraphQueryProjector, GraphQueryRequest, ProjectionLimits, RawEdgeRow, RawEntityRow,
};
use crate::memory::graph::traversal::{
    BreadthFirstTraversal, PathTraversal, TraversalEdge, TraversalNode,
};
use crate::memory::model::{EntityId, GraphRevision, TruthState};

// ── Test helpers ──────────────────────────────────────────────────────────

fn make_id(suffix: u8) -> EntityId {
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
        policy_version: "v1".into(),
        created_at: None,
        source_kind: None,
        actor_id: None,
        staleness_class: None,
        revision: GraphRevision::base(),
    }
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
        policy_version: "v1".into(),
        evidence_count: 0,
        created_at: None,
        source_kind: None,
        actor_id: None,
        staleness_class: None,
        revision: GraphRevision::base(),
    }
}

fn make_ctx() -> PolicyContext {
    PolicyContext {
        namespace: "user".into(),
        scope: "chat".into(),
        max_sensitivity: 2,
        policy_version: "v1".into(),
    }
}

#[allow(dead_code)]
fn make_policy_node(id: EntityId, authorized: bool) -> TraversalNode {
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
        policy_version: "v1".into(),
        created_at: None,
        source_kind: None,
        actor_id: None,
        staleness_class: None,
        revision: GraphRevision::base(),
    }
}

/// Build a traversal node used in EdgeAssembler/LabelGuard tests.
fn make_traversal_node_named(id: EntityId, display_name: Option<&str>) -> TraversalNode {
    let mut n = make_node(id, true);
    n.entity_type = Some("person".into());
    n.display_name = display_name.map(|s| s.to_owned());
    n
}

/// Build a minimal GraphQueryRequest for use in query projector tests.
fn make_query_request(
    seeds: Vec<EntityId>,
    max_nodes: Option<u32>,
    max_edges: Option<u32>,
    deadline_ms: Option<u32>,
) -> GraphQueryRequest {
    GraphQueryRequest {
        seeds,
        expand_children: None,
        max_nodes,
        max_edges,
        policy_scope: EffectivePolicySummary {
            namespace: "user".into(),
            scope: "chat".into(),
            sensitivity: 0,
            policy_version: "v1".into(),
        },
        at_revision: None,
        cursor: None,
        deadline_ms,
    }
}

fn make_raw_entity(id: EntityId) -> RawEntityRow {
    RawEntityRow {
        id,
        entity_type: Some("person".into()),
        display_name: Some("Alice".into()),
        truth_state: TruthState::Current,
        valid_from: None,
        valid_until: None,
        sensitivity: 0,
        namespace: "user".into(),
        scope: "chat".into(),
        policy_version: "v1".into(),
        created_at: None,
        source_kind: None,
        actor_id: None,
        staleness_class: None,
        revision: GraphRevision::base(),
    }
}

fn make_raw_edge(
    src: &EntityId,
    tgt: &EntityId,
    hash: &str,
    src_visible: bool,
    tgt_visible: bool,
) -> RawEdgeRow {
    RawEdgeRow {
        identity_hash: hash.into(),
        link_type: "relates_to".into(),
        link_type_version: 1,
        authority_class: EdgeAuthorityClass::Stored,
        direction: DirectionClass::Directed,
        source_entity_id: src.clone(),
        target_entity_id: tgt.clone(),
        source_visible: src_visible,
        target_visible: tgt_visible,
        truth_state: TruthState::Current,
        valid_from: None,
        valid_until: None,
        sensitivity: 0,
        namespace: "user".into(),
        scope: "chat".into(),
        policy_version: "v1".into(),
        evidence_count: 0,
        created_at: None,
        source_kind: None,
        actor_id: None,
        staleness_class: None,
        revision: GraphRevision::base(),
    }
}

// ── Group 1: Cyclic graphs ────────────────────────────────────────────────

/// Test 1: BFS on a complete directed cycle A→B→C→A.
/// All 3 nodes appear exactly once; no infinite loop; truncated=false.
#[test]
fn cyclic_directed_cycle_abc_all_nodes_once() {
    let a = make_id(1);
    let b = make_id(2);
    let c = make_id(3);
    let nodes = vec![
        make_node(a.clone(), true),
        make_node(b.clone(), true),
        make_node(c.clone(), true),
    ];
    let edges = vec![
        make_edge(&a, &b, "e-ab"),
        make_edge(&b, &c, "e-bc"),
        make_edge(&c, &a, "e-ca"),
    ];
    let result = BreadthFirstTraversal::execute(&[a.clone()], &nodes, &edges, 3);

    let ids: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(ids.contains(&a.as_str()), "A must be in result");
    assert!(ids.contains(&b.as_str()), "B must be in result");
    assert!(ids.contains(&c.as_str()), "C must be in result");
    assert_eq!(result.nodes.len(), 3, "each node appears exactly once");
    assert!(!result.truncated, "small cycle must not be truncated");
    // Edges: ab, bc; ca edge goes back to A (already visited), so it's included
    // as a seed-to-seed edge in the final pass over all_edges.
    assert!(result.edges.len() >= 2, "at least ab and bc edges present");
}

/// Test 2: BFS on a self-loop A→A.
/// Node A appears once; self-edge does not double-count.
#[test]
fn cyclic_self_loop_node_appears_once() {
    let a = make_id(1);
    let nodes = vec![make_node(a.clone(), true)];
    // Self-loop edge: source and target both A.
    let mut self_loop = make_edge(&a, &a, "e-aa");
    self_loop.target_id = a.clone();
    let edges = vec![self_loop];

    let result = BreadthFirstTraversal::execute(&[a.clone()], &nodes, &edges, 3);

    assert_eq!(result.nodes.len(), 1, "A appears exactly once");
    assert_eq!(result.nodes[0].id.as_str(), a.as_str());
    assert!(!result.truncated);
}

/// Test 3: Deeply cyclic graph — every node connects to every other.
/// BFS terminates; each node appears at most once.
#[test]
fn cyclic_fully_connected_graph_terminates_no_duplicates() {
    // 5 nodes, all-to-all directed edges.
    let ids: Vec<EntityId> = (1u8..=5).map(make_id).collect();
    let nodes: Vec<TraversalNode> = ids.iter().map(|id| make_node(id.clone(), true)).collect();
    let mut edges = Vec::new();
    for (i, src) in ids.iter().enumerate() {
        for (j, tgt) in ids.iter().enumerate() {
            if i != j {
                edges.push(make_edge(src, tgt, &format!("e-{i}-{j}")));
            }
        }
    }

    let result = BreadthFirstTraversal::execute(&[ids[0].clone()], &nodes, &edges, 3);

    // All 5 nodes appear — no duplicates.
    let mut seen_ids: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
    let before_dedup = seen_ids.len();
    seen_ids.dedup();
    assert_eq!(before_dedup, seen_ids.len(), "no duplicate nodes");
    assert_eq!(result.nodes.len(), 5, "all 5 nodes reached");
}

/// Test 4: PathTraversal on cycle A→B→C→A, find path A→C.
/// Finds A→B→C (shortest authorized path, 2 hops); no infinite recursion.
#[test]
fn cyclic_path_traversal_finds_shortest_path_in_cycle() {
    let a = make_id(1);
    let b = make_id(2);
    let c = make_id(3);
    let nodes = vec![
        make_node(a.clone(), true),
        make_node(b.clone(), true),
        make_node(c.clone(), true),
    ];
    let edges = vec![
        make_edge(&a, &b, "e-ab"),
        make_edge(&b, &c, "e-bc"),
        make_edge(&c, &a, "e-ca"),
    ];

    let path = PathTraversal::find_shortest_authorized_path(&a, &c, &nodes, &edges);
    assert!(path.is_some(), "path A→C must exist");
    let steps = path.unwrap();
    // Shortest path is A→B→C (2 hops, 3 steps).
    assert_eq!(steps.len(), 3, "A→B→C = 3 steps");
    assert_eq!(steps[0].node.id.as_str(), a.as_str());
    assert_eq!(steps[2].node.id.as_str(), c.as_str());
}

// ── Group 2: Parallel edges (parallel-evidence) ───────────────────────────

/// Test 5: Two directed edges between same pair A→B (edge-1, edge-2).
/// Both edges appear; nodes not duplicated.
#[test]
fn parallel_two_edges_same_pair_both_appear() {
    let a = make_id(1);
    let b = make_id(2);
    let nodes = vec![make_node(a.clone(), true), make_node(b.clone(), true)];
    let mut edge2 = make_edge(&a, &b, "e-ab-2");
    edge2.link_type = "also_relates_to".into();
    let edges = vec![make_edge(&a, &b, "e-ab-1"), edge2];

    let result = BreadthFirstTraversal::execute(&[a.clone()], &nodes, &edges, 1);

    let edge_hashes: Vec<&str> = result
        .edges
        .iter()
        .map(|e| e.identity_hash.as_str())
        .collect();
    assert!(
        edge_hashes.contains(&"e-ab-1"),
        "first edge must be included"
    );
    assert!(
        edge_hashes.contains(&"e-ab-2"),
        "second parallel edge must be included"
    );
    // Nodes appear only once each.
    assert_eq!(result.nodes.len(), 2, "A and B each appear once");
}

/// Test 6: Symmetric + directed edge on same pair A↔B and A→B.
/// Both appear, directions preserved.
#[test]
fn parallel_symmetric_and_directed_both_appear() {
    let a = make_id(1);
    let b = make_id(2);
    let nodes = vec![make_node(a.clone(), true), make_node(b.clone(), true)];
    let directed = make_edge(&a, &b, "e-directed");
    let mut symmetric = make_edge(&a, &b, "e-symmetric");
    symmetric.direction = DirectionClass::Symmetric;

    let edges = vec![directed, symmetric];
    let result = BreadthFirstTraversal::execute(&[a.clone()], &nodes, &edges, 1);

    let edge_hashes: Vec<&str> = result
        .edges
        .iter()
        .map(|e| e.identity_hash.as_str())
        .collect();
    assert!(
        edge_hashes.contains(&"e-directed"),
        "directed edge must appear"
    );
    assert!(
        edge_hashes.contains(&"e-symmetric"),
        "symmetric edge must appear"
    );
    // Verify direction is preserved on each edge.
    let dir_edge = result
        .edges
        .iter()
        .find(|e| e.identity_hash == "e-directed")
        .unwrap();
    let sym_edge = result
        .edges
        .iter()
        .find(|e| e.identity_hash == "e-symmetric")
        .unwrap();
    assert_eq!(dir_edge.direction, DirectionClass::Directed);
    assert_eq!(sym_edge.direction, DirectionClass::Symmetric);
}

// ── Group 3: Depth 0/1/2/3/4 ─────────────────────────────────────────────

/// Build a linear chain A→B→C→D→E for depth tests.
fn linear_chain_5() -> (Vec<EntityId>, Vec<TraversalNode>, Vec<TraversalEdge>) {
    let ids: Vec<EntityId> = (1u8..=5).map(make_id).collect();
    let nodes: Vec<TraversalNode> = ids.iter().map(|id| make_node(id.clone(), true)).collect();
    let edges = vec![
        make_edge(&ids[0], &ids[1], "e-01"),
        make_edge(&ids[1], &ids[2], "e-12"),
        make_edge(&ids[2], &ids[3], "e-23"),
        make_edge(&ids[3], &ids[4], "e-34"),
    ];
    (ids, nodes, edges)
}

/// Test 7: max_hops=0 → only seed nodes, no edges.
#[test]
fn depth_0_only_seed_no_edges() {
    let (ids, nodes, edges) = linear_chain_5();
    let result = BreadthFirstTraversal::execute(&[ids[0].clone()], &nodes, &edges, 0);

    assert_eq!(result.nodes.len(), 1, "only seed at depth 0");
    assert_eq!(result.nodes[0].id.as_str(), ids[0].as_str());
    assert!(result.edges.is_empty(), "no edges at depth 0");
}

/// Test 8: max_hops=1 → seed + immediate neighbors + edges.
#[test]
fn depth_1_seed_and_immediate_neighbors() {
    let (ids, nodes, edges) = linear_chain_5();
    let result = BreadthFirstTraversal::execute(&[ids[0].clone()], &nodes, &edges, 1);

    let result_ids: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(result_ids.contains(&ids[0].as_str()), "seed A present");
    assert!(
        result_ids.contains(&ids[1].as_str()),
        "hop-1 neighbor B present"
    );
    assert!(
        !result_ids.contains(&ids[2].as_str()),
        "C not reached at depth 1"
    );
    assert_eq!(result.edges.len(), 1, "only edge A→B");
}

/// Test 9: max_hops=2 → two-hop expansion.
#[test]
fn depth_2_two_hop_expansion() {
    let (ids, nodes, edges) = linear_chain_5();
    let result = BreadthFirstTraversal::execute(&[ids[0].clone()], &nodes, &edges, 2);

    let result_ids: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(result_ids.contains(&ids[0].as_str()));
    assert!(result_ids.contains(&ids[1].as_str()));
    assert!(
        result_ids.contains(&ids[2].as_str()),
        "C reached at depth 2"
    );
    assert!(
        !result_ids.contains(&ids[3].as_str()),
        "D not reached at depth 2"
    );
    assert_eq!(result.edges.len(), 2, "edges A→B and B→C");
}

/// Test 10: max_hops=3 → three-hop expansion reaches MAX_HOPS.
#[test]
fn depth_3_three_hop_expansion() {
    let (ids, nodes, edges) = linear_chain_5();
    let result = BreadthFirstTraversal::execute(&[ids[0].clone()], &nodes, &edges, 3);

    let result_ids: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(
        result_ids.contains(&ids[3].as_str()),
        "D reached at depth 3"
    );
    assert!(
        !result_ids.contains(&ids[4].as_str()),
        "E not reached at depth 3"
    );
}

/// Test 11: max_hops=4 is clamped to MAX_HOPS=3; node at hop 4 NOT included.
#[test]
fn depth_4_clamped_to_max_hops_3() {
    let (ids, nodes, edges) = linear_chain_5();
    // max_hops=4 must be clamped to MAX_HOPS (3).
    let result = BreadthFirstTraversal::execute(&[ids[0].clone()], &nodes, &edges, 4);

    let result_ids: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
    // ids[4] is E, which is at hop 4 — must NOT appear.
    assert!(
        !result_ids.contains(&ids[4].as_str()),
        "E at hop 4 must not appear when clamped"
    );
    // ids[3] at hop 3 SHOULD appear.
    assert!(
        result_ids.contains(&ids[3].as_str()),
        "D at hop 3 must appear"
    );
}

// ── Group 4: Hidden intermediary ──────────────────────────────────────────

/// Test 12: BFS with hidden node B in middle of A→B→C.
/// B counted in hidden_node_count but NOT expanded; C unreachable.
#[test]
fn hidden_intermediary_bfs_stops_at_hidden_node() {
    let a = make_id(1);
    let b = make_id(2); // hidden
    let c = make_id(3);
    let nodes = vec![
        make_node(a.clone(), true),
        make_node(b.clone(), false), // hidden
        make_node(c.clone(), true),
    ];
    let edges = vec![make_edge(&a, &b, "e-ab"), make_edge(&b, &c, "e-bc")];

    let result = BreadthFirstTraversal::execute(&[a.clone()], &nodes, &edges, 3);

    let result_ids: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(result_ids.contains(&a.as_str()), "A must be in result");
    assert!(
        !result_ids.contains(&b.as_str()),
        "hidden B must NOT appear"
    );
    assert!(
        !result_ids.contains(&c.as_str()),
        "C is unreachable through hidden B"
    );
    assert_eq!(result.hidden_node_count, 1, "B contributes to hidden count");
}

/// Test 13: PathTraversal A→B(hidden)→C returns None.
/// Entire path is omitted when intermediary is hidden.
#[test]
fn hidden_intermediary_path_returns_none() {
    let a = make_id(1);
    let b = make_id(2); // hidden intermediary
    let c = make_id(3);
    let nodes = vec![
        make_node(a.clone(), true),
        make_node(b.clone(), false), // hidden
        make_node(c.clone(), true),
    ];
    let edges = vec![make_edge(&a, &b, "e-ab"), make_edge(&b, &c, "e-bc")];

    let path = PathTraversal::find_shortest_authorized_path(&a, &c, &nodes, &edges);
    assert!(
        path.is_none(),
        "path through hidden intermediary must return None"
    );
}

/// Test 14: Direct path A→C exists AND A→B(hidden)→C also exists.
/// Direct path A→C is returned; hidden path not used.
#[test]
fn hidden_intermediary_direct_path_preferred_over_hidden() {
    let a = make_id(1);
    let b = make_id(2); // hidden
    let c = make_id(3);
    let nodes = vec![
        make_node(a.clone(), true),
        make_node(b.clone(), false), // hidden
        make_node(c.clone(), true),
    ];
    let edges = vec![
        make_edge(&a, &b, "e-ab"),
        make_edge(&b, &c, "e-bc"),
        make_edge(&a, &c, "e-ac"), // direct path
    ];

    let path = PathTraversal::find_shortest_authorized_path(&a, &c, &nodes, &edges);
    assert!(path.is_some(), "direct path A→C must be found");
    let steps = path.unwrap();
    // Direct path: A→C = 2 steps
    assert_eq!(steps.len(), 2, "direct path has 2 steps");
    assert_eq!(steps[0].node.id.as_str(), a.as_str());
    assert_eq!(steps[1].node.id.as_str(), c.as_str());
}

/// Test 15: PolicyFilter hides a seed node.
/// Filtered seed excluded from BFS; hidden_count = 1.
#[test]
fn hidden_intermediary_policy_filter_hides_seed() {
    let ctx = make_ctx();
    let a = make_id(1); // authorized
    let b = make_id(2); // not authorized (wrong namespace)
    let nodes = vec![
        TraversalNode {
            id: a.clone(),
            is_authorized: true,
            namespace: "user".into(),
            scope: "chat".into(),
            sensitivity: 0,
            policy_version: "v1".into(),
            entity_type: None,
            display_name: None,
            truth_state: TruthState::Current,
            valid_from: None,
            valid_until: None,
            created_at: None,
            source_kind: None,
            actor_id: None,
            staleness_class: None,
            revision: GraphRevision::base(),
        },
        TraversalNode {
            id: b.clone(),
            is_authorized: true, // store says authorized but wrong namespace
            namespace: "workspace".into(), // wrong namespace for ctx
            scope: "chat".into(),
            sensitivity: 0,
            policy_version: "v1".into(),
            entity_type: None,
            display_name: None,
            truth_state: TruthState::Current,
            valid_from: None,
            valid_until: None,
            created_at: None,
            source_kind: None,
            actor_id: None,
            staleness_class: None,
            revision: GraphRevision::base(),
        },
    ];
    let seeds = vec![a.clone(), b.clone()];
    let result = GraphPolicyFilter::filter_seeds(&ctx, &seeds, &nodes);

    assert_eq!(result.authorized.len(), 1, "only A passes filter");
    assert_eq!(result.authorized[0].as_str(), a.as_str());
    assert_eq!(result.hidden_count, 1, "B counted as hidden");
    assert!(result.has_hidden);
}

// ── Group 5: Mixed endpoints ──────────────────────────────────────────────

/// Helper: build a ValidatedRequest for projector tests.
fn make_validated_request() -> crate::memory::graph::query::ValidatedRequest {
    let req = make_query_request(vec![], None, None, None);
    GraphQueryProjector::validate_request(&req).unwrap()
}

/// Test 16: Edge with source_visible=false → edge omitted; FrontierAggregate added.
#[test]
fn mixed_endpoint_source_not_visible_produces_frontier_aggregate() {
    let a = make_id(1);
    let b = make_id(2);
    let validated = make_validated_request();
    let rev = GraphRevision::base();

    let entities = vec![make_raw_entity(a.clone()), make_raw_entity(b.clone())];
    // source_visible=false for the edge
    let edges = vec![make_raw_edge(&a, &b, "e-ab", false, true)];

    let response = GraphQueryProjector::build_entity_primary(&validated, entities, edges, rev);

    // Edge must be omitted.
    assert!(
        response.edges.is_empty(),
        "hidden-source edge must be omitted"
    );
    // FrontierAggregate node must appear.
    let aggregates: Vec<&ProjectedNode> = response
        .nodes
        .iter()
        .filter(|n| n.node_kind == ProjectedNodeKind::Aggregate)
        .collect();
    assert!(
        !aggregates.is_empty(),
        "FrontierAggregate must appear for hidden source"
    );
}

/// Test 17: Edge with target_visible=false → edge omitted; FrontierAggregate added.
#[test]
fn mixed_endpoint_target_not_visible_produces_frontier_aggregate() {
    let a = make_id(1);
    let b = make_id(2);
    let validated = make_validated_request();
    let rev = GraphRevision::base();

    let entities = vec![make_raw_entity(a.clone()), make_raw_entity(b.clone())];
    // target_visible=false for the edge
    let edges = vec![make_raw_edge(&a, &b, "e-ab", true, false)];

    let response = GraphQueryProjector::build_entity_primary(&validated, entities, edges, rev);

    assert!(
        response.edges.is_empty(),
        "hidden-target edge must be omitted"
    );
    let aggregates: Vec<&ProjectedNode> = response
        .nodes
        .iter()
        .filter(|n| n.node_kind == ProjectedNodeKind::Aggregate)
        .collect();
    assert!(
        !aggregates.is_empty(),
        "FrontierAggregate must appear for hidden target"
    );
}

/// Test 18: Edge with both endpoints visible → edge included normally.
#[test]
fn mixed_endpoint_both_visible_edge_included() {
    let a = make_id(1);
    let b = make_id(2);
    let validated = make_validated_request();
    let rev = GraphRevision::base();

    let entities = vec![make_raw_entity(a.clone()), make_raw_entity(b.clone())];
    let edges = vec![make_raw_edge(&a, &b, "e-ab", true, true)];

    let response = GraphQueryProjector::build_entity_primary(&validated, entities, edges, rev);

    assert_eq!(response.edges.len(), 1, "visible edge must be included");
    assert_eq!(response.edges[0].id, "e-ab");
    // No FrontierAggregate for this case.
    let aggregates: Vec<&ProjectedNode> = response
        .nodes
        .iter()
        .filter(|n| n.node_kind == ProjectedNodeKind::Aggregate)
        .collect();
    assert!(
        aggregates.is_empty(),
        "no frontier aggregate when both endpoints visible"
    );
}

// ── Group 6: Deadline simulation (structural) ─────────────────────────────

/// Test 19: validate_request sets DEFAULT_DEADLINE_MS=250 when None;
/// rejects deadline > MAX_DEADLINE_MS=2000.
#[test]
fn deadline_default_applied_and_max_enforced() {
    // None → DEFAULT_DEADLINE_MS=250
    let req_no_deadline = make_query_request(vec![], None, None, None);
    let validated = GraphQueryProjector::validate_request(&req_no_deadline).unwrap();
    assert_eq!(
        validated.deadline_ms,
        ProjectionLimits::DEFAULT_DEADLINE_MS,
        "missing deadline gets DEFAULT_DEADLINE_MS"
    );
    assert_eq!(ProjectionLimits::DEFAULT_DEADLINE_MS, 250);

    // Exactly at max → accepted.
    let req_at_max =
        make_query_request(vec![], None, None, Some(ProjectionLimits::MAX_DEADLINE_MS));
    let result_at_max = GraphQueryProjector::validate_request(&req_at_max);
    assert!(
        result_at_max.is_ok(),
        "deadline == MAX_DEADLINE_MS must be accepted"
    );

    // Strictly over max → rejected.
    let req_over_max = make_query_request(
        vec![],
        None,
        None,
        Some(ProjectionLimits::MAX_DEADLINE_MS + 1),
    );
    let result_over = GraphQueryProjector::validate_request(&req_over_max);
    assert!(
        result_over.is_err(),
        "deadline > MAX_DEADLINE_MS must be rejected"
    );
    if let Err(crate::memory::graph::query::ProjectionError::InvalidDeadline { max, got }) =
        result_over
    {
        assert_eq!(max, ProjectionLimits::MAX_DEADLINE_MS);
        assert_eq!(got, ProjectionLimits::MAX_DEADLINE_MS + 1);
    } else {
        panic!("expected InvalidDeadline error");
    }

    // Verify constant values match spec.
    assert_eq!(ProjectionLimits::MAX_DEADLINE_MS, 2000);
}

// ── Group 7: Cancellation token (structural) ──────────────────────────────

/// Test 20: FrontierToken build with empty frontier → token is non-empty,
/// decodes back to empty ids.
#[test]
fn frontier_token_empty_frontier_roundtrip() {
    let rev = GraphRevision::base();
    let token = FrontierTokenBuilder::build(&[], 0, rev, "v1");

    // Token string is non-empty (it's base64-encoded JSON).
    assert!(!token.token.is_empty(), "token must be non-empty");

    // Decodes back to empty frontier.
    let decoded = FrontierTokenBuilder::decode(&token, rev, "v1");
    assert!(decoded.is_some(), "decode must succeed");
    let decoded = decoded.unwrap();
    assert!(
        decoded.frontier_ids.is_empty(),
        "frontier ids must be empty"
    );
    assert_eq!(decoded.hop_depth, 0);
}

/// Test 21: FrontierToken decode fails on tampered token.
/// Changing 1 byte in the base64 content makes decode return None or a
/// different (invalid) result — it must NOT return the original valid data.
#[test]
fn frontier_token_tampered_decode_fails_or_differs() {
    let ids = vec![make_id(1), make_id(2)];
    let rev = GraphRevision::new(5);
    let pv = "policy-v1";

    let token = FrontierTokenBuilder::build(&ids, 1, rev, pv);
    let original_token_str = token.token.clone();

    // Tamper: flip the last character of the base64 string.
    let mut tampered_bytes = original_token_str.into_bytes();
    let last = tampered_bytes.last_mut().unwrap();
    *last = if *last == b'A' { b'B' } else { b'A' };
    let tampered_str = String::from_utf8(tampered_bytes).unwrap();

    let tampered_token = crate::memory::graph::frontier::FrontierToken {
        token: tampered_str,
    };

    let result = FrontierTokenBuilder::decode(&tampered_token, rev, pv);
    // Must either be None (invalid base64/json) or decode to different content.
    match result {
        None => { /* expected: tampered token rejected */ }
        Some(decoded) => {
            // If it decoded, it must not match the original IDs.
            let orig_ids: Vec<&str> = ids.iter().map(|id| id.as_str()).collect();
            let decoded_ids: Vec<&str> =
                decoded.frontier_ids.iter().map(|id| id.as_str()).collect();
            assert_ne!(
                orig_ids, decoded_ids,
                "tampered token must not decode to original ids"
            );
        }
    }
}

// ── Group 8: Combined property invariants ────────────────────────────────

/// Test 22: All nodes in BFS result appear exactly once.
/// 10-node ring (0→1→2→…→9→0); verify unique IDs.
#[test]
fn all_nodes_in_bfs_appear_exactly_once_ring_10() {
    let ids: Vec<EntityId> = (0u8..10).map(make_id).collect();
    let nodes: Vec<TraversalNode> = ids.iter().map(|id| make_node(id.clone(), true)).collect();
    // Ring: 0→1, 1→2, …, 8→9, 9→0
    let mut edges = Vec::new();
    for i in 0..10usize {
        let next = (i + 1) % 10;
        edges.push(make_edge(&ids[i], &ids[next], &format!("e-{i}-{next}")));
    }

    let result = BreadthFirstTraversal::execute(&[ids[0].clone()], &nodes, &edges, 3);

    // Collect IDs and check uniqueness.
    let mut seen: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
    let total = seen.len();
    seen.sort();
    seen.dedup();
    assert_eq!(total, seen.len(), "no duplicate node IDs in BFS result");

    // With max_hops=3 on a 10-node ring seeded at node 0, we can reach at most
    // hops 0,1,2,3 = nodes 0,1,2,3 = 4 nodes (plus node 9 via back-edge).
    // All that are reached must be unique.
    assert!(total > 0, "at least the seed node must appear");
}

/// Test 23: PolicyFilter + BFS integration.
/// Filter nodes first, then run BFS on filtered set; hidden nodes not in output.
#[test]
fn policy_filter_bfs_integration_hidden_nodes_absent() {
    let ctx = make_ctx();
    let a = make_id(1);
    let b = make_id(2); // will be hidden by policy (wrong namespace)
    let c = make_id(3);

    let all_nodes = vec![
        TraversalNode {
            id: a.clone(),
            is_authorized: true,
            namespace: "user".into(),
            scope: "chat".into(),
            sensitivity: 0,
            policy_version: "v1".into(),
            entity_type: None,
            display_name: None,
            truth_state: TruthState::Current,
            valid_from: None,
            valid_until: None,
            created_at: None,
            source_kind: None,
            actor_id: None,
            staleness_class: None,
            revision: GraphRevision::base(),
        },
        TraversalNode {
            id: b.clone(),
            is_authorized: true,
            namespace: "workspace".into(), // wrong namespace → policy hides it
            scope: "chat".into(),
            sensitivity: 0,
            policy_version: "v1".into(),
            entity_type: None,
            display_name: None,
            truth_state: TruthState::Current,
            valid_from: None,
            valid_until: None,
            created_at: None,
            source_kind: None,
            actor_id: None,
            staleness_class: None,
            revision: GraphRevision::base(),
        },
        TraversalNode {
            id: c.clone(),
            is_authorized: true,
            namespace: "user".into(),
            scope: "chat".into(),
            sensitivity: 0,
            policy_version: "v1".into(),
            entity_type: None,
            display_name: None,
            truth_state: TruthState::Current,
            valid_from: None,
            valid_until: None,
            created_at: None,
            source_kind: None,
            actor_id: None,
            staleness_class: None,
            revision: GraphRevision::base(),
        },
    ];

    // Step 1: apply policy filter.
    let filtered = GraphPolicyFilter::filter_nodes(&ctx, all_nodes.clone());
    assert_eq!(filtered.authorized.len(), 2, "only A and C pass policy");
    assert_eq!(filtered.hidden_count, 1);

    // Step 2: BFS on the filtered node set.
    let edges = vec![
        make_edge(&a, &b, "e-ab"),
        make_edge(&b, &c, "e-bc"),
        make_edge(&a, &c, "e-ac"),
    ];
    let result = BreadthFirstTraversal::execute(&[a.clone()], &filtered.authorized, &edges, 3);

    let result_ids: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
    // B was filtered out; since B's node is not in filtered.authorized,
    // BFS won't find it in node_map → won't add it to result.
    assert!(
        !result_ids.contains(&b.as_str()),
        "B hidden by policy must not appear in BFS"
    );
    assert!(result_ids.contains(&a.as_str()), "A must be in result");
}

/// Test 24: EdgeAssembler + LabelGuard integration.
/// Edge whose source node has a UUID display_name gets sanitized to None.
#[test]
fn edge_assembler_label_guard_uuid_display_name_sanitized() {
    let a = make_id(1);
    let b = make_id(2);
    let rev = GraphRevision::base();

    // Source node has a UUID as display_name — LabelGuard must sanitize to None.
    let uuid_label = "00000000-0000-7000-8000-000000000001";
    let source_node = make_traversal_node_named(a.clone(), Some(uuid_label));
    let target_node = make_traversal_node_named(b.clone(), Some("Bob"));

    let mut lookup = HashMap::new();
    lookup.insert(a.as_str().to_owned(), source_node);
    lookup.insert(b.as_str().to_owned(), target_node);

    let edge = make_edge(&a, &b, "e-ab");
    let result = EdgeAssembler::assemble_edge(&edge, &lookup, rev);

    assert!(result.is_some(), "edge must be assembled");
    let projected = result.unwrap();
    assert!(
        projected.source_endpoint.display_name.is_none(),
        "UUID display_name must be sanitized to None by LabelGuard"
    );
    assert_eq!(
        projected.target_endpoint.display_name.as_deref(),
        Some("Bob"),
        "valid display_name must pass through"
    );
}

/// Test 25: GraphProjectionResponse always has same graph_revision on all items.
/// Build a response with multiple nodes and edges; assert all graph_revision equal.
#[test]
fn graph_projection_response_all_items_same_revision() {
    let rev = GraphRevision::new(42);
    let validated = make_validated_request();

    let a = make_id(1);
    let b = make_id(2);
    let c = make_id(3);

    let entities = vec![
        make_raw_entity(a.clone()),
        make_raw_entity(b.clone()),
        make_raw_entity(c.clone()),
    ];
    let edges = vec![
        make_raw_edge(&a, &b, "e-ab", true, true),
        make_raw_edge(&b, &c, "e-bc", true, true),
    ];

    let response = GraphQueryProjector::build_entity_primary(&validated, entities, edges, rev);

    // Response-level revision.
    assert_eq!(response.graph_revision, rev);

    // All nodes must carry the same revision.
    for node in &response.nodes {
        assert_eq!(
            node.graph_revision,
            rev,
            "node {} must have revision {rev}",
            node.id.as_uuid_str()
        );
    }

    // All edges must carry the same revision.
    for edge in &response.edges {
        assert_eq!(
            edge.graph_revision, rev,
            "edge {} must have revision {rev}",
            edge.id
        );
    }
}
