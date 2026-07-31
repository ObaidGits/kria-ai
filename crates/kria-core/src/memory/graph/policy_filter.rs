//! Policy filter gate for the policy-safe bounded graph contract.
//!
//! **Task 2.3.4** — implements the explicit, testable, reusable policy filter
//! gate applied before every seed/expansion/count/frontier operation:
//!
//! * [`PolicyContext`] — the caller's effective policy context.
//! * [`PolicyFilterResult<T>`] — result of a filter operation; authorized items
//!   plus a hidden count (no hidden IDs exposed).
//! * [`GraphPolicyFilter`] — stateless filter gate with seed, node, edge, path,
//!   and frontier-summary operations.
//! * [`FrontierSummary`] — aggregate metadata for hidden frontiers (authorized
//!   count only — hidden IDs and topology are NEVER exposed).
//!
//! # Design Invariants
//! * A5: "Authorization and Effective Policy precede planning, counts, ranking,
//!   serialization, caching, and rendering." (design §A5)
//! * A4: No hidden ID, name, or topology is exposed. Missing or hidden data
//!   surfaces as a count or [`FrontierSummary`]. (design §A4)
//! * §6.5: "Hidden intermediary means the entire path is omitted; frontier
//!   metadata reveals only authorized aggregate tokens."
//! * MGR-004 AC 4/5: policy enforced before query planning and counts; hidden
//!   records contribute only to opaque aggregate counts, never to labeled output.

use crate::memory::graph::projection::TruncationReason;
use crate::memory::graph::traversal::{TraversalEdge, TraversalNode};
use crate::memory::model::EntityId;

// ── 1. PolicyContext ──────────────────────────────────────────────────────

/// The caller's effective policy context, used to decide which nodes/edges
/// are visible (design §A5; MGR-004 AC 4).
///
/// A node or edge is authorized when its `namespace`, `scope`, and
/// `sensitivity` all satisfy the constraints encoded here — AND the store-layer
/// authorization flags (`is_authorized` / `source_authorized` /
/// `target_authorized`) are also `true`.
///
/// `policy_version` is the provenance hash for cache invalidation; it is NOT
/// used in filter comparisons (only namespace/scope/max_sensitivity gate access).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyContext {
    /// Caller's policy namespace (e.g. `"user"`, `"workspace"`).
    pub namespace: String,
    /// Caller's policy scope (e.g. `"chat"`, `"global"`).
    pub scope: String,
    /// Maximum sensitivity the caller can see (`0..=3`). Items with
    /// `sensitivity > max_sensitivity` are hidden.
    pub max_sensitivity: u8,
    /// Policy version/provenance hash. Used for cache invalidation only; not
    /// compared during filter decisions.
    pub policy_version: String,
}

// ── 2. PolicyFilterResult<T> ──────────────────────────────────────────────

/// Result of a policy filter operation (MGR-004 AC 4/5).
///
/// `authorized` contains only items that passed the policy gate.
/// `hidden_count` is the number of items that were hidden — it NEVER includes
/// hidden identifiers, hidden names, or hidden topology.
#[derive(Debug, Clone)]
pub struct PolicyFilterResult<T> {
    /// Items that passed the policy filter (caller-authorized).
    pub authorized: Vec<T>,
    /// Count of items that were hidden by policy. Does NOT contain hidden IDs.
    pub hidden_count: u32,
    /// Whether any items were hidden.
    pub has_hidden: bool,
}

impl<T> PolicyFilterResult<T> {
    /// Construct a result from the authorized set and a hidden count.
    /// When `hidden_count == 0`, `has_hidden` is `false`.
    fn with_hidden(authorized: Vec<T>, hidden_count: u32) -> Self {
        PolicyFilterResult {
            authorized,
            has_hidden: hidden_count > 0,
            hidden_count,
        }
    }
}

// ── 3. FrontierSummary ────────────────────────────────────────────────────

/// Metadata about a hidden frontier — exposes ONLY caller-authorized aggregate
/// information (design §6.5; MGR-004 AC 5).
///
/// NEVER exposes hidden IDs, hidden counts from unauthorized nodes, or hidden
/// topology. The `authorized_count` is the count of items the caller CAN see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontierSummary {
    /// Count of items the CALLER is authorized to see in this set.
    pub authorized_count: u32,
    /// Whether there are hidden items (`true` = some items were filtered).
    pub has_hidden: bool,
    /// The reason items are hidden.
    pub truncation_reason: TruncationReason,
}

// ── 4. Policy matching helpers ─────────────────────────────────────────────

/// Returns `true` when the node's namespace, scope, and sensitivity all satisfy
/// the caller's [`PolicyContext`] AND the store-layer `is_authorized` flag is
/// set.
///
/// All four conditions must hold for a node to be visible (design §A5):
/// 1. `node.sensitivity <= ctx.max_sensitivity`
/// 2. `node.namespace == ctx.namespace`
/// 3. `node.scope == ctx.scope`
/// 4. `node.is_authorized == true`
#[inline]
fn node_is_authorized(ctx: &PolicyContext, node: &TraversalNode) -> bool {
    node.is_authorized
        && node.sensitivity <= ctx.max_sensitivity
        && node.namespace == ctx.namespace
        && node.scope == ctx.scope
}

/// Returns `true` when the edge's namespace, scope, and sensitivity satisfy the
/// caller's [`PolicyContext`] AND both store-layer endpoint-authorization flags
/// are set.
///
/// All five conditions must hold (design §A5):
/// 1. `edge.sensitivity <= ctx.max_sensitivity`
/// 2. `edge.namespace == ctx.namespace`
/// 3. `edge.scope == ctx.scope`
/// 4. `edge.source_authorized == true`
/// 5. `edge.target_authorized == true`
#[inline]
fn edge_is_authorized(ctx: &PolicyContext, edge: &TraversalEdge) -> bool {
    edge.source_authorized
        && edge.target_authorized
        && edge.sensitivity <= ctx.max_sensitivity
        && edge.namespace == ctx.namespace
        && edge.scope == ctx.scope
}

// ── 5. GraphPolicyFilter ──────────────────────────────────────────────────

/// Stateless policy filter gate applied before every seed/expansion/count/
/// frontier operation (design §A5; MGR-004 AC 4/5; design §6.5).
///
/// All methods are pure functions — no IO, no state, no side effects.
/// Hidden items are NEVER included in the authorized set and their IDs, names,
/// or topology are NEVER returned; only an opaque count is exposed.
pub struct GraphPolicyFilter;

impl GraphPolicyFilter {
    /// Filter a set of [`TraversalNode`]s against a caller [`PolicyContext`].
    ///
    /// Returns authorized nodes and the count of hidden nodes.
    /// Hidden node IDs and metadata are NOT exposed in the result.
    pub fn filter_nodes(
        ctx: &PolicyContext,
        nodes: Vec<TraversalNode>,
    ) -> PolicyFilterResult<TraversalNode> {
        let mut authorized = Vec::with_capacity(nodes.len());
        let mut hidden_count: u32 = 0;

        for node in nodes {
            if node_is_authorized(ctx, &node) {
                authorized.push(node);
            } else {
                // Do NOT expose any field of hidden node — only count it.
                hidden_count = hidden_count.saturating_add(1);
            }
        }

        PolicyFilterResult::with_hidden(authorized, hidden_count)
    }

    /// Filter a set of [`TraversalEdge`]s against a caller [`PolicyContext`].
    ///
    /// An edge is hidden when:
    /// - The edge's own sensitivity exceeds `ctx.max_sensitivity`, OR
    /// - The edge's namespace/scope does not match ctx, OR
    /// - `source_authorized == false`, OR
    /// - `target_authorized == false`
    ///
    /// Returns authorized edges (both endpoints visible) and the hidden count.
    /// Hidden edge IDs and topology are NOT exposed.
    pub fn filter_edges(
        ctx: &PolicyContext,
        edges: Vec<TraversalEdge>,
    ) -> PolicyFilterResult<TraversalEdge> {
        let mut authorized = Vec::with_capacity(edges.len());
        let mut hidden_count: u32 = 0;

        for edge in edges {
            if edge_is_authorized(ctx, &edge) {
                authorized.push(edge);
            } else {
                // Do NOT expose any field of hidden edge — only count it.
                hidden_count = hidden_count.saturating_add(1);
            }
        }

        PolicyFilterResult::with_hidden(authorized, hidden_count)
    }

    /// Filter a slice of seed [`EntityId`]s against a node set.
    ///
    /// A seed is authorized when it appears in `all_nodes` AND the matching
    /// node passes `filter_nodes`-equivalent authorization (i.e. the node in
    /// `all_nodes` with the same ID is authorized by ctx).
    ///
    /// Seeds that do not appear in `all_nodes`, or whose node is hidden by
    /// policy, are excluded. Returns authorized seed IDs and a hidden count.
    /// Hidden seed IDs are NOT returned.
    pub fn filter_seeds(
        ctx: &PolicyContext,
        seeds: &[EntityId],
        all_nodes: &[TraversalNode],
    ) -> PolicyFilterResult<EntityId> {
        // Build a set of authorized node IDs for fast lookup.
        let authorized_ids: std::collections::HashSet<&str> = all_nodes
            .iter()
            .filter(|n| node_is_authorized(ctx, n))
            .map(|n| n.id.as_str())
            .collect();

        let mut authorized = Vec::with_capacity(seeds.len());
        let mut hidden_count: u32 = 0;

        for seed in seeds {
            if authorized_ids.contains(seed.as_str()) {
                authorized.push(seed.clone());
            } else {
                // Seed is either not in all_nodes or its node is hidden —
                // do NOT expose the seed ID in error output, only count it.
                hidden_count = hidden_count.saturating_add(1);
            }
        }

        PolicyFilterResult::with_hidden(authorized, hidden_count)
    }

    /// Produce a [`FrontierSummary`] for a set containing both authorized and
    /// hidden items, without exposing hidden IDs (design §6.5).
    ///
    /// The summary only exposes the `authorized_count` (items the caller CAN
    /// see). `has_hidden` is `true` when `hidden_count > 0`. The
    /// `truncation_reason` is always [`TruncationReason::PolicyFiltered`].
    pub fn frontier_summary(authorized_count: u32, hidden_count: u32) -> FrontierSummary {
        FrontierSummary {
            authorized_count,
            has_hidden: hidden_count > 0,
            truncation_reason: TruncationReason::PolicyFiltered,
        }
    }

    /// Validate that a traversal path has no unauthorized intermediary nodes
    /// (design §6.5; MGR-007 AC 6).
    ///
    /// Returns `true` when ALL nodes in the path are authorized
    /// (`is_authorized == true`).
    /// Returns `false` when ANY node is unauthorized — the caller MUST omit
    /// the entire path.
    ///
    /// Note: this checks only the store-layer `is_authorized` flag on each
    /// node. Callers that need the full policy check (namespace/scope/
    /// sensitivity) should pre-filter the node set with [`filter_nodes`] before
    /// building paths.
    pub fn path_is_fully_authorized(path_nodes: &[&TraversalNode]) -> bool {
        path_nodes.iter().all(|n| n.is_authorized)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::graph::projection::{DirectionClass, EdgeAuthorityClass};
    use crate::memory::model::{EntityId, GraphRevision, TruthState};

    // ── Test helpers ──────────────────────────────────────────────────────

    fn make_ctx() -> PolicyContext {
        PolicyContext {
            namespace: "user".into(),
            scope: "chat".into(),
            max_sensitivity: 2,
            policy_version: "v1".into(),
        }
    }

    fn make_id(suffix: u8) -> EntityId {
        let s = format!("00000000-0000-7000-8000-0000000000{:02x}", suffix);
        EntityId::new(&s).unwrap()
    }

    fn make_node(
        id: EntityId,
        authorized: bool,
        sensitivity: u8,
        namespace: &str,
        scope: &str,
    ) -> TraversalNode {
        TraversalNode {
            id,
            is_authorized: authorized,
            entity_type: None,
            display_name: None,
            truth_state: TruthState::Current,
            valid_from: None,
            valid_until: None,
            namespace: namespace.into(),
            scope: scope.into(),
            sensitivity,
            policy_version: "v1".into(),
            created_at: None,
            source_kind: None,
            actor_id: None,
            staleness_class: None,
            revision: GraphRevision::base(),
        }
    }

    fn make_edge(
        src: &EntityId,
        tgt: &EntityId,
        hash: &str,
        src_auth: bool,
        tgt_auth: bool,
        sensitivity: u8,
        namespace: &str,
        scope: &str,
    ) -> TraversalEdge {
        TraversalEdge {
            identity_hash: hash.into(),
            link_type: "relates_to".into(),
            link_type_version: 1,
            authority_class: EdgeAuthorityClass::Stored,
            direction: DirectionClass::Directed,
            source_id: src.clone(),
            target_id: tgt.clone(),
            source_authorized: src_auth,
            target_authorized: tgt_auth,
            truth_state: TruthState::Current,
            valid_from: None,
            valid_until: None,
            namespace: namespace.into(),
            scope: scope.into(),
            sensitivity,
            policy_version: "v1".into(),
            evidence_count: 0,
            created_at: None,
            source_kind: None,
            actor_id: None,
            staleness_class: None,
            revision: GraphRevision::base(),
        }
    }

    // ── Node filter tests ─────────────────────────────────────────────────

    #[test]
    fn node_filter_authorized_node_passes() {
        let ctx = make_ctx();
        let node = make_node(make_id(1), true, 1, "user", "chat");
        let result = GraphPolicyFilter::filter_nodes(&ctx, vec![node]);
        assert_eq!(result.authorized.len(), 1);
        assert_eq!(result.hidden_count, 0);
        assert!(!result.has_hidden);
    }

    #[test]
    fn node_filter_sensitivity_exceeds_max_is_hidden() {
        let ctx = make_ctx(); // max_sensitivity = 2
                              // sensitivity 3 > max 2 → hidden
        let node = make_node(make_id(1), true, 3, "user", "chat");
        let result = GraphPolicyFilter::filter_nodes(&ctx, vec![node]);
        assert!(result.authorized.is_empty());
        assert_eq!(result.hidden_count, 1);
        assert!(result.has_hidden);
    }

    #[test]
    fn node_filter_sensitivity_at_max_passes() {
        let ctx = make_ctx(); // max_sensitivity = 2
        let node = make_node(make_id(1), true, 2, "user", "chat");
        let result = GraphPolicyFilter::filter_nodes(&ctx, vec![node]);
        assert_eq!(result.authorized.len(), 1);
        assert_eq!(result.hidden_count, 0);
    }

    #[test]
    fn node_filter_wrong_namespace_is_hidden() {
        let ctx = make_ctx(); // namespace = "user"
        let node = make_node(make_id(1), true, 1, "workspace", "chat");
        let result = GraphPolicyFilter::filter_nodes(&ctx, vec![node]);
        assert!(result.authorized.is_empty());
        assert_eq!(result.hidden_count, 1);
    }

    #[test]
    fn node_filter_wrong_scope_is_hidden() {
        let ctx = make_ctx(); // scope = "chat"
        let node = make_node(make_id(1), true, 1, "user", "global");
        let result = GraphPolicyFilter::filter_nodes(&ctx, vec![node]);
        assert!(result.authorized.is_empty());
        assert_eq!(result.hidden_count, 1);
    }

    #[test]
    fn node_filter_is_authorized_false_is_hidden() {
        let ctx = make_ctx();
        // Matching namespace/scope/sensitivity but store says unauthorized
        let node = make_node(make_id(1), false, 1, "user", "chat");
        let result = GraphPolicyFilter::filter_nodes(&ctx, vec![node]);
        assert!(result.authorized.is_empty());
        assert_eq!(result.hidden_count, 1);
    }

    #[test]
    fn node_filter_mixed_authorized_and_hidden_counts_correctly() {
        let ctx = make_ctx();
        let nodes = vec![
            make_node(make_id(1), true, 1, "user", "chat"), // passes
            make_node(make_id(2), true, 3, "user", "chat"), // sensitivity too high
            make_node(make_id(3), false, 1, "user", "chat"), // store unauthorized
            make_node(make_id(4), true, 1, "user", "chat"), // passes
            make_node(make_id(5), true, 1, "workspace", "chat"), // wrong namespace
        ];
        let result = GraphPolicyFilter::filter_nodes(&ctx, nodes);
        assert_eq!(result.authorized.len(), 2);
        assert_eq!(result.hidden_count, 3);
        assert!(result.has_hidden);
        // Verify hidden IDs are NOT exposed
        let authorized_ids: Vec<&str> = result.authorized.iter().map(|n| n.id.as_str()).collect();
        assert!(authorized_ids.contains(&make_id(1).as_str()));
        assert!(authorized_ids.contains(&make_id(4).as_str()));
    }

    // ── Edge filter tests ─────────────────────────────────────────────────

    #[test]
    fn edge_filter_fully_authorized_edge_passes() {
        let ctx = make_ctx();
        let a = make_id(1);
        let b = make_id(2);
        let edge = make_edge(&a, &b, "e-ab", true, true, 1, "user", "chat");
        let result = GraphPolicyFilter::filter_edges(&ctx, vec![edge]);
        assert_eq!(result.authorized.len(), 1);
        assert_eq!(result.hidden_count, 0);
    }

    #[test]
    fn edge_filter_source_unauthorized_hides_edge() {
        let ctx = make_ctx();
        let a = make_id(1);
        let b = make_id(2);
        let edge = make_edge(&a, &b, "e-ab", false, true, 1, "user", "chat");
        let result = GraphPolicyFilter::filter_edges(&ctx, vec![edge]);
        assert!(result.authorized.is_empty());
        assert_eq!(result.hidden_count, 1);
    }

    #[test]
    fn edge_filter_target_unauthorized_hides_edge() {
        let ctx = make_ctx();
        let a = make_id(1);
        let b = make_id(2);
        let edge = make_edge(&a, &b, "e-ab", true, false, 1, "user", "chat");
        let result = GraphPolicyFilter::filter_edges(&ctx, vec![edge]);
        assert!(result.authorized.is_empty());
        assert_eq!(result.hidden_count, 1);
    }

    #[test]
    fn edge_filter_high_sensitivity_hides_edge() {
        let ctx = make_ctx(); // max_sensitivity = 2
        let a = make_id(1);
        let b = make_id(2);
        let edge = make_edge(&a, &b, "e-ab", true, true, 3, "user", "chat");
        let result = GraphPolicyFilter::filter_edges(&ctx, vec![edge]);
        assert!(result.authorized.is_empty());
        assert_eq!(result.hidden_count, 1);
    }

    #[test]
    fn edge_filter_wrong_namespace_hides_edge() {
        let ctx = make_ctx();
        let a = make_id(1);
        let b = make_id(2);
        let edge = make_edge(&a, &b, "e-ab", true, true, 1, "other", "chat");
        let result = GraphPolicyFilter::filter_edges(&ctx, vec![edge]);
        assert!(result.authorized.is_empty());
        assert_eq!(result.hidden_count, 1);
    }

    // ── Seed filter tests ─────────────────────────────────────────────────

    #[test]
    fn seed_filter_authorized_seeds_pass() {
        let ctx = make_ctx();
        let a = make_id(1);
        let b = make_id(2);
        let nodes = vec![
            make_node(a.clone(), true, 1, "user", "chat"),
            make_node(b.clone(), true, 1, "user", "chat"),
        ];
        let seeds = vec![a.clone(), b.clone()];
        let result = GraphPolicyFilter::filter_seeds(&ctx, &seeds, &nodes);
        assert_eq!(result.authorized.len(), 2);
        assert_eq!(result.hidden_count, 0);
    }

    #[test]
    fn seed_filter_unauthorized_seed_excluded_and_counted() {
        let ctx = make_ctx();
        let a = make_id(1);
        let b = make_id(2);
        let nodes = vec![
            make_node(a.clone(), true, 1, "user", "chat"),
            // b is in the node set but hidden (wrong namespace)
            make_node(b.clone(), true, 1, "workspace", "chat"),
        ];
        let seeds = vec![a.clone(), b.clone()];
        let result = GraphPolicyFilter::filter_seeds(&ctx, &seeds, &nodes);
        assert_eq!(result.authorized.len(), 1);
        assert_eq!(result.authorized[0].as_str(), a.as_str());
        assert_eq!(result.hidden_count, 1);
        assert!(result.has_hidden);
    }

    #[test]
    fn seed_filter_seed_not_in_nodes_is_excluded() {
        let ctx = make_ctx();
        let a = make_id(1);
        let unknown = make_id(99); // not in all_nodes
        let nodes = vec![make_node(a.clone(), true, 1, "user", "chat")];
        let seeds = vec![a.clone(), unknown.clone()];
        let result = GraphPolicyFilter::filter_seeds(&ctx, &seeds, &nodes);
        assert_eq!(result.authorized.len(), 1);
        assert_eq!(result.hidden_count, 1);
    }

    #[test]
    fn seed_filter_store_unauthorized_seed_excluded() {
        let ctx = make_ctx();
        let a = make_id(1);
        // Node is in nodes but store says not authorized
        let nodes = vec![make_node(a.clone(), false, 1, "user", "chat")];
        let seeds = vec![a.clone()];
        let result = GraphPolicyFilter::filter_seeds(&ctx, &seeds, &nodes);
        assert!(result.authorized.is_empty());
        assert_eq!(result.hidden_count, 1);
    }

    // ── Path authorization tests ──────────────────────────────────────────

    #[test]
    fn path_fully_authorized_all_authorized_returns_true() {
        let n1 = make_node(make_id(1), true, 1, "user", "chat");
        let n2 = make_node(make_id(2), true, 1, "user", "chat");
        let n3 = make_node(make_id(3), true, 1, "user", "chat");
        assert!(GraphPolicyFilter::path_is_fully_authorized(&[
            &n1, &n2, &n3
        ]));
    }

    #[test]
    fn path_with_hidden_intermediary_returns_false() {
        let n1 = make_node(make_id(1), true, 1, "user", "chat");
        // n2 is the hidden intermediary
        let n2 = make_node(make_id(2), false, 1, "user", "chat");
        let n3 = make_node(make_id(3), true, 1, "user", "chat");
        assert!(!GraphPolicyFilter::path_is_fully_authorized(&[
            &n1, &n2, &n3
        ]));
    }

    #[test]
    fn path_with_hidden_start_node_returns_false() {
        let n1 = make_node(make_id(1), false, 1, "user", "chat");
        let n2 = make_node(make_id(2), true, 1, "user", "chat");
        assert!(!GraphPolicyFilter::path_is_fully_authorized(&[&n1, &n2]));
    }

    #[test]
    fn path_with_hidden_end_node_returns_false() {
        let n1 = make_node(make_id(1), true, 1, "user", "chat");
        let n2 = make_node(make_id(2), false, 1, "user", "chat");
        assert!(!GraphPolicyFilter::path_is_fully_authorized(&[&n1, &n2]));
    }

    #[test]
    fn path_empty_returns_true() {
        assert!(GraphPolicyFilter::path_is_fully_authorized(&[]));
    }

    #[test]
    fn path_single_authorized_node_returns_true() {
        let n = make_node(make_id(1), true, 1, "user", "chat");
        assert!(GraphPolicyFilter::path_is_fully_authorized(&[&n]));
    }

    #[test]
    fn path_single_unauthorized_node_returns_false() {
        let n = make_node(make_id(1), false, 1, "user", "chat");
        assert!(!GraphPolicyFilter::path_is_fully_authorized(&[&n]));
    }

    // ── FrontierSummary tests ─────────────────────────────────────────────

    #[test]
    fn frontier_summary_no_hidden() {
        let summary = GraphPolicyFilter::frontier_summary(5, 0);
        assert_eq!(summary.authorized_count, 5);
        assert!(!summary.has_hidden);
        assert_eq!(summary.truncation_reason, TruncationReason::PolicyFiltered);
    }

    #[test]
    fn frontier_summary_with_hidden() {
        let summary = GraphPolicyFilter::frontier_summary(3, 7);
        assert_eq!(summary.authorized_count, 3);
        assert!(summary.has_hidden);
        assert_eq!(summary.truncation_reason, TruncationReason::PolicyFiltered);
    }

    #[test]
    fn frontier_summary_zero_authorized_with_hidden() {
        let summary = GraphPolicyFilter::frontier_summary(0, 10);
        assert_eq!(summary.authorized_count, 0);
        assert!(summary.has_hidden);
    }

    #[test]
    fn frontier_summary_never_exposes_hidden_ids() {
        // Structural: FrontierSummary has no ID fields — verify at compile time
        // by accessing all public fields.
        let summary = GraphPolicyFilter::frontier_summary(2, 3);
        let _authorized_count: u32 = summary.authorized_count;
        let _has_hidden: bool = summary.has_hidden;
        let _reason: TruncationReason = summary.truncation_reason;
        // No `hidden_ids`, `hidden_count`, or `hidden_anything` field exists.
    }

    // ── PolicyContext matching tests ──────────────────────────────────────

    #[test]
    fn policy_context_all_conditions_required() {
        let ctx = PolicyContext {
            namespace: "user".into(),
            scope: "chat".into(),
            max_sensitivity: 1,
            policy_version: "v1".into(),
        };
        // All conditions met → authorized
        let pass = make_node(make_id(1), true, 1, "user", "chat");
        let result = GraphPolicyFilter::filter_nodes(&ctx, vec![pass]);
        assert_eq!(result.authorized.len(), 1);

        // Fail each condition individually
        let fail_ns = make_node(make_id(2), true, 1, "other", "chat");
        let fail_sc = make_node(make_id(3), true, 1, "user", "other");
        let fail_se = make_node(make_id(4), true, 2, "user", "chat"); // sensitivity 2 > max 1
        let fail_au = make_node(make_id(5), false, 1, "user", "chat");

        for failing in [fail_ns, fail_sc, fail_se, fail_au] {
            let r = GraphPolicyFilter::filter_nodes(&ctx, vec![failing]);
            assert!(r.authorized.is_empty(), "expected node to be hidden");
            assert_eq!(r.hidden_count, 1);
        }
    }

    #[test]
    fn empty_input_produces_empty_authorized_zero_hidden() {
        let ctx = make_ctx();
        let r_nodes = GraphPolicyFilter::filter_nodes(&ctx, vec![]);
        assert!(r_nodes.authorized.is_empty());
        assert_eq!(r_nodes.hidden_count, 0);
        assert!(!r_nodes.has_hidden);

        let r_edges = GraphPolicyFilter::filter_edges(&ctx, vec![]);
        assert!(r_edges.authorized.is_empty());
        assert_eq!(r_edges.hidden_count, 0);

        let r_seeds = GraphPolicyFilter::filter_seeds(&ctx, &[], &[]);
        assert!(r_seeds.authorized.is_empty());
        assert_eq!(r_seeds.hidden_count, 0);
    }
}
