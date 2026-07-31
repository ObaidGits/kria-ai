//! Entity-primary query projection for the typed graph contract.
//!
//! **Task 2.3.2** — implements the entity-primary query projection:
//!
//! * [`GraphQueryRequest`] — input to projection with explicit child expansion.
//! * [`ExpandChildrenRequest`] — controls memory/evidence/source expansion.
//! * [`ProjectionLimits`] — hard constant caps for nodes, edges, children, deadlines.
//! * [`ValidatedRequest`] — clamped validated query parameters (opaque output of
//!   [`GraphQueryProjector::validate_request`]).
//! * [`RawEntityRow`] / [`RawEdgeRow`] — simple store-layer input structs.
//! * [`ProjectionError`] — typed validation errors.
//! * [`GraphQueryProjector`] — pure stateless projection builder enforcing the
//!   entity-primary contract and generating labeled navigation facets.
//!
//! # Design Invariants (from design.md §2)
//! * **A4 / entity-primary default:** only `Entity` nodes are returned unless
//!   explicit expansion is requested.
//! * **A5:** Effective_Policy precedes any expansion.
//! * **A6:** max_nodes / max_edges are hard-capped.
//! * **A4 / hidden endpoints:** `source_visible=false` or `target_visible=false`
//!   on a [`RawEdgeRow`] → the edge is OMITTED and a [`FrontierAggregate`] node
//!   is added.  Hidden IDs are never exposed.
//! * Navigation facets produced by the projector carry
//!   `node_kind = ProjectedNodeKind::Aggregate` and
//!   `authority_class = EdgeAuthorityClass::Navigation`.
//! * All nodes and edges in a response share the same [`GraphRevision`].
//! * **MGR-001 AC 4:** generated navigation groups are labeled as navigation and
//!   excluded from authority topology.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::memory::graph::projection::{
    DirectionClass, EdgeAuthorityClass, EffectivePolicySummary, GraphProjectionResponse,
    NodeMetadata, ProjectedEdge, ProjectedItemId, ProjectedNode, ProjectedNodeKind,
    ProjectedValidTime, ProvenanceSummary, TotalSemantics, TruncationReason,
};
use crate::memory::model::{EntityId, GraphRevision, TruthState, UtcTimestamp};
use crate::memory::types::StalenessClass;

// ── 1. ProjectionLimits ───────────────────────────────────────────────────

/// Hard constant limits for all graph projection requests (design §A6).
///
/// These caps are enforced in [`GraphQueryProjector::validate_request`] and
/// in [`GraphQueryProjector::build_entity_primary`].  They cannot be overridden
/// by callers.
pub struct ProjectionLimits;

impl ProjectionLimits {
    /// Hard cap on nodes returned per request.
    pub const MAX_NODES: u32 = 500;
    /// Hard cap on edges returned per request.
    pub const MAX_EDGES: u32 = 1000;
    /// Default max nodes when the caller does not specify.
    pub const DEFAULT_MAX_NODES: u32 = 120;
    /// Default max edges when the caller does not specify.
    pub const DEFAULT_MAX_EDGES: u32 = 180;
    /// Hard cap on children per entity during expansion.
    pub const MAX_CHILDREN_PER_ENTITY: u32 = 50;
    /// Default children per entity when the caller does not specify.
    pub const DEFAULT_CHILDREN_PER_ENTITY: u32 = 10;
    /// Default query deadline in milliseconds.
    pub const DEFAULT_DEADLINE_MS: u32 = 250;
    /// Hard cap on query deadline in milliseconds.
    pub const MAX_DEADLINE_MS: u32 = 2000;
}

// ── 2. ExpandChildrenRequest ──────────────────────────────────────────────

/// Controls non-entity expansion when explicitly requested (MGR-002 AC 5).
///
/// All flags default to `false`; callers must opt in to each child kind they
/// want.  When the containing [`GraphQueryRequest::expand_children`] is `None`,
/// expansion is fully suppressed regardless of these flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandChildrenRequest {
    /// Include Memory record nodes as children of entities.
    pub include_memory: bool,
    /// Include Evidence nodes attached to relationships.
    pub include_evidence: bool,
    /// Include Source nodes.
    pub include_source: bool,
    /// Maximum children per entity.  Clamped to
    /// [`ProjectionLimits::MAX_CHILDREN_PER_ENTITY`].
    /// Defaults to [`ProjectionLimits::DEFAULT_CHILDREN_PER_ENTITY`] when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_children_per_entity: Option<u32>,
}

impl ExpandChildrenRequest {
    /// Returns `true` when no child kind is requested (effectively a no-op
    /// expansion).
    pub fn is_empty(&self) -> bool {
        !self.include_memory && !self.include_evidence && !self.include_source
    }
}

// ── 3. GraphQueryRequest ──────────────────────────────────────────────────

/// Input to entity-primary graph projection (MGR-002 AC 5; MGR-007 AC 2).
///
/// In entity-primary mode (`expand_children = None` or all flags false), only
/// [`ProjectedNodeKind::Entity`] nodes are returned.  Memory, evidence, and
/// source rows are silently excluded (not an error).
///
/// Effective_Policy is applied before any expansion (design §A5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQueryRequest {
    /// Seed entity IDs to project from.  Empty = overview / recent entities.
    pub seeds: Vec<EntityId>,
    /// Whether to include memory/evidence/source children of entities.
    /// `None` = entity-primary mode (default, bounded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expand_children: Option<ExpandChildrenRequest>,
    /// Maximum number of nodes in the response.  Clamped to
    /// [`ProjectionLimits::MAX_NODES`].  Defaults to
    /// [`ProjectionLimits::DEFAULT_MAX_NODES`] when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_nodes: Option<u32>,
    /// Maximum number of edges in the response.  Clamped to
    /// [`ProjectionLimits::MAX_EDGES`].  Defaults to
    /// [`ProjectionLimits::DEFAULT_MAX_EDGES`] when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_edges: Option<u32>,
    /// Caller policy context used to filter before any expansion (design §A5).
    pub policy_scope: EffectivePolicySummary,
    /// Graph revision to project at.  `None` = current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_revision: Option<GraphRevision>,
    /// Cursor from a previous response for continuation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Query deadline in milliseconds from now.  Default
    /// [`ProjectionLimits::DEFAULT_DEADLINE_MS`], hard max
    /// [`ProjectionLimits::MAX_DEADLINE_MS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u32>,
}

// ── 4. ProjectionError ────────────────────────────────────────────────────

/// Typed validation errors returned by [`GraphQueryProjector::validate_request`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    /// `seeds` count exceeded the hard limit (currently 200).
    InvalidSeedCount { max: u32, got: usize },
    /// `max_nodes` exceeds the hard cap [`ProjectionLimits::MAX_NODES`].
    InvalidMaxNodes { max: u32, got: u32 },
    /// `max_edges` exceeds the hard cap [`ProjectionLimits::MAX_EDGES`].
    InvalidMaxEdges { max: u32, got: u32 },
    /// `deadline_ms` exceeds the hard cap [`ProjectionLimits::MAX_DEADLINE_MS`].
    InvalidDeadline { max: u32, got: u32 },
    /// The revision carried on a result row does not match the expected revision.
    RevisionMismatch {
        expected: GraphRevision,
        got: GraphRevision,
    },
}

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectionError::InvalidSeedCount { max, got } => {
                write!(f, "seed count {got} exceeds hard limit {max}")
            }
            ProjectionError::InvalidMaxNodes { max, got } => {
                write!(f, "max_nodes {got} exceeds hard limit {max}")
            }
            ProjectionError::InvalidMaxEdges { max, got } => {
                write!(f, "max_edges {got} exceeds hard limit {max}")
            }
            ProjectionError::InvalidDeadline { max, got } => {
                write!(f, "deadline_ms {got} exceeds hard limit {max}")
            }
            ProjectionError::RevisionMismatch { expected, got } => {
                write!(f, "revision mismatch: expected {expected}, got {got}")
            }
        }
    }
}

impl std::error::Error for ProjectionError {}

/// Hard cap on seed count (validated but not in the public constants).
const MAX_SEED_COUNT: u32 = 200;

// ── 5. ValidatedRequest ───────────────────────────────────────────────────

/// Opaque, clamped output of [`GraphQueryProjector::validate_request`].
///
/// All limits are already clamped to their hard maxima.  Fields are
/// `pub(crate)` so only the projection layer — not external callers — can
/// construct or inspect them directly.
#[derive(Debug, Clone)]
#[allow(dead_code)] // cursor/deadline_ms/max_children_per_entity reserved for projection pagination/enforcement.
pub struct ValidatedRequest {
    pub(crate) seeds: Vec<EntityId>,
    pub(crate) expand_children: Option<ExpandChildrenRequest>,
    pub(crate) max_nodes: u32,
    pub(crate) max_edges: u32,
    pub(crate) max_children_per_entity: u32,
    pub(crate) policy_scope: EffectivePolicySummary,
    pub(crate) at_revision: Option<GraphRevision>,
    pub(crate) cursor: Option<String>,
    pub(crate) deadline_ms: u32,
}

// ── 6. Raw input types ────────────────────────────────────────────────────

/// A single entity row from the store, authorized and ready for projection.
///
/// This is the input type for [`GraphQueryProjector::build_entity_primary`].
/// Fields match the `entities_v2` / `entities` schema columns (design §4.2).
#[derive(Debug, Clone)]
pub struct RawEntityRow {
    /// Stable entity identity.
    pub id: EntityId,
    /// Free-text entity type (e.g. `"person"`, `"project"`).
    pub entity_type: Option<String>,
    /// Human-facing display name.  MUST NOT be a raw UUID label.
    pub display_name: Option<String>,
    /// Truth state of this entity.
    pub truth_state: TruthState,
    /// Valid-time lower bound.
    pub valid_from: Option<UtcTimestamp>,
    /// Valid-time upper bound.
    pub valid_until: Option<UtcTimestamp>,
    /// Effective sensitivity level `0..=3`.
    pub sensitivity: u8,
    /// Policy namespace.
    pub namespace: String,
    /// Policy scope.
    pub scope: String,
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

/// A single edge row from the store, authorized and ready for projection.
///
/// When `source_visible=false` or `target_visible=false` the edge MUST NOT
/// appear in the projected edges list.  The projector converts the hidden
/// endpoint into a [`FrontierAggregate`] node instead (design §A4; MGR-002 AC 6).
#[derive(Debug, Clone)]
pub struct RawEdgeRow {
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
    pub source_entity_id: EntityId,
    /// Target entity id.
    pub target_entity_id: EntityId,
    /// `false` = source endpoint is hidden by policy.
    pub source_visible: bool,
    /// `false` = target endpoint is hidden by policy.
    pub target_visible: bool,
    /// Truth state of this edge.
    pub truth_state: TruthState,
    /// Valid-time lower bound.
    pub valid_from: Option<UtcTimestamp>,
    /// Valid-time upper bound.
    pub valid_until: Option<UtcTimestamp>,
    /// Effective sensitivity level `0..=3`.
    pub sensitivity: u8,
    /// Policy namespace.
    pub namespace: String,
    /// Policy scope.
    pub scope: String,
    /// Policy provenance hash.
    pub policy_version: String,
    /// Count of evidence records attached to this edge.
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

// ── 7. GraphQueryProjector ────────────────────────────────────────────────

/// Pure stateless projection builder that enforces the entity-primary contract.
///
/// This is the DTO assembly layer — it does not touch SQL or IO.  The store
/// layer (task 2.3.3+) supplies the authorized raw rows; this projector maps
/// them to the canonical [`GraphProjectionResponse`] according to the rules in
/// design §A4–A6 and MGR-002/MGR-007.
pub struct GraphQueryProjector;

impl GraphQueryProjector {
    /// Validate and normalize a [`GraphQueryRequest`], enforcing all hard limits.
    ///
    /// Numeric limits are *clamped* (not rejected) when they exceed the default
    /// but are below the hard cap.  Only values strictly exceeding the hard cap
    /// produce a [`ProjectionError`].
    pub fn validate_request(req: &GraphQueryRequest) -> Result<ValidatedRequest, ProjectionError> {
        // Seed count
        if req.seeds.len() > MAX_SEED_COUNT as usize {
            return Err(ProjectionError::InvalidSeedCount {
                max: MAX_SEED_COUNT,
                got: req.seeds.len(),
            });
        }

        // max_nodes: reject if strictly over hard cap
        let max_nodes = match req.max_nodes {
            None => ProjectionLimits::DEFAULT_MAX_NODES,
            Some(n) if n > ProjectionLimits::MAX_NODES => {
                return Err(ProjectionError::InvalidMaxNodes {
                    max: ProjectionLimits::MAX_NODES,
                    got: n,
                })
            }
            Some(n) => n,
        };

        // max_edges: reject if strictly over hard cap
        let max_edges = match req.max_edges {
            None => ProjectionLimits::DEFAULT_MAX_EDGES,
            Some(n) if n > ProjectionLimits::MAX_EDGES => {
                return Err(ProjectionError::InvalidMaxEdges {
                    max: ProjectionLimits::MAX_EDGES,
                    got: n,
                })
            }
            Some(n) => n,
        };

        // deadline_ms: reject if strictly over hard cap
        let deadline_ms = match req.deadline_ms {
            None => ProjectionLimits::DEFAULT_DEADLINE_MS,
            Some(d) if d > ProjectionLimits::MAX_DEADLINE_MS => {
                return Err(ProjectionError::InvalidDeadline {
                    max: ProjectionLimits::MAX_DEADLINE_MS,
                    got: d,
                })
            }
            Some(d) => d,
        };

        // max_children_per_entity: clamp to hard cap
        let max_children_per_entity = req
            .expand_children
            .as_ref()
            .and_then(|e| e.max_children_per_entity)
            .unwrap_or(ProjectionLimits::DEFAULT_CHILDREN_PER_ENTITY)
            .min(ProjectionLimits::MAX_CHILDREN_PER_ENTITY);

        Ok(ValidatedRequest {
            seeds: req.seeds.clone(),
            expand_children: req.expand_children.clone(),
            max_nodes,
            max_edges,
            max_children_per_entity,
            policy_scope: req.policy_scope.clone(),
            at_revision: req.at_revision,
            cursor: req.cursor.clone(),
            deadline_ms,
        })
    }

    /// Build an entity-primary projection from raw authorized entity and edge rows.
    ///
    /// Behavioural rules (design §A4–A6, MGR-002 AC 3/5, MGR-001 AC 4):
    ///
    /// 1. **Entity-primary default:** when `expand_children` is `None` or all
    ///    flags are false, only `Entity` nodes appear.  Non-entity rows are
    ///    silently excluded.
    /// 2. **Hidden endpoints → FrontierAggregate:** edges with
    ///    `source_visible=false` or `target_visible=false` are omitted from
    ///    `edges`; a deduplicated [`ProjectedNode`] of kind `Aggregate` /
    ///    authority `Navigation` is added with `authorized_count: None` and
    ///    `truncation_reason: PolicyFiltered`.
    /// 3. **Hard limits:** nodes and edges are truncated at the validated caps.
    ///    `truncated=true` and `truncation_reason: ItemLimit` are set when any
    ///    limit fires.
    /// 4. **Same graph_revision:** every node and edge carries the supplied
    ///    `graph_revision`.
    /// 5. **Deterministic ordering:** nodes are sorted by entity_type then
    ///    display_name; edges are sorted by link_type then identity_hash.
    pub fn build_entity_primary(
        validated: &ValidatedRequest,
        authorized_entities: Vec<RawEntityRow>,
        authorized_edges: Vec<RawEdgeRow>,
        graph_revision: GraphRevision,
    ) -> GraphProjectionResponse {
        let entity_primary_mode = validated
            .expand_children
            .as_ref()
            .map_or(true, |e| e.is_empty());

        // ── Build entity nodes ──────────────────────────────────────────
        let mut nodes: Vec<ProjectedNode> = authorized_entities
            .iter()
            .map(|row| entity_row_to_node(row, graph_revision))
            .collect();

        // Sort deterministically: entity_type (None < Some) then display_name
        nodes.sort_by(|a, b| {
            let ta = extract_entity_type(&a.metadata);
            let tb = extract_entity_type(&b.metadata);
            ta.cmp(&tb)
                .then_with(|| a.display_name.as_deref().cmp(&b.display_name.as_deref()))
        });

        // ── Process edges — hidden endpoints become FrontierAggregate nodes ──
        let mut projected_edges: Vec<ProjectedEdge> = Vec::new();
        // Track which hidden entity IDs already have a frontier node, to deduplicate.
        let mut frontier_ids_added: HashSet<String> = HashSet::new();
        let mut frontier_nodes: Vec<ProjectedNode> = Vec::new();

        let mut edges_truncated = false;
        let mut nodes_truncated = false;

        // Sort edges deterministically before processing
        let mut sorted_edges = authorized_edges;
        sorted_edges.sort_by(|a, b| {
            a.link_type
                .cmp(&b.link_type)
                .then_with(|| a.identity_hash.cmp(&b.identity_hash))
        });

        for edge in &sorted_edges {
            // Skip edges whose entity-primary mode excludes non-entity rows.
            // (In entity-primary mode we still process edges between entities
            //  but we need entity nodes visible on both sides.)

            // Check hidden endpoints first (design §A4 — never expose hidden IDs)
            let source_hidden = !edge.source_visible;
            let target_hidden = !edge.target_visible;

            if source_hidden || target_hidden {
                // Edge is omitted; produce FrontierAggregate node(s)
                if source_hidden {
                    let key = format!("frontier:source:{}", edge.source_entity_id.as_str());
                    if !frontier_ids_added.contains(&key) {
                        frontier_ids_added.insert(key);
                        frontier_nodes.push(hidden_endpoint_frontier(
                            graph_revision,
                            validated.policy_scope.clone(),
                        ));
                    }
                }
                if target_hidden {
                    let key = format!("frontier:target:{}", edge.target_entity_id.as_str());
                    if !frontier_ids_added.contains(&key) {
                        frontier_ids_added.insert(key);
                        frontier_nodes.push(hidden_endpoint_frontier(
                            graph_revision,
                            validated.policy_scope.clone(),
                        ));
                    }
                }
                continue; // edge is NOT added to projected_edges
            }

            // Both endpoints visible: add edge if within limit
            if projected_edges.len() >= validated.max_edges as usize {
                edges_truncated = true;
                break;
            }
            projected_edges.push(edge_row_to_edge(edge, graph_revision));
        }

        // ── Merge frontier nodes into main node list ────────────────────
        // Only add frontier nodes if they fit within the node budget
        for fn_node in frontier_nodes {
            nodes.push(fn_node);
        }

        // ── Apply node limit ────────────────────────────────────────────
        if nodes.len() > validated.max_nodes as usize {
            nodes.truncate(validated.max_nodes as usize);
            nodes_truncated = true;
        }

        let truncated = nodes_truncated || edges_truncated;
        let truncation_reason = if truncated {
            Some(TruncationReason::ItemLimit)
        } else {
            None
        };

        let total_node_count = nodes.len() as u64;

        // ── entity_primary_mode: discard non-entity nodes (always for default) ─
        // In entity-primary mode, filter to keep only Entity and Aggregate nodes.
        // Aggregate nodes come from FrontierAggregates above, which are always kept.
        if entity_primary_mode {
            nodes.retain(|n| {
                matches!(
                    n.node_kind,
                    ProjectedNodeKind::Entity | ProjectedNodeKind::Aggregate
                )
            });
        }

        GraphProjectionResponse {
            schema_version: 2,
            graph_revision,
            query_hash: compute_query_hash(validated),
            policy_scope_summary: validated.policy_scope.clone(),
            nodes,
            edges: projected_edges,
            truncated,
            truncation_reason,
            frontier_token: None,
            total_semantics: TotalSemantics::Exact {
                count: total_node_count,
            },
            cursor: None,
        }
    }

    /// Build a navigation facet node — a generated grouping container labeled
    /// as navigation (not authority topology).
    ///
    /// The returned node has:
    /// * `node_kind = ProjectedNodeKind::Aggregate`
    /// * `authority_class = EdgeAuthorityClass::Navigation`
    /// * `display_name` set to `facet_label` (must not be a UUID)
    /// * `metadata = NodeMetadata::Aggregate { aggregate_kind: Entity, authorized_count: Some(member_count), truncation_reason: ItemLimit }`
    ///
    /// Design §MGR-001 AC 4 / design §A4: navigation groups are labeled and
    /// excluded from authority topology.
    pub fn build_navigation_facet(
        facet_label: &str,
        member_count: u32,
        policy: EffectivePolicySummary,
        graph_revision: GraphRevision,
    ) -> ProjectedNode {
        use crate::memory::model::RecordId;

        // Generate a stable synthetic aggregate ID (not a display label)
        let id = ProjectedItemId::Record(RecordId::new_v7());

        ProjectedNode {
            id,
            node_kind: ProjectedNodeKind::Aggregate,
            authority_class: EdgeAuthorityClass::Navigation,
            graph_revision,
            effective_policy: policy,
            truth_state: crate::memory::graph::projection::TruthStateSummary::bare(
                TruthState::Current,
            ),
            valid_time: None,
            provenance: ProvenanceSummary {
                source_kind: None,
                actor_id: None,
                method: Some("navigation_facet".into()),
                method_version: None,
                created_at: None,
            },
            // facet_label is a human-readable group name, NEVER a raw UUID.
            display_name: Some(facet_label.to_owned()),
            metadata: NodeMetadata::Aggregate {
                aggregate_kind: ProjectedNodeKind::Entity,
                authorized_count: Some(member_count),
                truncation_reason: TruncationReason::ItemLimit,
            },
            authorized_actions: vec![],
        }
    }
}

// ── Private helpers ───────────────────────────────────────────────────────

/// Convert a [`RawEntityRow`] to a [`ProjectedNode`] using the given revision.
fn entity_row_to_node(row: &RawEntityRow, graph_revision: GraphRevision) -> ProjectedNode {
    let valid_time = if row.valid_from.is_some() || row.valid_until.is_some() {
        Some(ProjectedValidTime {
            valid_from: row.valid_from.clone(),
            valid_until: row.valid_until.clone(),
            timezone_offset_min: None,
        })
    } else {
        None
    };

    let truth_state = match &row.staleness_class {
        Some(sc) => crate::memory::graph::projection::TruthStateSummary::with_staleness(
            row.truth_state.clone(),
            sc.clone(),
        ),
        None => crate::memory::graph::projection::TruthStateSummary::bare(row.truth_state.clone()),
    };

    ProjectedNode {
        id: ProjectedItemId::Entity(row.id.clone()),
        node_kind: ProjectedNodeKind::Entity,
        authority_class: EdgeAuthorityClass::Stored,
        graph_revision,
        effective_policy: EffectivePolicySummary {
            namespace: row.namespace.clone(),
            scope: row.scope.clone(),
            sensitivity: row.sensitivity,
            policy_version: row.policy_version.clone(),
        },
        truth_state,
        valid_time,
        provenance: ProvenanceSummary {
            source_kind: row.source_kind.clone(),
            actor_id: row.actor_id.clone(),
            method: None,
            method_version: None,
            created_at: row.created_at.clone(),
        },
        display_name: row.display_name.clone(),
        metadata: NodeMetadata::Entity {
            entity_type: row.entity_type.clone(),
            is_canonical: true,
        },
        authorized_actions: vec![],
    }
}

/// Convert a [`RawEdgeRow`] to a [`ProjectedEdge`].  Both endpoints must be
/// visible — callers MUST NOT call this when `source_visible=false` or
/// `target_visible=false`.
fn edge_row_to_edge(row: &RawEdgeRow, graph_revision: GraphRevision) -> ProjectedEdge {
    use crate::memory::graph::projection::{EndpointSummary, TruthStateSummary};

    let valid_time = if row.valid_from.is_some() || row.valid_until.is_some() {
        Some(ProjectedValidTime {
            valid_from: row.valid_from.clone(),
            valid_until: row.valid_until.clone(),
            timezone_offset_min: None,
        })
    } else {
        None
    };

    let truth_state = match &row.staleness_class {
        Some(sc) => TruthStateSummary::with_staleness(row.truth_state.clone(), sc.clone()),
        None => TruthStateSummary::bare(row.truth_state.clone()),
    };

    ProjectedEdge {
        id: row.identity_hash.clone(),
        link_type: row.link_type.clone(),
        link_type_version: row.link_type_version,
        authority_class: row.authority_class,
        direction: row.direction,
        source_endpoint: EndpointSummary {
            id: row.source_entity_id.clone(),
            entity_type: None,
            display_name: None,
            truth_state: row.truth_state.clone(),
        },
        target_endpoint: EndpointSummary {
            id: row.target_entity_id.clone(),
            entity_type: None,
            display_name: None,
            truth_state: row.truth_state.clone(),
        },
        truth_state,
        valid_time,
        provenance: ProvenanceSummary {
            source_kind: row.source_kind.clone(),
            actor_id: row.actor_id.clone(),
            method: None,
            method_version: None,
            created_at: row.created_at.clone(),
        },
        graph_revision,
        effective_policy: EffectivePolicySummary {
            namespace: row.namespace.clone(),
            scope: row.scope.clone(),
            sensitivity: row.sensitivity,
            policy_version: row.policy_version.clone(),
        },
        evidence_count: row.evidence_count,
        authorized_actions: vec![],
    }
}

/// Build a [`ProjectedNode`] representing a hidden (policy-filtered) endpoint
/// as a [`ProjectedNodeKind::Aggregate`] / [`EdgeAuthorityClass::Navigation`]
/// frontier node.
///
/// Design §A4: hidden IDs are NEVER exposed; the node only reveals that a
/// policy-hidden entity exists via `TruncationReason::PolicyFiltered`.
fn hidden_endpoint_frontier(
    graph_revision: GraphRevision,
    policy: EffectivePolicySummary,
) -> ProjectedNode {
    use crate::memory::model::RecordId;

    ProjectedNode {
        id: ProjectedItemId::Record(RecordId::new_v7()),
        node_kind: ProjectedNodeKind::Aggregate,
        authority_class: EdgeAuthorityClass::Navigation,
        graph_revision,
        effective_policy: policy,
        truth_state: crate::memory::graph::projection::TruthStateSummary::bare(TruthState::Current),
        valid_time: None,
        provenance: ProvenanceSummary {
            source_kind: None,
            actor_id: None,
            method: None,
            method_version: None,
            created_at: None,
        },
        display_name: None,
        metadata: NodeMetadata::Aggregate {
            aggregate_kind: ProjectedNodeKind::Entity,
            authorized_count: None,
            truncation_reason: TruncationReason::PolicyFiltered,
        },
        authorized_actions: vec![],
    }
}

/// Extract entity_type string from node metadata for sorting.
fn extract_entity_type(metadata: &NodeMetadata) -> Option<&str> {
    match metadata {
        NodeMetadata::Entity { entity_type, .. } => entity_type.as_deref(),
        _ => None,
    }
}

/// Compute a deterministic query hash for the validated request.
///
/// This is a best-effort stable string for caching/deduplication; it does not
/// need to be cryptographically strong in this layer.
fn compute_query_hash(validated: &ValidatedRequest) -> String {
    // Stable representation: seeds sorted + key params
    let mut seed_strs: Vec<&str> = validated.seeds.iter().map(|s| s.as_str()).collect();
    seed_strs.sort_unstable();

    let expand_flags = validated
        .expand_children
        .as_ref()
        .map_or("none".to_owned(), |e| {
            format!(
                "m{}e{}s{}",
                e.include_memory as u8, e.include_evidence as u8, e.include_source as u8
            )
        });
    let rev = validated
        .at_revision
        .map_or("current".to_owned(), |r| r.get().to_string());

    format!(
        "seeds=[{}]:expand={}:maxn={}:maxe={}:rev={}:ns={}:sc={}",
        seed_strs.join(","),
        expand_flags,
        validated.max_nodes,
        validated.max_edges,
        rev,
        validated.policy_scope.namespace,
        validated.policy_scope.scope,
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::graph::projection::{
        EdgeAuthorityClass, ProjectedNodeKind, TruncationReason,
    };
    use crate::memory::model::{EntityId, GraphRevision, TruthState};
    use crate::memory::types::StalenessClass;

    fn sample_policy() -> EffectivePolicySummary {
        EffectivePolicySummary {
            namespace: "user".into(),
            scope: "chat".into(),
            sensitivity: 1,
            policy_version: "a".repeat(64),
        }
    }

    fn sample_request() -> GraphQueryRequest {
        GraphQueryRequest {
            seeds: vec![],
            expand_children: None,
            max_nodes: None,
            max_edges: None,
            policy_scope: sample_policy(),
            at_revision: None,
            cursor: None,
            deadline_ms: None,
        }
    }

    fn make_entity_row(
        id: EntityId,
        entity_type: Option<&str>,
        display_name: Option<&str>,
    ) -> RawEntityRow {
        RawEntityRow {
            id,
            entity_type: entity_type.map(str::to_owned),
            display_name: display_name.map(str::to_owned),
            truth_state: TruthState::Current,
            valid_from: None,
            valid_until: None,
            sensitivity: 1,
            namespace: "user".into(),
            scope: "chat".into(),
            policy_version: "a".repeat(64),
            created_at: None,
            source_kind: None,
            actor_id: None,
            staleness_class: None,
            revision: GraphRevision::new(1),
        }
    }

    fn make_edge_row(
        src: EntityId,
        tgt: EntityId,
        source_visible: bool,
        target_visible: bool,
    ) -> RawEdgeRow {
        RawEdgeRow {
            identity_hash: "b".repeat(64),
            link_type: "derived_from".into(),
            link_type_version: 1,
            authority_class: EdgeAuthorityClass::Stored,
            direction: DirectionClass::Directed,
            source_entity_id: src,
            target_entity_id: tgt,
            source_visible,
            target_visible,
            truth_state: TruthState::Current,
            valid_from: None,
            valid_until: None,
            sensitivity: 1,
            namespace: "user".into(),
            scope: "chat".into(),
            policy_version: "a".repeat(64),
            evidence_count: 0,
            created_at: None,
            source_kind: None,
            actor_id: None,
            staleness_class: None,
            revision: GraphRevision::new(1),
        }
    }

    // ── validate_request ────────────────────────────────────────────────

    #[test]
    fn validate_defaults_fill_in_when_none() {
        let req = sample_request();
        let v = GraphQueryProjector::validate_request(&req).unwrap();
        assert_eq!(v.max_nodes, ProjectionLimits::DEFAULT_MAX_NODES);
        assert_eq!(v.max_edges, ProjectionLimits::DEFAULT_MAX_EDGES);
        assert_eq!(v.deadline_ms, ProjectionLimits::DEFAULT_DEADLINE_MS);
        assert_eq!(
            v.max_children_per_entity,
            ProjectionLimits::DEFAULT_CHILDREN_PER_ENTITY
        );
    }

    #[test]
    fn validate_accepts_values_at_hard_cap() {
        let mut req = sample_request();
        req.max_nodes = Some(ProjectionLimits::MAX_NODES);
        req.max_edges = Some(ProjectionLimits::MAX_EDGES);
        req.deadline_ms = Some(ProjectionLimits::MAX_DEADLINE_MS);
        let v = GraphQueryProjector::validate_request(&req).unwrap();
        assert_eq!(v.max_nodes, ProjectionLimits::MAX_NODES);
        assert_eq!(v.max_edges, ProjectionLimits::MAX_EDGES);
        assert_eq!(v.deadline_ms, ProjectionLimits::MAX_DEADLINE_MS);
    }

    #[test]
    fn validate_rejects_max_nodes_over_cap() {
        let mut req = sample_request();
        req.max_nodes = Some(ProjectionLimits::MAX_NODES + 1);
        let err = GraphQueryProjector::validate_request(&req).unwrap_err();
        assert!(matches!(err, ProjectionError::InvalidMaxNodes { .. }));
    }

    #[test]
    fn validate_rejects_max_edges_over_cap() {
        let mut req = sample_request();
        req.max_edges = Some(ProjectionLimits::MAX_EDGES + 1);
        let err = GraphQueryProjector::validate_request(&req).unwrap_err();
        assert!(matches!(err, ProjectionError::InvalidMaxEdges { .. }));
    }

    #[test]
    fn validate_rejects_deadline_over_cap() {
        let mut req = sample_request();
        req.deadline_ms = Some(ProjectionLimits::MAX_DEADLINE_MS + 1);
        let err = GraphQueryProjector::validate_request(&req).unwrap_err();
        assert!(matches!(err, ProjectionError::InvalidDeadline { .. }));
    }

    #[test]
    fn validate_rejects_too_many_seeds() {
        let mut req = sample_request();
        req.seeds = (0..=MAX_SEED_COUNT as usize)
            .map(|_| EntityId::new_v7())
            .collect();
        let err = GraphQueryProjector::validate_request(&req).unwrap_err();
        assert!(matches!(err, ProjectionError::InvalidSeedCount { .. }));
    }

    #[test]
    fn validate_clamps_children_per_entity() {
        let mut req = sample_request();
        req.expand_children = Some(ExpandChildrenRequest {
            include_memory: true,
            include_evidence: false,
            include_source: false,
            max_children_per_entity: Some(ProjectionLimits::MAX_CHILDREN_PER_ENTITY + 10),
        });
        let v = GraphQueryProjector::validate_request(&req).unwrap();
        assert_eq!(
            v.max_children_per_entity,
            ProjectionLimits::MAX_CHILDREN_PER_ENTITY
        );
    }

    // ── Entity-primary default ───────────────────────────────────────────

    #[test]
    fn entity_primary_mode_excludes_non_entity_nodes() {
        let req = sample_request(); // expand_children = None
        let validated = GraphQueryProjector::validate_request(&req).unwrap();
        let rev = GraphRevision::new(5);

        let entities = vec![make_entity_row(
            EntityId::new_v7(),
            Some("person"),
            Some("Alice"),
        )];
        // No edge rows; just verify entity nodes survive
        let resp = GraphQueryProjector::build_entity_primary(&validated, entities, vec![], rev);

        // All nodes must be Entity or Aggregate
        for node in &resp.nodes {
            assert!(
                matches!(
                    node.node_kind,
                    ProjectedNodeKind::Entity | ProjectedNodeKind::Aggregate
                ),
                "unexpected node kind in entity-primary mode: {:?}",
                node.node_kind
            );
        }
    }

    #[test]
    fn entity_primary_one_entity_in_response() {
        let req = sample_request();
        let validated = GraphQueryProjector::validate_request(&req).unwrap();
        let rev = GraphRevision::new(3);
        let eid = EntityId::new_v7();
        let entities = vec![make_entity_row(eid.clone(), Some("project"), Some("KRIA"))];
        let resp = GraphQueryProjector::build_entity_primary(&validated, entities, vec![], rev);

        assert_eq!(resp.nodes.len(), 1);
        assert_eq!(resp.nodes[0].node_kind, ProjectedNodeKind::Entity);
        assert_eq!(resp.nodes[0].display_name.as_deref(), Some("KRIA"));
    }

    // ── Navigation facet ────────────────────────────────────────────────

    #[test]
    fn navigation_facet_has_correct_authority_class() {
        let rev = GraphRevision::new(10);
        let facet =
            GraphQueryProjector::build_navigation_facet("Related People", 5, sample_policy(), rev);
        assert_eq!(facet.node_kind, ProjectedNodeKind::Aggregate);
        assert_eq!(facet.authority_class, EdgeAuthorityClass::Navigation);
        assert_eq!(facet.display_name.as_deref(), Some("Related People"));
        // Must not look like a UUID
        assert!(
            !facet.display_name.as_deref().unwrap_or("").contains('-')
                || facet.display_name.as_deref().unwrap_or("") == "Related People",
            "display_name must not be a raw UUID"
        );
    }

    #[test]
    fn navigation_facet_metadata_aggregate() {
        let rev = GraphRevision::new(1);
        let facet = GraphQueryProjector::build_navigation_facet("By Type", 3, sample_policy(), rev);
        match &facet.metadata {
            NodeMetadata::Aggregate {
                aggregate_kind,
                authorized_count,
                ..
            } => {
                assert_eq!(*aggregate_kind, ProjectedNodeKind::Entity);
                assert_eq!(*authorized_count, Some(3));
            }
            other => panic!("expected Aggregate metadata, got {:?}", other),
        }
    }

    // ── Hidden endpoint → FrontierAggregate ─────────────────────────────

    #[test]
    fn hidden_source_endpoint_omits_edge_adds_frontier_node() {
        let req = sample_request();
        let validated = GraphQueryProjector::validate_request(&req).unwrap();
        let rev = GraphRevision::new(7);

        let src = EntityId::new_v7();
        let tgt = EntityId::new_v7();
        let entities = vec![
            make_entity_row(src.clone(), Some("person"), Some("Alice")),
            make_entity_row(tgt.clone(), Some("project"), Some("KRIA")),
        ];
        // source_visible = false → edge omitted, frontier node added
        let edges = vec![make_edge_row(src, tgt, false, true)];

        let resp = GraphQueryProjector::build_entity_primary(&validated, entities, edges, rev);

        // No edges in response
        assert!(
            resp.edges.is_empty(),
            "hidden endpoint edge must be omitted"
        );

        // A frontier Aggregate node must exist
        let aggregate_nodes: Vec<_> = resp
            .nodes
            .iter()
            .filter(|n| n.node_kind == ProjectedNodeKind::Aggregate)
            .collect();
        assert!(
            !aggregate_nodes.is_empty(),
            "expected a FrontierAggregate node"
        );

        let frontier = &aggregate_nodes[0];
        assert_eq!(frontier.authority_class, EdgeAuthorityClass::Navigation);
        match &frontier.metadata {
            NodeMetadata::Aggregate {
                truncation_reason,
                authorized_count,
                ..
            } => {
                assert_eq!(*truncation_reason, TruncationReason::PolicyFiltered);
                assert!(authorized_count.is_none(), "frontier count must be None");
            }
            other => panic!("expected Aggregate metadata, got {:?}", other),
        }
    }

    #[test]
    fn hidden_target_endpoint_omits_edge_adds_frontier_node() {
        let req = sample_request();
        let validated = GraphQueryProjector::validate_request(&req).unwrap();
        let rev = GraphRevision::new(8);

        let src = EntityId::new_v7();
        let tgt = EntityId::new_v7();
        let entities = vec![make_entity_row(src.clone(), None, None)];
        let edges = vec![make_edge_row(src, tgt, true, false)]; // target hidden

        let resp = GraphQueryProjector::build_entity_primary(&validated, entities, edges, rev);

        assert!(resp.edges.is_empty());
        let agg_nodes: Vec<_> = resp
            .nodes
            .iter()
            .filter(|n| n.node_kind == ProjectedNodeKind::Aggregate)
            .collect();
        assert!(!agg_nodes.is_empty());
    }

    #[test]
    fn visible_endpoints_produce_projected_edge() {
        let req = sample_request();
        let validated = GraphQueryProjector::validate_request(&req).unwrap();
        let rev = GraphRevision::new(2);

        let src = EntityId::new_v7();
        let tgt = EntityId::new_v7();
        let entities = vec![
            make_entity_row(src.clone(), Some("tool"), None),
            make_entity_row(tgt.clone(), Some("memory"), None),
        ];
        let edges = vec![make_edge_row(src, tgt, true, true)];

        let resp = GraphQueryProjector::build_entity_primary(&validated, entities, edges, rev);

        assert_eq!(resp.edges.len(), 1);
        assert!(resp.edges[0].id.len() > 0);
    }

    // ── Limits and truncation ────────────────────────────────────────────

    #[test]
    fn node_limit_enforced_and_truncation_flagged() {
        let mut req = sample_request();
        req.max_nodes = Some(2);
        let validated = GraphQueryProjector::validate_request(&req).unwrap();
        let rev = GraphRevision::new(1);

        let entities: Vec<_> = (0..5)
            .map(|i| {
                make_entity_row(
                    EntityId::new_v7(),
                    Some("person"),
                    Some(&format!("Person {i}")),
                )
            })
            .collect();

        let resp = GraphQueryProjector::build_entity_primary(&validated, entities, vec![], rev);

        assert!(resp.nodes.len() <= 2);
        assert!(resp.truncated);
        assert_eq!(resp.truncation_reason, Some(TruncationReason::ItemLimit));
    }

    #[test]
    fn edge_limit_enforced_and_truncation_flagged() {
        let mut req = sample_request();
        req.max_edges = Some(1);
        let validated = GraphQueryProjector::validate_request(&req).unwrap();
        let rev = GraphRevision::new(1);

        let src = EntityId::new_v7();
        let entities = vec![make_entity_row(src.clone(), Some("tool"), None)];

        // Two edges both visible; only 1 should make it through
        let tgt1 = EntityId::new_v7();
        let tgt2 = EntityId::new_v7();
        let mut e1 = make_edge_row(src.clone(), tgt1, true, true);
        e1.identity_hash = "a".repeat(64);
        let mut e2 = make_edge_row(src, tgt2, true, true);
        e2.identity_hash = "b".repeat(64);

        let resp =
            GraphQueryProjector::build_entity_primary(&validated, entities, vec![e1, e2], rev);

        assert!(resp.edges.len() <= 1);
        assert!(resp.truncated);
    }

    // ── Consistent graph_revision ────────────────────────────────────────

    #[test]
    fn all_nodes_share_same_graph_revision() {
        let req = sample_request();
        let validated = GraphQueryProjector::validate_request(&req).unwrap();
        let rev = GraphRevision::new(42);

        let entities: Vec<_> = (0..3)
            .map(|_| make_entity_row(EntityId::new_v7(), Some("person"), None))
            .collect();

        let resp = GraphQueryProjector::build_entity_primary(&validated, entities, vec![], rev);

        for node in &resp.nodes {
            assert_eq!(node.graph_revision, rev, "node revision mismatch");
        }
        assert_eq!(resp.graph_revision, rev);
    }

    #[test]
    fn all_edges_share_same_graph_revision() {
        let req = sample_request();
        let validated = GraphQueryProjector::validate_request(&req).unwrap();
        let rev = GraphRevision::new(99);

        let src = EntityId::new_v7();
        let tgt = EntityId::new_v7();
        let entities = vec![
            make_entity_row(src.clone(), None, None),
            make_entity_row(tgt.clone(), None, None),
        ];
        let edges = vec![make_edge_row(src, tgt, true, true)];

        let resp = GraphQueryProjector::build_entity_primary(&validated, entities, edges, rev);

        for edge in &resp.edges {
            assert_eq!(edge.graph_revision, rev, "edge revision mismatch");
        }
    }

    // ── Response invariants ──────────────────────────────────────────────

    #[test]
    fn response_is_not_truncated_when_within_limits() {
        let req = sample_request();
        let validated = GraphQueryProjector::validate_request(&req).unwrap();
        let rev = GraphRevision::new(1);
        let entities = vec![make_entity_row(
            EntityId::new_v7(),
            Some("person"),
            Some("Bob"),
        )];
        let resp = GraphQueryProjector::build_entity_primary(&validated, entities, vec![], rev);
        assert!(!resp.truncated);
        assert!(resp.truncation_reason.is_none());
    }

    #[test]
    fn empty_input_produces_empty_valid_response() {
        let req = sample_request();
        let validated = GraphQueryProjector::validate_request(&req).unwrap();
        let rev = GraphRevision::new(0);
        let resp = GraphQueryProjector::build_entity_primary(&validated, vec![], vec![], rev);
        assert!(resp.nodes.is_empty());
        assert!(resp.edges.is_empty());
        assert!(!resp.truncated);
        assert_eq!(resp.graph_revision, rev);
        assert_eq!(resp.schema_version, 2);
    }

    // ── expand_children empty = entity-primary ───────────────────────────

    #[test]
    fn expand_children_all_false_is_entity_primary() {
        let mut req = sample_request();
        req.expand_children = Some(ExpandChildrenRequest {
            include_memory: false,
            include_evidence: false,
            include_source: false,
            max_children_per_entity: None,
        });
        let validated = GraphQueryProjector::validate_request(&req).unwrap();
        let rev = GraphRevision::new(1);
        let entities = vec![make_entity_row(
            EntityId::new_v7(),
            Some("person"),
            Some("Carol"),
        )];
        let resp = GraphQueryProjector::build_entity_primary(&validated, entities, vec![], rev);
        // All nodes must be Entity or Aggregate (entity-primary enforced)
        for node in &resp.nodes {
            assert!(matches!(
                node.node_kind,
                ProjectedNodeKind::Entity | ProjectedNodeKind::Aggregate
            ));
        }
    }

    // ── Deduplication of frontier nodes ─────────────────────────────────

    #[test]
    fn same_hidden_endpoint_produces_one_frontier_node() {
        let req = sample_request();
        let validated = GraphQueryProjector::validate_request(&req).unwrap();
        let rev = GraphRevision::new(1);

        let src = EntityId::new_v7();
        let tgt1 = EntityId::new_v7();
        let tgt2 = EntityId::new_v7();
        // Both edges from same hidden source → only one frontier node for src
        let mut e1 = make_edge_row(src.clone(), tgt1, false, true);
        e1.identity_hash = "a".repeat(64);
        let mut e2 = make_edge_row(src.clone(), tgt2, false, true);
        e2.identity_hash = "b".repeat(64);

        let entities = vec![];
        let resp =
            GraphQueryProjector::build_entity_primary(&validated, entities, vec![e1, e2], rev);

        // Should be exactly 1 frontier node (deduplicated on src entity id)
        let agg: Vec<_> = resp
            .nodes
            .iter()
            .filter(|n| n.node_kind == ProjectedNodeKind::Aggregate)
            .collect();
        assert_eq!(
            agg.len(),
            1,
            "expected exactly 1 deduplicated frontier node, got {}",
            agg.len()
        );
    }
}
